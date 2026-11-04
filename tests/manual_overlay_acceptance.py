"""Run the explicit native title-bar acceptance in the existing GUI host, then restore MCP."""
import argparse
import json
import queue
import subprocess
import threading


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', required=True)
    parser.add_argument('--run-live', action='store_true', required=True,
                        help='Explicitly allow two title-bar clicks; does not send messages')
    args = parser.parse_args()
    if args.host.startswith('-') or any(c.isspace() for c in args.host):
        parser.error('invalid SSH host')
    ssh = ['ssh', '-o', 'BatchMode=yes', '-o', 'LogLevel=ERROR', args.host]

    def remote(script):
        return subprocess.run(ssh + ['/bin/sh', '-s'], input=script, text=True,
                              capture_output=True, check=True, timeout=15).stdout.strip()

    # The app signature stays untouched. Only its externally hosted worker is replaced briefly.
    original_hash = remote('shasum -a 256 "$HOME/.wechat-rpa/bin/wechat-mcp"').split()[0]
    installed = False
    process = None
    try:
        remote('''set -eu
base="$HOME/.wechat-rpa/bin"
test -x "$base/overlay-acceptance"
test ! -e "$base/wechat-mcp.acceptance-original"
if pgrep -x wechat-mcp >/dev/null; then echo 'worker already active' >&2; exit 1; fi
mv "$base/wechat-mcp" "$base/wechat-mcp.acceptance-original"
''')
        installed = True
        remote('''set -eu
base="$HOME/.wechat-rpa/bin"
cat > "$base/wechat-mcp" <<'SCRIPT'
#!/bin/sh
exec "$HOME/.wechat-rpa/bin/overlay-acceptance" "$HOME/.wechat-rpa/bin"
SCRIPT
chmod 755 "$base/wechat-mcp"
''')
        process = subprocess.Popen(ssh + ['/usr/bin/nc -U "$HOME/.wechat-rpa/worker.sock"'],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        result = queue.Queue()

        def read():
            result.put(process.stdout.readline())

        threading.Thread(target=read, daemon=True).start()
        line = result.get(timeout=25)
        if not line:
            raise RuntimeError('GUI acceptance closed without a report')
        report = json.loads(line)
    finally:
        if process is not None:
            process.stdin.close()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=3)
        if installed:
            restored_hash = remote('''set -eu
base="$HOME/.wechat-rpa/bin"
mv "$base/wechat-mcp.acceptance-original" "$base/wechat-mcp"
shasum -a 256 "$base/wechat-mcp"
''').split()[0]
            if restored_hash != original_hash:
                raise RuntimeError('restored MCP binary hash mismatch')
    report['mcp_binary_restored'] = True
    print(json.dumps(report, ensure_ascii=False))
    if report['status'] != 'passed':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
