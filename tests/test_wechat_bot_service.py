import copy
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from scripts.wechat_bot.service import BotService, load_config
from scripts.wechat_bot.policy import echo_matches
from scripts.wechat_bot import control


def bubble(text,direction='inbound_visual'):
    return {'text':text,'digest':text,'direction':direction}


def frame(*bubbles,title='午夜咖啡'):
    return {'profile':'wechat-4-dark','scale':1,'title_lines':[title],'bubbles':list(bubbles),'clipped_bubble':False}


class ServiceTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.root=Path(self.temp.name)
        self.config={'schema_version':1,'host':'fixture','account_id':'fixture',
            'allowlist':[{'recipient':'午夜咖啡','kind':'private'}],'state_dir':str(self.root/'state'),
            'poll_seconds':2,'cooldown_seconds':0,'max_replies_per_hour':20}
        self.service=BotService(self.config)
        self.old=frame(bubble('old'))
        self.new=frame(bubble('old'),bubble('你好，介绍一下你自己'))
        self.calls=[]
        calls=self.calls
        class Client:
            async def tool(self,name,**args):
                calls.append((name,args))
                return {'status':'triggered','send_id':'fixture-send','delivery':'unknown'}
        self.client=Client()
        self.model=AsyncMock(return_value='你好，我是AI客服助手。')

    def tearDown(self): self.temp.cleanup()

    def restart(self):
        extra={'history':self.history,'contact_id':self.cid} if hasattr(self,'history') else {}
        return BotService(self.config,**extra)

    async def cycle(self,frames):
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy(frames))
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)

    async def test_legacy_cursor_rebaselines_without_reply(self):
        # Cursor saved before fingerprint v2: its image digest hashed pixels and has no phash, so the
        # image can never match again. Rebaseline once instead of replying to visible history.
        from scripts.wechat_bot.service import write_private
        legacy=frame(bubble('old'),{'text':'[图片]','digest':'pixels','direction':'inbound_visual','kind':'image'})
        write_private(self.service.cursor_path,legacy)
        self.service=self.restart()
        current=frame(bubble('old'),{'text':'[图片]','digest':'image:50:40','phash':'0'*16,
                                     'direction':'inbound_visual','kind':'image'},bubble('新消息'))
        await self.cycle([current,current])
        self.model.assert_not_awaited()
        self.assertEqual(self.service.cursor['bubbles'],current['bubbles'])

    async def test_text_only_cursor_from_before_v2_still_replies(self):
        # Text digests did not change in v2: an old text-only cursor keeps working without a rebaseline.
        from scripts.wechat_bot.service import write_private
        write_private(self.service.cursor_path,self.old)
        self.service=self.restart()
        await self.cycle([self.new,self.new,self.new,self.new])
        self.model.assert_awaited()

    async def test_initial_start_does_not_reply_to_history(self):
        await self.cycle([self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])

    async def test_empty_baseline_never_turns_visible_history_into_new_messages(self):
        # Restart or theme switch captured an empty chat first (no chat open): the next frame's
        # bubbles are old history, not arrivals, so they become the baseline without a reply.
        self.service.advance(frame())
        await self.cycle([self.new,self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])
        self.assertEqual(len(self.service.cursor['bubbles']),len(self.new['bubbles']))

    async def test_tall_media_scrolling_history_off_after_send_does_not_pause(self):
        # After a sent reply, tall images/file cards can push every remembered bubble off screen.
        # The send already has a durable receipt; the gap is handled like any other: rebaseline
        # once two frames agree, never pause and never send again.
        self.service.advance(self.old)
        scrolled=frame(bubble('[图片]'),bubble('在的，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,self.new,scrolled])
        self.assertFalse(self.service.paused)
        await self.cycle([scrolled,scrolled])
        self.assertFalse(self.service.paused)
        self.assertEqual(len([1 for name,_ in self.calls if name=='send_message']),1)
        self.assertEqual(self.service.cursor['bubbles'],scrolled['bubbles'])

    async def test_ordinary_incoming_has_no_test_prefix_and_replies_once(self):
        self.service.advance(self.old)
        after=frame(*self.new['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,self.new,after])
        sent=[args for name,args in self.calls if name=='send_message']
        self.assertEqual(len(sent),1)
        self.assertEqual(sent[0]['recipient'],'午夜咖啡')
        self.assertEqual(self.model.await_args.kwargs['mode'],'service')
        await self.cycle([after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_foreign_contact_never_calls_model(self):
        self.service.advance(self.old)
        with self.assertRaises(ValueError):
            await self.cycle([frame(bubble('你好'),title='其他人')])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])

    async def test_identical_repeated_message_is_skipped_without_pause(self):
        self.service.advance(frame(bubble('刚才发链接看了吗')))
        current=frame(bubble('刚才发链接看了吗'),bubble('刚才发链接看了吗'))
        await self.cycle([current,current])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])
        self.assertFalse(self.service.paused)
        self.assertEqual(self.service.cursor,current)
        self.assertEqual(list(self.service.root.glob('send-*.json')),[])
        # The next distinct message is handled normally from the new baseline.
        later=frame(*current['bubbles'],bubble('在吗'))
        after=frame(*later['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([later,later,later,after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_ambiguous_frame_must_be_stable_before_skipping(self):
        old=frame(bubble('刚才发链接看了吗'))
        self.service.advance(old)
        current=frame(bubble('刚才发链接看了吗'),bubble('刚才发链接看了吗'))
        await self.cycle([current,frame(*current['bubbles'],bubble('在吗'))])
        self.assertEqual(self.service.cursor,old)
        self.model.assert_not_awaited()

    async def test_continuity_gap_rebaselines_without_reply_or_pause(self):
        self.service.advance(self.old)
        cleared=frame(bubble('你好 在吗'))
        await self.cycle([cleared,cleared])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])
        self.assertFalse(self.service.paused)
        self.assertEqual(self.service.cursor,cleared)
        # The next message after the new baseline is answered normally.
        later=frame(bubble('你好 在吗'),bubble('有人吗'))
        after=frame(*later['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([later,later,later,after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_unstable_gap_frame_keeps_old_cursor(self):
        self.service.advance(self.old)
        await self.cycle([frame(bubble('完全不同')),frame(bubble('完全不同'),bubble('又变了'))])
        self.assertEqual(self.service.cursor,self.old)
        self.model.assert_not_awaited()

    async def test_own_message_is_not_input(self):
        self.service.advance(self.old)
        await self.cycle([frame(*self.old['bubbles'],bubble('我的消息','outbound_visual'))])
        self.model.assert_not_awaited()

    async def test_incoming_during_send_remains_pending(self):
        self.service.advance(self.old)
        after=frame(*self.new['bubbles'],bubble('补充问题'),bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,self.new,after])
        self.assertEqual(self.service.cursor,self.new)

    async def test_new_activity_during_model_generation_suppresses_stale_send(self):
        self.service.advance(self.old)
        changed=frame(*self.new['bubbles'],bubble('新问题'))
        await self.cycle([self.new,self.new,changed])
        self.assertEqual(self.calls,[])
        record=json.loads(next(self.service.root.glob('send-*.json')).read_text())
        self.assertEqual(record['status'],'superseded')

    async def test_scrolled_echo_with_repeated_greeting_does_not_pause_or_resend(self):
        old=[bubble(x) for x in ['hi','hi','there','question','received']]
        self.service.advance(frame(*old))
        incoming=frame(*old,bubble('hi'))
        echo=bubble('你好，我是AI客服助手。','outbound_visual')
        after=frame(*incoming['bubbles'][1:],echo)
        await self.cycle([incoming,incoming,incoming,after])
        self.assertEqual(self.service.cursor,after)
        self.assertFalse(self.service.paused)
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)
        await self.cycle([after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_model_failure_pauses_and_survives_restart(self):
        self.service.advance(self.old)
        self.model.side_effect=RuntimeError('model unavailable')
        with self.assertRaises(RuntimeError):
            await self.cycle([self.new,self.new])
        self.assertTrue(self.service.paused)
        self.assertTrue(BotService(self.config).paused)
        self.assertEqual(self.calls,[])

    async def test_restart_accepts_cursor_saved_before_theme_profiles(self):
        self.service.advance(self.old)
        legacy=json.loads(self.service.cursor_path.read_text())
        legacy['profile']='wechat-4-dark-1x'; legacy.pop('scale',None)
        self.service.cursor_path.write_text(json.dumps(legacy))
        extra={'history':self.history,'contact_id':self.cid} if hasattr(self,'history') else {}
        restarted=BotService(self.config,**extra)
        self.assertEqual((restarted.cursor['profile'],restarted.cursor['scale']),('wechat-4-dark',1))

    async def test_stop_during_generation_never_sends_afterward(self):
        self.service.advance(self.old)
        async def stop_model(*_,**__):
            self.service.stop.set()
            return '你好，我是AI客服助手。'
        self.model.side_effect=stop_model
        await self.cycle([self.new,self.new])
        self.assertEqual(self.calls,[])
        record=json.loads(next(self.service.root.glob('send-*.json')).read_text())
        self.assertEqual(record['status'],'stopped_before_send')

    async def test_rate_limit_preserves_pending_input(self):
        self.service.advance(self.old)
        (self.service.root/'send-limit.json').write_text(json.dumps({'status':'triggered','attempted_at':time.time()}))
        self.service.config['max_replies_per_hour']=1
        await self.cycle([self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.service.cursor,self.old)

    def test_state_cannot_cross_accounts_or_contacts(self):
        with self.assertRaises(ValueError):
            BotService({**self.config,'account_id':'other'})

    def test_config_rejects_group_and_ambiguous_allowlist(self):
        path=self.root/'config.json'
        for contacts in [[],[{'recipient':'群','kind':'group'}],self.config['allowlist']*2]:
            path.write_text(json.dumps({**self.config,'allowlist':contacts}))
            with self.assertRaises(ValueError): load_config(path)

    def test_idle_check_seconds_defaults_and_bounds(self):
        path=self.root/'config.json'
        path.write_text(json.dumps(self.config))
        self.assertEqual(load_config(path)['idle_check_seconds'],60)
        for value in [5,'60',7200]:
            path.write_text(json.dumps({**self.config,'idle_check_seconds':value}))
            with self.assertRaises(ValueError): load_config(path)

    def test_echo_normalizes_width_but_not_changed_numbers(self):
        self.assertTrue(echo_matches('收到~','收到～'))
        self.assertFalse(echo_matches('金额100','金额100.00'))

    def test_health_logs_changes_without_poll_spam_or_chat_body(self):
        with self.assertLogs('wechat_bot',level='INFO') as logs:
            self.service.health('watching')
            self.service.health('watching')
            self.service.health('generating',image_count=1)
        self.assertEqual(len(logs.output),2)
        self.assertIn('generating',logs.output[1])
        self.assertNotIn('input',logs.output[1])

    def test_health_logs_periodic_heartbeat(self):
        with self.assertLogs('wechat_bot',level='INFO') as logs:
            self.service.health('watching')
            self.service.last_health_log_at-=61
            self.service.health('watching')
        self.assertEqual(len(logs.output),2)

    def test_default_config_is_in_user_home(self):
        with patch.object(Path,'home',return_value=self.root):
            base, config, plist=control.locations()
        self.assertEqual(config,self.root/'.WeChatRPABot/config.json')
        self.assertEqual(base,config.parent)

    def test_status_reads_configured_state_after_config_move(self):
        base=self.root/'.WeChatRPABot'; base.mkdir()
        config=base/'config.json'; config.write_text(json.dumps(self.config))
        health={'state':'watching','pid':123}
        (self.service.root/'health.json').write_text(json.dumps(health))
        output=io.StringIO()
        with patch.object(sys,'argv',['bot-control','status']), \
             patch.object(control,'locations',return_value=(base,config,self.root/'fixture.plist')), \
             patch.object(control,'loaded',return_value=True),contextlib.redirect_stdout(output):
            control.main()
        self.assertEqual(json.loads(output.getvalue())['health'],health)

    def test_stop_waits_until_launchd_task_is_unloaded(self):
        with patch.object(sys,'argv',['bot-control','stop']), \
             patch.object(control,'loaded',side_effect=[True,True,False,False]) as state, \
             patch.object(control.subprocess,'run') as run, patch.object(control.time,'sleep') as sleep, \
             contextlib.redirect_stdout(io.StringIO()):
            control.main()
        self.assertEqual(state.call_count,4)
        sleep.assert_called_once_with(0.1)
        self.assertEqual([call.args[0][1] for call in run.call_args_list],['disable','bootout'])

    def test_start_reenables_and_loads_unloaded_task(self):
        with patch.object(sys,'argv',['bot-control','start']), patch.object(control,'loaded',return_value=False), \
             patch.object(control,'load_config'), patch.object(control.subprocess,'run') as run, \
             contextlib.redirect_stdout(io.StringIO()):
            control.main()
        self.assertEqual([call.args[0][1] for call in run.call_args_list],['enable','bootstrap'])


if __name__=='__main__': unittest.main()


class HistoryServiceTests(ServiceTests):
    """Same fixtures as ServiceTests, with a real history store attached."""

    def setUp(self):
        super().setUp()
        import sqlite3
        from scripts.wechat_bot.history.store import HistoryStore
        self.sqlite3 = sqlite3
        self.history = HistoryStore.open(self.root/'h')
        self.cid = self.history.ensure_contact('fixture', self.config['allowlist'][0])
        self.config['state_dir'] = str(self.root/'state-history')
        self.service = BotService(self.config, history=self.history, contact_id=self.cid)

    def tearDown(self):
        self.history.conn.close()
        super().tearDown()

    async def test_model_failure_pauses_and_survives_restart(self):
        self.service.advance(self.old)
        self.model.side_effect = RuntimeError('model unavailable')
        with self.assertRaises(RuntimeError):
            await self.cycle([self.new, self.new])
        self.assertTrue(self.service.paused)
        self.assertTrue(BotService(self.config, history=self.history, contact_id=self.cid).paused)
        self.assertEqual(self.calls, [])

    def senders(self):
        return [r[0] for r in self.history.conn.execute('select sender from messages order by seq')]

    def texts(self):
        return [r[0] for r in self.history.conn.execute('select text from messages order by seq')]

    async def test_messages_recorded_before_model_call(self):
        self.service.advance(self.old)
        self.history.record_frame(self.cid, self.old)
        order = []
        real = self.history.record_frame
        self.history.record_frame = lambda *a, **k: (order.append('db'), real(*a, **k))[1]
        async def model(*a, **k):
            order.append('model')
            return '你好，我是AI客服助手。'
        self.model.side_effect = model
        after = frame(*self.new['bubbles'], bubble('你好，我是AI客服助手。', 'outbound_visual'))
        await self.cycle([self.new, self.new, self.new, after])
        self.assertEqual(order[:2], ['db', 'model'])
        self.assertEqual(self.senders(), ['customer', 'customer', 'bot'])
        send = self.history.send(json.loads(next(self.service.root.glob('send-*.json')).read_text())['idempotency_key'])
        self.assertEqual(send['status'], 'triggered')

    async def test_context_uses_stored_history_limit(self):
        self.config['context_messages'] = 2
        base = frame(*[bubble(str(i)) for i in range(5)])
        self.history.record_frame(self.cid, base)
        self.service.advance(base)
        new = frame(*base['bubbles'], bubble('新问题'))
        await self.cycle([new, new, new, new])
        ctx = self.model.await_args.kwargs['context']
        self.assertEqual([c['text'] for c in ctx], ['3', '4', '新问题'])

    async def test_history_failure_blocks_model_and_send(self):
        self.service.advance(self.old)
        self.history.record_frame = unittest.mock.Mock(side_effect=self.sqlite3.OperationalError('disk full'))
        await self.cycle([self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls, [])
        self.assertEqual(self.service.cursor, self.old)
        health = json.loads((self.service.root/'health.json').read_text())
        self.assertEqual(health['state'], 'history_write_failed')

    async def test_history_read_failure_keeps_message_pending(self):
        self.service.advance(self.old)
        self.history.recent_context = unittest.mock.Mock(side_effect=self.sqlite3.OperationalError('locked'))
        await self.cycle([self.new, self.new, self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.service.cursor, self.old)
        self.assertEqual(list(self.service.root.glob('send-*.json')), [])
        self.assertEqual(json.loads((self.service.root/'health.json').read_text())['state'], 'history_write_failed')

    async def test_baseline_frame_is_recorded_without_reply(self):
        await self.cycle([self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.texts(), ['old', '你好，介绍一下你自己'])

    async def test_send_uses_observed_title_for_alias(self):
        entry = {'recipient': '🍓午夜咖啡', 'kind': 'private', 'aliases': ['午夜咖啡']}
        self.config['allowlist'] = [entry]
        cid = self.history.ensure_contact('fixture', entry)
        self.config['state_dir'] = str(self.root/'state-alias')
        self.service = BotService(self.config, history=self.history, contact_id=cid)
        self.service.advance(self.old)
        after = frame(*self.new['bubbles'], bubble('你好，我是AI客服助手。', 'outbound_visual'))
        await self.cycle([self.new, self.new, self.new, after])
        sent = [args for name, args in self.calls if name == 'send_message']
        self.assertEqual(sent[0]['recipient'], '午夜咖啡')

    def test_binding_uses_contact_id(self):
        binding = json.loads((self.service.root/'binding.json').read_text())
        self.assertEqual(binding['contact_id'], self.cid)
        self.assertNotIn('recipient', binding)


class ConfigFieldTests(unittest.TestCase):
    def test_config_context_messages_and_http_bounds(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'config.json'
            base = {'schema_version': 1, 'host': 'fixture', 'account_id': 'fixture',
                    'allowlist': [{'recipient': '午夜咖啡', 'kind': 'private'}], 'state_dir': str(Path(temp)/'s')}
            path.write_text(json.dumps(base))
            loaded = load_config(path)
            self.assertEqual(loaded['context_messages'], 5)
            self.assertEqual(loaded['history_http'], {'bind': '127.0.0.1', 'port': 8765})
            self.assertIs(loaded['manage_window'], True)  # dedicated machine: bot may restore/resize WeChat
            for extra in [{'manage_window': 'yes'}, {'manage_window': 1}, {'context_messages': -1}, {'context_messages': 101}, {'context_messages': '5'},
                          {'history_http': {'bind': '127.0.0.1', 'port': 0}}, {'history_http': {'host': 'x'}}]:
                path.write_text(json.dumps({**base, **extra}))
                with self.assertRaises(ValueError):
                    load_config(path)
