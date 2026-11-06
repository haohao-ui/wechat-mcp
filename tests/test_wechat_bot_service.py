import copy
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from scripts.wechat_bot.service import BotService, load_config
from scripts.wechat_bot.policy import echo_matches
from scripts.wechat_bot import control


def bubble(text,direction='inbound_visual'):
    return {'text':text,'digest':text,'direction':direction}


def frame(*bubbles,title='示例联系人甲'):
    return {'profile':'wechat-4-dark','scale':1,'title_lines':[title],'bubbles':list(bubbles),'clipped_bubble':False}


class ServiceTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.root=Path(self.temp.name)
        self.config={'schema_version':1,'host':'fixture','account_id':'fixture',
            'allowlist':[{'recipient':'示例联系人甲','kind':'private'}],'state_dir':str(self.root/'state'),
            'poll_seconds':2,'cooldown_seconds':0,'max_replies_per_hour':20,
            # Reply in the same cycle as the arrival; quiet-period behaviour has its own tests.
            'turn_quiet_seconds':0}
        self.service=BotService(self.config)
        self.old=frame(bubble('old'))
        self.new=frame(bubble('old'),bubble('你好，介绍一下你自己'))
        self.calls=[]
        calls=self.calls
        class Client:
            async def tool(self,name,**args):
                calls.append((name,args))
                return {'status':'triggered','send_id':'fixture-send','delivery':'unknown'}
        self.client=Client()
        self.model=AsyncMock(return_value='你好，我是AI客服助手。')

    def tearDown(self): self.temp.cleanup()

    def restart(self):
        extra={'history':self.history,'contact_id':self.cid} if hasattr(self,'history') else {}
        return BotService(self.config,**extra)

    async def cycle(self,frames):
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy(frames))
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)

    async def test_legacy_cursor_rebaselines_without_reply(self):
        # Cursor saved before fingerprint v2: its image digest hashed pixels and has no phash, so the
        # image can never match again. Rebaseline once instead of replying to visible history.
        from scripts.wechat_bot.service import write_private
        legacy=frame(bubble('old'),{'text':'[图片]','digest':'pixels','direction':'inbound_visual','kind':'image'})
        write_private(self.service.cursor_path,legacy)
        self.service=self.restart()
        current=frame(bubble('old'),{'text':'[图片]','digest':'image:50:40','phash':'0'*16,
                                     'direction':'inbound_visual','kind':'image'},bubble('新消息'))
        await self.cycle([current,current])
        self.model.assert_not_awaited()
        self.assertEqual(self.service.cursor['bubbles'],current['bubbles'])

    async def test_text_only_cursor_from_before_v2_still_replies(self):
        # Text digests did not change in v2: an old text-only cursor keeps working without a rebaseline.
        from scripts.wechat_bot.service import write_private
        write_private(self.service.cursor_path,self.old)
        self.service=self.restart()
        await self.cycle([self.new,self.new,self.new,self.new])
        self.model.assert_awaited()

    def at(self,seconds):
        self.service.clock=lambda:1000.0+seconds

    async def test_reply_waits_for_quiet_period(self):
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        one=frame(bubble('old'),bubble('图片说明'))
        two=frame(*one['bubbles'],bubble('这是什么'))
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        self.at(0); await self.cycle([one,one])
        self.at(5); await self.cycle([two,two])
        self.at(9); await self.cycle([two])
        self.model.assert_not_awaited()
        self.at(15); await self.cycle([two,two,frame(*two['bubbles'],reply)])
        self.model.assert_awaited_once()
        self.assertEqual(self.model.await_args.args[0],'图片说明\n这是什么')
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)
        self.assertIsNone(self.service.turn)

    async def test_turn_closes_at_max_even_while_customer_keeps_sending(self):
        self.config.update(turn_quiet_seconds=10,turn_max_seconds=60)
        self.service=self.restart()
        self.service.advance(self.old)
        bubbles=[bubble('old')]
        for second in range(0,60,5):
            bubbles.append(bubble('m%d'%second))
            self.at(second); await self.cycle([frame(*bubbles),frame(*bubbles)])
        self.model.assert_not_awaited()
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        self.at(60); await self.cycle([frame(*bubbles),frame(*bubbles),frame(*bubbles,reply)])
        self.model.assert_awaited_once()
        self.assertEqual(len(self.model.await_args.args[0].split('\n')),12)
        later=frame(*bubbles,reply,bubble('还有一个问题'))
        self.at(62); await self.cycle([later,later])
        self.model.assert_awaited_once()  # the next turn waits for its own quiet period
        self.assertEqual([b['text'] for b in self.service.turn['bubbles']],['还有一个问题'])

    async def test_open_turn_survives_restart(self):
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        self.at(0); await self.cycle([self.new,self.new])
        self.service=self.restart()
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        self.at(11); await self.cycle([self.new,self.new,frame(*self.new['bubbles'],reply)])
        self.model.assert_awaited_once()
        self.assertEqual(self.model.await_args.args[0],'你好，介绍一下你自己')

    async def test_image_only_turn_replies(self):
        img={'text':'[图片]','digest':'image:50:40','phash':'0'*16,'media_id':'a'*64,
             'direction':'inbound_visual','kind':'image'}
        self.service.advance(self.old)
        current=frame(bubble('old'),img)
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy([current,current,current,current,frame(bubble('old'),img,reply)]))
        shot=self.root/'x.png';shot.write_bytes(b'\x89PNG\r\n\x1a\nx')
        capture.attachments=AsyncMock(return_value=([{'path':shot,'label':'第1张图片','source':'chat_thumbnail','bubble':img}],[],[]))
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)
        self.model.assert_awaited_once()
        self.assertEqual(self.model.await_args.kwargs['images'],[shot])

    def test_config_turn_keys_validated(self):
        path=self.root/'config.json'
        path.write_text(json.dumps({k:v for k,v in self.config.items() if k!='turn_quiet_seconds'}))
        loaded=load_config(path)
        self.assertEqual((loaded['turn_quiet_seconds'],loaded['turn_max_seconds'],loaded['images_per_model_call']),(10,60,4))
        for key,value in [('turn_quiet_seconds',-1),('turn_quiet_seconds',121),('turn_max_seconds',5),
                          ('turn_max_seconds','60'),('images_per_model_call',0),('images_per_model_call',17)]:
            path.write_text(json.dumps({**self.config,key:value}))
            with self.assertRaises(ValueError): load_config(path)

    async def test_manual_reply_during_wait_cancels_turn(self):
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        self.at(0); await self.cycle([self.new,self.new])
        answered=frame(*self.new['bubbles'],bubble('人工：我是店长，马上处理','outbound_visual'))
        with patch('scripts.wechat_bot.service.log_event') as log:
            self.at(4); await self.cycle([answered])
        self.assertIn(('already_replied','screen'),[(c.args[0],c.kwargs.get('source')) for c in log.call_args_list])
        self.at(30); await self.cycle([answered])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])
        self.assertIsNone(self.service.turn)

    async def test_manual_reply_while_generating_blocks_send(self):
        self.service.advance(self.old)
        answered=frame(*self.new['bubbles'],bubble('人工：我是店长，马上处理','outbound_visual'))
        await self.cycle([self.new,self.new,answered])
        self.assertFalse(any(name=='send_message' for name,_ in self.calls))
        record=json.loads(next(self.service.root.glob('send-*.json')).read_text())
        self.assertEqual(record['status'],'already_replied')
        self.assertFalse(self.service.paused)

    async def test_reprocessed_triggered_turn_is_not_resent(self):
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        self.at(0); await self.cycle([self.new,self.new])
        key=self.service.batch_key(self.service.turn['baseline'],self.service.turn['bubbles'])
        (self.service.root/('send-'+key+'.json')).write_text(json.dumps({'status':'triggered','idempotency_key':key}))
        self.at(11); await self.cycle([self.new])
        self.assertFalse(self.service.paused)
        self.assertEqual(self.calls,[])
        self.assertIsNone(self.service.turn)

    async def test_own_echo_is_not_human_reply(self):
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        self.at(0); await self.cycle([self.new,self.new])
        # The customer wrote again just before our reply landed: the echo below it is ours, not a person's.
        again=frame(*self.new['bubbles'],bubble('第二个问题'),reply)
        self.at(11); await self.cycle([self.new,self.new,again])
        self.at(12); await self.cycle([again,again])
        self.assertEqual([b['text'] for b in self.service.turn['bubbles']],['第二个问题'])
        self.at(23); await self.cycle([again,again,frame(*again['bubbles'],bubble('好的','outbound_visual'))])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),2)

    async def test_ocr_noisy_own_echo_is_not_human_reply(self):
        # Review C3: OCR reads 'AI' as 'Al'; the echo is still ours, so the next turn must survive.
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        noisy=bubble('你好，我是Al客服助手。','outbound_visual')
        again=frame(*self.new['bubbles'],bubble('第二个问题'),noisy)
        self.at(0); await self.cycle([self.new,self.new])
        self.at(11); await self.cycle([self.new,self.new,again])
        self.at(12); await self.cycle([again,again])
        self.assertEqual([b['text'] for b in self.service.turn['bubbles']],['第二个问题'])

    async def test_upload_in_progress_delays_turn(self):
        # Acceptance 2026-11-05: the PDF card still read 对方上传中 when the quiet period ended.
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        card={'text':'报价.pdf\nPDF\n220.3K 对方上传中','digest':'file:报价.pdf:220.3K','direction':'inbound_visual',
              'kind':'file','filename':'报价.pdf','media_id':'1'*64,'transfer_pending':True}
        uploading=frame(bubble('old'),card)
        done=frame(bubble('old'),{**card,'text':'报价.pdf\nPDF\n220.3K','transfer_pending':False})
        self.at(0); await self.cycle([uploading,uploading])
        self.at(15); await self.cycle([uploading])
        self.model.assert_not_awaited()
        self.assertEqual(json.loads((self.service.root/'health.json').read_text())['state'],'waiting_for_upload')
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy([done,done,done,frame(*done['bubbles'],bubble('好的','outbound_visual'))]))
        capture.download=AsyncMock(side_effect=RuntimeError('no thumbnails'))
        capture.attachments=AsyncMock(return_value=([],[],[]))
        self.at(20)
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)
        self.model.assert_awaited_once()

    async def test_model_connection_failure_retries_before_pausing(self):
        # Nothing was sent yet: a transient model outage keeps the turn and retries, then pauses.
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        self.model=AsyncMock(side_effect=RuntimeError('model connection failed; not automatically retried'))
        self.at(0); await self.cycle([self.new,self.new])
        for second in [11,45]:
            self.at(second); await self.cycle([self.new,self.new])
            self.assertFalse(self.service.paused)
            self.assertIsNotNone(self.service.turn)
        self.at(80)
        with self.assertRaises(RuntimeError):
            await self.cycle([self.new,self.new])
        self.assertEqual(self.model.await_count,3)
        self.assertTrue(self.service.paused)
        self.assertEqual(self.calls,[])

    async def test_clipped_bottom_scrolls_down_instead_of_waiting_forever(self):
        # Acceptance 2026-11-05: images grew while loading and WeChat stayed scrolled up; the bottom
        # image stayed cut by the input box and the bot waited for a better frame that never came.
        self.service.advance(self.old)
        clipped={**self.new,'clipped_bubble':True}
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy([clipped,self.new,self.new,frame(*self.new['bubbles'],reply)]))
        capture.scroll=AsyncMock(return_value=copy.deepcopy(self.new))
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)
        capture.scroll.assert_awaited_once_with('down',pages=40)
        self.model.assert_awaited_once()

    async def test_group_turn_names_senders_and_strips_member_count(self):
        # 示例群 shows as "示例群 (3)"; each left bubble carries its sender name from the label above it.
        self.config['allowlist']=[{'recipient':'示例群','kind':'group'}]
        self.config['state_dir']=str(self.root/'group-state')
        extra={'history':self.history,'contact_id':self.history.ensure_contact('fixture',self.config['allowlist'][0])} if hasattr(self,'history') else {}
        self.service=BotService(self.config,**extra)
        old=frame(bubble('old'),title='示例群 (3)')
        self.service.advance(old)
        asked={**bubble('@享味来 有货吗'),'sender':'示例联系人甲'}
        new=frame(bubble('old'),asked,title='示例群 (3)')
        await self.cycle([new,new,new,frame(bubble('old'),asked,bubble('有的','outbound_visual'),title='示例群 (3)')])
        self.assertEqual(self.model.await_args.args[0],'示例联系人甲：@享味来 有货吗')
        self.assertEqual(self.model.await_args.kwargs.get('chat_kind'),'group')
        send=next(args for name,args in self.calls if name=='send_message')
        self.assertEqual(send['recipient'],'示例群')

    def test_private_title_with_parentheses_is_not_stripped(self):
        from scripts.wechat_bot.policy import Policy
        with self.assertRaises(ValueError):
            Policy('fixture','张三','service').check_frame(frame(title='张三 (3)'))
        Policy('fixture','示例群','service',group=True).check_frame(frame(title='示例群 (3)'))

    async def test_malformed_model_output_retries_before_pausing(self):
        # Acceptance 2026-11-05 (示例群): the model twice returned non-JSON for one image turn; the same
        # call succeeded when repeated. Nothing was sent, so it is retried like a connection failure.
        from scripts.wechat_bot.model_api import ModelOutputError
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        self.model=AsyncMock(side_effect=[ModelOutputError('model returned malformed JSON twice'),'你好，我是AI客服助手。'])
        self.at(0); await self.cycle([self.new,self.new])
        self.at(11); await self.cycle([self.new,self.new])
        self.assertFalse(self.service.paused)
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        self.at(45); await self.cycle([self.new,self.new,frame(*self.new['bubbles'],reply)])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_initial_start_does_not_reply_to_history(self):
        await self.cycle([self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])

    async def test_empty_baseline_never_turns_visible_history_into_new_messages(self):
        # Restart or theme switch captured an empty chat first (no chat open): the next frame's
        # bubbles are old history, not arrivals, so they become the baseline without a reply.
        self.service.advance(frame())
        await self.cycle([self.new,self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])
        self.assertEqual(len(self.service.cursor['bubbles']),len(self.new['bubbles']))

    async def test_frame_without_complete_bubbles_keeps_the_cursor(self):
        # Acceptance 2026-11-05 (group): tall images left no complete bubble on screen; that empty frame
        # replaced the cursor, so the next visible text was taken as history and never answered.
        self.service.advance(self.old)
        await self.cycle([frame()]*40)
        self.assertEqual(self.service.cursor['bubbles'],self.old['bubbles'])
        after=frame(*self.new['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,self.new,after])
        self.model.assert_awaited()
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_tall_media_scrolling_history_off_after_send_does_not_pause(self):
        # After a sent reply, tall images/file cards can push every remembered bubble off screen.
        # The send already has a durable receipt; the gap is handled like any other: rebaseline
        # once two frames agree, never pause and never send again.
        self.service.advance(self.old)
        scrolled=frame(bubble('[图片]'),bubble('在的，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,self.new,scrolled])
        self.assertFalse(self.service.paused)
        await self.cycle([scrolled,scrolled])
        self.assertFalse(self.service.paused)
        self.assertEqual(len([1 for name,_ in self.calls if name=='send_message']),1)
        self.assertEqual(self.service.cursor['bubbles'],scrolled['bubbles'])

    async def test_ordinary_incoming_has_no_test_prefix_and_replies_once(self):
        self.service.advance(self.old)
        after=frame(*self.new['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,self.new,after])
        sent=[args for name,args in self.calls if name=='send_message']
        self.assertEqual(len(sent),1)
        self.assertEqual(sent[0]['recipient'],'示例联系人甲')
        self.assertEqual(self.model.await_args.kwargs['mode'],'service')
        await self.cycle([after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_foreign_contact_never_calls_model(self):
        self.service.advance(self.old)
        with self.assertRaises(ValueError):
            await self.cycle([frame(bubble('你好'),title='其他人')])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])

    async def test_identical_repeated_message_is_skipped_without_pause(self):
        self.service.advance(frame(bubble('刚才发链接看了吗')))
        current=frame(bubble('刚才发链接看了吗'),bubble('刚才发链接看了吗'))
        await self.cycle([current,current])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])
        self.assertFalse(self.service.paused)
        self.assertEqual(self.service.cursor,current)
        self.assertEqual(list(self.service.root.glob('send-*.json')),[])
        # The next distinct message is handled normally from the new baseline.
        later=frame(*current['bubbles'],bubble('在吗'))
        after=frame(*later['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([later,later,later,after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_ambiguous_frame_must_be_stable_before_skipping(self):
        old=frame(bubble('刚才发链接看了吗'))
        self.service.advance(old)
        current=frame(bubble('刚才发链接看了吗'),bubble('刚才发链接看了吗'))
        await self.cycle([current,frame(*current['bubbles'],bubble('在吗'))])
        self.assertEqual(self.service.cursor,old)
        self.model.assert_not_awaited()

    async def test_continuity_gap_rebaselines_without_reply_or_pause(self):
        self.service.advance(self.old)
        cleared=frame(bubble('你好 在吗'))
        await self.cycle([cleared,cleared])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls,[])
        self.assertFalse(self.service.paused)
        self.assertEqual(self.service.cursor,cleared)
        # The next message after the new baseline is answered normally.
        later=frame(bubble('你好 在吗'),bubble('有人吗'))
        after=frame(*later['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([later,later,later,after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def scroll_cycle(self,frames,pages):
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy(frames))
        capture.scroll=AsyncMock(side_effect=copy.deepcopy(pages))
        capture.attachments=AsyncMock(return_value=([],[],[]))
        with patch('scripts.wechat_bot.service.generate',self.model), \
             patch('scripts.wechat_bot.service.log_event') as log:
            await self.service.cycle(self.client,capture)
        return capture,[call.args[0] for call in log.call_args_list]

    @staticmethod
    def chat(content,steps=0,height=436):
        """Frame of a chat whose content (bubble, height) list is bottom-aligned, scrolled up `steps` x 90pt."""
        placed,y=[],height-8
        for item,size in reversed(content):
            y-=size; placed.append((item,y,size)); y-=20
        visible=[{**item,'y':top+steps*90,'height':size} for item,top,size in reversed(placed)
                 if top+steps*90>=2 and top+steps*90+size<height-2]
        return frame(*visible)

    async def test_unrecoverable_gap_scrolls_at_most_five_pages(self):
        # Acceptance 2026-11-06: after a long absence the bot scrolled 30 pages up in front of the user and
        # then gave up. Tall media pushes the anchor away by a page or two; more is a real gap.
        self.service.advance(self.old)
        cleared=self.chat([(bubble('全新的消息'),40)])
        pages=[self.chat([(bubble('更早%d'%n),40)]) for n in range(60)]
        capture,events=await self.scroll_cycle([cleared]*8,pages)
        ups=[c for c in capture.scroll.await_args_list if c.args and c.args[0]=='up']
        self.assertLessEqual(len(ups),5)
        self.assertIn('gap_scroll_exhausted',events)

    async def test_interrupted_gap_scroll_retries_instead_of_rebaselining(self):
        # Review I2: a person moving the mouse during the gap scroll used to rebaseline, dropping the gap's messages.
        self.service.advance(frame(bubble('a'),bubble('b')))
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=[frame(bubble('x'))]*3)
        capture.scroll=AsyncMock(side_effect=ValueError('private visual operation failed: invalid("user_input_detected")'))
        with patch('scripts.wechat_bot.service.generate',self.model):
            with self.assertRaisesRegex(ValueError,'user_input_detected'):
                await self.service.cycle(self.client,capture)
        self.assertEqual([b['text'] for b in self.service.cursor['bubbles']],['a','b'])

    async def test_interrupted_media_fetch_keeps_the_turn_without_pausing(self):
        # Review I2: an aborted save-as must not pause the contact or tell the customer to resend.
        imgs=[{'text':'[图片]','digest':'image:50:40','phash':'0f'*8,'media_id':'1'*64,'direction':'inbound_visual','kind':'image'}]
        self.service.advance(self.old)
        current=frame(bubble('old'),*imgs)
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy([current]*4))
        capture.attachments=AsyncMock(side_effect=ValueError('private visual operation failed: invalid("user_input_detected")'))
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)
        self.assertFalse(self.service.paused)
        self.model.assert_not_awaited()
        self.assertIsNotNone(self.service.turn)
        self.assertEqual(list(self.service.root.glob('send-*.json')),[])

    async def test_tall_image_gap_recovered_by_scrolling(self):
        # A tall bubble pushed every remembered bubble off screen: scroll up to find the anchor.
        a,b,c,tall=bubble('a'),bubble('b'),bubble('c'),bubble('TALL')
        content=[(a,40),(b,40),(c,40),(tall,380)]
        self.service.advance(frame(a,b))
        bottom=self.chat(content)
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        ups=[self.chat(content,k) for k in (1,2)]
        capture,events=await self.scroll_cycle([bottom,bottom,bottom,bottom,frame(*bottom['bubbles'],reply)],ups+[bottom])
        self.assertNotIn('continuity_gap_rebaseline',events)
        self.assertEqual(capture.scroll.await_args_list[0].args,('up',))
        self.assertEqual(capture.scroll.await_args_list[-1].args[0],'down')
        text=self.model.await_args.args[0].split('\n')
        self.assertEqual(text,['c','TALL'])

    async def test_consecutive_tall_images_recovered_across_empty_pages(self):
        # Measured on the remote Mac: 247pt images 267pt apart leave pages with no complete bubble.
        img=lambda h:{'text':'[图片]','digest':'image:61:61','phash':h,'direction':'inbound_visual','kind':'image','media_id':h*4}
        a,b=bubble('a'),bubble('b')
        imgs=[img('0000000000000000'),img('ffffffff00000000'),img('00000000ffffffff')]
        content=[(a,40),(b,40)]+[(i,247) for i in imgs]
        self.service.advance(frame(a,b))
        bottom=self.chat(content)
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        ups=[self.chat(content,k) for k in range(1,9)]
        capture,events=await self.scroll_cycle([bottom,bottom,bottom,bottom,bottom,frame(*bottom['bubbles'],reply)],ups+[bottom])
        self.assertIn('continuity_gap_recovered',events)
        self.assertEqual(len(self.model.await_args.args[0].split('\n')),3)

    async def test_lost_scroll_down_is_retried_before_giving_up(self):
        # Review I9: the first scroll down did not reach the bottom; one overshoot burst fixes it.
        a,b,c,tall=bubble('a'),bubble('b'),bubble('c'),bubble('TALL')
        content=[(a,40),(b,40),(c,40),(tall,380)]
        self.service.advance(frame(a,b))
        bottom=self.chat(content)
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        ups=[self.chat(content,k) for k in (1,2)]
        capture,events=await self.scroll_cycle([bottom,ups[0],bottom,bottom,bottom,frame(*bottom['bubbles'],reply)],ups+[ups[0],bottom])
        self.assertIn('continuity_gap_recovered',events)
        self.model.assert_awaited_once()

    async def test_scroll_recovery_requires_return_to_bottom(self):
        a,b,c,tall=bubble('a'),bubble('b'),bubble('c'),bubble('TALL')
        content=[(a,40),(b,40),(c,40),(tall,380)]
        old=frame(a,b)
        self.service.advance(old)
        moved=frame(bubble('X'))
        ups=[self.chat(content,k) for k in (1,2)]
        capture,events=await self.scroll_cycle([self.chat(content),moved,moved],ups+[moved,moved])
        self.model.assert_not_awaited()
        self.assertEqual(self.service.cursor,old)
        self.assertEqual(json.loads((self.service.root/'health.json').read_text())['state'],'waiting_for_stable_frame')
        self.assertEqual(capture.scroll.await_args_list[-1].kwargs,{'pages':40})

    async def test_unstable_gap_frame_keeps_old_cursor(self):
        self.service.advance(self.old)
        await self.cycle([frame(bubble('完全不同')),frame(bubble('完全不同'),bubble('又变了'))])
        self.assertEqual(self.service.cursor,self.old)
        self.model.assert_not_awaited()

    async def test_own_message_is_not_input(self):
        self.service.advance(self.old)
        await self.cycle([frame(*self.old['bubbles'],bubble('我的消息','outbound_visual'))])
        self.model.assert_not_awaited()

    async def test_incoming_during_send_remains_pending(self):
        self.service.advance(self.old)
        after=frame(*self.new['bubbles'],bubble('补充问题'),bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,self.new,after])
        self.assertEqual(self.service.cursor,self.new)

    async def test_new_activity_during_generation_still_sends_and_stays_pending(self):
        # Spec §5.8: the closed turn is answered; what arrived meanwhile becomes the next turn.
        self.service.advance(self.old)
        changed=frame(*self.new['bubbles'],bubble('新问题'))
        after=frame(*changed['bubbles'],bubble('你好，我是AI客服助手。','outbound_visual'))
        await self.cycle([self.new,self.new,changed,after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)
        record=json.loads(next(self.service.root.glob('send-*.json')).read_text())
        self.assertEqual(record['status'],'triggered')
        self.assertEqual(self.service.cursor,self.new)  # '新问题' is still ahead of the cursor
        await self.cycle([after,after,after,frame(*after['bubbles'],bubble('好的','outbound_visual'))])
        self.assertEqual(self.model.await_args.args[0],'新问题')

    async def test_scrolled_echo_with_repeated_greeting_does_not_pause_or_resend(self):
        old=[bubble(x) for x in ['hi','hi','there','question','received']]
        self.service.advance(frame(*old))
        incoming=frame(*old,bubble('hi'))
        echo=bubble('你好，我是AI客服助手。','outbound_visual')
        after=frame(*incoming['bubbles'][1:],echo)
        await self.cycle([incoming,incoming,incoming,after])
        self.assertEqual(self.service.cursor,after)
        self.assertFalse(self.service.paused)
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)
        await self.cycle([after])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_model_failure_pauses_and_survives_restart(self):
        self.service.advance(self.old)
        self.model.side_effect=RuntimeError('model unavailable')
        with self.assertRaises(RuntimeError), patch('scripts.wechat_bot.service.MODEL_RETRIES',0):
            await self.cycle([self.new,self.new])
        self.assertTrue(self.service.paused)
        self.assertTrue(BotService(self.config).paused)
        self.assertEqual(self.calls,[])

    async def test_restart_accepts_cursor_saved_before_theme_profiles(self):
        self.service.advance(self.old)
        legacy=json.loads(self.service.cursor_path.read_text())
        legacy['profile']='wechat-4-dark-1x'; legacy.pop('scale',None)
        self.service.cursor_path.write_text(json.dumps(legacy))
        extra={'history':self.history,'contact_id':self.cid} if hasattr(self,'history') else {}
        restarted=BotService(self.config,**extra)
        self.assertEqual((restarted.cursor['profile'],restarted.cursor['scale']),('wechat-4-dark',1))

    async def test_stop_during_generation_never_sends_afterward(self):
        self.service.advance(self.old)
        async def stop_model(*_,**__):
            self.service.stop.set()
            return '你好，我是AI客服助手。'
        self.model.side_effect=stop_model
        await self.cycle([self.new,self.new])
        self.assertEqual(self.calls,[])
        record=json.loads(next(self.service.root.glob('send-*.json')).read_text())
        self.assertEqual(record['status'],'stopped_before_send')

    async def test_turn_stopped_before_send_is_answered_after_restart(self):
        # Review I4: a stop (launchd restart, deploy) after generation must not lose the turn.
        self.service.advance(self.old)
        async def stop_model(*_,**__):
            self.service.stop.set()
            return '你好，我是AI客服助手。'
        self.model.side_effect=stop_model
        await self.cycle([self.new,self.new])
        self.service=self.restart()
        self.model.side_effect=None
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        await self.cycle([self.new,self.new,frame(*self.new['bubbles'],reply)])
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)
        self.assertFalse(self.service.paused)

    async def test_rate_limit_preserves_pending_input(self):
        self.service.advance(self.old)
        (self.service.root/'send-limit.json').write_text(json.dumps({'status':'triggered','attempted_at':time.time()}))
        self.service.config['max_replies_per_hour']=1
        await self.cycle([self.new,self.new])
        self.model.assert_not_awaited()
        # The arrival waits in the open turn until the limit allows a reply.
        self.assertEqual([b['text'] for b in self.service.turn['bubbles']],['你好，介绍一下你自己'])
        self.assertEqual(list(self.service.root.glob('send-bot*.json')),[])

    def test_state_cannot_cross_accounts_or_contacts(self):
        with self.assertRaises(ValueError):
            BotService({**self.config,'account_id':'other'})

    def test_config_rejects_group_and_ambiguous_allowlist(self):
        path=self.root/'config.json'
        for contacts in [[],[{'recipient':'群','kind':'channel'}],self.config['allowlist']*2]:
            path.write_text(json.dumps({**self.config,'allowlist':contacts}))
            with self.assertRaises(ValueError): load_config(path)

    def test_idle_check_seconds_defaults_and_bounds(self):
        path=self.root/'config.json'
        path.write_text(json.dumps(self.config))
        self.assertEqual(load_config(path)['idle_check_seconds'],60)
        for value in [5,'60',7200]:
            path.write_text(json.dumps({**self.config,'idle_check_seconds':value}))
            with self.assertRaises(ValueError): load_config(path)

    def test_echo_normalizes_width_but_not_changed_numbers(self):
        self.assertTrue(echo_matches('收到~','收到～'))
        self.assertFalse(echo_matches('金额100','金额100.00'))

    def test_health_logs_changes_without_poll_spam_or_chat_body(self):
        with self.assertLogs('wechat_bot',level='INFO') as logs:
            self.service.health('watching')
            self.service.health('watching')
            self.service.health('generating',image_count=1)
        self.assertEqual(len(logs.output),2)
        self.assertIn('generating',logs.output[1])
        self.assertNotIn('input',logs.output[1])

    def test_health_logs_periodic_heartbeat(self):
        with self.assertLogs('wechat_bot',level='INFO') as logs:
            self.service.health('watching')
            self.service.last_health_log_at-=61
            self.service.health('watching')
        self.assertEqual(len(logs.output),2)

    def test_default_config_is_in_user_home(self):
        with patch.object(Path,'home',return_value=self.root):
            base, config, plist=control.locations()
        self.assertEqual(config,self.root/'.WeChatRPABot/config.json')
        self.assertEqual(base,config.parent)

    def test_status_reads_configured_state_after_config_move(self):
        base=self.root/'.WeChatRPABot'; base.mkdir()
        config=base/'config.json'; config.write_text(json.dumps(self.config))
        health={'state':'watching','pid':123}
        (self.service.root/'health.json').write_text(json.dumps(health))
        output=io.StringIO()
        with patch.object(sys,'argv',['bot-control','status']), \
             patch.object(control,'locations',return_value=(base,config,self.root/'fixture.plist')), \
             patch.object(control,'loaded',return_value=True),contextlib.redirect_stdout(output):
            control.main()
        self.assertEqual(json.loads(output.getvalue())['health'],health)

    def test_stop_waits_until_launchd_task_is_unloaded(self):
        with patch.object(sys,'argv',['bot-control','stop']), \
             patch.object(control,'loaded',side_effect=[True,True,False,False]) as state, \
             patch.object(control.subprocess,'run') as run, patch.object(control.time,'sleep') as sleep, \
             contextlib.redirect_stdout(io.StringIO()):
            control.main()
        self.assertEqual(state.call_count,4)
        sleep.assert_called_once_with(0.1)
        self.assertEqual([call.args[0][1] for call in run.call_args_list],['disable','bootout'])

    def test_start_reenables_and_loads_unloaded_task(self):
        with patch.object(sys,'argv',['bot-control','start']), patch.object(control,'loaded',return_value=False), \
             patch.object(control,'load_config'), patch.object(control.subprocess,'run') as run, \
             contextlib.redirect_stdout(io.StringIO()):
            control.main()
        self.assertEqual([call.args[0][1] for call in run.call_args_list],['enable','bootstrap'])


if __name__=='__main__': unittest.main()


class HistoryServiceTests(ServiceTests):
    """Same fixtures as ServiceTests, with a real history store attached."""

    def setUp(self):
        super().setUp()
        import sqlite3
        from scripts.wechat_bot.history.store import HistoryStore
        self.sqlite3 = sqlite3
        self.history = HistoryStore.open(self.root/'h')
        self.cid = self.history.ensure_contact('fixture', self.config['allowlist'][0])
        self.config['state_dir'] = str(self.root/'state-history')
        self.service = BotService(self.config, history=self.history, contact_id=self.cid)

    def tearDown(self):
        self.history.conn.close()
        super().tearDown()

    async def test_model_failure_pauses_and_survives_restart(self):
        self.service.advance(self.old)
        self.model.side_effect = RuntimeError('model unavailable')
        with self.assertRaises(RuntimeError), patch('scripts.wechat_bot.service.MODEL_RETRIES', 0):
            await self.cycle([self.new, self.new])
        self.assertTrue(self.service.paused)
        self.assertTrue(BotService(self.config, history=self.history, contact_id=self.cid).paused)
        self.assertEqual(self.calls, [])

    def senders(self):
        return [r[0] for r in self.history.conn.execute('select sender from messages order by seq')]

    def texts(self):
        return [r[0] for r in self.history.conn.execute('select text from messages order by seq')]

    async def test_messages_recorded_before_model_call(self):
        self.service.advance(self.old)
        self.history.record_frame(self.cid, self.old)
        order = []
        real = self.history.record_frame
        self.history.record_frame = lambda *a, **k: (order.append('db'), real(*a, **k))[1]
        async def model(*a, **k):
            order.append('model')
            return '你好，我是AI客服助手。'
        self.model.side_effect = model
        after = frame(*self.new['bubbles'], bubble('你好，我是AI客服助手。', 'outbound_visual'))
        await self.cycle([self.new, self.new, self.new, after])
        self.assertEqual(order[:2], ['db', 'model'])
        self.assertEqual(self.senders(), ['customer', 'customer', 'bot'])
        send = self.history.send(json.loads(next(self.service.root.glob('send-*.json')).read_text())['idempotency_key'])
        self.assertEqual(send['status'], 'triggered')

    async def test_context_uses_stored_history_limit(self):
        self.config['context_messages'] = 2
        base = frame(*[bubble(str(i)) for i in range(5)])
        self.history.record_frame(self.cid, base)
        self.service.advance(base)
        new = frame(*base['bubbles'], bubble('新问题'))
        await self.cycle([new, new, new, new])
        ctx = self.model.await_args.kwargs['context']
        self.assertEqual([c['text'] for c in ctx], ['3', '4', '新问题'])

    async def test_history_failure_blocks_model_and_send(self):
        self.service.advance(self.old)
        self.history.record_frame = unittest.mock.Mock(side_effect=self.sqlite3.OperationalError('disk full'))
        await self.cycle([self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.calls, [])
        self.assertEqual(self.service.cursor, self.old)
        health = json.loads((self.service.root/'health.json').read_text())
        self.assertEqual(health['state'], 'history_write_failed')

    async def test_history_read_failure_keeps_message_pending(self):
        self.service.advance(self.old)
        self.history.recent_context = unittest.mock.Mock(side_effect=self.sqlite3.OperationalError('locked'))
        await self.cycle([self.new, self.new, self.new])
        self.model.assert_not_awaited()
        self.assertEqual([b['text'] for b in self.service.turn['bubbles']], ['你好，介绍一下你自己'])
        self.assertEqual(list(self.service.root.glob('send-*.json')), [])
        self.assertEqual(json.loads((self.service.root/'health.json').read_text())['state'], 'history_write_failed')

    async def test_scroll_recovered_messages_are_recorded_in_history(self):
        # Review I7: messages found by scrolling up exist in history too (spec G4).
        a,b,c,tall=bubble('a'),bubble('b'),bubble('c'),bubble('TALL')
        content=[(a,40),(b,40),(c,40),(tall,380)]
        self.service.advance(frame(a,b))
        self.history.record_frame(self.cid,frame(a,b))
        bottom=self.chat(content)
        reply=bubble('你好，我是AI客服助手。','outbound_visual')
        await self.scroll_cycle([bottom,bottom,bottom,bottom,frame(*bottom['bubbles'],reply)],
                                [self.chat(content,1),self.chat(content,2),bottom])
        texts=[r[0] for r in self.history.conn.execute('SELECT text FROM messages WHERE contact_id=? ORDER BY seq',(self.cid,))]
        self.assertEqual(texts[:4],['a','b','c','TALL'])

    async def test_baseline_frame_is_recorded_without_reply(self):
        await self.cycle([self.new])
        self.model.assert_not_awaited()
        self.assertEqual(self.texts(), ['old', '你好，介绍一下你自己'])

    async def test_history_reply_blocks_send(self):
        self.service.advance(self.old)
        self.history.replied_after=unittest.mock.Mock(return_value=True)
        await self.cycle([self.new,self.new,self.new])
        self.assertFalse(any(name=='send_message' for name,_ in self.calls))
        record=json.loads(next(self.service.root.glob('send-*.json')).read_text())
        self.assertEqual(record['status'],'already_replied')

    async def test_turn_media_attached_to_history(self):
        img={'text':'[图片]','digest':'image:50:40','phash':'0'*16,'media_id':'a'*64,
             'direction':'inbound_visual','kind':'image'}
        self.service.advance(self.old)
        current=frame(bubble('old'),img)
        path=self.root/'x.png';path.write_bytes(b'\x89PNG\r\n\x1a\noriginal')
        thumb=self.root/'t.png';thumb.write_bytes(b'\x89PNG\r\n\x1a\nthumbnail')
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy([current,current,current,current,
                                frame(bubble('old'),img,bubble('你好，我是AI客服助手。','outbound_visual'))]))
        capture.download=AsyncMock(return_value=thumb)  # history records the thumbnail on sight
        capture.attachments=AsyncMock(return_value=([{'path':path,'label':'第1张图片','source':'original','bubble':img}],[],
                                                    [{'label':'a.zip','kind':'file','reason':'unsupported_type'}]))
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)
        sources=[r[0] for r in self.history.conn.execute('SELECT source FROM media')]
        self.assertIn('original',sources)
        self.assertEqual(self.model.await_args.kwargs['images'],[path])

    async def media_turn(self,count,failed=(),reply='已收到'):
        imgs=[{'text':'[图片]','digest':'image:50:40','phash':'%016x'%(n*0x0f0f0f0f0f),'media_id':('%x'%n)*64,
               'direction':'inbound_visual','kind':'image'} for n in range(1,count+1)]
        self.service.advance(self.old)
        current=frame(bubble('old'),*imgs)
        sent=frame(*current['bubbles'],bubble(reply,'outbound_visual'))
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy([current,current,current,current,sent]))
        capture.download=AsyncMock(side_effect=RuntimeError('no thumbnails in this fixture'))
        self.service.config['model_config']={'backend':'openai-compatible','base_url':'https://api.deepseek.com',
                                             'model':'m','api_key_env':'K','supports_images':True}
        for n in range(1,count+1): (self.root/('%d.png'%n)).write_bytes(b'\x89PNG\r\n\x1a\n%03d'%n)  # 11 bytes
        items=[{'path':self.root/('%d.png'%n),'label':'第%d张图片'%n,'source':'original','bubble':b} for n,b in enumerate(imgs,1)]
        capture.attachments=AsyncMock(return_value=(items,[],list(failed)))
        self.model=AsyncMock(return_value=reply)
        notes=AsyncMock(side_effect=lambda images,**kw:['要点'+i['label'] for i in images])
        with patch('scripts.wechat_bot.service.generate',self.model), patch('scripts.wechat_bot.service.understand',notes):
            await self.service.cycle(self.client,capture)
        return notes

    async def test_nine_image_turn_sends_one_reply(self):
        notes=await self.media_turn(9)
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)
        notes.assert_awaited_once()
        self.assertEqual(self.model.await_args.kwargs['images'],[])
        self.assertEqual(len(self.model.await_args.kwargs['notes']),9)

    async def test_few_images_go_to_the_model_directly(self):
        notes=await self.media_turn(3)
        notes.assert_not_awaited()
        self.assertEqual(len(self.model.await_args.kwargs['images']),3)

    async def test_failed_item_named_when_model_omits_it(self):
        await self.media_turn(8,failed=[{'label':'第9张图片','kind':'image','reason':'download_failed'}],reply='已收到')
        message=next(args['message'] for name,args in self.calls if name=='send_message')
        self.assertIn('第9张图片打不开，麻烦重发一下。',message)
        self.assertLessEqual(len(message),200)
        # The model gets each unreadable item with its reason (2026-11-06: labels alone made it guess).
        self.assertEqual(self.model.await_args.kwargs['failed'],[{'label':'第9张图片','kind':'image','reason':'download_failed'}])

    async def test_unsupported_file_reason_reaches_the_model(self):
        # Acceptance 2026-11-06: an .xls turn crashed with TypeError ('str' object is not a mapping) and
        # the chat paused, because the service passed only labels while the prompt expected items.
        await self.media_turn(1,failed=[{'label':'报价.xls','kind':'file','reason':'unsupported_type','detail':'unsupported_file_type'}])
        self.assertEqual(self.model.await_args.kwargs['failed'][-1]['reason'],'unsupported_type')
        self.assertEqual(sum(name=='send_message' for name,_ in self.calls),1)

    async def test_final_generation_failure_still_pauses(self):
        self.model=AsyncMock(side_effect=RuntimeError('reply generation failed'))
        self.service.advance(self.old)
        with patch('scripts.wechat_bot.service.generate',self.model), self.assertRaises(RuntimeError), \
             patch('scripts.wechat_bot.service.MODEL_RETRIES',0):
            capture=type('Capture',(),{})()
            capture.frame=AsyncMock(side_effect=copy.deepcopy([self.new,self.new]))
            await self.service.cycle(self.client,capture)
        self.assertTrue(self.service.paused)

    async def test_media_is_fetched_from_the_current_frame(self):
        # Review C2: file media IDs belong to the frame they were seen in; a later text changes the frame.
        pdf={'text':'报价.pdf\n12KB','digest':'file:报价.pdf:12KB','media_id':'1'*64,'filename':'报价.pdf',
             'direction':'inbound_visual','kind':'file'}
        self.config['turn_quiet_seconds']=10
        self.service=self.restart()
        self.service.advance(self.old)
        first=frame(bubble('old'),pdf)
        later=frame(bubble('old'),pdf,bubble('帮我看看'))
        now={**later,'snapshot_id':'s-now','bubbles':[bubble('old'),{**pdf,'media_id':'2'*64},bubble('帮我看看')]}
        self.at(0); await self.cycle([first,first])
        self.at(3); await self.cycle([later,later])
        capture=type('Capture',(),{})()
        capture.frame=AsyncMock(side_effect=copy.deepcopy([later,now,now,frame(*now['bubbles'],bubble('好的','outbound_visual'))]))
        capture.download=AsyncMock(side_effect=RuntimeError('no thumbnails'))
        capture.attachments=AsyncMock(return_value=([],[],[]))
        self.at(14)
        with patch('scripts.wechat_bot.service.generate',self.model):
            await self.service.cycle(self.client,capture)
        items,snapshot=capture.attachments.await_args.args
        self.assertEqual([i.get('media_id') for i in items if i.get('kind')=='file'],['2'*64])
        self.assertEqual(snapshot['snapshot_id'],'s-now')

    async def test_model_is_told_this_turns_image_count_separately(self):
        # Review I1: earlier images re-attached from context were counted as this turn's.
        await self.media_turn(3)
        kwargs=self.model.await_args.kwargs
        self.assertEqual(kwargs['image_count'],3)
        self.assertEqual(kwargs['earlier_images'],len(kwargs['images'])-3)

    async def test_earlier_context_images_follow_this_turns_and_are_counted(self):
        temp=Path(tempfile.mkdtemp()); this,old=temp/'this.png',temp/'old.png'
        this.write_bytes(b'x'*10); old.write_bytes(b'y'*10)
        notes,model_images,earlier=await self.service.understand_images(
            '[图片]',[{'label':'第1张图片','path':str(this)}],[],[{'role':'customer','text':'[图片]','media':{'kind':'image','path':str(old)}}])
        self.assertEqual(model_images,[str(this),str(old)])
        self.assertEqual(earlier,1)

    async def test_images_over_the_request_size_are_named_not_paused(self):
        # Review I8: the API rejects more than 30MB of images; the excess is unreadable, not a pause.
        with patch('scripts.wechat_bot.service.MAX_MODEL_IMAGE_BYTES',25):
            await self.media_turn(3)
        self.assertFalse(self.service.paused)
        self.assertEqual(len(self.model.await_args.kwargs['images']),2)
        self.assertEqual(self.model.await_args.kwargs['failed'],[{'label':'第3张图片','kind':'image','reason':'too_large'}])

    async def test_send_uses_observed_title_for_alias(self):
        entry = {'recipient': '🍓示例联系人甲', 'kind': 'private', 'aliases': ['示例联系人甲']}
        self.config['allowlist'] = [entry]
        cid = self.history.ensure_contact('fixture', entry)
        self.config['state_dir'] = str(self.root/'state-alias')
        self.service = BotService(self.config, history=self.history, contact_id=cid)
        self.service.advance(self.old)
        after = frame(*self.new['bubbles'], bubble('你好，我是AI客服助手。', 'outbound_visual'))
        await self.cycle([self.new, self.new, self.new, after])
        sent = [args for name, args in self.calls if name == 'send_message']
        self.assertEqual(sent[0]['recipient'], '示例联系人甲')

    def test_binding_uses_contact_id(self):
        binding = json.loads((self.service.root/'binding.json').read_text())
        self.assertEqual(binding['contact_id'], self.cid)
        self.assertNotIn('recipient', binding)


class ConfigFieldTests(unittest.TestCase):
    def test_config_context_messages_and_http_bounds(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'config.json'
            base = {'schema_version': 1, 'host': 'fixture', 'account_id': 'fixture',
                    'allowlist': [{'recipient': '示例联系人甲', 'kind': 'private'}], 'state_dir': str(Path(temp)/'s')}
            path.write_text(json.dumps(base))
            loaded = load_config(path)
            self.assertEqual(loaded['context_messages'], 5)
            self.assertEqual(loaded['history_http'], {'bind': '127.0.0.1', 'port': 8765})
            self.assertIs(loaded['manage_window'], True)  # dedicated machine: bot may restore/resize WeChat
            for extra in [{'manage_window': 'yes'}, {'manage_window': 1}, {'context_messages': -1}, {'context_messages': 101}, {'context_messages': '5'},
                          {'history_http': {'bind': '127.0.0.1', 'port': 0}}, {'history_http': {'host': 'x'}}]:
                path.write_text(json.dumps({**base, **extra}))
                with self.assertRaises(ValueError):
                    load_config(path)
