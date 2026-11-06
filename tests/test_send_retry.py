import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from scripts.wechat_bot.service import BotService, write_private
from scripts.wechat_bot.policy import echo_matches


def bubble(text, direction='inbound_visual'):
    return {'text': text, 'digest': text, 'direction': direction}


def frame(*bubbles, title='示例联系人甲'):
    return {
        'profile': 'wechat-4-dark',
        'scale': 1,
        'title_lines': [title],
        'bubbles': list(bubbles),
        'clipped_bubble': False,
    }


class SendRetryTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.config = {
            'schema_version': 1,
            'host': 'fixture',
            'account_id': 'fixture',
            'allowlist': [{'recipient': '示例联系人甲', 'kind': 'private'}],
            'state_dir': str(self.root / 'state'),
            'poll_seconds': 2,
            'cooldown_seconds': 0,
            'max_replies_per_hour': 20,
            'turn_quiet_seconds': 0,
        }
        self.service = BotService(self.config)
        self.service.sleep = AsyncMock()
        self.old = frame(bubble('old'))
        self.new = frame(bubble('old'), bubble('你好，介绍一下你自己'))
        self.calls = []
        self.model = AsyncMock(return_value='你好，我是AI客服助手。')

    def tearDown(self):
        self.temp.cleanup()

    async def test_not_sent_click_failure_retries_and_does_not_pause(self):
        self.service.advance(self.old)
        reply = bubble('你好，我是AI客服助手。', 'outbound_visual')
        after = frame(bubble('old'), bubble('你好，介绍一下你自己'), reply)

        # 第一次 send_message 抛出未触发发送错误，第二次成功
        attempt = 0
        calls = self.calls

        class FailOnceClient:
            async def tool(self, name, **args):
                nonlocal attempt
                calls.append((name, args))
                if name == 'send_message':
                    attempt += 1
                    if attempt == 1:
                        raise RuntimeError(
                            'MCP tool failed: send_message: 发送尝试 send-1 未触发发送: wechat_click 退出状态异常: Some(1)'
                        )
                    return {'status': 'triggered', 'send_id': 'send-1', 'delivery': 'unknown'}
                if name == 'get_send_status':
                    if attempt == 1:
                        return {'status': 'unknown', 'delivery': 'not_sent', 'send_id': 'send-1'}
                    return {'status': 'triggered', 'delivery': 'unknown', 'send_id': 'send-1'}
                return {}

        capture = type('Capture', (), {})()
        # 序列：新消息检测，pre-send 1，pre-send 2 (retry)，echo 校验
        capture.frame = AsyncMock(side_effect=[self.new, self.new, self.new, after, after])

        with patch('scripts.wechat_bot.service.generate', self.model):
            await self.service.cycle(FailOnceClient(), capture)

        self.assertFalse(self.service.paused)
        send_calls = [args for name, args in self.calls if name == 'send_message']
        self.assertEqual(len(send_calls), 2)
        # 确保重试使用相同的 idempotency_key
        self.assertEqual(send_calls[0]['idempotency_key'], send_calls[1]['idempotency_key'])
        # 检查 journal
        journals = list((self.root / 'state').glob('send-*.json'))
        self.assertEqual(len(journals), 1)
        data = json.loads(journals[0].read_text())
        self.assertEqual(data.get('status'), 'triggered')
        self.assertTrue(data.get('client_echo_observed'))

    async def test_not_sent_three_failures_keeps_session_running(self):
        self.service.advance(self.old)
        calls = self.calls

        class AlwaysFailNotSentClient:
            async def tool(self, name, **args):
                calls.append((name, args))
                if name == 'send_message':
                    raise RuntimeError(
                        'MCP tool failed: send_message: 发送尝试 fail 未触发发送: wechat_click 退出状态异常: Some(1)'
                    )
                if name == 'get_send_status':
                    return {'status': 'unknown', 'delivery': 'not_sent', 'send_id': 'fail'}
                return {}

        capture = type('Capture', (), {})()
        capture.frame = AsyncMock(return_value=self.new)

        with patch('scripts.wechat_bot.service.generate', self.model):
            await self.service.cycle(AlwaysFailNotSentClient(), capture)

        self.assertFalse(self.service.paused)
        send_calls = [args for name, args in self.calls if name == 'send_message']
        # 初试 + 2 次重试 = 3 次
        self.assertEqual(len(send_calls), 3)
        self.assertEqual(send_calls[0]['idempotency_key'], send_calls[1]['idempotency_key'])
        self.assertEqual(send_calls[1]['idempotency_key'], send_calls[2]['idempotency_key'])

        journals = list((self.root / 'state').glob('send-*.json'))
        self.assertEqual(len(journals), 1)
        data = json.loads(journals[0].read_text())
        self.assertEqual(data.get('status'), 'not_sent')

        # 检查 health 状态保持 watching 且包含 reason
        health = json.loads((self.root / 'state' / 'health.json').read_text())
        self.assertEqual(health.get('state'), 'watching')
        self.assertIn('last reply not sent', health.get('reason', ''))

    async def test_unknown_with_echo_marks_triggered_no_resend(self):
        self.service.advance(self.old)
        reply = bubble('你好，我是AI客服助手。', 'outbound_visual')
        frame_with_echo = frame(bubble('old'), bubble('你好，介绍一下你自己'), reply)
        calls = self.calls

        class UnknownWithEchoClient:
            async def tool(self, name, **args):
                calls.append((name, args))
                if name == 'send_message':
                    raise RuntimeError('MCP tool failed: send_message: 结果未知: connection closed')
                if name == 'get_send_status':
                    return {'status': 'unknown', 'delivery': 'unknown', 'send_id': 'unk-1'}
                return {}

        capture = type('Capture', (), {})()
        # 初始检测 -> confirmed -> pre_send -> screen check 发现 echo -> postflight
        capture.frame = AsyncMock(side_effect=[self.new, self.new, self.new, frame_with_echo, frame_with_echo])

        with patch('scripts.wechat_bot.service.generate', self.model):
            await self.service.cycle(UnknownWithEchoClient(), capture)

        self.assertFalse(self.service.paused)
        send_calls = [args for name, args in self.calls if name == 'send_message']
        # 看到 echo，绝不发起重发
        self.assertEqual(len(send_calls), 1)

        journals = list((self.root / 'state').glob('send-*.json'))
        self.assertEqual(len(journals), 1)
        data = json.loads(journals[0].read_text())
        self.assertEqual(data.get('status'), 'triggered')
        self.assertTrue(data.get('client_echo_observed'))

    async def test_unknown_without_echo_retries_once(self):
        self.service.advance(self.old)
        reply = bubble('你好，我是AI客服助手。', 'outbound_visual')
        after = frame(bubble('old'), bubble('你好，介绍一下你自己'), reply)
        calls = self.calls
        send_count = 0

        class UnknownNoEchoRetryClient:
            async def tool(self, name, **args):
                nonlocal send_count
                calls.append((name, args))
                if name == 'send_message':
                    send_count += 1
                    if send_count == 1:
                        raise RuntimeError('MCP tool failed: send_message: 结果未知: timeout')
                    return {'status': 'triggered', 'send_id': 'retry-1', 'delivery': 'unknown'}
                if name == 'get_send_status':
                    if send_count == 1:
                        return {'status': 'unknown', 'delivery': 'unknown', 'send_id': 'unk-1'}
                    return {'status': 'triggered', 'delivery': 'unknown', 'send_id': 'retry-1'}
                return {}

        capture = type('Capture', (), {})()
        # 初始检测 -> pre-send 1 -> 屏幕检查 3 次无 echo -> pre-send 重试 -> echo 校验
        capture.frame = AsyncMock(side_effect=[
            self.new, self.new,
            self.new, self.new, self.new,  # 3 次 screen check 无 echo
            self.new,  # pre-send 重试
            after, after  # 发送后 echo 出现
        ])

        with patch('scripts.wechat_bot.service.generate', self.model):
            await self.service.cycle(UnknownNoEchoRetryClient(), capture)

        self.assertFalse(self.service.paused)
        send_calls = [args for name, args in self.calls if name == 'send_message']
        self.assertEqual(len(send_calls), 2)

        journals = list((self.root / 'state').glob('send-*.json'))
        self.assertEqual(len(journals), 1)
        data = json.loads(journals[0].read_text())
        self.assertEqual(data.get('status'), 'triggered')

    async def test_unknown_unverifiable_does_not_pause_and_never_resends(self):
        self.service.advance(self.old)
        calls = self.calls

        class UnknownUnverifiableClient:
            async def tool(self, name, **args):
                calls.append((name, args))
                if name == 'send_message':
                    raise RuntimeError('MCP tool failed: send_message: 结果未知: osascript timeout')
                if name == 'get_send_status':
                    return {'status': 'unknown', 'delivery': 'unknown', 'send_id': 'unk-err'}
                return {}

        capture = type('Capture', (), {})()
        # 初始帧成功，confirmed 成功，pre-send 成功，随后的 screen check 一直 capture 抛出异常
        capture.frame = AsyncMock(side_effect=[
            self.new, self.new, self.new,
            RuntimeError('capture device error 1'),
            RuntimeError('capture device error 2'),
            RuntimeError('capture device error 3'),
        ])

        with patch('scripts.wechat_bot.service.generate', self.model):
            await self.service.cycle(UnknownUnverifiableClient(), capture)

        # 不暂停会话
        self.assertFalse(self.service.paused)
        # 绝不重发
        send_calls = [args for name, args in self.calls if name == 'send_message']
        self.assertEqual(len(send_calls), 1)

        journals = list((self.root / 'state').glob('send-*.json'))
        self.assertEqual(len(journals), 1)
        data = json.loads(journals[0].read_text())
        self.assertEqual(data.get('status'), 'unknown')

    async def test_startup_attempting_journal_does_not_pause(self):
        state_dir = self.root / 'state'
        state_dir.mkdir(parents=True, exist_ok=True)
        attempting_journal = state_dir / 'send-attempting.json'
        write_private(attempting_journal, {
            'status': 'attempting',
            'reply': '你好，我是AI客服助手。',
            'idempotency_key': 'key-attempting'
        })

        service = BotService(self.config)
        self.assertFalse(service.paused)

        reply = bubble('你好，我是AI客服助手。', 'outbound_visual')
        frame_with_echo = frame(bubble('old'), reply)
        capture = type('Capture', (), {})()
        capture.frame = AsyncMock(return_value=frame_with_echo)

        await service.recover_startup_journals(capture)
        self.assertFalse(service.paused)
        data = json.loads(attempting_journal.read_text())
        self.assertEqual(data.get('status'), 'triggered')
        self.assertTrue(data.get('client_echo_observed'))

    def test_startup_generated_journal_closed_not_sent(self):
        state_dir = self.root / 'state'
        state_dir.mkdir(parents=True, exist_ok=True)
        gen_journal = state_dir / 'send-generated.json'
        write_private(gen_journal, {
            'status': 'generated',
            'reply': '你好',
            'idempotency_key': 'key-gen'
        })
        prep_journal = state_dir / 'send-preparing.json'
        write_private(prep_journal, {
            'status': 'preparing_media',
            'idempotency_key': 'key-prep'
        })

        service = BotService(self.config)
        self.assertFalse(service.paused)
        self.assertEqual(json.loads(gen_journal.read_text()).get('status'), 'not_sent')
        self.assertEqual(json.loads(prep_journal.read_text()).get('status'), 'not_sent')


if __name__ == '__main__':
    unittest.main()
