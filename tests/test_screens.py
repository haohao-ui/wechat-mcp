"""Tests for screen identification skeleton (native/eyes/Screens.swift).

Verifies identifyScreen and windowInfos across:
- main window only
- context menu popup
- save panel focus
- image viewer makes other window
- missing focus info is unknown
- other app frontmost is unknown
- CGWindowListCopyWindowInfo row conversion & filtering
"""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ScreenIdentificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('screen identification tests need macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name) / 'eyes_selftest'
        sources = [
            str(ROOT / 'native' / 'eyes' / 'Screens.swift'),
            str(ROOT / 'tests' / 'eyes_selftest' / 'main.swift'),
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
    def run_cli(cls, command, payload):
        proc = subprocess.run(
            [str(cls.cli), command, json.dumps(payload)],
            capture_output=True,
            text=True,
            timeout=30
        )
        if proc.returncode != 0:
            raise AssertionError(f"CLI error (exit code {proc.returncode}): {proc.stderr}\nOutput: {proc.stdout}")
        return json.loads(proc.stdout)

    def test_main_window_only(self):
        payload = {
            "windows": [
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 0,
                    "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                    "on_screen": True
                }
            ],
            "focus": {
                "frontmost_bundle": "com.tencent.xinWeChat",
                "focused_identifier": None
            },
            "wechat_pid": 1234,
            "main": [880.0, 640.0]
        }
        res = self.run_cli("identify", payload)
        self.assertEqual(res.get("screen"), "main")

        # Tolerates <= 2 points difference
        payload["windows"][0]["bounds"] = {"x": 100.0, "y": 80.0, "w": 881.5, "h": 638.5}
        res2 = self.run_cli("identify", payload)
        self.assertEqual(res2.get("screen"), "main")

    def test_context_menu_popup(self):
        payload = {
            "windows": [
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 0,
                    "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                    "on_screen": True
                },
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 101,
                    "bounds": {"x": 350.0, "y": 200.0, "w": 160.0, "h": 220.0},
                    "on_screen": True
                }
            ],
            "focus": {
                "frontmost_bundle": "com.tencent.xinWeChat",
                "focused_identifier": None
            },
            "wechat_pid": 1234,
            "main": [880.0, 640.0]
        }
        res = self.run_cli("identify", payload)
        self.assertEqual(res.get("screen"), "context_menu")

    def test_save_panel_focus(self):
        payload = {
            "windows": [
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 0,
                    "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                    "on_screen": True
                }
            ],
            "focus": {
                "frontmost_bundle": "com.tencent.xinWeChat",
                "focused_identifier": "saveAsNameTextField"
            },
            "wechat_pid": 1234,
            "main": [880.0, 640.0]
        }
        res = self.run_cli("identify", payload)
        self.assertEqual(res.get("screen"), "save_panel")

    def test_image_viewer_makes_other_window(self):
        payload = {
            "windows": [
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 0,
                    "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                    "on_screen": True
                },
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 0,
                    "bounds": {"x": 200.0, "y": 150.0, "w": 1024.0, "h": 768.0},
                    "on_screen": True
                }
            ],
            "focus": {
                "frontmost_bundle": "com.tencent.xinWeChat",
                "focused_identifier": None
            },
            "wechat_pid": 1234,
            "main": [880.0, 640.0]
        }
        res = self.run_cli("identify", payload)
        self.assertEqual(res.get("screen"), "other_wechat_window")

    def test_missing_focus_info_is_unknown(self):
        payload = {
            "windows": [
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 0,
                    "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                    "on_screen": True
                }
            ],
            "focus": {
                "frontmost_bundle": None,
                "focused_identifier": None
            },
            "wechat_pid": 1234,
            "main": [880.0, 640.0]
        }
        res = self.run_cli("identify", payload)
        self.assertEqual(res.get("screen"), "unknown")

    def test_other_app_frontmost_is_unknown(self):
        payload = {
            "windows": [
                {
                    "owner": "WeChat",
                    "pid": 1234,
                    "layer": 0,
                    "bounds": {"x": 100.0, "y": 80.0, "w": 880.0, "h": 640.0},
                    "on_screen": True
                }
            ],
            "focus": {
                "frontmost_bundle": "com.apple.finder",
                "focused_identifier": None
            },
            "wechat_pid": 1234,
            "main": [880.0, 640.0]
        }
        res = self.run_cli("identify", payload)
        self.assertEqual(res.get("screen"), "unknown")

    def test_cg_rows_convert(self):
        rows = [
            # 1. Standard CGWindowList dictionary representation
            {
                "kCGWindowOwnerName": "WeChat",
                "kCGWindowOwnerPID": 1234,
                "kCGWindowLayer": 0,
                "kCGWindowBounds": {"X": 50, "Y": 60, "Width": 880, "Height": 640},
                "kCGWindowIsOnscreen": True
            },
            # 2. Simplified keys representation
            {
                "owner": "微信",
                "pid": 1234,
                "layer": 101,
                "bounds": {"x": 10, "y": 20, "w": 100, "h": 200},
                "on_screen": True
            },
            # 3. Missing owner -> drop
            {
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 0, "y": 0, "w": 100, "h": 100},
                "on_screen": True
            },
            # 4. Missing pid -> drop
            {
                "owner": "WeChat",
                "layer": 0,
                "bounds": {"x": 0, "y": 0, "w": 100, "h": 100},
                "on_screen": True
            },
            # 5. Missing layer -> drop
            {
                "owner": "WeChat",
                "pid": 1234,
                "bounds": {"x": 0, "y": 0, "w": 100, "h": 100},
                "on_screen": True
            },
            # 6. Missing bounds -> drop
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "on_screen": True
            },
            # 7. Missing onScreen -> drop
            {
                "owner": "WeChat",
                "pid": 1234,
                "layer": 0,
                "bounds": {"x": 0, "y": 0, "w": 100, "h": 100}
            }
        ]
        res = self.run_cli("convert_rows", rows)
        self.assertEqual(len(res), 2)
        self.assertEqual(res[0], {
            "owner": "WeChat",
            "pid": 1234,
            "layer": 0,
            "bounds": {"x": 50.0, "y": 60.0, "w": 880.0, "h": 640.0},
            "on_screen": True
        })
        self.assertEqual(res[1], {
            "owner": "微信",
            "pid": 1234,
            "layer": 101,
            "bounds": {"x": 10.0, "y": 20.0, "w": 100.0, "h": 200.0},
            "on_screen": True
        })


if __name__ == '__main__':
    unittest.main()
