import asyncio
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from scripts.wechat_bot.coordinator import BotCoordinator, RECALIBRATION_LIMIT
from scripts.wechat_bot import control


def target(name, kind='private'):
    return {'recipient': name, 'kind': kind}


class RecalibrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.path = self.root / 'config.json'
        self.config = {
            'schema_version': 1,
            'host': 'fixture',
            'account_id': 'fixture',
            'state_dir': str(self.root / 'state'),
            'allowlist': [target('a'), target('b')],
            'ignorelist': [],
            'poll_seconds': 1,
            'turn_quiet_seconds': 0,
        }
        self.save()
        self.coordinator = BotCoordinator(self.path)

    def save(self):
        self.path.write_text(json.dumps(self.config))

    def tearDown(self):
        self.temp.cleanup()

    async def test_mismatch_waits_without_pausing(self):
        child = self.coordinator.session(target('a'))
        err_msg = 'private visual operation failed: invalid("layout_mismatch:input_divider,title_text")'
        self.coordinator.connect = AsyncMock(side_effect=ValueError(err_msg))

        from scripts.wechat_bot.native_capture import NativeCapture
        capture = NativeCapture('fixture', AsyncMock(), 'a', self.root / 'a', '0' * 64, names=['a'])
        capture.ssh = AsyncMock(); capture.connect = AsyncMock(); capture.navigate = AsyncMock(); capture.close_transport = AsyncMock()
        capture.call = AsyncMock(side_effect=ValueError(err_msg))
        with self.assertRaisesRegex(ValueError, 'layout_mismatch'):
            await capture.start()
        capture.navigate.assert_not_awaited()

        await self.coordinator.visit(target('a'))
        self.assertFalse(child.paused)
        self.assertEqual(self.coordinator.layout_mismatches, 1)

        health = json.loads((child.root / 'health.json').read_text())
        self.assertEqual(health['state'], 'waiting_for_wechat_window')
        self.assertEqual(health['failures'], ['input_divider', 'title_text'])
        self.assertEqual(health['attempt'], 1)
        self.assertIn('layout_mismatch', health['reason'])

    async def test_five_mismatches_need_recalibration(self):
        child = self.coordinator.session(target('a'))
        err_msg = 'private visual operation failed: invalid("layout_mismatch:input_divider,title_text")'
        self.coordinator.connect = AsyncMock(side_effect=ValueError(err_msg))

        for i in range(1, 5):
            await self.coordinator.visit(target('a'))
            self.assertFalse(child.paused)
            self.assertEqual(self.coordinator.layout_mismatches, i)
            health = json.loads((child.root / 'health.json').read_text())
            self.assertEqual(health['state'], 'waiting_for_wechat_window')
            self.assertEqual(health['attempt'], i)

        # 5th mismatch triggers needs_recalibration
        await self.coordinator.visit(target('a'))
        self.assertFalse(child.paused)
        self.assertEqual(self.coordinator.layout_mismatches, 5)
        health = json.loads((child.root / 'health.json').read_text())
        self.assertEqual(health['state'], 'needs_recalibration')
        self.assertEqual(health['failures'], ['input_divider', 'title_text'])
        self.assertEqual(health['attempt'], 5)

    async def test_success_resets_counter(self):
        child_a = self.coordinator.session(target('a'))
        child_b = self.coordinator.session(target('b'))
        mismatch_err = ValueError('private visual operation failed: invalid("layout_mismatch:input_divider")')

        # 4 consecutive mismatches
        self.coordinator.connect = AsyncMock(side_effect=mismatch_err)
        for _ in range(4):
            await self.coordinator.visit(target('a'))
        self.assertEqual(self.coordinator.layout_mismatches, 4)

        # 1 successful cycle on target('b')
        self.coordinator.connect = AsyncMock()
        child_b.cycle = AsyncMock()
        await self.coordinator.visit(target('b'))
        self.assertEqual(self.coordinator.layout_mismatches, 0)

        # 1 more mismatch on target('a') -> attempt 1, waiting_for_wechat_window
        self.coordinator.connect = AsyncMock(side_effect=ValueError('private visual operation failed: invalid("layout_mismatch:title_text")'))
        await self.coordinator.visit(target('a'))
        self.assertEqual(self.coordinator.layout_mismatches, 1)
        health = json.loads((child_a.root / 'health.json').read_text())
        self.assertEqual(health['state'], 'waiting_for_wechat_window')
        self.assertEqual(health['attempt'], 1)
        self.assertEqual(health['failures'], ['title_text'])

    async def test_layout_missing_not_counted(self):
        child = self.coordinator.session(target('a'))
        missing_err = ValueError('private visual operation failed: invalid("layout_missing")')
        self.coordinator.connect = AsyncMock(side_effect=missing_err)

        for _ in range(5):
            await self.coordinator.visit(target('a'))
            self.assertFalse(child.paused)

        self.assertEqual(self.coordinator.layout_mismatches, 0)
        health = json.loads((child.root / 'health.json').read_text())
        self.assertEqual(health['state'], 'waiting_for_wechat_window')
        self.assertNotIn('failures', health)

    def test_status_flags_recalibration(self):
        base = self.root / '.WeChatRPABot'
        base.mkdir()
        config_path = base / 'config.json'
        config_path.write_text(json.dumps(self.config))
        state_dir = self.root / 'state'
        state_dir.mkdir(parents=True, exist_ok=True)

        # Case 1: health has needs_recalibration in sessions
        health = {
            'state': 'watching',
            'pid': 123,
            'sessions': {'a': {'state': 'needs_recalibration', 'failures': ['input_divider']}}
        }
        (state_dir / 'health.json').write_text(json.dumps(health))

        out_rec = io.StringIO()
        err_rec = io.StringIO()
        with patch.object(sys, 'argv', ['bot-control', 'status']), \
             patch.object(control, 'locations', return_value=(base, config_path, self.root / 'fixture.plist')), \
             patch.object(control, 'loaded', return_value=True), \
             contextlib.redirect_stdout(out_rec), \
             contextlib.redirect_stderr(err_rec):
            control.main()

        res_rec = json.loads(out_rec.getvalue())
        self.assertTrue(res_rec['needs_recalibration'])
        self.assertIn('界面核对连续失败，请运行 control calibrate 重新定位', err_rec.getvalue())

        # Case 2: normal health (state watching, all sessions normal)
        normal_health = {
            'state': 'watching',
            'pid': 123,
            'sessions': {'a': {'state': 'watching'}}
        }
        (state_dir / 'health.json').write_text(json.dumps(normal_health))

        out_norm = io.StringIO()
        err_norm = io.StringIO()
        with patch.object(sys, 'argv', ['bot-control', 'status']), \
             patch.object(control, 'locations', return_value=(base, config_path, self.root / 'fixture.plist')), \
             patch.object(control, 'loaded', return_value=True), \
             contextlib.redirect_stdout(out_norm), \
             contextlib.redirect_stderr(err_norm):
            control.main()

        res_norm = json.loads(out_norm.getvalue())
        self.assertFalse(res_norm['needs_recalibration'])
        self.assertEqual(err_norm.getvalue(), '')

    async def test_mismatch_keeps_worker_connection(self):
        child = self.coordinator.session(target('a'))
        err_msg = 'private visual operation failed: invalid("layout_mismatch:input_divider")'
        self.coordinator.connect = AsyncMock()
        child.cycle = AsyncMock(side_effect=ValueError(err_msg))
        self.coordinator.capture = MagicMock()
        self.coordinator.close_connection = AsyncMock()

        await self.coordinator.visit(target('a'))
        self.coordinator.close_connection.assert_not_called()

    async def test_session_probe_mismatch_keeps_worker_connection(self):
        # Stage C drill 2026-11-06: the session-list probe closed the connection on every mismatch,
        # the worker exited, and each retry relaunched it through MCP navigation.
        from scripts.wechat_bot.native_capture import NativeCapture
        capture = MagicMock(spec=NativeCapture)
        capture.process = object()
        capture.sessions = AsyncMock(side_effect=ValueError('private visual operation failed: invalid("layout_mismatch:list_border")'))
        self.coordinator.capture = capture
        self.coordinator.close_connection = AsyncMock()
        await self.coordinator.due_sessions(['a'])
        self.coordinator.close_connection.assert_not_called()

    async def test_serve_calibration_writes_candidate_via_live_capture(self):
        worker_result = {
            'wechat_version': '4.1.15',
            'layout_path': '/path/to/layout',
            'layout': {
                'schema': 1,
                'window': {'width': 800, 'height': 640},
                'regions': {
                    'session_list': {'x': 0, 'y': 0, 'w': 250, 'h': 640},
                    'title': {'x': 250, 'y': 0, 'w': 550, 'h': 60},
                    'chat': {'x': 250, 'y': 60, 'w': 550, 'h': 436},
                    'input': {'x': 250, 'y': 496, 'w': 550, 'h': 144},
                },
                'scroll': {'points_per_tick': 20, 'lines_per_tick': 3},
                'anchors': []
            },
            'measured': {'input_divider': 500.0},
        }
        fake_remote = MagicMock(return_value=subprocess.CompletedProcess([], 0, b'', b''))
        fake_capture = MagicMock()
        fake_capture.call = AsyncMock(return_value=worker_result)
        self.coordinator.capture = fake_capture

        req_file = self.coordinator.root / 'calibrate-request.json'
        res_file = self.coordinator.root / 'calibrate-result.json'
        req_file.write_text(json.dumps({'id': 'req123', 'action': 'calibrate', 'created_at': time.time()}))

        with patch('scripts.wechat_bot.control.default_remote_runner', fake_remote):
            await self.coordinator.serve_calibration()

        self.assertFalse(req_file.exists())
        self.assertTrue(res_file.exists())
        res = json.loads(res_file.read_text())
        self.assertEqual(res['id'], 'req123')
        self.assertTrue(res['ok'])
        fake_capture.call.assert_awaited_once_with('calibrate')

    async def test_serve_accept_uses_live_version(self):
        candidate_data = {
            'schema': 1,
            'wechat_version': '4.1.15',
            'window': {'width': 800, 'height': 640},
            'regions': {
                'session_list': {'x': 0, 'y': 0, 'w': 250, 'h': 640},
                'title': {'x': 250, 'y': 0, 'w': 550, 'h': 60},
                'chat': {'x': 250, 'y': 60, 'w': 550, 'h': 436},
                'input': {'x': 250, 'y': 496, 'w': 550, 'h': 144},
            },
            'scroll': {'points_per_tick': 20, 'lines_per_tick': 3},
            'anchors': []
        }
        worker_result = {
            'wechat_version': '4.1.15',
            'layout': candidate_data,
        }
        def fake_runner(host, command, input_data=None):
            if 'cat ' in command:
                return subprocess.CompletedProcess([], 0, json.dumps(candidate_data).encode('utf-8'), b'')
            return subprocess.CompletedProcess([], 0, b'', b'')

        fake_capture = MagicMock()
        fake_capture.call = AsyncMock(return_value=worker_result)
        self.coordinator.capture = fake_capture

        req_file = self.coordinator.root / 'calibrate-request.json'
        res_file = self.coordinator.root / 'calibrate-result.json'
        req_file.write_text(json.dumps({'id': 'req_accept_1', 'action': 'accept', 'created_at': time.time()}))

        with patch('scripts.wechat_bot.control.default_remote_runner', fake_runner):
            await self.coordinator.serve_calibration()

        self.assertFalse(req_file.exists())
        self.assertTrue(res_file.exists())
        res = json.loads(res_file.read_text())
        self.assertEqual(res['id'], 'req_accept_1')
        self.assertTrue(res['ok'])
        self.assertTrue(res['result']['accepted'])
        fake_capture.call.assert_awaited_once_with('calibrate')

    async def test_request_without_connection_reports_worker_not_connected(self):
        self.coordinator.capture = None
        req_file = self.coordinator.root / 'calibrate-request.json'
        res_file = self.coordinator.root / 'calibrate-result.json'
        req_file.write_text(json.dumps({'id': 'req_noconn', 'action': 'calibrate', 'created_at': time.time()}))

        await self.coordinator.serve_calibration()

        self.assertFalse(req_file.exists())
        self.assertTrue(res_file.exists())
        res = json.loads(res_file.read_text())
        self.assertEqual(res['id'], 'req_noconn')
        self.assertFalse(res['ok'])
        self.assertEqual(res['error'], 'worker_not_connected')

    async def test_stale_request_expires(self):
        fake_capture = MagicMock()
        fake_capture.call = AsyncMock()
        self.coordinator.capture = fake_capture
        req_file = self.coordinator.root / 'calibrate-request.json'
        res_file = self.coordinator.root / 'calibrate-result.json'
        req_file.write_text(json.dumps({'id': 'req_stale', 'action': 'calibrate', 'created_at': time.time() - 601}))

        await self.coordinator.serve_calibration()

        self.assertFalse(req_file.exists())
        self.assertTrue(res_file.exists())
        res = json.loads(res_file.read_text())
        self.assertEqual(res['id'], 'req_stale')
        self.assertFalse(res['ok'])
        self.assertEqual(res['error'], 'expired')
        fake_capture.call.assert_not_called()
