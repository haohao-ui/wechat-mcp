"""Tests for anchor verification (native/eyes/Anchors.swift).

Verifies anchor positioning, theme detection, and screen layout integrity across:
- all 8 1x and 2x fixtures pass and detect correct theme
- measured anchor coordinates match layout definition (input_divider ~ 496, list_border ~ 300)
- vertical/horizontal shifts trigger failure of corresponding anchor
- blanking title region triggers title_text failure
- unknown gray frame triggers theme failure
- popup patch covering divider triggers input_divider failure
"""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
FRAMES = ROOT / 'tests' / 'fixtures' / 'frames'
LAYOUT = ROOT / 'native' / 'layout' / 'wechat-4.1.15.json'

FIXTURES = {
    'r1-dark-top': 'wechat-4-dark',
    'r1-dark-bottom': 'wechat-4-dark',
    'r1-light-top': 'wechat-4-light',
    'r1-light-bottom': 'wechat-4-light',
    'retina-dark-top': 'wechat-4-dark',
    'retina-dark-bottom': 'wechat-4-dark',
    'retina-light-top': 'wechat-4-light',
    'retina-light-bottom': 'wechat-4-light',
}


class AnchorVerificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('anchor verification tests need macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name) / 'anchors_selftest'
        sources = [
            str(ROOT / 'native' / 'common' / 'Geometry.swift'),
            str(ROOT / 'native' / 'common' / 'WechatLayout.swift'),
            str(ROOT / 'native' / 'eyes' / 'Anchors.swift'),
            str(ROOT / 'tests' / 'eyes_selftest' / 'anchors_main.swift'),
        ]
        proc = subprocess.run(
            ['swiftc', '-O', *sources, '-o', str(cls.cli)],
            capture_output=True,
            text=True
        )
        if proc.returncode != 0:
            raise RuntimeError(f"swiftc compilation failed:\nSTDOUT:\n{proc.stdout}\nSTDERR:\n{proc.stderr}")

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, 'tmp'):
            cls.tmp.cleanup()

    @classmethod
    def run_anchors(cls, frame_name, extra_args=None):
        if (FRAMES / f'{frame_name}.png').exists():
            png_path = FRAMES / f'{frame_name}.png'
        else:
            png_path = FRAMES / frame_name
        cmd = [str(cls.cli), str(png_path), str(LAYOUT)]
        if extra_args:
            cmd.extend(extra_args)
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=30
        )
        if proc.returncode != 0:
            raise AssertionError(f"CLI error (code {proc.returncode}):\nSTDERR: {proc.stderr}\nSTDOUT: {proc.stdout}")
        return json.loads(proc.stdout)

    def test_measure_finds_real_edges_far_from_a_wrong_map(self):
        # Stage C drill 2026-11-06: calibrate searched only +/-12 points around the map's edge, so a map
        # 30 points off measured nothing. Re-measuring must search widely and find the real edges.
        import tempfile
        layout = json.loads(LAYOUT.read_text())
        for r in ('chat', 'chat_content'):
            layout['regions'][r]['h'] -= 30
        layout['regions']['input']['y'] -= 30; layout['regions']['input']['h'] += 30
        layout['regions']['session_list']['w'] -= 40
        with tempfile.NamedTemporaryFile('w', suffix='.json', delete=False) as handle:
            json.dump(layout, handle)
        wrong = handle.name
        for name in ('r1-dark-top', 'r1-light-bottom', 'retina-dark-bottom', 'retina-light-top', 'r1-light-xls-card'):
            for path in (str(LAYOUT), wrong):
                with self.subTest(name=name, layout=path):
                    cmd = [str(self.cli), str(FRAMES / f'{name}.png'), path, '--measure']
                    out = json.loads(subprocess.run(cmd, capture_output=True, text=True, timeout=30, check=True).stdout)
                    # The map is rewritten from these values: exact, not within a point.
                    self.assertEqual(out['measured']['input_divider'], 496)
                    self.assertAlmostEqual(out['measured']['list_border'], 300, delta=1)

    def test_no_chat_open_blocks_reading_but_not_the_session_list(self):
        # Acceptance 2026-11-06: with no chat selected (blank right pane, no input box) every check
        # failed, so the bot never clicked a row to open a chat again and reported needs_recalibration.
        full = self.run_anchors('r1-light-no-chat')
        self.assertFalse(full['passed'])
        self.assertIn('input_divider', full['failures'])
        listed = self.run_anchors('r1-light-no-chat', ['--regions', 'session_list'])
        self.assertTrue(listed['passed'], listed)
        self.assertEqual(listed['theme'], 'wechat-4-light')

    def test_session_list_scope_still_catches_a_moved_list(self):
        listed = self.run_anchors('r1-light-bottom', ['--regions', 'session_list', '--shift', '30,0'])
        self.assertIn('list_border', listed['failures'])

    def test_all_fixtures_pass(self):
        for name, expected_theme in FIXTURES.items():
            with self.subTest(name=name):
                res = self.run_anchors(name)
                self.assertTrue(res.get('passed'), f"Fixture {name} failed: {res}")
                self.assertEqual(res.get('failures'), [])
                self.assertEqual(res.get('theme'), expected_theme)

    def test_measured_positions_match_layout(self):
        for name in FIXTURES:
            with self.subTest(name=name):
                res = self.run_anchors(name)
                measured = res.get('measured', {})
                self.assertIn('input_divider', measured)
                self.assertIn('list_border', measured)
                self.assertAlmostEqual(measured['input_divider'], 496.0, delta=1.0)
                self.assertAlmostEqual(measured['list_border'], 300.0, delta=1.0)

    def test_shifted_frame_fails(self):
        # Shifting vertically by 30 points must cause input_divider to fail
        res_v = self.run_anchors('r1-dark-top', extra_args=['--shift', '0,30'])
        self.assertFalse(res_v.get('passed'))
        self.assertIn('input_divider', res_v.get('failures', []))

        # Shifting horizontally by 30 points must cause list_border to fail
        res_h = self.run_anchors('r1-dark-top', extra_args=['--shift', '30,0'])
        self.assertFalse(res_h.get('passed'))
        self.assertIn('list_border', res_h.get('failures', []))

    def test_blank_title_fails(self):
        # Blanking the title region must cause title_text anchor to fail
        res = self.run_anchors('r1-dark-top', extra_args=['--blank', 'title'])
        self.assertFalse(res.get('passed'))
        self.assertIn('title_text', res.get('failures', []))

    def test_unknown_gray_fails_theme(self):
        res = self.run_anchors('unknown-gray.png')
        self.assertFalse(res.get('passed'))
        self.assertIn('theme', res.get('failures', []))
        self.assertIsNone(res.get('theme'))

    def test_popup_over_divider_fails(self):
        # A popup covering the left portion of the input divider line
        res = self.run_anchors('r1-dark-top', extra_args=['--patch', '315,420,200,150,240,240,240'])
        self.assertFalse(res.get('passed'))
        self.assertIn('input_divider', res.get('failures', []))


if __name__ == '__main__':
    unittest.main()
