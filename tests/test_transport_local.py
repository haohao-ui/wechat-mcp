import ast
import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

ROOT = Path(__file__).resolve().parents[1]


class TransportLocalTests(unittest.TestCase):
    def test_local_argv_has_no_ssh(self):
        from scripts.wechat_bot.transport import remote_argv
        cmd = '/usr/bin/nc -U "$HOME/.wechat-rpa/worker.sock"'
        argv = remote_argv('local', cmd)
        self.assertEqual(argv, ['/bin/sh', '-c', cmd])
        self.assertNotIn('ssh', argv)
        for part in argv:
            self.assertNotIn('ssh', part)

    def test_remote_argv_unchanged(self):
        from scripts.wechat_bot.transport import remote_argv
        cmd = '/usr/bin/nc -U "$HOME/.wechat-rpa/worker.sock"'
        argv = remote_argv('wechat-host', cmd)
        self.assertEqual(argv, ['ssh', '-o', 'BatchMode=yes', '-o', 'LogLevel=ERROR', 'wechat-host', cmd])

    def test_every_call_site_uses_remote_argv(self):
        scripts_dir = ROOT / 'scripts'
        violations = []
        for p in scripts_dir.rglob('*.py'):
            if p.name == 'transport.py':
                continue
            tree = ast.parse(p.read_text(encoding='utf-8'), filename=str(p))
            for node in ast.walk(tree):
                if isinstance(node, ast.Constant) and node.value == 'ssh':
                    violations.append(f"{p.relative_to(ROOT)}:{getattr(node, 'lineno', '?')}")
        self.assertEqual(violations, [], f"Literal 'ssh' found in scripts outside transport.py: {violations}")

    def test_load_config_accepts_local(self):
        from scripts.wechat_bot.service import load_config
        with tempfile.TemporaryDirectory() as td:
            cfg_file = Path(td) / 'config.json'
            state_dir = Path(td) / 'state'
            state_dir.mkdir(mode=0o700, parents=True)
            data = {
                'schema_version': 1,
                'host': 'local',
                'account_id': 'test-account',
                'allowlist': [{'recipient': 'test_user', 'kind': 'private'}],
                'state_dir': str(state_dir),
            }
            cfg_file.write_text(json.dumps(data))
            loaded = load_config(cfg_file)
            self.assertEqual(loaded['host'], 'local')


class MCPTimeoutTests(unittest.IsolatedAsyncioTestCase):
    async def test_mcp_timeout_raises_timeout_error(self):
        from scripts.agent_bridge.mcp import MCP
        mcp = MCP(['echo'], {'platform': 'wechat', 'account_id': 'test'})
        mcp.process = MagicMock()
        mcp.process.stdin = MagicMock()
        mcp.process.stdin.write = MagicMock()
        mcp.process.stdin.drain = AsyncMock()
        mcp.process.stdout = MagicMock()
        mcp.process.stdout.readline = AsyncMock(side_effect=asyncio.TimeoutError())
        mcp.process.wait = AsyncMock()
        mcp.process.returncode = 0

        async def fake_wait_for(coro, timeout):
            coro.close()
            raise asyncio.TimeoutError()

        with patch('asyncio.wait_for', side_effect=fake_wait_for):
            with self.assertRaises(TimeoutError):
                await mcp.request('ping', {})


if __name__ == '__main__':
    unittest.main()
