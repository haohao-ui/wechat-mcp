"""Regression tests for the native bubble parser on redacted WeChat 4.1.15 screenshots.

Fixtures come from test conversations only; the session list is blanked and media pixelated.
Light frames are captured at the same scroll position as their dark twin, so both must parse
to the same bubbles (direction, kind, text): a theme must never flip who sent a message.
"""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
FRAMES = ROOT/'tests'/'fixtures'/'frames'

TOP = [('inbound_visual', 'text', '你好'), ('inbound_visual', 'text', '在吗'),
       ('outbound_visual', 'text', '客服助手'), ('inbound_visual', 'text', '你是美女吗'),
       ('outbound_visual', 'text', '没有外貌')]
BOTTOM = [('inbound_visual', 'file', '岗位信息更新2.13.pdf'), ('inbound_visual', 'text', '测试结束'),
          ('inbound_visual', 'text', '浅色测试')]

# name -> (profile, scale, title, bubbles)
CASES = {
    'r1-dark-top': ('wechat-4-dark', 1, '午夜咖啡', TOP),
    'r1-dark-bottom': ('wechat-4-dark', 1, '午夜咖啡', BOTTOM),
    'r1-light-top': ('wechat-4-light', 1, '午夜咖啡', TOP),
    'r1-light-bottom': ('wechat-4-light', 1, '午夜咖啡', BOTTOM),
    # Retina (2x) captures of the same chat on macOS 27.2; the window JSON stays in 880x640 points.
    'retina-dark-top': ('wechat-4-dark', 2, '午夜咖啡', TOP),
    'retina-light-top': ('wechat-4-light', 2, '午夜咖啡', TOP),
    'retina-dark-bottom': ('wechat-4-dark', 2, '午夜咖啡', BOTTOM+[('inbound_visual', 'text', '你好')]),
    'retina-light-bottom': ('wechat-4-light', 2, '午夜咖啡', BOTTOM+[('inbound_visual', 'text', '你好')]),
}


class NativeFrameTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('native parser needs macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name)/'wechat_bubbles'
        subprocess.run(['swiftc', '-O', '-parse-as-library', str(ROOT/'native'/'wechat_bubbles.swift'), '-o', str(cls.cli)],
                       check=True, capture_output=True)
        cls.window = (FRAMES/'window-880x640.json').read_text()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def parse(self, name, window=None):
        run = subprocess.run([str(self.cli), str(FRAMES/f'{name}.png'), window or self.window],
                             capture_output=True, text=True, timeout=120)
        if run.returncode:
            return {'error': run.stderr.strip()}
        return json.loads(run.stdout)

    def test_frames_parse_to_expected_bubbles(self):
        for name, (profile, scale, title, bubbles) in CASES.items():
            with self.subTest(name):
                frame = self.parse(name)
                self.assertNotIn('error', frame)
                self.assertEqual(frame['profile'], profile)
                if scale is not None:
                    self.assertEqual(frame['scale'], scale)
                self.assertEqual(frame['title_lines'], [title])
                self.assertFalse(frame['clipped_bubble'])
                got = [(b['direction'], b['kind'], b['text']) for b in frame['bubbles']]
                self.assertEqual([g[:2] for g in got], [b[:2] for b in bubbles])
                for (_, _, text), (_, _, part) in zip(got, bubbles):
                    self.assertIn(part, text.replace('\n', ''))

    def test_image_bubbles_carry_phash(self):
        images = [b for name in CASES for b in self.parse(name).get('bubbles', []) if b['kind'] == 'image']
        if not images:
            self.skipTest('no image bubbles in the committed fixtures yet (added by remote calibration)')
        for b in images:
            self.assertEqual(len(b['phash']), 16)
            int(b['phash'], 16)

    def test_unknown_theme_refused(self):
        # Neither calibrated background dominates the chat area: never guess bubble directions.
        frame = self.parse('unknown-gray')
        self.assertIn('uncalibrated_theme', frame.get('error', ''))


if __name__ == '__main__':
    unittest.main()
