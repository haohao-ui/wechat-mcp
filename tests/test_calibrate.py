"""Tests for control calibrate and calibrate --accept (Stage C Task C4).

Tests:
1. test_candidate_from_measurements: candidate layout derived from measurements with diffs and missing listed.
2. test_accept_replaces_with_backup: valid candidate matching running version replaces active layout atomically with backup.
3. test_accept_refuses_invalid_candidate: invalid candidate layout is refused and original file stays untouched.
4. test_accept_refuses_version_mismatch: candidate version mismatching running WeChat version is refused.
5. test_calibrate_never_clicks: worker calibrate branch never calls botActions, click, rightClick, or key events.
"""
import base64
import json
import os
from pathlib import Path
import re
import tempfile
import time
import unittest
from unittest.mock import AsyncMock, patch

from scripts.wechat_bot import control

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LAYOUT_PATH = ROOT / 'native' / 'layout' / 'wechat-4.1.15.json'


class CalibrateTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.layout_dir = Path(self.temp_dir.name) / 'layout'
        self.layout_dir.mkdir(parents=True, exist_ok=True)
        with open(DEFAULT_LAYOUT_PATH, 'r', encoding='utf-8') as f:
            self.base_layout = json.load(f)
        self.version = self.base_layout['wechat_version']

    def tearDown(self):
        self.temp_dir.cleanup()

    def test_candidate_from_measurements(self):
        """Candidate layout derived from measured coordinates, missing items preserved and listed."""
        active_path = self.layout_dir / f'wechat-{self.version}.json'
        active_path.write_text(json.dumps(self.base_layout), encoding='utf-8')

        # Measured input_divider shifted to 490 (originally 496), list_border missing
        measured = {
            'input_divider': 490.0
        }
        mock_worker_result = {
            'measured': measured,
            'theme': 'wechat-4-dark',
            'window': {'width': 880, 'height': 640},
            'scale': 1,
            'annotated_png_base64': base64.b64encode(b'fake-png-data').decode('ascii'),
        }

        diffs, missing, candidate = control.generate_candidate(
            self.base_layout,
            mock_worker_result,
            layout_dir=self.layout_dir
        )

        # 1. input_divider derived fields
        self.assertEqual(candidate['regions']['chat']['h'], 490.0 - candidate['regions']['chat']['y'])
        self.assertEqual(candidate['regions']['input']['y'], 490.0)
        self.assertEqual(candidate['regions']['input']['h'], 640.0 - 490.0)

        # 2. list_border was missing: preserved original value and listed in missing
        self.assertEqual(candidate['regions']['session_list']['w'], self.base_layout['regions']['session_list']['w'])
        self.assertIn('list_border', missing)

        # 3. Diffs list includes changed fields
        diff_keys = [d['field'] for d in diffs]
        self.assertIn('chat.h', diff_keys)
        self.assertIn('input.y', diff_keys)
        self.assertIn('input.h', diff_keys)

        # 4. Candidate JSON and PNG files written with 0o600 permissions
        cand_json_path = self.layout_dir / f'candidate-wechat-{self.version}.json'
        cand_png_path = self.layout_dir / f'candidate-wechat-{self.version}.png'
        self.assertTrue(cand_json_path.exists())
        self.assertTrue(cand_png_path.exists())
        self.assertEqual(oct(cand_json_path.stat().st_mode & 0o777), oct(0o600))
        self.assertEqual(oct(cand_png_path.stat().st_mode & 0o777), oct(0o600))
        self.assertEqual(cand_png_path.read_bytes(), b'fake-png-data')

    def test_accept_replaces_with_backup(self):
        """control calibrate --accept backs up current layout and atomically replaces it."""
        active_path = self.layout_dir / f'wechat-{self.version}.json'
        active_path.write_text(json.dumps(self.base_layout), encoding='utf-8')
        active_path.chmod(0o600)

        # Create candidate layout
        candidate_layout = json.loads(json.dumps(self.base_layout))
        candidate_layout['regions']['input']['y'] = 492.0
        candidate_layout['regions']['input']['h'] = 148.0
        candidate_layout['regions']['chat']['h'] = 432.0
        cand_json_path = self.layout_dir / f'candidate-wechat-{self.version}.json'
        cand_json_path.write_text(json.dumps(candidate_layout), encoding='utf-8')
        cand_json_path.chmod(0o600)

        # Run accept
        result = control.calibrate_accept(
            layout_dir=self.layout_dir,
            running_version=self.version
        )
        self.assertTrue(result['accepted'])
        self.assertTrue(Path(result['backup']).exists())
        self.assertEqual(oct(Path(result['backup']).stat().st_mode & 0o777), oct(0o600))

        # Check backup content is old layout
        old_data = json.loads(Path(result['backup']).read_text(encoding='utf-8'))
        self.assertEqual(old_data['regions']['input']['y'], 496.0)

        # Check active file is now the candidate layout
        new_active = json.loads(active_path.read_text(encoding='utf-8'))
        self.assertEqual(new_active['regions']['input']['y'], 492.0)
        self.assertEqual(oct(active_path.stat().st_mode & 0o777), oct(0o600))

    def test_accept_refuses_invalid_candidate(self):
        """Invalid candidate layout (e.g. missing required region or out of bounds) is refused."""
        active_path = self.layout_dir / f'wechat-{self.version}.json'
        original_content = json.dumps(self.base_layout)
        active_path.write_text(original_content, encoding='utf-8')
        active_path.chmod(0o600)

        invalid_cases = [
            # 1. Missing required region
            lambda d: d['regions'].pop('chat'),
            # 2. Region out of bounds
            lambda d: d['regions']['input'].update({'y': 600.0, 'h': 100.0}),
            # 3. Unsupported schema
            lambda d: d.update({'schema': 2}),
            # 4. Zero/negative dimensions
            lambda d: d['regions']['title'].update({'w': 0}),
        ]

        cand_json_path = self.layout_dir / f'candidate-wechat-{self.version}.json'

        for i, mutator in enumerate(invalid_cases):
            candidate_layout = json.loads(json.dumps(self.base_layout))
            mutator(candidate_layout)
            cand_json_path.write_text(json.dumps(candidate_layout), encoding='utf-8')

            with self.assertRaises((ValueError, SystemExit)):
                control.calibrate_accept(
                    layout_dir=self.layout_dir,
                    running_version=self.version
                )

            # Active layout must remain untouched
            self.assertEqual(active_path.read_text(encoding='utf-8'), original_content)
            # No backup files created
            backups = list(self.layout_dir.glob(f'wechat-{self.version}.json.bak-*'))
            self.assertEqual(backups, [])

    def test_accept_refuses_version_mismatch(self):
        """Candidate layout version mismatching running WeChat version is refused."""
        active_path = self.layout_dir / f'wechat-{self.version}.json'
        original_content = json.dumps(self.base_layout)
        active_path.write_text(original_content, encoding='utf-8')
        active_path.chmod(0o600)

        # Candidate is for version 4.1.15, but running is 4.1.16
        cand_json_path = self.layout_dir / f'candidate-wechat-{self.version}.json'
        cand_json_path.write_text(original_content, encoding='utf-8')

        with self.assertRaises((ValueError, SystemExit)):
            control.calibrate_accept(
                layout_dir=self.layout_dir,
                running_version='4.1.16'
            )

        # Original file unchanged, no backup created
        self.assertEqual(active_path.read_text(encoding='utf-8'), original_content)
        backups = list(self.layout_dir.glob(f'wechat-{self.version}.json.bak-*'))
        self.assertEqual(backups, [])

    def test_accept_refuses_corrupt_or_oversized_candidate(self):
        """Corrupt JSON or oversized candidate file is refused without modifying original."""
        active_path = self.layout_dir / f'wechat-{self.version}.json'
        original_content = json.dumps(self.base_layout)
        active_path.write_text(original_content, encoding='utf-8')
        cand_json_path = self.layout_dir / f'candidate-wechat-{self.version}.json'

        # 1. Corrupt JSON
        cand_json_path.write_text('{broken json', encoding='utf-8')
        with self.assertRaises(ValueError):
            control.calibrate_accept(layout_dir=self.layout_dir, running_version=self.version)
        self.assertEqual(active_path.read_text(encoding='utf-8'), original_content)

        # 2. Oversized file (>256KB)
        huge_candidate = json.loads(original_content)
        huge_candidate['extra_padding'] = 'x' * (300 * 1024)
        cand_json_path.write_text(json.dumps(huge_candidate), encoding='utf-8')
        with self.assertRaises(ValueError):
            control.calibrate_accept(layout_dir=self.layout_dir, running_version=self.version)
        self.assertEqual(active_path.read_text(encoding='utf-8'), original_content)

    def test_cli_calibrate_accept(self):
        """control.main(['calibrate', '--accept', ...]) runs correctly from CLI."""
        active_path = self.layout_dir / f'wechat-{self.version}.json'
        active_path.write_text(json.dumps(self.base_layout), encoding='utf-8')
        cand_json_path = self.layout_dir / f'candidate-wechat-{self.version}.json'
        cand_data = json.loads(json.dumps(self.base_layout))
        cand_data['regions']['input']['y'] = 490.0
        cand_json_path.write_text(json.dumps(cand_data), encoding='utf-8')

        with patch.dict(os.environ, {'WECHAT_VERSION': self.version}):
            import io
            from contextlib import redirect_stdout
            f = io.StringIO()
            with redirect_stdout(f):
                control.main(['calibrate', '--accept', '--layout-dir', str(self.layout_dir)])
            output = f.getvalue()
            res = json.loads(output)
            self.assertTrue(res.get('accepted'))
            self.assertEqual(res.get('version'), self.version)

    def test_calibrate_never_clicks(self):
        """Worker calibrate branch never calls botActions, click, rightClick, or key events."""
        worker_swift = (ROOT / 'native' / 'bot' / 'Worker.swift').read_text(encoding='utf-8')

        # Locate the calibrate operation block in Worker.swift
        self.assertIn('operation == "calibrate"', worker_swift, "Worker.swift must have 'operation == \"calibrate\"' handling")

        # Extract the calibrate block
        calibrate_match = re.search(r'if\s+operation\s*==\s*"calibrate"\s*\{([\s\S]*?)\n\s*\}', worker_swift)
        self.assertIsNotNone(calibrate_match, "Could not find calibrate branch in Worker.swift")
        calibrate_block = calibrate_match.group(1)

        # Forbidden calls in calibrate branch
        forbidden = [
            'botActions',
            'click(',
            'rightClick(',
            'key(',
            'act(',
            'scroll(',
        ]
        for term in forbidden:
            self.assertNotIn(
                term,
                calibrate_block,
                f"calibrate operation in Worker.swift must NOT call {term}; must be read-only (zero clicks/keys/actions)"
            )


if __name__ == '__main__':
    unittest.main()
