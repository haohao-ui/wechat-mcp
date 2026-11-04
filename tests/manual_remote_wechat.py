"""Explicit, read-only real-host probe through the GUI worker's MCP socket."""
import argparse
import json
import queue
import subprocess
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', required=True)
    parser.add_argument('--recipient', required=True)
    parser.add_argument('--expect-text', help='Check visible OCR text without printing the transcript')
    args = parser.parse_args()
    if args.host.startswith('-') or any(char.isspace() for char in args.host):
        parser.error('invalid SSH host')
    process = subprocess.Popen([
        'ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=8', '-o', 'LogLevel=ERROR',
        args.host, '/usr/bin/nc -U "$HOME/.wechat-rpa/worker.sock"'
    ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    received = queue.Queue()

    def read():
        try:
            for line in process.stdout:
                received.put(json.loads(line))
        finally:
            received.put(None)

    threading.Thread(target=read, daemon=True).start()

    def request(identifier, method, params):
        process.stdin.write(json.dumps({
            'jsonrpc': '2.0', 'id': identifier, 'method': method, 'params': params
        }) + '\n')
        process.stdin.flush()
        response = received.get(timeout=60)
        if response is None:
            raise RuntimeError('GUI worker closed the connection before replying')
        if response.get('id') != identifier:
            raise RuntimeError('unexpected MCP response ID')
        return response

    try:
        request(1, 'initialize', {
            'protocolVersion': '2025-06-18', 'capabilities': {},
            'clientInfo': {'name': 'manual-readonly-acceptance', 'version': '1'}
        })
        process.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
        process.stdin.flush()
        started = time.monotonic()
        response = request(2, 'tools/call', {
            'name': 'get_messages', 'arguments': {
                'platform': 'wechat', 'account_id': 'test-desktop', 'recipient': args.recipient
            }
        })
        result = response.get('result', {})
        if result.get('isError') or 'error' in response:
            summary = {'isError': True, 'error': result.get('content', response.get('error'))}
        else:
            output = result.get('structuredContent') or json.loads(result['content'][0]['text'])
            data = output['data']
            summary = {'isError': False, 'recipient': data.get('recipient'),
                       'text_line_count': len(data.get('text_lines', [])),
                       'direction': data.get('direction'), 'coverage': data.get('coverage')}
            lines = data.get('text_lines', [])
            summary['input_placeholder_observed'] = any('语音输入文字' in line for line in lines)
            if args.expect_text:
                compact = lambda value: ''.join(value.split())
                summary['expected_text_observed'] = any(
                    compact(args.expect_text) in compact(line) for line in lines
                )
        summary['elapsed_ms'] = round((time.monotonic() - started) * 1000)
        summary['send_requested'] = False
        print(json.dumps(summary, ensure_ascii=False))
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=3)


if __name__ == '__main__':
    main()
