import io
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch, MagicMock

from scripts.wechat_bot import control
from scripts.wechat_bot.coordinator import BotCoordinator

ROOT = Path(__file__).resolve().parents[1]


class ControlAdminTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        self.root = Path(self.td.name)
        self.state_dir = self.root / 'state'
        self.state_dir.mkdir(mode=0o700, parents=True)
        self.config_data = {
            'schema_version': 1,
            'host': 'local',
            'account_id': 'test-account',
            'allowlist': [{'recipient': 'test_user', 'kind': 'private'}],
            'state_dir': str(self.state_dir),
            'poll_seconds': 2,
            'cooldown_seconds': 15,
            'max_replies_per_hour': 20,
        }
        self.config_path = self.root / 'config.json'
        self.config_path.write_text(json.dumps(self.config_data, ensure_ascii=False))

    def tearDown(self):
        self.td.cleanup()

    def test_config_check_ok_and_error_exit_codes(self):
        # 1. Valid config -> returns 0, prints {"ok": true}
        stdout = io.StringIO()
        with patch('sys.stdout', stdout):
            try:
                control.main(['config-check', str(self.config_path)])
                exit_code = 0
            except SystemExit as e:
                exit_code = e.code
        self.assertEqual(exit_code, 0)
        output = json.loads(stdout.getvalue().strip())
        self.assertEqual(output, {'ok': True})

        # 2. Invalid config -> returns 2, prints {"ok": false, "error": "<msg>"}
        invalid_path = self.root / 'invalid_config.json'
        invalid_path.write_text(json.dumps({'schema_version': 999}))
        stdout = io.StringIO()
        with patch('sys.stdout', stdout):
            with self.assertRaises(SystemExit) as cm:
                control.main(['config-check', str(invalid_path)])
            self.assertEqual(cm.exception.code, 2)
        output = json.loads(stdout.getvalue().strip())
        self.assertFalse(output['ok'])
        self.assertIn('error', output)
        self.assertIn('unsupported bot config schema or field', output['error'])

    def test_config_check_writes_nothing(self):
        # Snapshot state of directory before checks
        def snapshot(dir_path):
            state = {}
            for p in dir_path.rglob('*'):
                st = p.stat()
                state[str(p)] = (st.st_mtime_ns, st.st_size)
            return state

        before = snapshot(self.root)

        # Run valid check
        with patch('sys.stdout', io.StringIO()):
            try:
                control.main(['config-check', str(self.config_path)])
            except SystemExit:
                pass

        # Run invalid check
        invalid_path = self.root / 'bad.json'
        invalid_path.write_text('{"bad": 1}')
        before_bad = snapshot(self.root)
        with patch('sys.stdout', io.StringIO()):
            try:
                control.main(['config-check', str(invalid_path)])
            except SystemExit:
                pass
        after_bad = snapshot(self.root)
        self.assertEqual(before_bad, after_bad)

        invalid_path.unlink()
        after = snapshot(self.root)
        self.assertEqual(before, after)

    async def test_pause_file_skips_cycle(self):
        pause_file = self.state_dir / 'admin-pause.json'
        self.assertFalse(pause_file.exists())

        # Execute control pause
        with patch('sys.stdout', io.StringIO()):
            control.main(['pause', '--config', str(self.config_path)])

        self.assertTrue(pause_file.exists())
        pause_content = json.loads(pause_file.read_text())
        self.assertTrue(pause_content.get('paused'))
        self.assertIn('at', pause_content)
        mode = stat.S_IMODE(pause_file.stat().st_mode)
        self.assertEqual(mode, 0o600)

        # Coordinator cycle must skip reading, clicking, sending
        coordinator = BotCoordinator(self.config_path)

        class FakeCapture:
            def __init__(self):
                self.frame_called = 0
                self.call_called = 0
                self.ocr_status = None
                self.last_switch = {}
                self.process = MagicMock()

            async def frame(self):
                self.frame_called += 1
                return {'bubbles': [], 'snapshot_id': 'snap'}

            async def call(self, *args, **kwargs):
                self.call_called += 1
                return {}

            async def start(self):
                pass

            async def retarget(self, *args, **kwargs):
                pass

        fake_capture = FakeCapture()
        coordinator.capture = fake_capture

        await coordinator.sweep()

        self.assertEqual(fake_capture.frame_called, 0)
        self.assertEqual(fake_capture.call_called, 0)

        health_file = self.state_dir / 'health.json'
        self.assertTrue(health_file.exists())
        health = json.loads(health_file.read_text())
        self.assertEqual(health.get('state'), 'paused')
        self.assertEqual(health.get('reason'), 'admin_pause')

    async def test_resume_continues(self):
        # First pause
        with patch('sys.stdout', io.StringIO()):
            control.main(['pause', '--config', str(self.config_path)])
        pause_file = self.state_dir / 'admin-pause.json'
        self.assertTrue(pause_file.exists())

        # Then resume
        with patch('sys.stdout', io.StringIO()):
            control.main(['resume', '--config', str(self.config_path)])
        self.assertFalse(pause_file.exists())

        coordinator = BotCoordinator(self.config_path)

        class FakeCapture:
            def __init__(self):
                self.frame_called = 0
                self.ocr_status = None
                self.last_switch = {}
                self.process = MagicMock()

            async def frame(self):
                self.frame_called += 1
                return {
                    'bubbles': [{'direction': 'inbound_visual', 'text': 'hi', 'bubble_id': '1'}],
                    'snapshot_id': 'snap'
                }

            async def call(self, *args, **kwargs):
                return {}

            async def start(self):
                pass

            async def retarget(self, *args, **kwargs):
                pass

        fake_capture = FakeCapture()
        coordinator.capture = fake_capture

        # Running sweep should not be paused by admin_pause
        await coordinator.sweep()
        health_file = self.state_dir / 'health.json'
        health = json.loads(health_file.read_text())
        self.assertNotEqual(health.get('reason'), 'admin_pause')


if __name__ == '__main__':
    unittest.main()
