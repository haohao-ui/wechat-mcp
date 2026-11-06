"""Tests for native/common/WechatLayout.swift: layout format, validation and fallback version selection.
"""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class CommonLayoutTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('layout tests need macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name) / 'layout_selftest'
        sources = sorted(str(p) for p in (ROOT / 'native' / 'common').glob('*.swift')) + [
            str(ROOT / 'tests' / 'common_selftest' / 'layout_main.swift')
        ]
        # Compilation will fail during Step 2 before WechatLayout.swift is created
        proc = subprocess.run(
            ['swiftc', '-O', *sources, '-o', str(cls.cli)],
            capture_output=True,
            text=True
        )
        if proc.returncode != 0:
            raise RuntimeError(f"swiftc compilation failed:\nSTDOUT:\n{proc.stdout}\nSTDERR:\n{proc.stderr}")

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    @classmethod
    def run_cli(cls, *args):
        run = subprocess.run([str(cls.cli), *args], capture_output=True, text=True, timeout=30)
        if run.returncode != 0:
            raise AssertionError(f"CLI error (exit code {run.returncode}): {run.stderr}")
        return json.loads(run.stdout)

    def test_default_layout_loads(self):
        default_path = ROOT / 'native' / 'layout' / 'wechat-4.1.15.json'
        res = self.run_cli('layout', str(default_path))
        self.assertTrue(res.get('ok'), res)
        self.assertEqual(res.get('schema'), 1)
        self.assertEqual(res.get('wechat_version'), '4.1.15')
        self.assertEqual(res.get('window'), {'width': 880.0, 'height': 640.0})
        self.assertEqual(res.get('chat'), {'x': 315.0, 'y': 60.0, 'w': 565.0, 'h': 436.0})
        self.assertEqual(res.get('session_list'), {'x': 60.0, 'y': 60.0, 'w': 240.0, 'h': 580.0})
        self.assertEqual(res.get('title'), {'x': 250.0, 'y': 0.0, 'w': 620.0, 'h': 50.0})
        self.assertEqual(res.get('input'), {'x': 315.0, 'y': 496.0, 'w': 565.0, 'h': 144.0})
        self.assertEqual(res.get('points_per_tick'), 90.0)
        self.assertEqual(res.get('lines_per_tick'), 10)
        self.assertEqual(res.get('anchors_count'), 3)
        self.assertIn('另存为…', res.get('menu_labels', {}).get('save_as', []))

    def test_invalid_layouts_are_refused(self):
        default_path = ROOT / 'native' / 'layout' / 'wechat-4.1.15.json'
        if not default_path.exists():
            self.skipTest('default layout not found')
        with open(default_path, 'r', encoding='utf-8') as f:
            valid_data = json.load(f)

        cases = [
            # 1. Missing required region 'chat'
            ("missing_chat",
             lambda d: d["regions"].pop("chat"),
             "layout_missing_region:chat"),
            # 2. Region exceeds window bounds (chat.w + chat.x > 880)
            ("out_of_bounds",
             lambda d: d["regions"]["chat"].update({"w": 600.0}),
             "layout_region_out_of_window:chat"),
            # 3. Region width is zero
            ("zero_width",
             lambda d: d["regions"]["chat"].update({"w": 0.0}),
             "layout_invalid_value:chat.w"),
            # 4. Unsupported schema version
            ("unsupported_schema",
             lambda d: d.update({"schema": 2}),
             "layout_schema_unsupported"),
            # 5. Invalid scroll points_per_tick <= 0
            ("zero_scroll_step",
             lambda d: d["scroll"].update({"points_per_tick": 0}),
             "layout_invalid_value:scroll.points_per_tick"),
        ]

        with tempfile.TemporaryDirectory() as tdir:
            tpath = Path(tdir)
            for name, mutator, expected_err in cases:
                data = json.loads(json.dumps(valid_data))
                mutator(data)
                fpath = tpath / f"{name}.json"
                fpath.write_text(json.dumps(data), encoding='utf-8')

                res = self.run_cli('layout', str(fpath))
                self.assertFalse(res.get('ok'), f"Case {name} should have failed")
                self.assertEqual(res.get('error'), expected_err, f"Case {name}")

            # 6. Non-JSON file
            non_json_path = tpath / "not_json.json"
            non_json_path.write_text("THIS IS NOT JSON {{{", encoding='utf-8')
            res = self.run_cli('layout', str(non_json_path))
            self.assertFalse(res.get('ok'))
            self.assertEqual(res.get('error'), 'layout_unreadable')

            # 7. File > 256KB (300KB)
            large_path = tpath / "large.json"
            large_content = json.dumps(valid_data).encode('utf-8')
            # Pad with comment/whitespace or long string inside valid json
            pad_size = 300 * 1024 - len(large_content)
            large_path.write_bytes(large_content + b" " * max(pad_size, 1024))
            res = self.run_cli('layout', str(large_path))
            self.assertFalse(res.get('ok'))
            self.assertEqual(res.get('error'), 'layout_too_large')

    def test_version_memo_survives_flaky_lookup(self):
        # Acceptance 2026-11-06: a long-running worker intermittently failed to read WeChat's version
        # (layout_missing) while the map file was present. The version read for a running WeChat
        # process is reused while that process lives; a new process must be read afresh.
        res = self.run_cli('memo', '100:4.1.15', 'none', '100:-', '200:-', 'none', '200:4.2.0', 'gone')
        self.assertEqual(res['resolved'], ['4.1.15', '4.1.15', '4.1.15', None, None, '4.2.0', None])

    def test_version_memo_needs_a_first_reading(self):
        res = self.run_cli('memo', 'none', '100:-')
        self.assertEqual(res['resolved'], [None, None])

    def test_version_read_from_the_process_executable(self):
        # Acceptance 2026-11-06: a long-running worker logged "wechat not among running applications"
        # while WeChat was up. The version now comes from the window owner's executable path.
        app = Path(self.tmp.name)/'Apps'/'WeChat.app'
        (app/'Contents'/'MacOS').mkdir(parents=True)
        import plistlib
        (app/'Contents'/'Info.plist').write_bytes(plistlib.dumps({'CFBundleShortVersionString': '4.1.15'}))
        res = self.run_cli('bundle', str(app/'Contents'/'MacOS'/'WeChat'))
        self.assertEqual(res, {'bundle': str(app), 'version': '4.1.15'})

    def test_executable_path_of_a_live_process(self):
        # sysctl KERN_PROCARGS2 works on the macOS 12 SDK (proc_pidpath is not exposed there).
        res = self.run_cli('exe')
        self.assertEqual(Path(res['path']).resolve(), Path(self.cli).resolve())

    def test_executable_outside_an_app_has_no_version(self):
        res = self.run_cli('bundle', '/usr/bin/true')
        self.assertEqual(res, {'bundle': None, 'version': None})

    def test_version_fallback_picks_nearest_lower(self):
        with tempfile.TemporaryDirectory() as tdir:
            p = Path(tdir)
            (p / "wechat-4.1.12.json").write_text("{}", encoding='utf-8')
            (p / "wechat-4.1.15.json").write_text("{}", encoding='utf-8')
            (p / "wechat-4.2.0.json").write_text("{}", encoding='utf-8')

            # Exact match
            res = self.run_cli('fallback', str(p), '4.1.15')
            self.assertEqual(res.get('path'), str(p / "wechat-4.1.15.json"))

            # Nearest lower version (4.1.20 -> 4.1.15)
            res = self.run_cli('fallback', str(p), '4.1.20')
            self.assertEqual(res.get('path'), str(p / "wechat-4.1.15.json"))

            # No version <= 4.0.1 -> None
            res = self.run_cli('fallback', str(p), '4.0.1')
            self.assertIsNone(res.get('path'))


if __name__ == '__main__':
    unittest.main()
