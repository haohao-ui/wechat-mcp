"""Tests for host app Rust core supervisor (serve-core), mutex check, UTF-8 env, backoff, and status panel display."""
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]


class HostCoreTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which('swiftc'):
            raise unittest.SkipTest('swiftc not available for compiling host_selftest')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name) / 'host_selftest_core'

        files_to_compile = [
            ROOT / 'native' / 'bootstrap' / 'Supervisor.swift',
            ROOT / 'native' / 'bootstrap' / 'CoreSupervisor.swift',
            ROOT / 'native' / 'bootstrap' / 'SocketWorker.swift',
            ROOT / 'native' / 'bootstrap' / 'StatusModel.swift',
            ROOT / 'native' / 'bootstrap' / 'ConfigForm.swift',
            ROOT / 'native' / 'bootstrap' / 'Actions.swift',
            ROOT / 'native' / 'eyes' / 'Screens.swift',
            ROOT / 'tests' / 'host_selftest' / 'main.swift',
        ]

        # TDD Step 1: if any required file does not exist yet, we do not compile and let test fail cleanly
        for f in files_to_compile:
            if not f.exists():
                return

        proc = subprocess.run(
            ['swiftc', '-O', '-DHOST_APP_T3', '-DHOST_APP_CORE'] + [str(f) for f in files_to_compile] + ['-o', str(cls.cli)],
            capture_output=True,
            text=True
        )
        if proc.returncode != 0:
            raise RuntimeError(f"swiftc compilation failed:\nSTDOUT:\n{proc.stdout}\nSTDERR:\n{proc.stderr}")

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, 'tmp'):
            cls.tmp.cleanup()

    def run_cmd(self, command, payload):
        if not hasattr(self, 'cli') or not self.cli.exists():
            self.fail("host_selftest_core executable does not exist (TDD step 1)")
        proc = subprocess.run(
            [str(self.cli), command, json.dumps(payload)],
            capture_output=True,
            text=True
        )
        self.assertEqual(proc.returncode, 0, f"host_selftest {command} failed (code {proc.returncode}):\n{proc.stderr}")
        return json.loads(proc.stdout)

    def _default_status_input(self):
        return {
            'permissions': {
                'screen_recording_authorized': True,
                'accessibility_trusted': True
            },
            'desktop_service': {
                'socket_exists': True,
                'status': 'running',
                'pid': 12345
            },
            'bot': {
                'launchd_loaded': False,
                'core_enabled': True,
                'core_status': 'running',
                'core_pid': 54321,
                'health_state': 'watching',
                'health_updated_at': 10000.0,
                'now': 10010.0,
                'needs_recalibration': False
            },
            'sessions': {
                'session_a': {
                    'state': 'watching'
                }
            },
            'wechat': {
                'is_running': True,
                'pid': 8888,
                'version': '4.1.15',
                'main_bounds': {'x': 100, 'y': 100, 'width': 880, 'height': 640},
                'windows': [
                    {'owner': 'WeChat', 'pid': 8888, 'layer': 0, 'on_screen': True,
                     'bounds': {'x': 100, 'y': 100, 'width': 880, 'height': 640}}
                ]
            },
            'layout': {
                'layout_exists': True,
                'version': '4.1.15'
            },
            'disk': {
                'free_bytes': 20 * 1024 * 1024 * 1024,
                'attachment_bytes': 100 * 1024 * 1024
            }
        }

    # ---------------- Pure Decision Logic Tests ----------------

    def test_decision_conflict_when_both_enabled_and_loaded(self):
        """core.enabled 存在且 Python launchd 已加载时，输出 conflict。"""
        res = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': True,
            'process_state': 'not_running'
        })
        self.assertEqual(res['decision'], 'conflict')

        res_running = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': True,
            'process_state': 'running',
            'pid': 1234
        })
        self.assertEqual(res_running['decision'], 'conflict')

    def test_decision_python_mode_stops_running_or_keeps_idle(self):
        """core.enabled 不存在时，若子进程运行中则 stop，否则 keep。"""
        res_running = self.run_cmd('core_decision', {
            'core_enabled': False,
            'python_loaded': False,
            'process_state': 'running',
            'pid': 1234
        })
        self.assertEqual(res_running['decision'], 'stop')

        res_idle = self.run_cmd('core_decision', {
            'core_enabled': False,
            'python_loaded': True,
            'process_state': 'not_running'
        })
        self.assertEqual(res_idle['decision'], 'keep')

    def test_decision_rust_mode_starts_or_keeps(self):
        """core.enabled 存在且 Python launchd 未加载时：未运行则 start，运行中则 keep。"""
        res_not_running = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': False,
            'process_state': 'not_running'
        })
        self.assertEqual(res_not_running['decision'], 'start')

        res_running = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': False,
            'process_state': 'running',
            'pid': 1234
        })
        self.assertEqual(res_running['decision'], 'keep')

    def test_decision_rust_mode_restarting_backoff(self):
        """处于退避中的进程，只有当前时间到达 restarting_at 才 start，否则 keep。"""
        t0 = 1000.0
        res_wait = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': False,
            'process_state': 'restarting',
            'now': t0,
            'restarting_at': t0 + 10.0
        })
        self.assertEqual(res_wait['decision'], 'keep')

        res_ready = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': False,
            'process_state': 'restarting',
            'now': t0 + 15.0,
            'restarting_at': t0 + 10.0
        })
        self.assertEqual(res_ready['decision'], 'start')

    def test_decision_rust_mode_gave_up_and_config_error(self):
        """连续崩溃已放弃（gave_up）或配置错误（config_error）时，不自动重启，输出 keep。"""
        res_gave_up = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': False,
            'process_state': 'gave_up'
        })
        self.assertEqual(res_gave_up['decision'], 'keep')

        res_config_err = self.run_cmd('core_decision', {
            'core_enabled': True,
            'python_loaded': False,
            'process_state': 'config_error'
        })
        self.assertEqual(res_config_err['decision'], 'keep')

    # ---------------- Panel Status Display Tests ----------------

    def test_panel_bot_status_rust_core_running(self):
        """Rust 核心运行中：显示 PID 且为绿色。"""
        inp = self._default_status_input()
        inp['bot']['core_enabled'] = True
        inp['bot']['launchd_loaded'] = False
        inp['bot']['core_status'] = 'running'
        inp['bot']['core_pid'] = 88888

        items = self.run_cmd('status', inp)
        bot_item = next(i for i in items if i['key'] == 'bot')
        self.assertEqual(bot_item['level'], 'green')
        self.assertIn('Rust 核心', bot_item['reason'])
        self.assertIn('88888', bot_item['reason'])

    def test_panel_bot_status_python_mode(self):
        """core.enabled 不存在时显示为 Python 模式。"""
        inp = self._default_status_input()
        inp['bot']['core_enabled'] = False
        inp['bot']['launchd_loaded'] = True
        inp['bot']['health_state'] = 'watching'

        items = self.run_cmd('status', inp)
        bot_item = next(i for i in items if i['key'] == 'bot')
        self.assertEqual(bot_item['level'], 'green')
        self.assertIn('Python 模式', bot_item['reason'])

    def test_panel_bot_status_mutex_conflict(self):
        """两者都要求运行（core.enabled 存在且 launchd 已加载）时，显示互斥冲突且为红色。"""
        inp = self._default_status_input()
        inp['bot']['core_enabled'] = True
        inp['bot']['launchd_loaded'] = True

        items = self.run_cmd('status', inp)
        bot_item = next(i for i in items if i['key'] == 'bot')
        self.assertEqual(bot_item['level'], 'red')
        self.assertIn('互斥冲突', bot_item['reason'])

    def test_panel_bot_status_backoff_and_gave_up(self):
        """退避为黄色，已放弃与配置错误为红色。"""
        # 1. 退避中
        inp_restarting = self._default_status_input()
        inp_restarting['bot']['core_enabled'] = True
        inp_restarting['bot']['launchd_loaded'] = False
        inp_restarting['bot']['core_status'] = 'restarting'
        items = self.run_cmd('status', inp_restarting)
        bot_item = next(i for i in items if i['key'] == 'bot')
        self.assertEqual(bot_item['level'], 'yellow')
        self.assertIn('退避', bot_item['reason'])

        # 2. 已放弃
        inp_gave_up = self._default_status_input()
        inp_gave_up['bot']['core_enabled'] = True
        inp_gave_up['bot']['launchd_loaded'] = False
        inp_gave_up['bot']['core_status'] = 'gave_up'
        items = self.run_cmd('status', inp_gave_up)
        bot_item = next(i for i in items if i['key'] == 'bot')
        self.assertEqual(bot_item['level'], 'red')
        self.assertTrue('上限' in bot_item['reason'] or '已放弃' in bot_item['reason'] or '停止' in bot_item['reason'])

        # 3. 配置错误
        inp_cfg_err = self._default_status_input()
        inp_cfg_err['bot']['core_enabled'] = True
        inp_cfg_err['bot']['launchd_loaded'] = False
        inp_cfg_err['bot']['core_status'] = 'config_error'
        items = self.run_cmd('status', inp_cfg_err)
        bot_item = next(i for i in items if i['key'] == 'bot')
        self.assertEqual(bot_item['level'], 'red')
        self.assertIn('配置错误', bot_item['reason'])

    # ---------------- Subprocess Lifecycle & Env Tests ----------------

    def _create_fake_script(self, body):
        script_file = Path(self.tmp.name) / f"fake_script_{time.time_ns()}.sh"
        script_file.write_text(f"#!/bin/bash\n{body}\n")
        script_file.chmod(0o755)
        return script_file

    def test_supervisor_env_and_log_permissions(self):
        """显式设置 LANG 和 LC_ALL 为 en_US.UTF-8，且 core.log 权限为 0600。"""
        env_dump_path = Path(self.tmp.name) / "env_dump.txt"
        log_path = Path(self.tmp.name) / "core.log"
        fake_script = self._create_fake_script(f"""
            echo "LANG=$LANG" >> "{env_dump_path}"
            echo "LC_ALL=$LC_ALL" >> "{env_dump_path}"
            echo "Rust Core Log Line"
            exit 0
        """)

        res = self.run_cmd('core_supervisor_run', {
            'action': 'run_once',
            'binary_path': str(fake_script),
            'log_path': str(log_path)
        })
        self.assertTrue(res['ok'])
        self.assertEqual(res['exit_code'], 0)

        # 检查环境变量文件
        self.assertTrue(env_dump_path.exists())
        env_content = env_dump_path.read_text()
        self.assertIn("LANG=en_US.UTF-8", env_content)
        self.assertIn("LC_ALL=en_US.UTF-8", env_content)

        # 检查 core.log 内容和权限
        self.assertTrue(log_path.exists())
        log_content = log_path.read_text()
        self.assertIn("Rust Core Log Line", log_content)
        log_mode = stat.S_IMODE(log_path.stat().st_mode)
        self.assertEqual(log_mode, 0o600, f"Expected 0600, got {oct(log_mode)}")

    def test_supervisor_exit_code_0_not_counted_as_crash(self):
        """退出码 0 为正常退出，不计入崩溃，不触发退避。"""
        log_path = Path(self.tmp.name) / "core_0.log"
        fake_script = self._create_fake_script("exit 0")
        res = self.run_cmd('core_supervisor_run', {
            'action': 'run_once',
            'binary_path': str(fake_script),
            'log_path': str(log_path)
        })
        self.assertEqual(res['exit_code'], 0)
        self.assertEqual(res['supervisor_status'], 'not_running')
        self.assertFalse(res['gave_up'])

    def test_supervisor_exit_code_2_config_error_no_restart(self):
        """退出码 2 为配置错误，停止并不重启。"""
        log_path = Path(self.tmp.name) / "core_2.log"
        fake_script = self._create_fake_script("exit 2")
        res = self.run_cmd('core_supervisor_run', {
            'action': 'run_once',
            'binary_path': str(fake_script),
            'log_path': str(log_path)
        })
        self.assertEqual(res['exit_code'], 2)
        self.assertEqual(res['supervisor_status'], 'config_error')
        self.assertFalse(res['gave_up'])

    def test_supervisor_sigterm_graceful_exit(self):
        """向子进程发 SIGTERM 并能优雅退出。"""
        log_path = Path(self.tmp.name) / "core_term.log"
        ready_path = Path(self.tmp.name) / "ready_term.txt"
        fake_script = self._create_fake_script(f"""
            trap 'echo "gracefully exiting on term"; exit 0' TERM
            touch "{ready_path}"
            while true; do
                sleep 0.05
            done
        """)

        res = self.run_cmd('core_supervisor_run', {
            'action': 'start_and_stop',
            'binary_path': str(fake_script),
            'log_path': str(log_path),
            'ready_file': str(ready_path),
            'grace_seconds': 5.0
        })
        self.assertTrue(res['ok'])
        self.assertEqual(res['stopped_by'], 'sigterm')

        time.sleep(0.1)
        log_content = log_path.read_text()
        self.assertIn("gracefully exiting on term", log_content)

    def test_supervisor_sigkill_on_timeout(self):
        """子进程忽略 SIGTERM，超过宽限期后被 SIGKILL 强杀。"""
        log_path = Path(self.tmp.name) / "core_kill.log"
        ready_path = Path(self.tmp.name) / "ready_kill.txt"
        fake_script = self._create_fake_script(f"""
            trap '' TERM
            touch "{ready_path}"
            while true; do
                sleep 0.05
            done
        """)

        res = self.run_cmd('core_supervisor_run', {
            'action': 'start_and_stop',
            'binary_path': str(fake_script),
            'log_path': str(log_path),
            'ready_file': str(ready_path),
            'grace_seconds': 0.3  # 短超时测试
        })
        self.assertTrue(res['ok'])
        self.assertEqual(res['stopped_by'], 'sigkill')


if __name__ == '__main__':
    unittest.main()
