import unittest
from unittest.mock import AsyncMock, patch

from scripts.wechat_bot.understanding import understand

CONFIG = {'backend': 'openai-compatible', 'base_url': 'https://api.deepseek.com', 'model': 'm', 'api_key_env': 'K',
          'supports_images': True}


def images(count):
    return [{'path': 'p%d.png' % n, 'label': '第%d张图片' % n} for n in range(1, count+1)]


class UnderstandTests(unittest.IsolatedAsyncioTestCase):
    async def test_understand_batches_by_size(self):
        calls = []

        async def fake(prompt, *, images, config, metadata=None, field, check):
            calls.append(list(images))
            return check(['要点%s' % p for p in images])
        with patch('scripts.wechat_bot.understanding.generate_api', side_effect=fake):
            notes = await understand(images(9), text='这些是什么', config=CONFIG, batch_size=4)
        self.assertEqual([len(c) for c in calls], [4, 4, 1])
        self.assertEqual(len(notes), 9)
        self.assertEqual(notes[8], '要点p9.png')

    async def test_failed_batch_yields_none_not_exception(self):
        async def fake(prompt, *, images, config, metadata=None, field, check):
            if 'p1.png' in images:
                raise ValueError('model response missing or incomplete')
            return check(['ok'] * len(images))
        with patch('scripts.wechat_bot.understanding.generate_api', side_effect=fake):
            notes = await understand(images(6), text='', config=CONFIG, batch_size=4)
        self.assertEqual(notes, [None]*4 + ['ok', 'ok'])

    async def test_wrong_note_count_fails_that_batch(self):
        async def fake(prompt, *, images, config, metadata=None, field, check):
            return check(['only one'])
        with patch('scripts.wechat_bot.understanding.generate_api', side_effect=fake):
            notes = await understand(images(2), text='', config=CONFIG, batch_size=4)
        self.assertEqual(notes, [None, None])


if __name__ == '__main__':
    unittest.main()
