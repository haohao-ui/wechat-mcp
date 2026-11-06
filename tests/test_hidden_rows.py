"""A chat whose session-list row is not visible is never searched for (acceptance 2026-11-06).

The bot opened rows it could not see by typing the name into WeChat's search every 16 seconds: the
row was off screen because the chat had no new message, and search could not match "测试群(3)"."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock

from scripts.wechat_bot.coordinator import BotCoordinator
from scripts.wechat_bot.native_capture import NativeCapture

MISSING = ValueError('private visual operation failed: invalid("session_row_not_unique_or_missing")')


def target(name):
    return {'recipient': name, 'kind': 'private'}


class HiddenRowTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        state = root / 'state'
        state.mkdir(mode=0o700)
        self.path = root / 'config.json'
        self.path.write_text(json.dumps({
            'schema_version': 1, 'host': 'fixture', 'account_id': 'test', 'state_dir': str(state),
            'allowlist': [target('a'), target('b')], 'ignorelist': [],
            'poll_seconds': 2, 'cooldown_seconds': 15, 'max_replies_per_hour': 20}))
        self.path.chmod(0o600)
        self.coordinator = BotCoordinator(self.path)

    def tearDown(self):
        self.tmp.cleanup()

    def capture_with_rows(self, *names):
        capture = MagicMock(spec=NativeCapture)
        capture.process = object()
        capture.sessions = AsyncMock(return_value={'rows': [
            {'name': n, 'preview_digest': 'd', 'preview_key': 'k', 'preview_time': '', 'red_pixels': 0, 'y': 100}
            for n in names]})
        return capture

    async def test_hidden_row_is_not_due(self):
        self.coordinator.capture = self.capture_with_rows('a')
        due = await self.coordinator.due_sessions(['a', 'b'])
        self.assertIn('a', due)
        self.assertNotIn('b', due)

    async def test_missing_row_waits_without_search(self):
        capture = NativeCapture('fixture', AsyncMock(), 'a', Path(self.tmp.name)/'a', '0'*64, names=['a'])
        capture.navigate = AsyncMock(); capture.close_transport = AsyncMock()
        capture.call = AsyncMock(side_effect=MISSING)
        with self.assertRaisesRegex(ValueError, 'session_row_not_unique_or_missing'):
            await capture.retarget('b', Path(self.tmp.name)/'b', ['b'])
        capture.navigate.assert_not_awaited()

    async def test_missing_row_visit_waits_and_keeps_connection(self):
        child = self.coordinator.session(target('b'))
        self.coordinator.connect = AsyncMock(side_effect=MISSING)
        self.coordinator.close_connection = AsyncMock()
        await self.coordinator.visit(target('b'))
        health = json.loads((child.root/'health.json').read_text())
        self.assertEqual(health['state'], 'waiting_for_wechat_window')
        self.assertFalse(child.paused)
        self.coordinator.close_connection.assert_not_called()


class NoChatOpenTests(HiddenRowTests):
    async def test_mismatch_forgets_the_open_chat_so_a_row_click_reopens_it(self):
        # Acceptance 2026-11-06: with no chat selected the bot kept reading "the active chat" (blank pane,
        # mismatch) and never clicked a row. Forgetting it makes the next visit open the chat by its row.
        child = self.coordinator.session(target('a'))
        self.coordinator.active_name = 'a'
        self.coordinator.capture = MagicMock()
        self.coordinator.connect = AsyncMock()
        self.coordinator.close_connection = AsyncMock()
        child.cycle = AsyncMock(side_effect=ValueError('private visual operation failed: invalid("layout_mismatch:input_divider")'))
        await self.coordinator.visit(target('a'))
        self.assertIsNone(self.coordinator.active_name)
        self.coordinator.close_connection.assert_not_called()
