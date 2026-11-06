"""Tests for host app supervisor backoff, Info.plist, LaunchAgent, and deploy script."""
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class HostSupervisorLogicTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which('swiftc'):
            raise unittest.SkipTest('swiftc not available for compiling host_selftest')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name) / 'host_selftest'
        supervisor_swift = ROOT / 'native' / 'bootstrap' / 'Supervisor.swift'
        selftest_swift = ROOT / 'tests' / 'host_selftest' / 'main.swift'

        if not supervisor_swift.exists() or not selftest_swift.exists():
            # If files do not exist yet (TDD Step 1), we don't compile and let tests fail on missing binary
            return

        proc = subprocess.run(
            ['swiftc', '-O', str(supervisor_swift), str(selftest_swift), '-o', str(cls.cli)],
            capture_output=True,
            text=True
        )
        if proc.returncode != 0:
            raise RuntimeError(f"swiftc compilation failed:\nSTDOUT:\n{proc.stdout}\nSTDERR:\n{proc.stderr}")

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, 'tmp'):
            cls.tmp.cleanup()

    def run_policy_actions(self, actions):
        if not hasattr(self, 'cli') or not self.cli.exists():
            self.fail("host_selftest executable does not exist (TDD step 1)")
        proc = subprocess.run(
            [str(self.cli), 'restart_policy', json.dumps({'actions': actions})],
            capture_output=True,
            text=True
        )
        self.assertEqual(proc.returncode, 0, f"host_selftest stderr: {proc.stderr}")
        return json.loads(proc.stdout)

    def run_child_exit_events(self, events):
        if not hasattr(self, 'cli') or not self.cli.exists():
            self.fail("host_selftest executable does not exist (TDD step 1)")
        proc = subprocess.run(
            [str(self.cli), 'child_exit', json.dumps({'events': events})],
            capture_output=True,
            text=True
        )
        self.assertEqual(proc.returncode, 0, f"host_selftest stderr: {proc.stderr}")
        return json.loads(proc.stdout)

    def test_clean_exits_do_not_trigger_give_up_and_abnormal_exits_give_up(self):
        """Six clean exits within an hour leave host accepting connections; five abnormal exits give up."""
        t0 = 10000.0
        # 1. Six clean exits (status 0, reason exit) within an hour
        clean_events = [
            {'status': 0, 'reason': 'exit', 'epoch': t0 + i * 300}
            for i in range(6)
        ]
        res = self.run_child_exit_events(clean_events)
        self.assertFalse(res['is_given_up'], "Host must NOT give up on clean client disconnects")
        self.assertTrue(res['accepting_connections'], "Host must keep accepting connections after clean disconnects")
        self.assertEqual(res['status']['state'], 'stopped')
        self.assertEqual(res['status']['reason'], '等待客户端连接')

        # 2. Five abnormal exits (status != 0 or uncaught signal) trigger giveUp
        abnormal_events = [
            {'status': 1, 'reason': 'exit', 'epoch': t0 + i * 10}
            for i in range(5)
        ]
        res = self.run_child_exit_events(abnormal_events)
        self.assertTrue(res['is_given_up'], "Host must give up after 5 abnormal exits in an hour")
        self.assertFalse(res['accepting_connections'], "Host must refuse connections once given up")
        self.assertEqual(res['status']['state'], 'stopped')
        self.assertIn('上限', res['status']['reason'])

        # 3. reset/restartChild clears given_up
        reset_events = abnormal_events + [{'op': 'reset'}]
        res_reset = self.run_child_exit_events(reset_events)
        self.assertFalse(res_reset['is_given_up'], "reset must clear given_up flag")
        self.assertTrue(res_reset['accepting_connections'])

    def test_backoff_sequence(self):
        """Backoff intervals should be 5s, 15s, 60s, 60s."""
        t0 = 10000.0
        actions = [
            {'op': 'exit', 'epoch': t0 + 0},
            {'op': 'exit', 'epoch': t0 + 10},
            {'op': 'exit', 'epoch': t0 + 30},
            {'op': 'exit', 'epoch': t0 + 100},
        ]
        res = self.run_policy_actions(actions)
        decisions = res['results']
        self.assertEqual(len(decisions), 4)
        self.assertEqual(decisions[0], {'decision': 'restart', 'after': 5.0})
        self.assertEqual(decisions[1], {'decision': 'restart', 'after': 15.0})
        self.assertEqual(decisions[2], {'decision': 'restart', 'after': 60.0})
        self.assertEqual(decisions[3], {'decision': 'restart', 'after': 60.0})

    def test_give_up_after_five_in_an_hour(self):
        """Give up on the 5th exit within a rolling 1-hour window."""
        t0 = 10000.0
        actions = [
            {'op': 'exit', 'epoch': t0 + 0},
            {'op': 'exit', 'epoch': t0 + 10},
            {'op': 'exit', 'epoch': t0 + 30},
            {'op': 'exit', 'epoch': t0 + 100},
            {'op': 'exit', 'epoch': t0 + 200},
        ]
        res = self.run_policy_actions(actions)
        decisions = res['results']
        self.assertEqual(len(decisions), 5)
        self.assertEqual(decisions[4], {'decision': 'give_up'})

    def test_window_rolls(self):
        """Exits older than 1 hour roll off and do not count toward the 5-exit limit."""
        t0 = 10000.0
        actions = [
            {'op': 'exit', 'epoch': t0 + 0},
            {'op': 'exit', 'epoch': t0 + 10},
            {'op': 'exit', 'epoch': t0 + 20},
            {'op': 'exit', 'epoch': t0 + 30},
            # After 4000 seconds, all 4 previous exits are > 1 hour (3600s) ago
            {'op': 'exit', 'epoch': t0 + 4000},
        ]
        res = self.run_policy_actions(actions)
        decisions = res['results']
        self.assertEqual(len(decisions), 5)
        # 5th exit is effectively the 1st exit in the new 1-hour window
        self.assertEqual(decisions[4], {'decision': 'restart', 'after': 5.0})

    def test_reset_clears(self):
        """reset() clears all exit history and restarts backoff from the beginning."""
        t0 = 10000.0
        actions = [
            {'op': 'exit', 'epoch': t0 + 0},
            {'op': 'exit', 'epoch': t0 + 10},
            {'op': 'reset'},
            {'op': 'exit', 'epoch': t0 + 20},
        ]
        res = self.run_policy_actions(actions)
        decisions = res['results']
        # 2 exits, 1 reset, 1 exit -> results should have 3 decision entries
        self.assertEqual(len(decisions), 3)
        self.assertEqual(decisions[0], {'decision': 'restart', 'after': 5.0})
        self.assertEqual(decisions[1], {'decision': 'restart', 'after': 15.0})
        self.assertEqual(decisions[2], {'decision': 'restart', 'after': 5.0})


class HostAppConfigurationTests(unittest.TestCase):
    def test_info_plist_names(self):
        """Info.plist must match Global Constraints & spec 5.1."""
        plist_path = ROOT / 'native' / 'bootstrap' / 'Info.plist'
        self.assertTrue(plist_path.exists(), "Info.plist must exist")
        with open(plist_path, 'rb') as f:
            data = plistlib.load(f)

        self.assertEqual(data.get('CFBundleName'), '微信客服助手')
        self.assertEqual(data.get('CFBundleDisplayName'), '微信客服助手')
        self.assertEqual(data.get('CFBundleIdentifier'), 'local.wechat-mcp.host')
        self.assertEqual(data.get('CFBundleExecutable'), 'wechat-cs-host')

    def test_launchagent_keepalive_only_on_crash(self):
        """LaunchAgent must restart only on abnormal exit and throttle 30s."""
        plist_path = ROOT / 'native' / 'bootstrap' / 'local.wechat-mcp.host.plist'
        self.assertTrue(plist_path.exists(), "LaunchAgent plist template must exist")
        with open(plist_path, 'rb') as f:
            data = plistlib.load(f)

        self.assertEqual(data.get('Label'), 'local.wechat-mcp.host')
        self.assertTrue(data.get('RunAtLoad'))
        self.assertEqual(data.get('ThrottleInterval'), 30)
        self.assertEqual(data.get('KeepAlive'), {'SuccessfulExit': False})
        args = data.get('ProgramArguments', [])
        self.assertTrue(len(args) >= 1)
        self.assertTrue(args[0].endswith('微信客服助手.app/Contents/MacOS/wechat-cs-host'))


class DeployScriptTests(unittest.TestCase):
    def test_deploy_script_dry_run_by_default(self):
        """deploy_host.sh without --yes must be dry-run and invoke no ssh/scp."""
        script_path = ROOT / 'scripts' / 'deploy_host.sh'
        self.assertTrue(script_path.exists(), "deploy_host.sh must exist")
        self.assertTrue(os.access(script_path, os.X_OK), "deploy_host.sh must be executable")

        with tempfile.TemporaryDirectory() as fake_bin:
            fake_ssh = Path(fake_bin) / 'ssh'
            fake_scp = Path(fake_bin) / 'scp'
            for cmd in (fake_ssh, fake_scp):
                cmd.write_text('#!/bin/sh\necho "FORBIDDEN_CALL: $0 $*" >&2\nexit 99\n')
                cmd.chmod(0o755)

            env = dict(os.environ)
            env['PATH'] = f"{fake_bin}:{env.get('PATH', '')}"

            proc = subprocess.run(
                [str(script_path), 'fake-host'],
                capture_output=True,
                text=True,
                env=env
            )
            self.assertEqual(proc.returncode, 0, f"dry-run should succeed with exit 0, got {proc.returncode}:\n{proc.stderr}")
            self.assertNotIn("FORBIDDEN_CALL", proc.stderr)
            self.assertIn("dry-run", proc.stdout.lower())

    def test_deploy_script_has_no_pkill(self):
        """deploy_host.sh must not use pkill anywhere; restart via launchctl kickstart -k."""
        script_path = ROOT / 'scripts' / 'deploy_host.sh'
        self.assertTrue(script_path.exists(), "deploy_host.sh must exist")
        content = script_path.read_text(encoding='utf-8')
        self.assertNotIn('pkill', content, "deploy_host.sh must never use pkill")
