"""Unreadable attachments reach the model with the real reason (acceptance 2026-11-06).

An .xls card was refused as unsupported, but the model only saw "unsupported_type" next to card text
"对方上传中" and replied that the file was still uploading."""
import asyncio
import json
import unittest
from unittest.mock import patch

from scripts.wechat_bot import reply


class UnreadableHintTests(unittest.TestCase):
    def prompt_for(self, failed, max_attachment_mb=10):
        seen = {}
        async def fake_api(prompt, **kwargs):
            seen['prompt'] = prompt
            return {'reply': '好的'}
        with patch('scripts.wechat_bot.model_api.generate_api', fake_api):
            asyncio.run(reply.generate('[文件]', mode='service', failed=failed,
                                       model_config={'backend': 'openai-compatible'},
                                       max_attachment_mb=max_attachment_mb))
        return json.loads(seen['prompt'][seen['prompt'].index('\n{') + 1:])

    def test_unsupported_type_says_format_not_upload(self):
        data = self.prompt_for([{'label': '报价.xls', 'kind': 'file', 'reason': 'unsupported_type'}])
        hint = data['unreadable'][0]['hint']
        self.assertIn('格式', hint)
        self.assertIn('xlsx', hint)
        self.assertIn('不要说还在上传', hint)

    def test_too_large_mentions_limit(self):
        data = self.prompt_for([{'label': '大表.xlsx', 'kind': 'file', 'reason': 'too_large'}])
        self.assertIn('10MB', data['unreadable'][0]['hint'])

    def test_too_large_default_mentions_50mb(self):
        seen = {}
        async def fake_api(prompt, **kwargs):
            seen['prompt'] = prompt
            return {'reply': '好的'}
        with patch('scripts.wechat_bot.model_api.generate_api', fake_api):
            asyncio.run(reply.generate('[文件]', mode='service',
                                       failed=[{'label': '大表.xlsx', 'kind': 'file', 'reason': 'too_large'}],
                                       model_config={'backend': 'openai-compatible'}))
        data = json.loads(seen['prompt'][seen['prompt'].index('\n{') + 1:])
        self.assertIn('50MB', data['unreadable'][0]['hint'])
        self.assertNotIn('10MB', data['unreadable'][0]['hint'])

    def test_every_known_reason_has_a_hint(self):
        reasons = ['unsupported_type', 'too_large', 'download_failed', 'snapshot_changed', 'extract_failed', 'empty']
        data = self.prompt_for([{'label': r, 'kind': 'file', 'reason': r} for r in reasons])
        self.assertTrue(all(item.get('hint') for item in data['unreadable']))


    def test_plain_labels_are_still_accepted(self):
        data = self.prompt_for(['旧调用方只给文件名.pdf'])
        self.assertEqual(data['unreadable'][0]['label'], '旧调用方只给文件名.pdf')
        self.assertTrue(data['unreadable'][0]['hint'])
