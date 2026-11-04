import argparse
import asyncio
import json
import time

from .codex import CodexSink, acknowledge
from .codex_exec import CodexExecSink
from .mcp import MCP
from .pump import EventPump


async def run(args):
    command = json.loads(args.mcp_command)
    if not isinstance(command, list) or not command or not all(isinstance(x, str) and x for x in command):
        raise ValueError('mcp-command must be a JSON array of executable and arguments')
    client = MCP(command, {'platform': 'wechat', 'account_id': args.account})
    sink = CodexSink(args.spool, args.thread) if args.receiver == 'desktop' else CodexExecSink(args.spool)
    pump = EventPump(client, sink, 'codex-validation-' + (args.thread or 'exec'))
    started = False
    try:
        await client.start()
        previous = await client.tool('get_listener_status')
        if previous.get('enabled'):
            raise RuntimeError('validation requires listener disabled; will not replace an existing listener')
        await client.tool('start_listener', interval_seconds=args.interval)
        started = True
        deadline = time.monotonic() + args.duration
        while time.monotonic() < deadline:
            health = await client.tool('get_listener_status')
            if health.get('health', {}).get('last_success_at') is not None:
                break
            if health.get('health', {}).get('state') == 'degraded':
                raise RuntimeError('listener degraded before baseline')
            await asyncio.sleep(0.5)
        else:
            raise RuntimeError('listener baseline timeout')
        print(json.dumps({'status': 'ready', 'recipient': args.recipient, 'expected_preview': args.expect_preview,
                          'event_type': 'interface_change', 'incoming_verified': False}), flush=True)
        while time.monotonic() < deadline:
            await client.tool('wait_for_events', timeout_seconds=5, limit=100)
            event = await pump.claim(args.recipient, args.expect_preview)
            if event is not None:
                result = await pump.deliver(event, receipt_timeout=args.receipt_timeout)
                print(json.dumps(result), flush=True)
                return
            await asyncio.sleep(1)
        print(json.dumps({'status': 'no_matching_event', 'send_calls': 0}), flush=True)
    finally:
        if started and client.process is not None:
            try:
                await client.tool('stop_listener')
            finally:
                await client.close()
        else:
            await client.close()


def main():
    parser = argparse.ArgumentParser(description='Explicit bounded WeChat → Agent delivery validation; never sends WeChat messages.')
    sub = parser.add_subparsers(dest='operation', required=True)
    ack = sub.add_parser('acknowledge')
    ack.add_argument('--spool', required=True)
    ack.add_argument('--delivery-id', required=True)
    ack.add_argument('--thread', required=True)
    worker = sub.add_parser('validate-codex')
    worker.add_argument('--mcp-command', required=True)
    worker.add_argument('--account', required=True)
    worker.add_argument('--thread', help='Existing Codex chat UUID; required for desktop receiver')
    worker.add_argument('--spool', required=True)
    worker.add_argument('--receiver', choices=['desktop', 'exec'], default='exec')
    worker.add_argument('--recipient', required=True)
    worker.add_argument('--expect-preview', required=True)
    worker.add_argument('--interval', type=int, choices=range(2, 61), default=2)
    worker.add_argument('--duration', type=int, default=300)
    worker.add_argument('--receipt-timeout', type=int, default=120)
    args = parser.parse_args()
    if args.operation == 'acknowledge':
        acknowledge(args.spool, args.delivery_id, args.thread)
        print(json.dumps({'status': 'agent_receipt_written', 'delivery_id': args.delivery_id}))
    else:
        if not 1 <= args.duration <= 1800 or not 1 <= args.receipt_timeout <= 600:
            parser.error('duration must be 1..1800 and receipt-timeout 1..600 seconds')
        if args.receiver == 'desktop' and not args.thread:
            parser.error('desktop receiver requires --thread')
        asyncio.run(run(args))


if __name__ == '__main__':
    main()
