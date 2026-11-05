import asyncio
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

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
