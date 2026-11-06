"""Tests for control calibrate and calibrate --accept (Stage C Task C4 review fixes).

Covers review issues:
I1: Version & Layout from Worker:
    - Worker calibrate response must include wechat_version, layout_path, layout.
    - Missing version or layout raises/fails without writing any files.
    - No '4.1.15' literal fallback in source code.
    - Version must match regex ^[0-9]+(\\.[0-9]+){1,3}$.
I2: Remote candidate write on WeChat host:
    - Candidate JSON and PNG written to remote host ~/.wechat-rpa/layout/ via injectable remote runner.
    - umask 077 and chmod 600 enforced remotely.
    - shlex.quote used for all remote paths and arguments.
I3: Remote --accept on WeChat host:
    - Remote read of candidate -> local validation -> worker calibrate version verification.
    - Version mismatch or invalid candidate refused with ZERO remote write commands executed.
    - Single atomic remote command executes cp to .bak-<timestamp> then mv candidate to active.
Invariants:
    - Worker calibrate never clicks, scrolls, or types.
"""
import base64
import json
from pathlib import Path
import re
import shlex
import subprocess
import time
import unittest
from unittest.mock import patch, MagicMock

from scripts.wechat_bot import control

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LAYOUT_PATH = ROOT / 'native' / 'layout' / 'wechat-4.1.15.json'


class FakeRemote:
    def __init__(self, initial_files=None):
        self.files = dict(initial_files or {})
        self.commands = []

    def __call__(self, host, command, input_data=None):
        self.commands.append({
            'host': host,
            'command': command,
            'input_data': input_data
        })
        # Simulate writing file: cat > path
        if 'cat > ' in command:
            m = re.search(r'cat\s*>\s*(\S+)', command)
            if m:
                path = m.group(1).strip("'\"")
                self.files[path] = input_data
            return subprocess.CompletedProcess(args=['ssh', host, command], returncode=0, stdout=b'', stderr=b'')

        # Simulate reading file: cat path
        if command.strip().startswith('cat '):
            parts = command.strip().split()
            path = parts[1].strip("'\"")
            if path in self.files:
                data = self.files[path]
                if isinstance(data, str):
                    data = data.encode('utf-8')
                return subprocess.CompletedProcess(args=['ssh', host, command], returncode=0, stdout=data, stderr=b'')
            return subprocess.CompletedProcess(args=['ssh', host, command], returncode=1, stdout=b'', stderr=b'No such file')

        # Simulate atomic backup & mv
        if 'cp ' in command and 'mv ' in command:
            m_cp = re.search(r'cp\s+(\S+)\s+(\S+)', command)
            m_mv = re.search(r'mv\s+(\S+)\s+(\S+)', command)
            if m_cp:
                src, dst = m_cp.group(1).strip("'\""), m_cp.group(2).strip("'\"")
                if src in self.files:
                    self.files[dst] = self.files[src]
            if m_mv:
                src, dst = m_mv.group(1).strip("'\""), m_mv.group(2).strip("'\"")
                if src in self.files:
                    self.files[dst] = self.files.pop(src)
            return subprocess.CompletedProcess(args=['ssh', host, command], returncode=0, stdout=b'', stderr=b'')

        return subprocess.CompletedProcess(args=['ssh', host, command], returncode=0, stdout=b'', stderr=b'')


class CalibrateTests(unittest.TestCase):
    def setUp(self):
        with open(DEFAULT_LAYOUT_PATH, 'r', encoding='utf-8') as f:
            self.base_layout = json.load(f)
        self.version = self.base_layout['wechat_version']

    # --- I1: Version & Layout from Worker ---

    def test_no_4_1_15_fallback_in_source(self):
        """Source code in control.py and Worker.swift must NOT contain hardcoded '4.1.15' fallback."""
        control_source = (ROOT / 'scripts' / 'wechat_bot' / 'control.py').read_text(encoding='utf-8')
        worker_source = (ROOT / 'native' / 'bot' / 'Worker.swift').read_text(encoding='utf-8')

        self.assertNotIn('4.1.15', control_source, "control.py must not hardcode '4.1.15' fallback")
        self.assertNotIn('4.1.15', worker_source, "Worker.swift must not hardcode '4.1.15' fallback")
        self.assertFalse(hasattr(control, 'get_running_wechat_version'), "get_running_wechat_version must be deleted")

    def test_version_regex_validation(self):
        """Version must match regex ^[0-9]+(\\.[0-9]+){1,3}$ before being used in paths or commands."""
        valid_versions = ['4.1.15', '4.0.0', '1.0', '1.2.3.4']
        invalid_versions = ['4.1.15;rm -rf /', 'v4.1', 'beta', '', '4.', '.1', '4.1.15.1.2']

        for v in valid_versions:
            self.assertTrue(control.validate_version_string(v), f"Expected {v} to be valid")
        for inv in invalid_versions:
            self.assertFalse(control.validate_version_string(inv), f"Expected {inv} to be invalid")

    def test_worker_missing_version_aborts_without_writing(self):
        """If worker calibrate response lacks wechat_version or layout, control aborts without writing."""
        fake_remote = FakeRemote()
        bad_worker_result = {
            'measured': {'input_divider': 490.0},
            # missing wechat_version and layout
        }

        with self.assertRaises(ValueError):
            control.run_calibrate(
                worker_result=bad_worker_result,
                host='wechat-host',
                remote_runner=fake_remote
            )
        # Assert no remote commands were executed to write files
        write_commands = [c for c in fake_remote.commands if 'cat > ' in c['command']]
        self.assertEqual(write_commands, [])

    def test_worker_calibrate_response_fields_in_swift(self):
        """Worker.swift calibrate operation must populate wechat_version, layout_path, layout."""
        worker_swift = (ROOT / 'native' / 'bot' / 'Worker.swift').read_text(encoding='utf-8')
        calibrate_match = re.search(r'if\s+operation\s*==\s*"calibrate"\s*\{([\s\S]*?)guard\s+let\s+recipient', worker_swift)
        self.assertIsNotNone(calibrate_match, "Worker.swift must have calibrate operation")
        calibrate_block = calibrate_match.group(1)

        self.assertIn('"wechat_version"', calibrate_block)
        self.assertIn('"layout_path"', calibrate_block)
        self.assertIn('"layout"', calibrate_block)
        self.assertIn('layout_missing', calibrate_block)

    # --- I2: Remote candidate write on WeChat host ---

    def test_candidate_written_to_remote_wechat_host(self):
        """Candidate layout and PNG must be written to remote host ~/.wechat-rpa/layout/ with umask 077 and 600."""
        fake_remote = FakeRemote()
        worker_result = {
            'wechat_version': self.version,
            'layout_path': f'/path/to/.wechat-rpa/layout/wechat-{self.version}.json',
            'layout': self.base_layout,
            'measured': {'input_divider': 490.0},
            'annotated_png_base64': base64.b64encode(b'annotated-bytes').decode('ascii'),
        }

        diffs, missing, candidate = control.run_calibrate(
            worker_result=worker_result,
            host='wechat-host',
            layout_dir='~/.wechat-rpa/layout',
            remote_runner=fake_remote
        )

        # 1. Derived values in candidate
        self.assertEqual(candidate['regions']['input']['y'], 490.0)

        # 2. Assert fake_remote received commands
        json_cand_path = f"~/.wechat-rpa/layout/candidate-wechat-{self.version}.json"
        png_cand_path = f"~/.wechat-rpa/layout/candidate-wechat-{self.version}.png"

        self.assertIn(json_cand_path, fake_remote.files)
        self.assertIn(png_cand_path, fake_remote.files)
        self.assertEqual(fake_remote.files[png_cand_path], b'annotated-bytes')

        # 3. Assert permissions and umask
        for call in fake_remote.commands:
            cmd = call['command']
            self.assertIn('umask 077', cmd)
            self.assertIn('chmod 600', cmd)
            self.assertEqual(call['host'], 'wechat-host')

    # --- I3: Remote --accept on WeChat host ---

    def test_accept_refuses_version_mismatch_with_zero_remote_writes(self):
        """--accept with candidate version mismatching current running version makes NO remote writes."""
        cand_path = f"~/.wechat-rpa/layout/candidate-wechat-{self.version}.json"
        fake_remote = FakeRemote(initial_files={
            cand_path: json.dumps(self.base_layout).encode('utf-8')
        })

        # Worker reports running version 4.1.16 while candidate is 4.1.15
        worker_mock = MagicMock(return_value={
            'wechat_version': '4.1.16',
            'layout': self.base_layout,
        })

        with self.assertRaises(ValueError):
            control.calibrate_accept(
                host='wechat-host',
                layout_dir='~/.wechat-rpa/layout',
                worker_func=worker_mock,
                remote_runner=fake_remote
            )

        # Assert zero write commands (no cp, mv, cat >, or rm)
        write_cmds = [c['command'] for c in fake_remote.commands if any(op in c['command'] for op in ['cp ', 'mv ', 'cat >', 'rm '])]
        self.assertEqual(write_cmds, [])

    def test_accept_refuses_corrupt_candidate_with_zero_remote_writes(self):
        """--accept with corrupt candidate JSON makes NO remote writes."""
        cand_path = f"~/.wechat-rpa/layout/candidate-wechat-{self.version}.json"
        fake_remote = FakeRemote(initial_files={
            cand_path: b'{broken json'
        })
        worker_mock = MagicMock(return_value={
            'wechat_version': self.version,
            'layout': self.base_layout,
        })

        with self.assertRaises(ValueError):
            control.calibrate_accept(
                host='wechat-host',
                layout_dir='~/.wechat-rpa/layout',
                worker_func=worker_mock,
                remote_runner=fake_remote
            )

        write_cmds = [c['command'] for c in fake_remote.commands if any(op in c['command'] for op in ['cp ', 'mv ', 'cat >', 'rm '])]
        self.assertEqual(write_cmds, [])

    def test_accept_executes_backup_then_mv_atomically(self):
        """--accept executes single atomic command: backup current file then mv candidate to active."""
        cand_path = f"~/.wechat-rpa/layout/candidate-wechat-{self.version}.json"
        active_path = f"~/.wechat-rpa/layout/wechat-{self.version}.json"

        updated_layout = json.loads(json.dumps(self.base_layout))
        updated_layout['regions']['input']['y'] = 492.0
        updated_layout['regions']['input']['h'] = 148.0
        updated_layout['regions']['chat']['h'] = 432.0

        fake_remote = FakeRemote(initial_files={
            cand_path: json.dumps(updated_layout).encode('utf-8'),
            active_path: json.dumps(self.base_layout).encode('utf-8')
        })
        worker_mock = MagicMock(return_value={
            'wechat_version': self.version,
            'layout': self.base_layout,
        })

        result = control.calibrate_accept(
            host='wechat-host',
            layout_dir='~/.wechat-rpa/layout',
            worker_func=worker_mock,
            remote_runner=fake_remote
        )

        self.assertTrue(result['accepted'])
        # Find the atomic command
        atomic_cmds = [c['command'] for c in fake_remote.commands if 'cp ' in c['command'] and 'mv ' in c['command']]
        self.assertEqual(len(atomic_cmds), 1, f"Expected 1 atomic cp+mv command, got: {atomic_cmds}")

        cmd = atomic_cmds[0]
        # Must do cp before mv
        cp_pos = cmd.find('cp ')
        mv_pos = cmd.find('mv ')
        self.assertTrue(0 <= cp_pos < mv_pos, f"cp must precede mv in command: {cmd}")
        self.assertIn('.bak-', cmd)
        self.assertIn('chmod 600', cmd)

    def test_cli_calibrate_command(self):
        """control.main(['calibrate', ...]) runs correctly with fake remote."""
        fake_remote = FakeRemote()
        worker_result = {
            'wechat_version': self.version,
            'layout_path': f'/path/to/.wechat-rpa/layout/wechat-{self.version}.json',
            'layout': self.base_layout,
            'measured': {'input_divider': 490.0},
            'annotated_png_base64': base64.b64encode(b'png').decode('ascii'),
        }
        import tempfile
        tmp_dir = tempfile.TemporaryDirectory()
        state_dir = Path(tmp_dir.name)
        def fake_write_private(path, val):
            if path.name == 'calibrate-request.json':
                diffs, missing, candidate = control.run_calibrate(
                    config={'host': 'wechat-mac'}, worker_result=worker_result,
                    layout_dir='~/.wechat-rpa/layout', remote_runner=fake_remote
                )
                version = candidate.get('wechat_version')
                res_path = path.with_name('calibrate-result.json')
                res_path.write_text(json.dumps({
                    'id': val['id'],
                    'ok': True,
                    'result': {
                        'candidate_json': f"~/.wechat-rpa/layout/candidate-wechat-{version}.json",
                        'candidate_png': f"~/.wechat-rpa/layout/candidate-wechat-{version}.png",
                        'differences': diffs,
                        'missing': missing,
                    }
                }))
        with patch('scripts.wechat_bot.control.write_private', side_effect=fake_write_private), \
             patch('scripts.wechat_bot.control.loaded', return_value=True), \
             patch('scripts.wechat_bot.control.load_config', return_value={'host': 'wechat-mac', 'state_dir': str(state_dir)}):
            import io
            from contextlib import redirect_stdout
            out = io.StringIO()
            with redirect_stdout(out):
                control.main(['calibrate', '--config', __file__, '--layout-dir', '~/.wechat-rpa/layout'])
            res = json.loads(out.getvalue().splitlines()[-1])
            self.assertIn('candidate_json', res)
            self.assertEqual(len(res['differences']), 4)
        tmp_dir.cleanup()

    def test_cli_calibrate_accept(self):
        """control.main(['calibrate', '--accept', ...]) runs correctly with fake remote."""
        cand_path = f"~/.wechat-rpa/layout/candidate-wechat-{self.version}.json"
        active_path = f"~/.wechat-rpa/layout/wechat-{self.version}.json"
        fake_remote = FakeRemote(initial_files={
            cand_path: json.dumps(self.base_layout).encode('utf-8'),
            active_path: json.dumps(self.base_layout).encode('utf-8')
        })
        worker_result = {
            'wechat_version': self.version,
            'layout': self.base_layout,
        }
        import tempfile
        tmp_dir = tempfile.TemporaryDirectory()
        state_dir = Path(tmp_dir.name)
        def fake_write_private(path, val):
            if path.name == 'calibrate-request.json':
                accept_res = control.calibrate_accept(
                    host='wechat-mac', layout_dir='~/.wechat-rpa/layout',
                    config={'host': 'wechat-mac'}, worker_func=lambda: worker_result,
                    remote_runner=fake_remote
                )
                res_path = path.with_name('calibrate-result.json')
                res_path.write_text(json.dumps({
                    'id': val['id'],
                    'ok': True,
                    'result': accept_res
                }))
        with patch('scripts.wechat_bot.control.write_private', side_effect=fake_write_private), \
             patch('scripts.wechat_bot.control.loaded', return_value=True), \
             patch('scripts.wechat_bot.control.load_config', return_value={'host': 'wechat-mac', 'state_dir': str(state_dir)}):
            import io
            from contextlib import redirect_stdout
            out = io.StringIO()
            with redirect_stdout(out):
                control.main(['calibrate', '--accept', '--config', __file__, '--layout-dir', '~/.wechat-rpa/layout'])
            res = json.loads(out.getvalue().strip())
            self.assertTrue(res.get('accepted'))
            self.assertEqual(res.get('version'), self.version)
        tmp_dir.cleanup()

    # --- Invariants ---

    def test_calibrate_never_clicks(self):
        """Worker calibrate branch never calls botActions, click, rightClick, or key events."""
        worker_swift = (ROOT / 'native' / 'bot' / 'Worker.swift').read_text(encoding='utf-8')
        calibrate_match = re.search(r'if\s+operation\s*==\s*"calibrate"\s*\{([\s\S]*?)guard\s+let\s+recipient', worker_swift)
        self.assertIsNotNone(calibrate_match, "Worker.swift must have calibrate operation")
        calibrate_block = calibrate_match.group(1)

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
                f"calibrate operation in Worker.swift must NOT call {term}; must be read-only"
            )


if __name__ == '__main__':
    unittest.main()


class RealShellRemote:
    """Runs remote commands with bash in a fake home, the way ssh runs them in the remote home."""
    def __init__(self, home):
        self.home = Path(home)
        self.commands = []

    def __call__(self, host, command, input_data=None):
        self.commands.append(command)
        return subprocess.run(['bash', '-c', command], cwd=self.home, input=input_data, capture_output=True,
                              env={'HOME': str(self.home), 'PATH': '/usr/bin:/bin'})


class CalibrateOnDeviceTests(unittest.TestCase):
    """Integration review of C4: defaults must work through a real shell on the WeChat Mac."""
    def setUp(self):
        import tempfile
        self.tmp = tempfile.TemporaryDirectory()
        self.home = Path(self.tmp.name)
        layout = json.loads(DEFAULT_LAYOUT_PATH.read_text())
        self.layout_dir = self.home / '.wechat-rpa' / 'layout'
        self.layout_dir.mkdir(parents=True)
        (self.layout_dir / 'wechat-4.1.15.json').write_text(json.dumps(layout))
        self.worker = {'wechat_version': '4.1.15', 'layout': layout, 'layout_path': 'x',
                       'measured': {'input_divider': 500, 'list_border': 300}, 'scale': 1,
                       'annotated_png_base64': base64.b64encode(b'png').decode()}
        self.remote = RealShellRemote(self.home)

    def tearDown(self):
        self.tmp.cleanup()

    def test_default_paths_land_in_remote_home(self):
        control.run_calibrate(config={'host': 'wechat-mac'}, worker_result=self.worker, remote_runner=self.remote)
        candidate = self.layout_dir / 'candidate-wechat-4.1.15.json'
        self.assertTrue(candidate.exists(), self.remote.commands)
        self.assertEqual(json.loads(candidate.read_text())['regions']['input']['y'], 500)
        self.assertEqual(candidate.stat().st_mode & 0o777, 0o600)
        self.assertFalse((self.home / '~').exists())

    def test_accept_with_defaults_backs_up_then_replaces(self):
        control.run_calibrate(config={'host': 'wechat-mac'}, worker_result=self.worker, remote_runner=self.remote)
        result = control.calibrate_accept(config={'host': 'wechat-mac'}, worker_func=lambda: self.worker,
                                          remote_runner=self.remote)
        self.assertTrue(result['accepted'])
        active = json.loads((self.layout_dir / 'wechat-4.1.15.json').read_text())
        self.assertEqual(active['regions']['input']['y'], 500)
        backups = list(self.layout_dir.glob('wechat-4.1.15.json.bak-*'))
        self.assertEqual(len(backups), 1)
        self.assertEqual(json.loads(backups[0].read_text())['regions']['input']['y'], 496)

    def test_missing_host_is_refused(self):
        with self.assertRaises(ValueError):
            control.run_calibrate(config={}, worker_result=self.worker, remote_runner=self.remote)
        self.assertEqual(self.remote.commands, [])

    def test_failed_remote_write_raises(self):
        failing = lambda host, command, input_data=None: subprocess.CompletedProcess(command, 1, b'', b'denied')
        with self.assertRaises(RuntimeError):
            control.run_calibrate(config={'host': 'wechat-mac'}, worker_result=self.worker, remote_runner=failing)

    def test_worker_probe_names_a_recipient(self):
        seen = {}
        class FakeCapture:
            def __init__(self, host, client, recipient, *args, names=None, **kwargs):
                seen.update(host=host, recipient=recipient, names=names)
            async def connect(self): pass
            async def calibrate(self): return {'ok': True}
            async def close_transport(self): pass
        with patch('scripts.wechat_bot.native_capture.NativeCapture', FakeCapture):
            control.run_worker_calibrate(config={'host': 'wechat-mac'})
        self.assertEqual(seen['host'], 'wechat-mac')
        self.assertTrue(seen['recipient'])

    def test_calibrate_refuses_while_bot_running(self):
        args = MagicMock(accept=False, layout_dir=None, config=str(self.home / 'none.json'))
        with patch('scripts.wechat_bot.control.loaded', return_value=False):
            with self.assertRaises(SystemExit):
                control.calibrate_command(args, remote_runner=self.remote, worker_func=lambda: self.worker)
        self.assertEqual(self.remote.commands, [])

    def test_control_calibrate_round_trip(self):
        """control calibrate delegates to running bot by writing request and polling result."""
        state_dir = self.home / 'state'
        state_dir.mkdir(parents=True, exist_ok=True)
        config_path = self.home / 'config.json'
        config_data = {
            'schema_version': 1,
            'host': 'wechat-mac',
            'account_id': 'fixture',
            'state_dir': str(state_dir),
            'allowlist': [{'recipient': 'a', 'kind': 'private'}],
            'ignorelist': [],
            'poll_seconds': 1,
        }
        config_path.write_text(json.dumps(config_data))

        import threading
        def fake_bot_worker():
            req_path = state_dir / 'calibrate-request.json'
            res_path = state_dir / 'calibrate-result.json'
            for _ in range(50):
                if req_path.exists():
                    try:
                        req = json.loads(req_path.read_text())
                        res_path.write_text(json.dumps({
                            'id': req['id'],
                            'ok': True,
                            'result': {
                                'candidate_json': '.wechat-rpa/layout/candidate-wechat-4.1.15.json',
                                'candidate_png': '.wechat-rpa/layout/candidate-wechat-4.1.15.png',
                                'differences': [{'field': 'input.y', 'old': 496, 'new': 500}],
                                'missing': []
                            }
                        }))
                        req_path.unlink()
                        return
                    except Exception:
                        pass
                time.sleep(0.05)

        t = threading.Thread(target=fake_bot_worker)
        t.start()

        args = MagicMock(accept=False, layout_dir=None, config=str(config_path))
        with patch('scripts.wechat_bot.control.loaded', return_value=True):
            import io
            from contextlib import redirect_stdout
            out = io.StringIO()
            with redirect_stdout(out):
                control.calibrate_command(args)
            t.join(timeout=2)
            res = json.loads(out.getvalue().splitlines()[-1])
            self.assertIn('candidate_json', res)
            self.assertEqual(len(res['differences']), 1)

    def test_control_calibrate_requires_running_bot(self):
        """control calibrate errors out if bot is not running."""
        state_dir = self.home / 'state'
        state_dir.mkdir(parents=True, exist_ok=True)
        config_path = self.home / 'config.json'
        config_data = {
            'schema_version': 1,
            'host': 'wechat-mac',
            'account_id': 'fixture',
            'state_dir': str(state_dir),
            'allowlist': [{'recipient': 'a', 'kind': 'private'}],
            'ignorelist': [],
            'poll_seconds': 1,
        }
        config_path.write_text(json.dumps(config_data))
        args = MagicMock(accept=False, layout_dir=None, config=str(config_path))
        with patch('scripts.wechat_bot.control.loaded', return_value=False):
            import io
            from contextlib import redirect_stderr
            err = io.StringIO()
            with redirect_stderr(err):
                with self.assertRaises(SystemExit) as cm:
                    control.calibrate_command(args)
            self.assertEqual(cm.exception.code, 1)
            self.assertIn('机器人未运行：请先 control start', err.getvalue())


class CalibrateScaleTests(unittest.TestCase):
    def test_anchors_measure_the_raw_capture(self):
        # normalizedFrame returns a points-sized image with the capture's scale; pairing those two
        # would measure Retina frames at half size. Anchors take the raw capture and its scale.
        source = (ROOT / 'native' / 'bot' / 'Worker.swift').read_text()
        branch = source[source.index('if operation == "calibrate"'):]
        branch = branch[:branch.index('return result')]
        self.assertRegex(branch, r'verifyAnchors\(rawImage,\s*scale:')
        self.assertRegex(branch, r'drawAnnotatedLayout\(rawImage,\s*scale:')
