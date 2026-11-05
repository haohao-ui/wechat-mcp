"""The default layout map must equal the constants the code still hard-codes (stage A).

Later stages move each caller onto the layout map; until then this test fails as soon as either side
changes alone, so a migration can never silently move a click or a crop.
"""
import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
LAYOUT = json.loads((ROOT/'native'/'layout'/'wechat-4.1.15.json').read_text())
REGIONS = LAYOUT['regions']


def source(path):
    return (ROOT/path).read_text()


def has(test, snippet, path):
    test.assertTrue(snippet in source(path), '%s: %s not found' % (path, snippet))


def rect(name):
    r = REGIONS[name]
    return (r['x'], r['y'], r['w'], r['h'])


class DefaultLayoutMatchesCode(unittest.TestCase):
    def test_window_size(self):
        self.assertEqual((LAYOUT['window']['width'], LAYOUT['window']['height']), (880, 640))
        has(self, 'captured.width == 880*$0 && captured.height == 640*$0', 'native/wechat_bubbles.swift')

    def test_session_list(self):
        m = re.search(r'let sessionListRect = CGRect\(x:(\d+), y:(\d+), width:(\d+), height:(\d+)\)', source('native/bot/Sessions.swift'))
        self.assertEqual(tuple(map(int, m.groups())), rect('session_list'))
        has(self, 'sessionListRect.minY + row.y + %d' % LAYOUT['session_row']['name_offset'], 'native/bot/Sessions.swift')

    def test_chat_area_for_scroll_and_media(self):
        has(self, 'CGRect(x:wx+%d,y:wy+%d,width:%d,height:%d)' % rect('chat'), 'native/bot/Worker.swift')
        has(self, '+%d+Double(x),y:(window["y"] as! Double)+%d+Double(y)' % rect('chat')[:2], 'native/bot/Files.swift')

    def test_chat_content_crop_for_bubbles(self):
        x, y, w, h = rect('chat_content')
        crop = 'CGRect(x:%d,y:%d,width:full.width-%d,height:full.height-%d)' % (x, y, 880-w, 640-h)
        has(self, crop, 'native/wechat_bubbles.swift')

    def test_title_crop(self):
        x, y, w, h = rect('title')
        has(self, 'CGRect(x:%d,y:%d,width:full.width-%d,height:%d)' % (x, y, 880-w, h), 'native/wechat_bubbles.swift')

    def test_input_ocr_region_in_host(self):
        x, y, w, h = rect('input_ocr')
        rust = source('src/wechat.rs')
        for snippet in ('win_x + %d.0' % x, 'win_y + win_h - %d.0' % (640-y), 'win_w - %d.0' % (880-w)):
            has(self, snippet, 'src/wechat.rs')
        self.assertRegex(rust, r'win_w - %d\.0,\s*%d\.0' % (880-w, h))

    def test_scroll_tick(self):
        has(self, 'SCROLL_STEP_POINTS = %d' % LAYOUT['scroll']['points_per_tick'], 'scripts/wechat_bot/policy.py')
        has(self, 'direction=="up" ? %d : -%d' % ((LAYOUT['scroll']['lines_per_tick'],)*2), 'native/bot/Worker.swift')

    def test_save_as_menu_labels(self):
        m = re.search(r'let titles=\[(.*?)\]', source('native/bot/Files.swift'))
        self.assertEqual(re.findall(r'"([^"]+)"', m.group(1)), LAYOUT['menu_labels']['save_as'])

    def test_theme_thresholds(self):
        f = 'native/wechat_bubbles.swift'
        dark, light = LAYOUT['themes']['wechat-4-dark'], LAYOUT['themes']['wechat-4-light']
        has(self, 'r < %d && g < %d && b < %d' % ((dark['background_max'],)*3), f)
        has(self, 'r >= %d && r <= %d' % tuple(dark['incoming']), f)
        has(self, 'r >= %d && g >= %d && b >= %d' % ((light['background_min'],)*3), f)
        has(self, 'r >= %d && r <= %d' % tuple(light['incoming']), f)


if __name__ == '__main__':
    unittest.main()
