"""Tests for host app status evaluation, config form, actions, and diagnostics."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]


class HostStatusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which('swiftc'):
            raise unittest.SkipTest('swiftc not available for compiling host_selftest')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name) / 'host_selftest_t3'

        files_to_compile = [
            ROOT / 'native' / 'bootstrap' / 'Supervisor.swift',
            ROOT / 'native' / 'bootstrap' / 'SocketWorker.swift',
            ROOT / 'native' / 'bootstrap' / 'StatusModel.swift',
            ROOT / 'native' / 'bootstrap' / 'ConfigForm.swift',
            ROOT / 'native' / 'bootstrap' / 'Actions.swift',
            ROOT / 'native' / 'eyes' / 'Screens.swift',
            ROOT / 'tests' / 'host_selftest' / 'main.swift',
        ]

        # In TDD Step 1, if any required file does not exist yet, we do not compile and fail cleanly.
        for f in files_to_compile:
            if not f.exists():
                return

        proc = subprocess.run(
            ['swiftc', '-O', '-DHOST_APP_T3'] + [str(f) for f in files_to_compile] + ['-o', str(cls.cli)],
            capture_output=True,
            text=True
        )
        if proc.returncode != 0:
            raise RuntimeError(f"swiftc compilation failed:\nSTDOUT:\n{proc.stdout}\nSTDERR:\n{proc.stderr}")

    @classmethod
    def tearDownClass(cls):
        if hasattr(cls, 'tmp'):
            cls.tmp.cleanup()

    def run_status_eval(self, payload):
        if not hasattr(self, 'cli') or not self.cli.exists():
            self.fail("host_selftest_t3 executable does not exist (TDD step 1)")
        proc = subprocess.run(
            [str(self.cli), 'status', json.dumps(payload)],
            capture_output=True,
            text=True
        )
        self.assertEqual(proc.returncode, 0, f"host_selftest stderr: {proc.stderr}")
        return json.loads(proc.stdout)

    def run_config_action(self, payload):
        if not hasattr(self, 'cli') or not self.cli.exists():
            self.fail("host_selftest_t3 executable does not exist (TDD step 1)")
        proc = subprocess.run(
            [str(self.cli), 'config', json.dumps(payload)],
            capture_output=True,
            text=True
        )
        self.assertEqual(proc.returncode, 0, f"host_selftest stderr: {proc.stderr}")
        return json.loads(proc.stdout)

    def run_diagnostics(self, payload):
        if not hasattr(self, 'cli') or not self.cli.exists():
            self.fail("host_selftest_t3 executable does not exist (TDD step 1)")
        proc = subprocess.run(
            [str(self.cli), 'diagnostics', json.dumps(payload)],
            capture_output=True,
            text=True
        )
        self.assertEqual(proc.returncode, 0, f"host_selftest stderr: {proc.stderr}")
        return json.loads(proc.stdout)

    def _default_input(self):
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
                'launchd_loaded': True,
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

    def test_status_stale_health_red(self):
        """health 超过 120 秒未更新为红。"""
        inp = self._default_input()
        inp['bot']['health_updated_at'] = 10000.0
        inp['bot']['now'] = 10125.0  # 125s passed (> 120s)

        items = self.run_status_eval(inp)
        item_dict = {i['key']: i for i in items}
        self.assertIn('bot', item_dict)
        self.assertEqual(item_dict['bot']['level'], 'red')
        self.assertIn('120', item_dict['bot']['reason'])

    def test_status_recalibration_red(self):
        """needs_recalibration 为红。"""
        inp = self._default_input()
        inp['bot']['health_state'] = 'needs_recalibration'

        items = self.run_status_eval(inp)
        item_dict = {i['key']: i for i in items}
        self.assertIn('bot', item_dict)
        self.assertEqual(item_dict['bot']['level'], 'red')
        self.assertIn('校准', item_dict['bot']['reason'])

    def test_status_waiting_session_yellow_names_code(self):
        """任何会话处于 waiting_for_wechat_window 为黄，原因带上会话名和错误码。"""
        inp = self._default_input()
        inp['sessions']['user_vip'] = {
            'state': 'waiting_for_wechat_window',
            'code': 'screen_not_main:other_wechat_window'
        }

        items = self.run_status_eval(inp)
        item_dict = {i['key']: i for i in items}
        self.assertIn('sessions', item_dict)
        self.assertEqual(item_dict['sessions']['level'], 'yellow')
        self.assertIn('user_vip', item_dict['sessions']['reason'])
        self.assertIn('screen_not_main:other_wechat_window', item_dict['sessions']['reason'])

    def test_status_front_window_red_behind_ok(self):
        """有窗口挡在主窗口前面为红；窗口在主窗口后面则不挡住（绿）。"""
        # 1. 挡在前面 (前一个窗口在 main 前面且相交)
        inp_front = self._default_input()
        inp_front['wechat']['windows'] = [
            {'owner': 'WeChat', 'pid': 8888, 'layer': 0, 'on_screen': True,
             'bounds': {'x': 150, 'y': 150, 'width': 300, 'height': 200}},  # front overlapping
            {'owner': 'WeChat', 'pid': 8888, 'layer': 0, 'on_screen': True,
             'bounds': {'x': 100, 'y': 100, 'width': 880, 'height': 640}}   # main window
        ]
        items_front = self.run_status_eval(inp_front)
        dict_front = {i['key']: i for i in items_front}
        self.assertEqual(dict_front['wechat']['level'], 'red')
        self.assertIn('挡', dict_front['wechat']['reason'])

        # 2. 窗口在后面 (后一个窗口在 main 后面)
        inp_behind = self._default_input()
        inp_behind['wechat']['windows'] = [
            {'owner': 'WeChat', 'pid': 8888, 'layer': 0, 'on_screen': True,
             'bounds': {'x': 100, 'y': 100, 'width': 880, 'height': 640}},  # main window
            {'owner': 'WeChat', 'pid': 8888, 'layer': 0, 'on_screen': True,
             'bounds': {'x': 150, 'y': 150, 'width': 360, 'height': 124}}   # behind main
        ]
        items_behind = self.run_status_eval(inp_behind)
        dict_behind = {i['key']: i for i in items_behind}
        self.assertEqual(dict_behind['wechat']['level'], 'green')

    def test_status_disk_low_yellow(self):
        """剩余空间少于 5GB 为黄。"""
        inp = self._default_input()
        inp['disk']['free_bytes'] = 4 * 1024 * 1024 * 1024  # 4GB < 5GB

        items = self.run_status_eval(inp)
        item_dict = {i['key']: i for i in items}
        self.assertIn('disk', item_dict)
        self.assertEqual(item_dict['disk']['level'], 'yellow')
        self.assertIn('5GB', item_dict['disk']['reason'])

    def test_config_form_rejects_invalid_without_write(self):
        """用假的 config-check 退出码 2：校验失败时在表单上显示 error，不写入文件，不产生备份。"""
        with tempfile.TemporaryDirectory() as td:
            target_path = Path(td) / "config.json"
            original_content = json.dumps({"schema_version": 1, "account_id": "original", "manage_window": True})
            target_path.write_text(original_content)

            payload = {
                'op': 'save_config',
                'target_path': str(target_path),
                'new_config': {
                    'schema_version': 999,  # invalid
                    'account_id': 'invalid'
                },
                'simulated_check_result': {
                    'ok': False,
                    'exit_code': 2,
                    'error': 'unsupported bot config schema or field'
                }
            }

            res = self.run_config_action(payload)
            self.assertFalse(res['ok'])
            self.assertIn('unsupported bot config', res['error'])

            # 目标配置文件内容保持不变
            self.assertEqual(target_path.read_text(), original_content)
            # 不产生任何备份文件
            bak_files = list(Path(td).glob("*.bak*"))
            self.assertEqual(bak_files, [])

    def test_config_save_backs_up_then_replaces(self):
        """校验通过后，把旧配置备份为 config.json.bak-<时间戳>，再原子替换并触发重启。"""
        with tempfile.TemporaryDirectory() as td:
            target_path = Path(td) / "config.json"
            original_content = json.dumps({"schema_version": 1, "account_id": "test_bot", "max_attachment_mb": 50})
            target_path.write_text(original_content)

            payload = {
                'op': 'save_config',
                'target_path': str(target_path),
                'new_config': {
                    'schema_version': 1,
                    'account_id': 'test_bot',
                    'max_attachment_mb': 60
                },
                'simulated_check_result': {
                    'ok': True,
                    'exit_code': 0
                }
            }

            res = self.run_config_action(payload)
            self.assertTrue(res['ok'])
            self.assertTrue(res.get('reloaded', False))

            # 目标配置已被原子替换
            saved = json.loads(target_path.read_text())
            self.assertEqual(saved['max_attachment_mb'], 60)

            # 检查备份文件存在且内容与旧配置完全一致
            bak_files = list(Path(td).glob("config.json.bak-*"))
            self.assertEqual(len(bak_files), 1)
            self.assertEqual(bak_files[0].read_text(), original_content)

    def test_config_form_never_shows_key(self):
        """配置表单永远不显示明文密钥，只显示'已配置'或'未配置'。"""
        # 1. 包含 api_key
        payload_with_key = {
            'op': 'mask_config',
            'config': {
                'account_id': 'bot1',
                'model_config': {
                    'backend': 'deepseek',
                    'model': 'chat',
                    'api_key': 'mock_key'
                }
            }
        }
        res1 = self.run_config_action(payload_with_key)
        self.assertNotIn('mock_key', json.dumps(res1))
        self.assertEqual(res1.get('api_key_status'), '已配置')

        # 2. 未配置 key
        payload_no_key = {
            'op': 'mask_config',
            'config': {
                'account_id': 'bot1',
                'model_config': {
                    'backend': 'deepseek',
                    'model': 'chat'
                }
            }
        }
        res2 = self.run_config_action(payload_no_key)
        self.assertEqual(res2.get('api_key_status'), '未配置')

    def test_diagnostics_has_no_secrets(self):
        """把 evaluate 的结果和版本号拼成纯文本，不含密钥，不含消息内容。"""
        inp = self._default_input()
        payload = {
            'status_input': inp,
            'app_version': '1.0.0',
            'sensitive_sample': 'mock_sensitive_text'
        }
        res = self.run_diagnostics(payload)
        diag_text = res.get('text', '')
        self.assertIn('1.0.0', diag_text)
        self.assertIn('权限', diag_text)
        self.assertIn('桌面服务', diag_text)
        self.assertIn('机器人', diag_text)
        self.assertNotIn('mock_sensitive_text', diag_text)
        self.assertNotIn('secret', diag_text.lower())


if __name__ == '__main__':
    unittest.main()
