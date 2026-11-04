import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from scripts.agent_bridge.codex import CodexSink, acknowledge, delivery_id, private_directory
from scripts.agent_bridge.pump import EventPump
from scripts.agent_bridge.mcp import MCP
from scripts.agent_bridge.codex_exec import CodexExecSink

THREAD = '01a17ad9-224c-7db2-907b-79d3f694b4fc'
ACCOUNT = {'platform': 'wechat', 'account_id': 'test'}


def event(identifier='event-1', recipient='示例联系人甲'):
    return {'event_id': identifier, 'kind': 'interface_change', 'source': 'desktop_ocr', 'verified': False,
            'payload': {'conversation': {'display_name': recipient, 'preview': 'Agent推送测试1104'},
                        'incoming_verified': False}, 'state': 'leased', 'consumer': 'fixture',
            'lease_until': 999999, 'lease_token': 'secret-token'}


class Client:
    account = ACCOUNT

    def __init__(self, events=()):
        self.events, self.calls = events, []

    async def tool(self, name, **args):
        self.calls.append((name, args))
        return {'events': copy.deepcopy(self.events)}


class BridgeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        fixture = self.root / 'queue.py'
        fixture.write_text("import sys\nfrom pathlib import Path\np=Path(sys.argv[1])\np.write_text(p.read_text()+'1' if p.exists() else '1')\nprint('queued')\n")
        self.log = self.root / 'queue-calls'
        self.sink = CodexSink(self.root / 'spool', THREAD, (sys.executable, str(fixture), str(self.log)))

    def tearDown(self):
        self.temp.cleanup()

    async def test_queue_is_not_agent_consumption(self):
        client = Client()
        result = await EventPump(client, self.sink, 'fixture').deliver(event(), receipt_timeout=0.3)
        self.assertEqual(result['status'], 'awaiting_agent_receipt')
        self.assertNotIn('ack_event', [name for name, _ in client.calls])
        saved = json.loads(next((self.root / 'spool').glob('*.event.json')).read_text())
        self.assertNotIn('lease_token', saved['event'])
        self.assertFalse(saved['event']['verified'])
        self.assertEqual(saved['event']['kind'], 'interface_change')

    async def test_acknowledges_only_actual_receipt_with_original_lease(self):
        source = event()
        identifier = await self.sink.notify(ACCOUNT, source)
        acknowledge(self.sink.spool, identifier, THREAD)
        client = Client()
        result = await EventPump(client, self.sink, 'fixture').deliver(source)
        self.assertEqual(result['status'], 'acknowledged')
        self.assertEqual(client.calls, [('ack_event', {'event_id': 'event-1', 'consumer': 'fixture', 'lease_token': 'secret-token'})])

    async def test_restart_keeps_delivery_identity_and_does_not_requeue(self):
        source = event()
        first = await self.sink.notify(ACCOUNT, source)
        source.update(lease_token='new-token', consumer='another-worker', lease_until=1000000)
        restarted = CodexSink(self.sink.spool, THREAD, self.sink.command)
        self.assertEqual(await restarted.notify(ACCOUNT, source), first)
        self.assertEqual(self.log.read_text(), '1')

    async def test_conflicting_payload_cannot_reuse_event_identity(self):
        await self.sink.notify(ACCOUNT, event())
        changed = event()
        changed['payload']['conversation']['preview'] = 'changed'
        with self.assertRaisesRegex(RuntimeError, 'conflicting'):
            await self.sink.notify(ACCOUNT, changed)

    async def test_notification_interruption_is_not_blindly_resubmitted(self):
        identifier = await self.sink.notify(ACCOUNT, event())
        (self.sink.spool / (identifier + '.notification.json')).write_text('{"state":"submitting"}')
        await self.sink.notify(ACCOUNT, event())
        self.assertEqual(self.log.read_text(), '1')

    async def test_filter_releases_unrelated_events_without_acknowledging_them(self):
        target = event()
        target['payload']['conversation']['preview'] = 'Agent 推送测试 1104'
        client = Client([event('other', '其他人'), target])
        selected = await EventPump(client, self.sink, 'fixture').claim('示例联系人甲', 'Agent推送测试1104')
        self.assertEqual(selected['event_id'], 'event-1')
        self.assertEqual(client.calls[1][0], 'release_event')
        self.assertEqual(client.calls[1][1]['event_id'], 'other')

    async def test_lease_renewed_while_waiting_for_agent(self):
        client = Client()
        pump = EventPump(client, self.sink, 'fixture', lease_seconds=5)
        result = await pump.deliver(event(), receipt_timeout=1.9)
        self.assertEqual(result['status'], 'awaiting_agent_receipt')
        self.assertIn('renew_event_lease', [name for name, _ in client.calls])
        self.assertNotIn('ack_event', [name for name, _ in client.calls])

    async def test_receipt_cannot_cross_threads_or_deliveries(self):
        identifier = await self.sink.notify(ACCOUNT, event())
        with self.assertRaises(ValueError):
            acknowledge(self.sink.spool, identifier, '00000000-0000-0000-0000-000000000001')
        with self.assertRaises(ValueError):
            acknowledge(self.sink.spool, '../escape', THREAD)
        self.assertNotEqual(delivery_id(ACCOUNT, 'a'), delivery_id({**ACCOUNT, 'account_id': 'other'}, 'a'))

    async def test_lost_lease_never_acknowledges(self):
        class LostLease(Client):
            async def tool(self, name, **args):
                self.calls.append((name, args))
                if name == 'renew_event_lease':
                    raise RuntimeError('stale lease')
                return {}
        client = LostLease()
        with self.assertRaisesRegex(RuntimeError, 'stale lease'):
            await EventPump(client, self.sink, 'fixture', lease_seconds=5).deliver(event(), receipt_timeout=3)
        self.assertNotIn('ack_event', [name for name, _ in client.calls])

    async def test_notification_failure_keeps_event_unacknowledged(self):
        sink = CodexSink(self.root / 'failure-spool', THREAD, (sys.executable, '-c', 'raise SystemExit(1)'))
        client = Client()
        with self.assertRaisesRegex(RuntimeError, 'outcome unknown'):
            await EventPump(client, sink, 'fixture').deliver(event())
        self.assertNotIn('ack_event', [name for name, _ in client.calls])

    async def test_transport_rejects_cross_account_result(self):
        client = MCP([], ACCOUNT)
        async def response(*_):
            return {'structuredContent': {'account': {**ACCOUNT, 'account_id': 'other'}, 'data': {}}}
        client.request = response
        with self.assertRaisesRegex(RuntimeError, 'account mismatch'):
            await client.tool('claim_events')

    def test_unsafe_spool_rejected(self):
        unsafe = self.root / 'public'
        unsafe.mkdir(mode=0o755)
        with self.assertRaises(ValueError):
            private_directory(unsafe)

    async def test_exec_receiver_validates_ack_and_does_not_execute_twice(self):
        fixture = self.root / 'exec.py'
        fixture.write_text('''import json,sys
from pathlib import Path
event=json.loads(sys.stdin.read().split('\\n',1)[1])
output=Path(sys.argv[sys.argv.index('--output-last-message')+1])
output.write_text(json.dumps({'delivery_id':event['delivery_id'],'accepted':True,
 'event_kind':event['event']['kind'],'incoming_verified':False}))
''')
        sink = CodexExecSink(self.root / 'exec-spool', (sys.executable, str(fixture)))
        identifier = await sink.notify(ACCOUNT, event())
        self.assertTrue(sink.receipt(identifier))
        fixture.write_text('raise SystemExit(1)')
        self.assertEqual(await sink.notify(ACCOUNT, event()), identifier)
        changed = event()
        changed['payload']['conversation']['preview'] = 'different'
        with self.assertRaisesRegex(RuntimeError, 'conflicting'):
            await sink.notify(ACCOUNT, changed)

    async def test_failed_exec_requires_explicit_review_before_retry(self):
        sink = CodexExecSink(self.root / 'exec-fail-spool', (sys.executable, '-c', 'raise SystemExit(1)'))
        with self.assertRaisesRegex(RuntimeError, 'not automatically retried'):
            await sink.notify(ACCOUNT, event())
        with self.assertRaisesRegex(RuntimeError, 'outcome unknown'):
            await sink.notify(ACCOUNT, event())

    async def test_exec_receiver_does_not_promote_observation_to_verified_incoming(self):
        fixture = self.root / 'wrong-exec.py'
        fixture.write_text('''import json,sys
from pathlib import Path
event=json.loads(sys.stdin.read().split('\\n',1)[1])
Path(sys.argv[sys.argv.index('--output-last-message')+1]).write_text(json.dumps({
 'delivery_id':event['delivery_id'],'accepted':True,'event_kind':event['event']['kind'],'incoming_verified':True}))
''')
        sink = CodexExecSink(self.root / 'wrong-exec-spool', (sys.executable, str(fixture)))
        with self.assertRaisesRegex(RuntimeError, 'does not match'):
            await sink.notify(ACCOUNT, event())
        self.assertFalse(sink.receipt(delivery_id(ACCOUNT, 'event-1')))


if __name__ == '__main__':
    unittest.main()
