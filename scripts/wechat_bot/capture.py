import asyncio
import json
from pathlib import Path

from .transport import remote_argv


class RemoteCapture:
    """Validation-only private diagnostic capture through the already-authorized GUI worker."""

    def __init__(self, host, client, recipient, directory, owner_token=''):
        if host.startswith('-') or any(c.isspace() for c in host):
            raise ValueError('invalid SSH host')
        self.host, self.client, self.recipient = host, client, recipient
        self.directory = Path(directory)
        if owner_token and (len(owner_token) != 64 or any(c not in '0123456789abcdef' for c in owner_token)):
            raise ValueError('invalid diagnostic ownership token')
        self.owner_token = owner_token
        self.active = False

    async def ssh(self, script, stdin=None):
        process = await asyncio.create_subprocess_exec(
            *remote_argv(self.host, script),
            stdin=asyncio.subprocess.PIPE if stdin is not None else asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
        try:
            out, err = await asyncio.wait_for(process.communicate(stdin), 20)
        except BaseException:
            if process.returncode is None:
                process.kill()
                await process.wait()
            raise
        if process.returncode:
            raise RuntimeError('remote diagnostic command failed: ' + err.decode(errors='replace')[:300])
        return out

    async def start(self):
        script = b'''set -eu
dir="$HOME/.wechat-rpa/diagnostics"
mkdir -p "$dir"
test "$(stat -f %u "$dir")" = "$(id -u)"
chmod 700 "$dir"
owner='__OWNER__'
if test -e "$dir/capture.enabled"; then
  test -n "$owner"
  test "$(cat "$dir/capture.enabled")" = "$owner"
else
  set -C
  umask 077
  printf '%s' "$owner" > "$dir/capture.enabled"
fi
'''
        await self.ssh('/bin/sh -s', script.replace(b'__OWNER__', self.owner_token.encode()))
        self.active = True

    async def frame(self):
        await self.client.tool('get_messages', recipient=self.recipient)
        out = await self.ssh('"$HOME/.wechat-rpa/bin/wechat_bubbles" "$HOME/.wechat-rpa/diagnostics/window.png" "$(cat "$HOME/.wechat-rpa/diagnostics/window.json")"')
        return json.loads(out)

    async def close(self):
        if self.active:
            script = b'''set -eu
dir="$HOME/.wechat-rpa/diagnostics"
test "$(cat "$dir/capture.enabled")" = '__OWNER__'
rm -f "$dir/capture.enabled" "$dir/window.png" "$dir/window.json" "$dir/visible-window.png" "$dir/chat.png" "$dir/title.png" "$dir/sessions.png" "$dir/search.png" "$dir/click.json"
'''
            await self.ssh('/bin/sh -s', script.replace(b'__OWNER__', self.owner_token.encode()))
            self.active = False
