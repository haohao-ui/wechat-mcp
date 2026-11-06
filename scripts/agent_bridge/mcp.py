import asyncio
import json


class MCP:
    """One persistent, serialized MCP transport; no shell expansion of command arguments."""

    def __init__(self, command, account):
        self.command, self.account = command, account
        self.process = None
        self.counter = 0
        self.lock = asyncio.Lock()

    async def start(self):
        self.process = await asyncio.create_subprocess_exec(
            *self.command, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
            limit=4 * 1024 * 1024)
        await self.request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                           'clientInfo': {'name': 'wechat-agent-bridge', 'version': '1'}})
        self.process.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        await self.process.stdin.drain()

    async def request(self, method, params):
        async with self.lock:
            self.counter += 1
            request = {'jsonrpc': '2.0', 'id': self.counter, 'method': method, 'params': params}
            self.process.stdin.write(json.dumps(request).encode() + b'\n')
            await self.process.stdin.drain()
            async def _read_reply():
                while True:
                    line = await self.process.stdout.readline()
                    if not line:
                        raise RuntimeError('MCP closed before response')
                    reply = json.loads(line)
                    if 'id' not in reply:
                        continue
                    if reply['id'] != self.counter:
                        raise RuntimeError('MCP response ID mismatch')
                    if 'error' in reply:
                        error = reply['error']
                        raise RuntimeError(f"MCP RPC {error.get('code')}: {str(error.get('message'))[:512]}")
                    return reply['result']

            try:
                try:
                    return await asyncio.wait_for(_read_reply(), 40)
                except (asyncio.TimeoutError, TimeoutError):
                    raise TimeoutError()
            except BaseException:
                # Discard a stream after cancellation/timeout, including any unread response.
                await self.close()
                raise

    async def tool(self, name, **arguments):
        result = await self.request('tools/call', {'name': name, 'arguments': {**self.account, **arguments}})
        if result.get('isError'):
            detail = ' '.join(str(c.get('text', '')) for c in result.get('content') or [] if isinstance(c, dict))
            raise RuntimeError(f'MCP tool failed: {name}' + (f': {detail[:300]}' if detail else ''))
        output = result.get('structuredContent') or json.loads(result['content'][0]['text'])
        if output.get('account') != self.account:
            raise RuntimeError('MCP account mismatch')
        return output['data']

    async def close(self):
        if self.process is None:
            return
        process, self.process = self.process, None
        if process.stdin:
            process.stdin.close()
        try:
            await asyncio.wait_for(process.wait(), 2)
        except asyncio.TimeoutError:
            process.terminate()
            await process.wait()
