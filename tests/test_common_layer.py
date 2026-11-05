"""Tests for common geometry and coordinate conversion layer (native/common).

Tests the Swift self-test driver (tests/common_selftest/main.swift) which tests
WindowPoint, ScreenPoint, ImagePixel, WindowRect, WindowFrame, and displays().
"""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class CommonLayerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('common layer tests require macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name) / 'common_selftest'
        sources = sorted(str(p) for p in (ROOT / 'native' / 'common').glob('*.swift'))
        # Must include Geometry.swift; if native/common/*.swift is empty, compiler will fail
        cmd = ['swiftc', '-O', *sources, str(ROOT / 'tests' / 'common_selftest' / 'main.swift'), '-o', str(cls.cli)]
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode != 0:
            raise RuntimeError(f"Compilation failed: {res.stderr}")

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, 'tmp'):
            cls.tmp.cleanup()

    @classmethod
    def run_cli(cls, case_name):
        run = subprocess.run([str(cls.cli), 'geometry', case_name], capture_output=True, text=True, timeout=30)
        if run.returncode != 0:
            raise AssertionError(f"Process failed (rc={run.returncode}): {run.stderr}")
        return json.loads(run.stdout)

    def test_window_screen_round_trip(self):
        """窗口原点 (100,50)、点 (315,60) → 屏幕 (415,110) → 回到 (315,60)"""
        data = self.run_cli('window_screen_round_trip')
        self.assertEqual(data['screen'], {'x': 415.0, 'y': 110.0})
        self.assertEqual(data['round_trip'], {'x': 315.0, 'y': 60.0})

    def test_negative_display_origin(self):
        """窗口原点 (-1800,-120) 时换算正确；display(containing:) 在 [主屏, 左副屏] 中选出左副屏"""
        data = self.run_cli('negative_display_origin')
        self.assertEqual(data['screen_point'], {'x': -1700.0, 'y': -100.0})
        self.assertEqual(data['round_trip'], {'x': 100.0, 'y': 20.0})
        self.assertEqual(data['matched_display'], {'x': -1920.0, 'y': -180.0, 'w': 1920.0, 'h': 1080.0})

    def test_shared_display_edge_belongs_to_one_display(self):
        data = self.run_cli('shared_display_edge')
        self.assertEqual(data['edge'], {'x': 0.0, 'y': 0.0, 'w': 1440.0, 'h': 900.0})
        self.assertIsNone(data['beyond'])

    def test_retina_crop_matches_points(self):
        """scale 2 时聊天区 (315,60,565,436) → 像素 (630,120,1130,872)；scale 1 时与点相同"""
        data = self.run_cli('retina_crop_matches_points')
        self.assertEqual(data['scale2_crop'], {'x': 630.0, 'y': 120.0, 'w': 1130.0, 'h': 872.0})
        self.assertEqual(data['scale1_crop'], {'x': 315.0, 'y': 60.0, 'w': 565.0, 'h': 436.0})
        self.assertEqual(data['screen_rect'], {'x': 315.0, 'y': 60.0, 'w': 565.0, 'h': 436.0})

    def test_rejects_bad_scale(self):
        """scale 3/0/-1 返回 nil；scale 1/2 成功构造"""
        data = self.run_cli('rejects_bad_scale')
        self.assertTrue(data['scale0_is_nil'])
        self.assertTrue(data['scale3_is_nil'])
        self.assertTrue(data['scale_neg_is_nil'])
        self.assertTrue(data['scale1_is_not_nil'])
        self.assertTrue(data['scale2_is_not_nil'])

    def test_rect_contains(self):
        """WindowRect.contains 点包含测试"""
        data = self.run_cli('rect_contains')
        self.assertTrue(data['inside'])
        self.assertTrue(data['on_edge'])
        self.assertFalse(data['outside'])

    def test_displays(self):
        """displays() 调用测试"""
        data = self.run_cli('displays')
        self.assertIn('count', data)
        self.assertIsInstance(data['displays'], list)


if __name__ == '__main__':
    unittest.main()
