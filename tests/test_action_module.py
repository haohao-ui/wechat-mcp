"""Property tests for the shared action module (native/action): human-like but always inside the target.

A small Swift self-test program (tests/action_selftest/main.swift) samples thousands of seeded click
plans and prints JSON; nothing here posts real mouse events.
"""
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
EPS = 1e-6


def inside(point, rect):
    x, y = point
    return rect['x'] - EPS <= x <= rect['x'] + rect['w'] + EPS and rect['y'] - EPS <= y <= rect['y'] + rect['h'] + EPS


class ActionModuleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('action module needs macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name)/'action_selftest'
        sources = sorted(str(p) for p in (ROOT/'native'/'action').glob('*.swift'))
        subprocess.run(['swiftc', '-O', *sources, str(ROOT/'tests'/'action_selftest'/'main.swift'), '-o', str(cls.cli)],
                       check=True, capture_output=True)
        cls.samples = cls.run_cli('clicks', '5000', '7')

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    @classmethod
    def run_cli(cls, *args):
        run = subprocess.run([str(cls.cli), *args], capture_output=True, text=True, timeout=300)
        if run.returncode:
            raise AssertionError(run.stderr)
        return json.loads(run.stdout)

    def test_press_always_inside_effective_region_or_refused(self):
        kinds = set()
        for s in self.samples:
            kinds.add(s['kind'])
            if s['region'] is None:
                self.assertIsNone(s['press'], s)
            else:
                self.assertIsNotNone(s['press'], s)
                self.assertTrue(inside(s['press'], s['region']), s)
        self.assertEqual(kinds, {'normal', 'tiny', 'half_off_screen', 'start_inside', 'off_screen'})
        self.assertTrue(any(s['region'] is None for s in self.samples))

    def test_peak_speed_in_first_half(self):
        moves = [s for s in self.samples if s['press'] and s['distance'] >= 30]
        early = sum(1 for s in moves if s['peak_ratio'] < 0.5)
        self.assertGreaterEqual(early/len(moves), 0.95)

    def test_duration_follows_fitts(self):
        for s in self.samples:
            if s['press'] is None:
                continue
            self.assertGreaterEqual(s['duration'], 0.5*s['fitts'] - EPS, s)
            self.assertLessEqual(s['duration'], 2.0*s['fitts'] + EPS, s)
        hard = [s['duration'] for s in self.samples if s['press'] and s['distance'] > 600 and s['kind'] == 'tiny']
        easy = [s['duration'] for s in self.samples if s['press'] and s['distance'] < 150 and s['target_width'] > 60]
        self.assertTrue(hard and easy)
        self.assertGreater(sum(hard)/len(hard), sum(easy)/len(easy))

    def test_tremor_bounded_and_vanishes(self):
        for s in self.samples:
            if s['press'] is None:
                continue
            self.assertLessEqual(s['max_deviation'], 2*s['tremor'] + s['drift'] + EPS, s)
            self.assertLessEqual(s['tail_deviation'], 0.5, s)

    def test_points_stay_on_screen_and_no_nan(self):
        negative = 0
        for s in self.samples:
            self.assertTrue(s['finite'], s)
            self.assertTrue(s['on_screen'], s)
            negative += s['screen']['x'] < 0
        self.assertGreater(negative, 0, 'some samples use a screen left of the main display')

    def test_small_targets_need_more_corrections(self):
        tiny = [s['corrections'] for s in self.samples if s['press'] and s['kind'] == 'tiny']
        large = [s['corrections'] for s in self.samples if s['press'] and s['target_width'] >= 120 and s['target_height'] >= 40]
        self.assertTrue(tiny and large)
        self.assertGreater(sum(tiny)/len(tiny), sum(large)/len(large))
        self.assertLessEqual(max(s['corrections'] for s in self.samples), 3)
        # Corrections tighten with the target: tiny targets are usually reached before the last (snap) attempt.
        self.assertLess(sum(tiny)/len(tiny), 2.5)

    def test_same_seed_same_plan(self):
        self.assertEqual(self.run_cli('clicks', '50', '11'), self.run_cli('clicks', '50', '11'))
        self.assertNotEqual(self.run_cli('clicks', '50', '11'), self.run_cli('clicks', '50', '12'))


class ExecutorTests(unittest.TestCase):
    """Executor + Actions against a recording sink (simulated cursor and clock)."""

    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('action module needs macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name)/'action_selftest'
        sources = sorted(str(p) for p in (ROOT/'native'/'action').glob('*.swift'))
        subprocess.run(['swiftc', '-O', *sources, str(ROOT/'tests'/'action_selftest'/'main.swift'), '-o', str(cls.cli)],
                       check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def scenario(self, name, seed=3):
        run = subprocess.run([str(self.cli), 'exec', name, str(seed)], capture_output=True, text=True, timeout=60)
        self.assertEqual(run.returncode, 0, run.stderr)
        return json.loads(run.stdout)

    def kinds(self, out):
        return [e['type'] for e in out['events']]

    def test_click_event_order(self):
        for seed in range(1, 40):
            out = self.scenario('click', seed)
            self.assertIsNone(out['error'])
            kinds = self.kinds(out)
            self.assertEqual(kinds[-2:], ['left_down', 'left_up'])
            self.assertTrue(set(kinds[:-2]) == {'moved'} and len(kinds) > 3, kinds)
            down, up = out['events'][-2:]
            self.assertTrue(inside((down['x'], down['y']), out['region']))
            self.assertEqual((down['x'], down['y']), (up['x'], up['y']))
            self.assertTrue(0.06 - EPS <= up['t'] - down['t'] <= 0.12 + EPS)

    def test_unreachable_target_never_presses(self):
        out = self.scenario('unreachable')
        self.assertEqual(out['error'], 'target_not_reached')
        self.assertFalse({'left_down', 'left_up'} & set(self.kinds(out)))

    def test_user_move_aborts(self):
        out = self.scenario('interfere_move')
        self.assertEqual(out['error'], 'user_input_detected')
        self.assertNotIn('left_down', self.kinds(out))

    def test_user_move_between_down_and_up_still_releases(self):
        out = self.scenario('interfere_down')
        self.assertEqual(out['error'], 'user_input_detected')
        self.assertEqual(self.kinds(out)[-2:], ['left_down', 'left_up'])

    def test_instant_profile_single_move(self):
        out = self.scenario('instant')
        self.assertIsNone(out['error'])
        self.assertEqual(self.kinds(out), ['moved', 'left_down', 'left_up'])
        self.assertTrue(inside((out['events'][0]['x'], out['events'][0]['y']), out['region']))

    def test_scroll_keeps_tick_size(self):
        out = self.scenario('scroll')
        self.assertIsNone(out['error'])
        ticks = [e for e in out['events'] if e['type'] == 'scroll']
        self.assertEqual([t['lines'] for t in ticks], [10, 10, 10])
        for tick in ticks:
            self.assertTrue(inside((tick['x'], tick['y']), out['region']))
        for a, b in zip(ticks, ticks[1:]):
            self.assertTrue(0.08 - EPS <= b['t'] - a['t'] <= 0.2 + EPS)
        self.assertNotIn('left_down', self.kinds(out))

    def test_right_click_uses_right_button(self):
        out = self.scenario('right_click')
        self.assertIsNone(out['error'])
        self.assertEqual(self.kinds(out)[-2:], ['right_down', 'right_up'])
        # The context menu opens where the press landed; callers get that point back.
        down = out['events'][-2]
        self.assertEqual(out['pressed'], [down['x'], down['y']])


if __name__ == '__main__':
    unittest.main()


class WechatClickCliTests(unittest.TestCase):
    """wechat_click keeps `wechat_click <x> <y>` and gains an optional size and --plan-only (posts nothing)."""

    @classmethod
    def setUpClass(cls):
        if platform.system() != 'Darwin' or not shutil.which('swiftc'):
            raise unittest.SkipTest('wechat_click needs macOS swiftc')
        cls.tmp = tempfile.TemporaryDirectory()
        cls.cli = Path(cls.tmp.name)/'wechat_click'
        sources = sorted(str(p) for p in (ROOT/'native'/'action').glob('*.swift'))
        subprocess.run(['swiftc', str(ROOT/'native'/'click'/'main.swift'), *sources, '-o', str(cls.cli)],
                       check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def plan(self, *args):
        run = subprocess.run([str(self.cli), '--plan-only', *args], capture_output=True, text=True, timeout=30)
        return run, (json.loads(run.stdout) if run.returncode == 0 else None)

    def press(self, events):
        kinds = [e['type'] for e in events]
        self.assertEqual(kinds[-2:], ['left_down', 'left_up'])
        return events[-2]

    def test_point_form_clicks_within_a_small_square(self):
        for _ in range(10):
            run, out = self.plan('500', '400')
            self.assertEqual(run.returncode, 0, run.stderr)
            down = self.press(out['events'])
            self.assertLessEqual(abs(down['x'] - 500), 4 + EPS)
            self.assertLessEqual(abs(down['y'] - 400), 4 + EPS)

    def test_size_form_uses_the_effective_region(self):
        for _ in range(10):
            run, out = self.plan('500', '400', '200', '40')
            self.assertEqual(run.returncode, 0, run.stderr)
            down = self.press(out['events'])
            # 200×40 centred on (500,400), 20% inset: x 440..560, y 388..412.
            self.assertTrue(440 - EPS <= down['x'] <= 560 + EPS and 388 - EPS <= down['y'] <= 412 + EPS, down)

    def test_invalid_arguments_fail(self):
        for args in (['500'], ['a', 'b'], ['500', '400', '-3', '10'], ['500', '400', '10']):
            run, _ = self.plan(*args)
            self.assertNotEqual(run.returncode, 0, args)
