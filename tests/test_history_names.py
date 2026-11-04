import unittest

from scripts.wechat_bot.history.names import candidate_names, clean_title, name_key


class NameKeyTests(unittest.TestCase):
    def test_emoji_and_symbols_removed(self):
        self.assertEqual(name_key('🍓小美丨鞋'), name_key('小美丨鞋'))
        self.assertEqual(name_key('✨Ａｂｃ ★'), 'Abc')

    def test_invisible_characters_removed(self):
        self.assertEqual(name_key('小​美️'), '小美')

    def test_all_symbol_name_falls_back(self):
        self.assertEqual(name_key('🍓🍓'), '🍓🍓')

    def test_clean_title_keeps_symbols(self):
        self.assertEqual(clean_title(' ​🍓小美 '), '🍓小美')

    def test_candidate_names(self):
        self.assertEqual(candidate_names({'recipient':'a','kind':'private','aliases':['b']}),['a','b'])
        self.assertEqual(candidate_names({'recipient':'a','kind':'private'}),['a'])
