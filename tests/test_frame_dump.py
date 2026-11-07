"""Frame dump for the P2 Task 10 replay: every captured frame is appended when the flag file exists."""
import asyncio
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from scripts.wechat_bot.native_capture import NativeCapture


FRAME = {'bubbles': [{'text': '你好', 'digest': 'd1', 'direction': 'inbound_visual'}], 'title_lines': ['测试群 (3)']}


class FrameDumpTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.state = Path(self.tmp.name) / 'state'
        self.session = self.state / 'sessions' / 'c_0123456789abcdef'
        self.session.mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def capture(self, result=FRAME):
        async def build():  # Python 3.9: asyncio.Lock in __init__ needs a running loop.
            return NativeCapture('local', None, '测试群', self.session, '')
        cap = asyncio.run(build())
        cap.call = AsyncMock(return_value=json.loads(json.dumps(result)))
        return cap

    def dumps(self):
        return sorted(self.session.glob('frames-*.jsonl'))

    def test_no_flag_writes_nothing(self):
        asyncio.run(self.capture().frame())
        self.assertEqual(self.dumps(), [])

    def test_flag_appends_capture_and_scroll_privately(self):
        (self.state / 'frame-dump.enabled').touch()
        cap = self.capture()
        self.assertEqual(asyncio.run(cap.frame()), FRAME)
        asyncio.run(cap.scroll('up', 2))
        files = self.dumps()
        self.assertEqual(len(files), 1)
        self.assertEqual(os.stat(files[0]).st_mode & 0o777, 0o600)
        rows = [json.loads(line) for line in files[0].read_text(encoding='utf-8').splitlines()]
        self.assertEqual([r['op'] for r in rows], ['capture', 'scroll_chat'])
        self.assertEqual(rows[0]['recipient'], '测试群')
        self.assertEqual(rows[0]['frame'], FRAME)
        self.assertIsInstance(rows[0]['t'], float)

    def test_dump_failure_never_breaks_capture(self):
        (self.state / 'frame-dump.enabled').touch()
        cap = self.capture()
        with patch('scripts.wechat_bot.native_capture.os.open', side_effect=OSError('disk full')):
            self.assertEqual(asyncio.run(cap.frame()), FRAME)

    def test_old_files_pruned_and_size_capped(self):
        (self.state / 'frame-dump.enabled').touch()
        old = self.session / 'frames-20000101.jsonl'
        old.write_text('{}\n')
        cap = self.capture()
        asyncio.run(cap.frame())
        self.assertFalse(old.exists())
        today = self.dumps()[0]
        with patch('scripts.wechat_bot.native_capture.FRAME_DUMP_MAX_BYTES', today.stat().st_size):
            asyncio.run(cap.frame())
        self.assertEqual(len(today.read_text(encoding='utf-8').splitlines()), 1)


if __name__ == '__main__':
    unittest.main()
