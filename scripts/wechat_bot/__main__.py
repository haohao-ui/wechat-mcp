import argparse
import asyncio
import json
from pathlib import Path
import time

from scripts.agent_bridge.codex import private_directory, write_private
from scripts.agent_bridge.mcp import MCP
from .capture import RemoteCapture
from .policy import Policy, fingerprint, validate_reply, echo_matches
from .reply import generate
from .transport import remote_argv


async def run(args):
    root = private_directory(args.state)
    policy = Policy(args.account, args.recipient, args.trigger)
    command = remote_argv(args.host, '/usr/bin/nc -U "$HOME/.wechat-rpa/worker.sock"')
    client = MCP(command, {'platform': 'wechat', 'account_id': args.account})
    capture = RemoteCapture(args.host, client, args.recipient, root)
    try:
        await client.start()
        status = await client.tool('get_listener_status')
        if status.get('enabled'):
            raise RuntimeError('stop the existing observer before running bounded bot validation')
        await capture.start()
        baseline = await capture.frame()
        policy.check_frame(baseline)
        write_private(root / 'baseline.json', baseline)
        print(json.dumps({'status': 'ready', 'allowlist': [args.recipient], 'mode': 'model_reply',
                          'trigger': args.trigger, 'max_replies': 1, 'baseline_bubbles': len(baseline['bubbles'])}, ensure_ascii=False), flush=True)
        deadline = time.monotonic() + args.duration
        while time.monotonic() < deadline:
            await asyncio.sleep(1)
            frame = await capture.frame()
            policy.check_frame(frame)
            if list(map(fingerprint, baseline['bubbles'])) == list(map(fingerprint, frame['bubbles'])):
                continue
            confirmed = await capture.frame()
            candidate = policy.candidate(baseline, frame, confirmed)
            if candidate is None:
                baseline = confirmed
                continue
            key = policy.key(baseline, candidate)
            journal = root / (key + '.json')
            if journal.exists():
                print(json.dumps({'status': 'already_attempted', 'idempotency_key': key}), flush=True)
                return
            record = {'status': 'generating', 'account_id': args.account, 'recipient': args.recipient,
                      'input': candidate, 'idempotency_key': key, 'direction_evidence': 'visual_candidate',
                      'platform_message_verified': False}
            write_private(journal, record)
            print(json.dumps({'status': 'inbound_visual_candidate_confirmed', 'idempotency_key': key}), flush=True)
            reply = validate_reply(await generate(candidate['text']))
            record.update(status='generated', reply=reply)
            write_private(journal, record)
            # Model latency must not turn a stale input into a new authorization to send.
            latest = await capture.frame()
            policy.check_frame(latest)
            if list(map(fingerprint, latest['bubbles'])) != list(map(fingerprint, confirmed['bubbles'])):
                record['status'] = 'chat_changed_before_send'
                write_private(journal, record)
                raise RuntimeError('chat changed before sending; reply retained, no send')
            record['status'] = 'attempting'
            write_private(journal, record)
            result = await client.tool('send_message', recipient=args.recipient, message=reply, idempotency_key=key)
            durable = await client.tool('get_send_status', idempotency_key=key)
            record.update(status='send_returned', send_result=result, durable_status=durable)
            write_private(journal, record)
            after = await capture.frame()
            policy.check_frame(after)
            echoed = any(x['direction'] == 'outbound_visual' and echo_matches(x['text'], reply)
                         for x in after['bubbles'])
            record.update(client_echo_observed=echoed, delivery='unknown')
            write_private(journal, record)
            print(json.dumps({'status': 'replied_once', 'recipient': args.recipient, 'reply': reply,
                              'idempotency_key': key, 'client_echo_observed': echoed, 'delivery': 'unknown'}, ensure_ascii=False), flush=True)
            return
        print(json.dumps({'status': 'timeout_without_reply', 'send_calls': 0}), flush=True)
    finally:
        try:
            await capture.close()
        finally:
            await client.close()


def main():
    parser = argparse.ArgumentParser(description='Explicit private-chat allowlist model-reply validation; at most one reply.')
    parser.add_argument('--host', required=True)
    parser.add_argument('--account', required=True)
    parser.add_argument('--recipient', required=True, help='One explicitly authorized private contact')
    parser.add_argument('--trigger', required=True, help='Fresh validation message prefix')
    parser.add_argument('--state', required=True)
    parser.add_argument('--duration', type=int, default=300)
    parser.add_argument('--allow-send', action='store_true', required=True)
    args = parser.parse_args()
    if not 1 <= args.duration <= 1800:
        parser.error('duration must be 1..1800 seconds')
    asyncio.run(run(args))


if __name__ == '__main__':
    main()
