"""Shape of the WeChat emoji lexicon (native/layout/emoji-lexicon.json); its content is reviewed by people."""
import json
from pathlib import Path
import re
import unittest

PATH = Path(__file__).resolve().parents[1]/'native'/'layout'/'emoji-lexicon.json'


class EmojiLexiconTests(unittest.TestCase):
    def setUp(self):
        self.doc = json.loads(PATH.read_text())

    def test_header(self):
        self.assertEqual(self.doc['schema'], 1)
        self.assertTrue(self.doc['wechat_version'])

    def test_every_entry_is_complete(self):
        for name, entry in self.doc['entries'].items():
            with self.subTest(name):
                self.assertRegex(name, r'^\[[^\[\]]+\]$')
                self.assertIsInstance(entry['templates'], dict)
                self.assertTrue(entry['meaning'].strip())
                self.assertTrue(entry['chat_uses'] and all(u.strip() for u in entry['chat_uses']))
                self.assertTrue(entry['service_hint'].strip())
                self.assertEqual(set(entry), {'templates', 'meaning', 'chat_uses', 'service_hint'})

    def test_easily_misread_emoji_carry_their_wechat_sense(self):
        # The point of the lexicon: these must not be read literally.
        entries = self.doc['entries']
        self.assertIn('不满', ''.join(entries['[微笑]']['chat_uses']))
        self.assertIn('不要当作满意', entries['[微笑]']['service_hint'])
        self.assertIn('玩笑', entries['[旺柴]']['service_hint'])
        self.assertIn('必须回复', entries['[疑问]']['service_hint'])
        self.assertIn('不是真红包', entries['[红包]']['meaning'])

    def test_templates_are_perceptual_hashes_when_present(self):
        for name, entry in self.doc['entries'].items():
            for scale, value in entry['templates'].items():
                self.assertIn(scale, ('1x', '2x'), name)
                self.assertRegex(value, r'^[0-9a-f]{16}$', name)


if __name__ == '__main__':
    unittest.main()
