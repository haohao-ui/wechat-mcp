"""Regression tests for the native bubble parser on redacted WeChat 4.1.15 screenshots.

Fixtures come from test conversations only; the session list is blanked and media pixelated.
Light frames are captured at the same scroll position as their dark twin, so both must parse
to the same bubbles (direction, kind, text): a theme must never flip who sent a message.
"""
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
FRAMES = ROOT/'tests'/'fixtures'/'frames'
HELPER = ROOT/'native'/'rapidocr_server.py'
# Set to a python with rapidocr_onnxruntime installed to also run the RapidOCR comparisons.
RAPID_PYTHON = os.environ.get('WECHAT_RAPIDOCR_PYTHON')

TOP = [('inbound_visual', 'text', '你好'), ('inbound_visual', 'text', '在吗'),
       ('outbound_visual', 'text', '客服助手'), ('inbound_visual', 'text', '你是美女吗'),
       ('outbound_visual', 'text', '没有外貌')]
BOTTOM = [('inbound_visual', 'file', '岗位信息更新2.13.pdf'), ('inbound_visual', 'text', '测试结束'),
          ('inbound_visual', 'text', '浅色测试')]

# name -> (profile, scale, title, bubbles)
CASES = {
    'r1-dark-top': ('wechat-4-dark', 1, '示例联系人甲', TOP),
    'r1-dark-bottom': ('wechat-4-dark', 1, '示例联系人甲', BOTTOM),
    'r1-light-top': ('wechat-4-light', 1, '示例联系人甲', TOP),
    'r1-light-bottom': ('wechat-4-light', 1, '示例联系人甲', BOTTOM),
    # Retina (2x) captures of the same chat on macOS 27.2; the window JSON stays in 880x640 points.
    'retina-dark-top': ('wechat-4-dark', 2, '示例联系人甲', TOP),
    'retina-light-top': ('wechat-4-light', 2, '示例联系人甲', TOP),
    'retina-dark-bottom': ('wechat-4-dark', 2, '示例联系人甲', BOTTOM+[('inbound_visual', 'text', '你好')]),
    'retina-light-bottom': ('wechat-4-light', 2, '示例联系人甲', BOTTOM+[('inbound_visual', 'text', '你好')]),
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

    def parse(self, name, window=None, env=None):
        # Never read this machine's real ~/.wechat-rpa/ocr.json; tests choose the engine explicitly.
        merged = {**os.environ, 'WECHAT_OCR_CONFIG': str(Path(self.tmp.name)/'no-ocr.json'), **(env or {})}
        if 'WECHAT_OCR_ENGINE' not in (env or {}):
            merged.pop('WECHAT_OCR_ENGINE', None)
        run = subprocess.run([str(self.cli), str(FRAMES/f'{name}.png'), window or self.window],
                             capture_output=True, text=True, timeout=120, env=merged)
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

    def test_default_engine_reports_vision(self):
        frame = self.parse('r1-dark-top')
        self.assertEqual(frame['ocr'], {'engine': 'vision', 'engine_used': 'vision'})

    def test_rapidocr_without_helper_falls_back_visibly(self):
        env = {'WECHAT_OCR_ENGINE': 'rapidocr', 'WECHAT_RAPIDOCR_SCRIPT': '/nonexistent/rapidocr_server.py'}
        frame = self.parse('r1-dark-top', env=env)
        self.assertNotIn('error', frame)
        self.assertEqual(frame['ocr']['engine'], 'rapidocr')
        self.assertEqual(frame['ocr']['engine_used'], 'vision')
        self.assertIn('fallback', frame['ocr'])
        self.assertEqual([(b['direction'], b['kind']) for b in frame['bubbles']], [t[:2] for t in TOP])

    def test_invalid_engine_is_refused(self):
        config = Path(self.tmp.name)/'bad-ocr.json'
        config.write_text('{"engine":"paddle"}')
        self.assertIn('invalid_ocr_engine', self.parse('r1-dark-top', env={'WECHAT_OCR_CONFIG': str(config)})['error'])
        self.assertIn('invalid_ocr_engine', self.parse('r1-dark-top', env={'WECHAT_OCR_ENGINE': 'paddle'})['error'])

    def test_switch_file_selects_rapidocr(self):
        config = Path(self.tmp.name)/'rapid-ocr.json'
        config.write_text('{"engine":"rapidocr"}')
        frame = self.parse('r1-dark-top', env={'WECHAT_OCR_CONFIG': str(config),
                                               'WECHAT_RAPIDOCR_SCRIPT': '/nonexistent/rapidocr_server.py'})
        self.assertEqual(frame['ocr']['engine'], 'rapidocr')

    def fake_helper(self, name, body):
        script = Path(self.tmp.name)/name
        script.write_text('import json, sys\n' + body)
        return {'WECHAT_OCR_ENGINE': 'rapidocr', 'WECHAT_RAPIDOCR_PYTHON': shutil.which('python3'),
                'WECHAT_RAPIDOCR_SCRIPT': str(script), 'WECHAT_OCR_TMP': str(Path(self.tmp.name)/'ocr-tmp')}

    def test_helper_noise_lines_are_skipped(self):
        # Library warnings on stdout must not be taken as results; the title comes from the helper.
        env = self.fake_helper('noisy.py', (
            'print("loading model...", flush=True)\n'
            'print(json.dumps({"ready": True}), flush=True)\n'
            'for line in sys.stdin:\n'
            '    print("[W] onnx warning", flush=True)\n'
            '    print(json.dumps({"lines": [{"text": "噪声测试", "box": [[60,10],[120,10],[120,30],[60,30]], "score": 1}]}), flush=True)\n'))
        frame = self.parse('r1-dark-top', env=env)
        self.assertNotIn('error', frame)
        self.assertEqual(frame['ocr'], {'engine': 'rapidocr', 'engine_used': 'rapidocr'})
        self.assertEqual(frame['title_lines'], ['噪声测试'])

    def test_hung_helper_times_out_and_falls_back(self):
        env = self.fake_helper('hung.py', (
            'import time\nprint(json.dumps({"ready": True}), flush=True)\n'
            'for line in sys.stdin:\n    time.sleep(600)\n'))
        frame = self.parse('r1-dark-top', env=env)
        self.assertNotIn('error', frame)
        self.assertEqual(frame['ocr']['engine_used'], 'vision')
        self.assertEqual(frame['ocr']['fallback'], 'rapidocr_timeout')
        self.assertEqual(frame['title_lines'], ['示例联系人甲'])
        self.assertEqual(list((Path(self.tmp.name)/'ocr-tmp').iterdir()), [], 'temporary images removed')

    def test_frames_parse_the_same_with_rapidocr(self):
        if not RAPID_PYTHON:
            self.skipTest('set WECHAT_RAPIDOCR_PYTHON to a python with rapidocr_onnxruntime')
        env = {'WECHAT_OCR_ENGINE': 'rapidocr', 'WECHAT_RAPIDOCR_PYTHON': RAPID_PYTHON,
               'WECHAT_RAPIDOCR_SCRIPT': str(HELPER)}
        differences = []
        for name, (profile, scale, title, bubbles) in CASES.items():
            with self.subTest(name):
                frame = self.parse(name, env=env)
                self.assertNotIn('error', frame)
                self.assertEqual(frame['ocr'], {'engine': 'rapidocr', 'engine_used': 'rapidocr'})
                self.assertEqual(frame['title_lines'], [title])
                got = [(b['direction'], b['kind'], b['text']) for b in frame['bubbles']]
                self.assertEqual([g[:2] for g in got], [b[:2] for b in bubbles])
                for (_, _, text), (_, _, part) in zip(got, bubbles):
                    if part not in text.replace('\n', ''):
                        differences.append((name, part, text))
        # Spec acceptance 2: text differences are listed for the user, not asserted.
        print('\nRapidOCR text differences:', differences or 'none')

    def test_unknown_theme_refused(self):
        # Neither calibrated background dominates the chat area: never guess bubble directions.
        frame = self.parse('unknown-gray')
        self.assertIn('uncalibrated_theme', frame.get('error', ''))


if __name__ == '__main__':
    unittest.main()


class WechatOcrCliTests(unittest.TestCase):
    """The standalone wechat_ocr command (send_file input check) follows the same engine switch."""

    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('wechat_ocr needs macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name)/'wechat_ocr'
        subprocess.run(['swiftc', str(ROOT/'native'/'wechat_ocr.swift'), '-o', str(cls.cli)], check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def run_ocr(self, env):
        merged = {**os.environ, 'WECHAT_OCR_CONFIG': str(Path(self.tmp.name)/'no-ocr.json'),
                  'WECHAT_OCR_TMP': str(Path(self.tmp.name)/'ocr-tmp'), **env}
        if 'WECHAT_OCR_ENGINE' not in env:
            merged.pop('WECHAT_OCR_ENGINE', None)
        return subprocess.run([str(self.cli), str(FRAMES/'r1-dark-top.png')], capture_output=True, text=True,
                              timeout=120, env=merged)

    def texts(self, run):
        self.assertEqual(run.returncode, 0, run.stderr)
        lines = json.loads(run.stdout)
        for line in lines:
            self.assertTrue(0 <= line['x'] <= 1 and 0 <= line['y'] <= 1, line)
        return [line['text'] for line in lines]

    def test_cli_rejects_invalid_engine(self):
        run = self.run_ocr({'WECHAT_OCR_ENGINE': 'paddle'})
        self.assertNotEqual(run.returncode, 0)
        self.assertIn('WECHAT_OCR_ENGINE', run.stderr)
        config = Path(self.tmp.name)/'bad.json'
        config.write_text('{"engine":"paddle"}')
        run = self.run_ocr({'WECHAT_OCR_CONFIG': str(config)})
        self.assertNotEqual(run.returncode, 0)
        self.assertIn('invalid_ocr_engine', run.stderr)

    def test_cli_rapidocr_missing_helper_falls_back(self):
        run = self.run_ocr({'WECHAT_OCR_ENGINE': 'rapidocr', 'WECHAT_RAPIDOCR_SCRIPT': '/nonexistent/rapidocr_server.py'})
        self.assertIn('ocr_fallback', run.stderr)
        self.assertTrue(any('示例联系人甲' in t for t in self.texts(run)))

    def test_cli_reads_engine_from_switch_file(self):
        config = Path(self.tmp.name)/'rapid.json'
        config.write_text('{"engine":"rapidocr"}')
        run = self.run_ocr({'WECHAT_OCR_CONFIG': str(config), 'WECHAT_RAPIDOCR_SCRIPT': '/nonexistent/rapidocr_server.py'})
        self.assertIn('ocr_fallback', run.stderr)

    def test_cli_rapidocr_reads_title(self):
        if not RAPID_PYTHON:
            self.skipTest('set WECHAT_RAPIDOCR_PYTHON to a python with rapidocr_onnxruntime')
        run = self.run_ocr({'WECHAT_OCR_ENGINE': 'rapidocr', 'WECHAT_RAPIDOCR_PYTHON': RAPID_PYTHON,
                            'WECHAT_RAPIDOCR_SCRIPT': str(HELPER)})
        self.assertNotIn('ocr_fallback', run.stderr)
        self.assertTrue(any('示例联系人甲' in t for t in self.texts(run)))
        self.assertEqual(list((Path(self.tmp.name)/'ocr-tmp').iterdir()), [], 'temporary image removed')
