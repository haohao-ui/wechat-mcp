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
        self.assertNotIn('captured.width == 880*$0', source('native/wechat_bubbles.swift'))

    def test_session_list(self):
        self.assertNotIn('let sessionListRect = CGRect', source('native/bot/Sessions.swift'))
        self.assertNotIn('row.y + 12', source('native/bot/Sessions.swift'))

    def test_chat_area_for_scroll_and_media(self):
        self.assertNotIn('CGRect(x:wx+315,y:wy+60,width:565,height:436)', source('native/bot/Worker.swift'))
        self.assertNotIn('+315+Double(x),y:(window["y"] as! Double)+60+Double(y)', source('native/bot/Files.swift'))

    def test_chat_content_crop_for_bubbles(self):
        self.assertNotIn('CGRect(x:315,y:60,width:full.width-325', source('native/wechat_bubbles.swift'))

    def test_title_crop(self):
        self.assertNotIn('CGRect(x:250,y:0,width:full.width-260,height:50)', source('native/wechat_bubbles.swift'))

    def test_input_ocr_region_in_host(self):
        x, y, w, h = rect('input_ocr')
        rust = source('src/wechat.rs')
        for snippet in ('win_x + %d.0' % x, 'win_y + win_h - %d.0' % (640-y), 'win_w - %d.0' % (880-w)):
            has(self, snippet, 'src/wechat.rs')
        self.assertRegex(rust, r'win_w - %d\.0,\s*%d\.0' % (880-w, h))

    def test_scroll_tick(self):
        has(self, 'SCROLL_STEP_POINTS = %d' % LAYOUT['scroll']['points_per_tick'], 'scripts/wechat_bot/policy.py')
        self.assertNotIn('direction=="up" ? 10 : -10', source('native/bot/Worker.swift'))

    def test_save_as_menu_labels(self):
        # Menu labels come only from the layout map: no hard-coded copy, not even as a fallback.
        self.assertNotIn('"另存为…"', source('native/bot/Files.swift'))

    def test_theme_thresholds(self):
        f = 'native/wechat_bubbles.swift'
        self.assertNotIn('r < 37 && g < 37 && b < 37', source(f))
        self.assertNotIn('r >= 38 && r <= 82', source(f))
        self.assertNotIn('r >= 246 && g >= 246 && b >= 246', source(f))
        self.assertNotIn('r >= 228 && r <= 244', source(f))


if __name__ == '__main__':
    unittest.main()


class AnchorGateTests(unittest.TestCase):
    def test_every_click_or_read_path_checks_anchors_first(self):
        # Stage C: session-list reads and row clicks use the same screen check as chat reads.
        worker = (ROOT/'native'/'bot'/'Worker.swift').read_text()
        for op in ('sessions', 'open_session'):
            branch = worker[worker.index(f'if operation == "{op}"'):]
            branch = branch[:branch.index('\n        }\n')]
            self.assertIn('checkedScreen(', branch, op)
            self.assertNotIn('normalizedFrame(', branch, op)
        bubbles = (ROOT/'native'/'wechat_bubbles.swift').read_text()
        analyze = bubbles[bubbles.index('func analyzeFrame('):]
        self.assertIn('try checkAnchors(captured', analyze[:analyze.index('guard let title')])
