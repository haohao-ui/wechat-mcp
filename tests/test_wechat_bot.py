import copy
import argparse
import asyncio
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from scripts.wechat_bot.policy import AmbiguousAnchor, Policy, new_bubbles, validate_reply
from scripts.wechat_bot import __main__ as runner


def bubble(text, direction='inbound_visual', digest=None):
    return {'text': text, 'direction': direction, 'digest': digest or text}


def frame(*bubbles, title='示例联系人甲'):
    return {'profile': 'wechat-4-dark', 'scale': 1, 'title_lines': [title], 'bubbles': list(bubbles)}


class BotPolicyTests(unittest.TestCase):
    def setUp(self):
        self.policy = Policy('fixture', '示例联系人甲', 'Bot验证1104')
        self.baseline = frame(bubble('old'))

    def test_exact_allowlist_and_private_title(self):
        for title in ['其他人', '示例联系人甲群', '示例联系人甲 (3)', '示例联系人甲2']:
            with self.assertRaisesRegex(ValueError, 'allowlisted title'):
                self.policy.check_frame(frame(title=title))

    def test_only_a_new_stable_incoming_test_message_triggers(self):
        message = bubble('Bot验证1104 你好')
        current = frame(bubble('old'), message)
        self.assertEqual(self.policy.candidate(self.baseline, current, copy.deepcopy(current)), message)
        self.assertIsNone(self.policy.candidate(current, current, current))

    def test_own_echo_unknown_direction_and_unrequested_messages_do_not_trigger(self):
        for message in [bubble('Bot验证1104', 'outbound_visual'), bubble('Bot验证1104', 'unknown'), bubble('你好')]:
            current = frame(bubble('old'), message)
            self.assertIsNone(self.policy.candidate(self.baseline, current, current))

    def test_two_frames_must_agree(self):
        current = frame(bubble('old'), bubble('Bot验证1104'))
        changed = frame(bubble('old'), bubble('other'))
        with self.assertRaisesRegex(ValueError, 'changed during confirmation'):
            self.policy.candidate(self.baseline, current, changed)

    def test_gap_and_repeated_anchor_stop(self):
        with self.assertRaisesRegex(ValueError, 'gap'):
            new_bubbles([bubble('old')], [bubble('new')])
        with self.assertRaisesRegex(ValueError, 'ambiguous'):
            new_bubbles([bubble('same'), bubble('same')], [bubble('same'), bubble('same'), bubble('extra')])

    def test_repeated_short_greeting_does_not_override_long_unique_context(self):
        old=[bubble(x) for x in ['hi','hi','there','question','received','hi']]
        new=old[1:]+[bubble('reply','outbound_visual')]
        self.assertEqual(new_bubbles(old,new),new[-1:])
        new=old[2:]+[bubble('reply','outbound_visual'),bubble('next question')]
        self.assertEqual(new_bubbles(old,new),new[-2:])

    def test_ambiguous_anchor_exposes_only_certainly_new_tail(self):
        same=bubble('same')
        with self.assertRaises(AmbiguousAnchor) as caught:
            new_bubbles([same],[same,same])
        self.assertEqual(caught.exception.certain,[])
        with self.assertRaises(AmbiguousAnchor) as caught:
            new_bubbles([same,same],[same,same,bubble('next')])
        self.assertEqual(caught.exception.certain,[bubble('next')])

    def test_periodic_repeated_context_is_still_ambiguous(self):
        old=[bubble(x) for x in ['a','b','a','b']]
        with self.assertRaisesRegex(ValueError,'ambiguous'):
            new_bubbles(old,old+[bubble('next')])

    def test_multiple_new_messages_are_not_replied_to_as_one(self):
        current = frame(bubble('old'), bubble('Bot验证1104'), bubble('second'))
        self.assertIsNone(self.policy.candidate(self.baseline, current, current))

    def test_same_body_different_occurrences_remain_distinct(self):
        old = [bubble('anchor'), bubble('same')]
        current = old + [bubble('same')]
        self.assertEqual(new_bubbles(old, current), [bubble('same')])

    def test_key_scoped_to_account_recipient_and_input(self):
        message = bubble('Bot验证1104')
        key = self.policy.key(self.baseline, message)
        self.assertEqual(key, self.policy.key(copy.deepcopy(self.baseline), copy.deepcopy(message)))
        self.assertNotEqual(key, Policy('other', '示例联系人甲', 'Bot验证1104').key(self.baseline, message))
        self.assertNotEqual(key, Policy('fixture', '其他人', 'Bot验证1104').key(self.baseline, message))

    def test_policy_accepts_alias_titles_only(self):
        policy=Policy('fixture','🍓示例联系人甲','service',names=['🍓示例联系人甲','示例联系人甲'])
        policy.check_frame(frame(bubble('x'),title='示例联系人甲'))
        policy.check_frame(frame(bubble('x'),title='\u200b🍓示例联系人甲 '))
        with self.assertRaisesRegex(ValueError,'allowlisted title'):
            policy.check_frame(frame(bubble('x'),title='别人'))

    def test_unknown_theme_rejected(self):
        for profile, scale in [('light', 1), ('wechat-4-dark-1x', 1), ('wechat-4-dark', 3), ('wechat-4-light', None)]:
            wrong = frame()
            wrong['profile'] = profile
            if scale is None: wrong.pop('scale')
            else: wrong['scale'] = scale
            with self.subTest(profile=profile, scale=scale), self.assertRaisesRegex(ValueError, 'uncalibrated'):
                self.policy.check_frame(wrong)

    def test_calibrated_themes_and_scales_accepted(self):
        for profile in ['wechat-4-dark', 'wechat-4-light']:
            for scale in [1, 2]:
                ok = frame(bubble('x'))
                ok['profile'], ok['scale'] = profile, scale
                self.policy.check_frame(ok)

    def test_incomplete_latest_bubble_blocks_auto_reply(self):
        partial = frame(bubble('old'), bubble('Bot验证1104'))
        partial['clipped_bubble'] = True
        with self.assertRaisesRegex(ValueError, 'incomplete bottom bubble'):
            self.policy.candidate(self.baseline, partial, partial)

    def test_reply_bounds(self):
        self.assertEqual(validate_reply('你好，这是自动回复测试。'), '你好，这是自动回复测试。')
        for body in ['', '  ', 'x'*201, '你好\x00']:
            with self.assertRaises(ValueError):
                validate_reply(body)


class BotRuntimeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.args = argparse.Namespace(state=str(Path(self.temp.name)/'state'), host='fixture',
            account='fixture', recipient='示例联系人甲', trigger='Bot验证1104', duration=10)
        self.calls = []
        self.generated = AsyncMock(return_value='你好，这是自动回复测试。')
        self.old = frame(bubble('old'))
        self.new = frame(bubble('old'), bubble('Bot验证1104 你好'))

    def tearDown(self):
        self.temp.cleanup()

    async def execute(self, frames, send_error=False):
        calls = self.calls
        class FakeMCP:
            def __init__(self, *_): pass
            async def start(self): pass
            async def close(self): pass
            async def tool(self, name, **args):
                calls.append((name,args))
                if name == 'send_message' and send_error:
                    raise RuntimeError('send outcome unknown')
                return {'status':'triggered','delivery':'unknown'}
        class FakeCapture:
            def __init__(self, *_): self.index = 0
            async def start(self): pass
            async def close(self): pass
            async def frame(self):
                result = copy.deepcopy(frames[self.index]); self.index += 1
                return result
        with patch.object(runner,'MCP',FakeMCP), patch.object(runner,'RemoteCapture',FakeCapture), \
             patch.object(runner,'generate',self.generated), patch.object(runner.asyncio,'sleep',AsyncMock()), \
             contextlib.redirect_stdout(io.StringIO()):
            await runner.run(self.args)

    async def test_one_model_reply_through_existing_idempotent_send(self):
        after = frame(*self.new['bubbles'],bubble('你好，这是自动回复测试。','outbound_visual'))
        await self.execute([self.old,self.new,self.new,self.new,after])
        sent = [args for name,args in self.calls if name=='send_message']
        self.assertEqual(len(sent),1)
        self.assertEqual(sent[0]['recipient'],'示例联系人甲')
        self.assertEqual(sent[0]['message'],'你好，这是自动回复测试。')
        self.assertTrue(sent[0]['idempotency_key'].startswith('bot-'))
        record = json.loads(next(Path(self.args.state).glob('bot-*.json')).read_text())
        self.assertTrue(record['client_echo_observed'])
        self.assertEqual(record['delivery'],'unknown')
        self.assertEqual(self.generated.await_count,1)

    async def test_model_latency_cannot_authorize_sending_into_changed_chat(self):
        changed = frame(*self.new['bubbles'],bubble('another'))
        with self.assertRaisesRegex(RuntimeError,'changed before sending'):
            await self.execute([self.old,self.new,self.new,changed])
        self.assertNotIn('send_message',[name for name,_ in self.calls])

    async def test_non_allowlisted_title_never_reaches_model(self):
        with self.assertRaisesRegex(ValueError,'allowlisted title'):
            await self.execute([frame(title='其他人')])
        self.generated.assert_not_awaited()
        self.assertNotIn('send_message',[name for name,_ in self.calls])

    async def test_unknown_send_is_not_regenerated_or_resent_on_restart(self):
        with self.assertRaisesRegex(RuntimeError,'send outcome unknown'):
            await self.execute([self.old,self.new,self.new,self.new],send_error=True)
        await self.execute([self.old,self.new,self.new])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)
        self.assertEqual(self.generated.await_count,1)


if __name__ == '__main__':
    unittest.main()


def image(phash, digest='image:50:40', media_id=None):
    return {'text': '[图片]', 'direction': 'inbound_visual', 'kind': 'image', 'digest': digest,
            'phash': phash, 'media_id': media_id or phash*4}


class ImageIdentityTests(unittest.TestCase):
    def test_loading_image_reconciles_to_same_fingerprint(self):
        from scripts.wechat_bot.policy import fingerprint, reconcile
        prev = [bubble('hi'), image('0f0f0f0f0f0f0f0f')]
        cur = [bubble('hi'), image('0f0f0f0f0f0f0f00')]  # loaded: 4 bits differ
        fixed = reconcile(prev, cur)
        self.assertEqual(fingerprint(fixed[1]), fingerprint(prev[1]))
        self.assertEqual(fixed[1]['media_id'], cur[1]['media_id'])  # download the loaded pixels
        self.assertEqual(new_bubbles(prev, fixed), [])
        self.assertEqual(cur[1]['phash'], '0f0f0f0f0f0f0f00')  # input not mutated

    def test_same_size_different_images_are_distinct(self):
        a, b, c = image('0000000000000000'), image('ffffffff00000000'), image('00000000ffffffff')
        self.assertEqual(new_bubbles([a, b], [a, b, c]), [c])

    def test_far_phash_is_not_reconciled(self):
        from scripts.wechat_bot.policy import reconcile
        prev = [image('0000000000000000')]
        cur = [image('00000000000fffff')]  # 20 bits
        self.assertEqual(reconcile(prev, cur)[0]['phash'], '00000000000fffff')


class StitchTests(unittest.TestCase):
    def test_stitch_joins_pages_after_cursor(self):
        from scripts.wechat_bot.policy import stitch
        a, b, c, d = bubble('a'), bubble('b'), bubble('c'), bubble('D')
        self.assertEqual(stitch([a, b], [[a, b, c], [c, d]]), [c, d])

    def test_stitch_without_overlap_raises(self):
        from scripts.wechat_bot.policy import ContinuityGap, stitch
        with self.assertRaises(ContinuityGap):
            stitch([bubble('a')], [[bubble('x')], [bubble('y')]])


class ReviewFixTests(unittest.TestCase):
    def test_similar_new_image_is_not_taken_for_scrolled_away_one(self):
        # Review C1: cursor ends with screenshot A; B, C, D (same size, B close to A) push A away.
        from scripts.wechat_bot.policy import ContinuityGap, reconcile
        a = image('0000000000000000')
        b, c, d = image('0000000000000fff'), image('ffffffff00000000'), image('00000000ffffffff')
        prev, cur = [bubble('x'), a], [b, c, d]
        fixed = reconcile(prev, cur)
        self.assertEqual(fixed[0]['phash'], b['phash'])
        with self.assertRaises(ContinuityGap):
            new_bubbles(prev, fixed)

    def test_loading_image_beside_text_still_reconciles(self):
        from scripts.wechat_bot.policy import reconcile
        prev = [bubble('x'), image('0000000000000000')]
        cur = [bubble('x'), image('0000000000000fff'), bubble('y')]
        self.assertEqual(new_bubbles(prev, reconcile(prev, cur)), [cur[2]])


def at_y(b, y, height=40):
    return {**b, 'y': y, 'height': height}


class MergePagesTests(unittest.TestCase):
    """Calibrated 2026-11-05: one wheel step moves the chat 90pt; tall images leave empty pages."""

    def test_pages_merge_by_position_across_empty_pages(self):
        from scripts.wechat_bot.policy import merge_pages
        a, b = bubble('a'), bubble('b')
        i1, i2 = image('0000000000000000'), image('ffffffff00000000')
        # Content top->bottom: a, b, i1 (247pt), i2 (247pt). Bottom page shows only i2.
        pages = [[at_y(i2, 23, 247)],                       # bottom
                 [at_y(i2, 113, 247)],                      # up 1
                 [],                                        # up 2: nothing complete
                 [at_y(i1, 26, 247)],                       # up 3
                 [at_y(a, 40), at_y(b, 100), at_y(i1, 116, 247)]]  # up 4
        merged = merge_pages(pages, step=90)
        self.assertEqual([m['text'] if m.get('kind') != 'image' else m['phash'] for m in merged],
                         ['a', 'b', i1['phash'], i2['phash']])

    def test_measured_offset_overrides_nominal_step(self):
        from scripts.wechat_bot.policy import merge_pages
        x, y = bubble('x'), bubble('y')
        pages = [[at_y(y, 300)], [at_y(x, 200), at_y(y, 380)]]  # actually moved 80pt
        self.assertEqual([m['text'] for m in merge_pages(pages, step=90)], ['x', 'y'])

    def test_disagreeing_pages_raise_gap(self):
        from scripts.wechat_bot.policy import ContinuityGap, merge_pages
        pages = [[at_y(bubble('y'), 300)], [at_y(bubble('z'), 390)]]  # 'z' sits where 'y' must be
        with self.assertRaises(ContinuityGap):
            merge_pages(pages, step=90)
