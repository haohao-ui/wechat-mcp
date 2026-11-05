import asyncio
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

from scripts.wechat_bot.coordinator import BotCoordinator
from scripts.wechat_bot.routing import validate_rules, decision
from scripts.wechat_bot.service import load_config


def target(name,kind='private'): return {'recipient':name,'kind':kind}


class RoutingTests(unittest.TestCase):
    def test_multiple_private_and_group_exclusions_and_deny_priority(self):
        config={'allowlist':[target('a'),target('b')], 'ignorelist':[target('a'),target('group','group'),target('other')]}
        validate_rules(config)
        self.assertEqual(decision(config,'a'),'ignored')
        self.assertEqual(decision(config,'b'),'allowed')
        self.assertEqual(decision(config,'other'),'ignored')
        self.assertEqual(decision(config,'stranger'),'not_allowlisted')
        self.assertEqual(decision(config,'group','group'),'ignored')
        self.assertEqual(decision(config,'stranger','group'),'not_allowlisted')

    def test_aliases_match_and_conflicts_rejected(self):
        config={'allowlist':[{'recipient':'🍓小美','kind':'private','aliases':['@小美']}]}
        validate_rules(config)
        self.assertEqual(decision(config,'小美'),'allowed')
        self.assertEqual(decision(config,'@小美'),'allowed')
        with self.assertRaises(ValueError):
            validate_rules({'allowlist':[target('小美'),target('🍓小美')]})
        with self.assertRaises(ValueError):
            validate_rules({'allowlist':[{'recipient':'a','kind':'private','aliases':'a'}]})

    def test_ignorelist_uses_name_key(self):
        config={'allowlist':[target('🍓小美')],'ignorelist':[target('小美')]}
        self.assertEqual(decision(config,'🍓小美'),'ignored')

    def test_match_entry_returns_allowlist_entry(self):
        from scripts.wechat_bot.routing import match_entry
        entry={'recipient':'🍓小美','kind':'private','aliases':['@小美']}
        self.assertIs(match_entry({'allowlist':[entry]},'小美'),entry)
        self.assertIsNone(match_entry({'allowlist':[entry]},'别人'))

    def test_names_longer_than_worker_limit_rejected(self):
        with self.assertRaises(ValueError): validate_rules({'allowlist':[target('长'*65)]})
        with self.assertRaises(ValueError): validate_rules({'allowlist':[{'recipient':'a','kind':'private','aliases':['长'*65]}]})
        validate_rules({'allowlist':[target('长'*64)]})

    def test_same_name_exclusion_blocks_ambiguous_kind(self):
        config={'allowlist':[target('same')],'ignorelist':[target('same','group')]}
        self.assertEqual(decision(config,'same','private'),'ignored')

    def test_malformed_rules_never_become_empty_or_permissive(self):
        for ignore in (None,{},[target('')],[target(' a')],[target('a','unknown')],[target('a'),target('a')],[{'recipient':'a'}],[None]):
            with self.assertRaises(ValueError): validate_rules({'allowlist':[target('a')],'ignorelist':ignore})
        with self.assertRaises(ValueError): validate_rules({'allowlist':[target('group','group')]})


class CoordinatorTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.path=self.root/'config.json'
        self.config={'schema_version':1,'host':'fixture','account_id':'fixture','state_dir':str(self.root/'state'),
            'allowlist':[target('a'),target('b')],'ignorelist':[],'poll_seconds':1}
        self.save();self.coordinator=BotCoordinator(self.path)

    def save(self): self.path.write_text(json.dumps(self.config))
    def tearDown(self): self.temp.cleanup()

    async def test_ignored_target_never_connects_captures_models_or_sends(self):
        self.config['ignorelist']=[target('a'),target('b','group')];self.save()
        self.coordinator.connect=AsyncMock()
        with patch('scripts.wechat_bot.service.generate',new_callable=AsyncMock) as model:
            await self.coordinator.sweep()
        self.coordinator.connect.assert_not_awaited();model.assert_not_awaited()
        health=json.loads((self.coordinator.root/'health.json').read_text())
        self.assertEqual(health['state'],'ignored')
        self.assertTrue(all(s['state']=='ignored' for s in health['sessions'].values()))

    async def test_sessions_isolate_history_limits_and_pause(self):
        a=self.coordinator.session(target('a'));b=self.coordinator.session(target('b'))
        self.assertNotEqual(a.root,b.root)
        a.recent_files=[{'filename':'private-a'}]
        self.assertEqual(b.recent_files,[])
        a.paused=True;a.pause_reason='fixture'
        self.coordinator.connect=AsyncMock()
        b.cycle=AsyncMock()
        await self.coordinator.sweep()
        self.coordinator.connect.assert_awaited_once_with(b)
        b.cycle.assert_awaited_once()
        self.assertEqual(json.loads((self.coordinator.root/'health.json').read_text())['state'],'watching')

    async def test_slots_are_serial_and_preserve_selection(self):
        events=[];active=0
        async def connect(child): events.append(('connect',child.recipient))
        self.coordinator.connect=connect
        for name in ('a','b'):
            child=self.coordinator.session(target(name))
            async def cycle(*_,name=name):
                nonlocal active
                active+=1;self.assertEqual(active,1)
                events.append(('cycle',name));await asyncio.sleep(0);active-=1
            child.cycle=cycle
        await self.coordinator.sweep()
        self.assertEqual(events,[('connect','a'),('cycle','a'),('connect','b'),('cycle','b')])

    async def test_hot_exclusion_after_generation_blocks_send(self):
        child=self.coordinator.session(target('a'))
        old={'profile':'wechat-4-dark','scale':1,'title_lines':['a'],'bubbles':[{'text':'old','digest':'old','direction':'inbound_visual'}]}
        new={**old,'bubbles':old['bubbles']+[{'text':'new','digest':'new','direction':'inbound_visual'}]}
        child.advance(old)
        capture=type('Capture',(),{})();capture.frame=AsyncMock(return_value=new)
        client=type('Client',(),{})();client.tool=AsyncMock()
        async def model(*_,**__):
            self.config['ignorelist']=[target('a')];self.save()
            return '你好，我是AI客服助手。'
        with patch('scripts.wechat_bot.service.generate',side_effect=model): await child.cycle(client,capture)
        client.tool.assert_not_awaited()
        journal=json.loads(next(child.root.glob('send-*.json')).read_text())
        self.assertEqual(journal['status'],'skipped_by_rule')

    async def test_removed_contact_stays_disabled_across_restart(self):
        self.coordinator.session(target('a'))
        self.config['allowlist']=[target('b')];self.save()
        restarted=BotCoordinator(self.path)
        self.assertIn('a',restarted.sessions)
        restarted.connect=AsyncMock()
        restarted.session(target('b')).cycle=AsyncMock()
        await restarted.sweep()
        self.assertTrue((restarted.sessions['a'].root/'routing-disabled.json').exists())

    async def test_initial_exclusion_checked_before_capture(self):
        child=self.coordinator.session(target('a'))
        self.config['ignorelist']=[target('a')];self.save()
        capture=type('Capture',(),{})();capture.frame=AsyncMock()
        await child.cycle(None,capture)
        capture.frame.assert_not_awaited()

    async def test_reenabled_chat_baselines_ignored_period(self):
        child=self.coordinator.session(target('a'));child.cursor={'old':'not read'}
        self.coordinator.disable_session(target('a'),'ignored')
        self.coordinator.connect=AsyncMock()
        async def cycle(*_):
            self.assertIsNone(child.cursor)
            child.cursor={'new':'baseline'}
        child.cycle=cycle
        await self.coordinator.visit(target('a'))
        self.assertFalse((child.root/'routing-disabled.json').exists())

    async def test_incomplete_intake_frame_waits_without_permanent_pause(self):
        child=self.coordinator.session(target('a'))
        self.coordinator.connect=AsyncMock()
        child.cycle=AsyncMock(side_effect=ValueError('incomplete bottom bubble; no automatic reply'))
        await self.coordinator.visit(target('a'))
        self.assertFalse(child.paused)
        self.assertEqual(json.loads((child.root/'health.json').read_text())['state'],'waiting_for_stable_frame')

    async def test_unusable_wechat_window_retries_without_permanent_pause(self):
        # e.g. macOS switched to light appearance: nothing was read or sent, so retry next sweep.
        child=self.coordinator.session(target('a'))
        self.coordinator.connect=AsyncMock(side_effect=ValueError('private visual operation failed: invalid("uncalibrated_theme")'))
        self.coordinator.close_connection=AsyncMock()
        await self.coordinator.visit(target('a'))
        self.assertFalse(child.paused)
        health=json.loads((child.root/'health.json').read_text())
        self.assertEqual(health['state'],'waiting_for_wechat_window')
        self.assertIn('uncalibrated_theme',health['reason'])
        self.coordinator.close_connection.assert_awaited()
        self.coordinator.connect=AsyncMock()
        child.cycle=AsyncMock()
        await self.coordinator.visit(target('a'))
        child.cycle.assert_awaited_once()

    async def test_vision_timeout_retries_without_permanent_pause(self):
        child=self.coordinator.session(target('a'))
        self.coordinator.connect=AsyncMock(side_effect=ValueError('private visual operation failed: invalid("vision_timeout")'))
        await self.coordinator.visit(target('a'))
        self.assertFalse(child.paused)
        self.assertEqual(json.loads((child.root/'health.json').read_text())['state'],'waiting_for_wechat_window')

    async def test_other_worker_refusal_still_pauses(self):
        child=self.coordinator.session(target('a'))
        self.coordinator.connect=AsyncMock(side_effect=ValueError('private visual operation failed: invalid("allowlisted_title_mismatch")'))
        await self.coordinator.visit(target('a'))
        self.assertTrue(child.paused)

    async def test_native_retarget_keeps_worker_and_isolates_media_destination(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        child=self.coordinator.session(target('b'))
        capture=NativeCapture('fixture',None,'a',self.root/'a')
        capture.retarget=AsyncMock()
        self.coordinator.capture=capture;self.coordinator.active_name='a'
        await self.coordinator.connect(child)
        self.assertIs(self.coordinator.capture,capture)
        capture.retarget.assert_awaited_once_with('b',child.root,['b'])
        self.assertEqual(self.coordinator.active_name,'b')

    async def test_reconnect_reuses_running_worker_before_mcp_search(self):
        # A minimized window makes MCP search fail; the running worker can restore it and click the row.
        from scripts.wechat_bot.native_capture import NativeCapture
        capture=NativeCapture('fixture',AsyncMock(),'a',self.root/'a','0'*64,names=['a'])
        capture.ssh=AsyncMock(); capture.connect=AsyncMock(); capture.navigate=AsyncMock()
        capture.call=AsyncMock(return_value={'opened':True})
        await capture.start()
        capture.call.assert_awaited_once_with('open_session',target='a',target_names=['a'])
        capture.navigate.assert_not_awaited()
        capture.call=AsyncMock(side_effect=ValueError('private visual operation failed: invalid("wechat_not_visible")'))
        capture.close_transport=AsyncMock()
        await capture.start()
        capture.navigate.assert_awaited_once_with(['a'])

    async def test_worker_calls_carry_manage_window_switch(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        for flag in (True, False):
            capture=NativeCapture('fixture',None,'a',self.root/'a',manage_window=flag)
            sent=[]
            class Pipe:
                def write(self,data): sent.append(json.loads(data))
                async def drain(self): pass
            class Out:
                async def readline(self): return (json.dumps({'protocol_version':1,'id':sent[-1]['id'],'ok':True,'result':{}})+'\n').encode()
            capture.process=type('P',(),{'stdin':Pipe(),'stdout':Out()})()
            await capture.call('sessions',names=['a'])
            self.assertIs(sent[-1]['params']['manage_window'],flag)

    async def test_window_resize_by_worker_is_logged(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        capture=NativeCapture('fixture',None,'a',self.root/'a')
        capture.call=AsyncMock(return_value={'profile':'wechat-4-light','scale':2,'window_resized':True})
        with patch('scripts.wechat_bot.service.log_event') as logged:
            await capture.frame()
        logged.assert_called_once_with('window_resized',profile='wechat-4-light',scale=2)
        capture.call=AsyncMock(return_value={'rows':[],'window_resized':True})
        with patch('scripts.wechat_bot.service.log_event') as logged:
            await capture.sessions(['a'])
        logged.assert_called_once_with('window_resized',profile=None,scale=None)

    async def test_native_retarget_switches_recipient_and_directory(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        capture=NativeCapture('fixture',AsyncMock(),'a',self.root/'a')
        capture.frame=AsyncMock()
        capture.call=AsyncMock(return_value={'opened':True,'clicked':True,'seconds':0.4})
        await capture.retarget('b',str(self.root/'b'))
        capture.call.assert_awaited_once_with('open_session',target='b',target_names=['b'])
        capture.client.tool.assert_not_awaited()  # fast click path skips MCP navigation
        self.assertEqual((capture.recipient,capture.directory),('b',self.root/'b'))
        capture.frame.assert_awaited_once()

    async def test_native_retarget_falls_back_when_click_is_refused(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        capture=NativeCapture('fixture',AsyncMock(),'a',self.root/'a')
        capture.frame=AsyncMock()
        capture.call=AsyncMock(side_effect=ValueError('private visual operation failed: wechat_row_not_topmost'))
        await capture.retarget('b',str(self.root/'b'))
        capture.client.tool.assert_awaited_once_with('get_messages',recipient='b')
        self.assertEqual(capture.recipient,'b')
        self.assertFalse(capture.last_switch['opened'])

    async def test_fallback_navigation_tries_alias_names(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        client=AsyncMock()
        client.tool=AsyncMock(side_effect=[RuntimeError('title mismatch'),{'ok':True}])
        capture=NativeCapture('fixture',client,'🍓a',self.root/'a',names=['🍓a','a'])
        capture.frame=AsyncMock()
        capture.call=AsyncMock(side_effect=ValueError('private visual operation failed: wechat_row_not_topmost'))
        await capture.retarget('🍓a',str(self.root/'a'),['🍓a','a'])
        self.assertEqual([c.kwargs['recipient'] for c in client.tool.await_args_list],['🍓a','a'])

    async def test_reconcile_skips_never_sent_replies_and_keeps_journal_time(self):
        child=self.coordinator.session(target('a'))
        from scripts.agent_bridge.codex import write_private
        write_private(child.root/'send-s1.json',{'idempotency_key':'s1','status':'superseded','reply':'作废','input':[]})
        write_private(child.root/'send-t1.json',{'idempotency_key':'t1','status':'triggered','reply':'已发','input':[],
                                                 'created_at':1000.0,'attempted_at':1001.0})
        self.coordinator.reconcile_journals()
        h=self.coordinator.history
        self.assertIsNone(h.send('s1'))
        at=h.conn.execute("select observed_at from messages where send_key='t1'").fetchone()[0]
        self.assertEqual(at,1001000)

    async def test_worker_refusal_keeps_transport_open(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        capture=NativeCapture('fixture',None,'a',self.root/'a')
        class Pipe:
            def __init__(self): self.lines=[]
            def write(self,data): self.lines.append(json.loads(data))
            async def drain(self): pass
        class Out:
            async def readline(inner):
                return (json.dumps({'id':capture.counter,'protocol_version':1,'ok':False,'error':'refused'})+'\n').encode()
        capture.process=type('P',(),{'stdin':Pipe(),'stdout':Out()})()
        with self.assertRaisesRegex(ValueError,'refused'): await capture.call('open_session',target='b')
        self.assertIsNotNone(capture.process)

    def probe_capture(self,rows):
        from scripts.wechat_bot.native_capture import NativeCapture
        capture=NativeCapture('fixture',AsyncMock(),'a',self.root/'a')
        capture.process=object()
        capture.sessions=AsyncMock(side_effect=lambda names:{'rows':[{'name':n,**r} for n,r in rows().items()]})
        self.coordinator.capture=capture;self.coordinator.active_name='a'
        self.coordinator.visit=AsyncMock()
        return capture

    def visited(self):
        names=[c.args[0]['recipient'] for c in self.coordinator.visit.await_args_list]
        self.coordinator.visit.reset_mock();return names

    async def test_unchanged_session_list_does_not_switch_chats(self):
        state={'b':{'preview_digest':'p1','red_pixels':19}}
        self.probe_capture(lambda:state)
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a','b'])  # active first, then initial check
        for _ in range(3):
            await self.coordinator.sweep()
            self.assertEqual(self.visited(),['a'])

    async def test_preview_change_or_unread_badge_triggers_switch(self):
        state={'b':{'preview_digest':'p1','red_pixels':19}}
        self.probe_capture(lambda:state)
        await self.coordinator.sweep();await self.coordinator.sweep();self.visited()
        state['b']={'preview_digest':'p2','red_pixels':19}
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a','b'])
        state['b']={'preview_digest':'p2','red_pixels':19+5}  # avatar noise is not a badge
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a'])
        state['b']={'preview_digest':'p2','red_pixels':19+150}  # identical repeat: only the badge changes
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a','b'])

    async def test_selected_row_ocr_difference_does_not_ping_pong(self):
        # OCR of a highlighted (selected) row differs from the same row unselected.
        def rows():
            active=self.coordinator.active_name
            return {n:{'preview_digest':('sel-' if n==active else 'un-')+n,'red_pixels':0} for n in ('a','b')}
        self.probe_capture(rows)
        async def visit(target): self.coordinator.active_name=target['recipient']
        self.coordinator.visit=AsyncMock(side_effect=visit)
        for _ in range(3): await self.coordinator.sweep()
        self.visited()
        for _ in range(5):
            await self.coordinator.sweep()
            self.assertEqual(len(self.visited()),1)  # only the open chat
        # A forced idle check switches away once; returning must not be triggered by OCR alone.
        other='a' if self.coordinator.active_name=='b' else 'b'
        self.coordinator.last_visit[other]-=self.coordinator.config['idle_check_seconds']
        await self.coordinator.sweep();self.visited()
        for _ in range(5):
            await self.coordinator.sweep()
            self.assertEqual(len(self.visited()),1)

    async def test_multiple_changed_sessions_are_visited_in_arrival_order(self):
        self.config['allowlist']=[target('a'),target('b'),target('c'),target('d')];self.save()
        self.coordinator=BotCoordinator(self.path)
        state={n:{'preview_digest':'p','red_pixels':0,'y':y} for n,y in [('b',15),('c',79),('d',143)]}
        self.probe_capture(lambda:state)
        await self.coordinator.sweep();await self.coordinator.sweep();self.visited()
        # c and d change in the same probe: d is lower in the list, so it changed first.
        state['c']={**state['c'],'preview_digest':'c2','y':15}
        state['d']={**state['d'],'preview_digest':'d2','y':79}
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a','d','c'])
        self.assertEqual(self.coordinator.pending,{})

    async def test_earlier_detection_is_served_before_later_one(self):
        self.config['allowlist']=[target('a'),target('b'),target('c')];self.save()
        self.coordinator=BotCoordinator(self.path)
        state={n:{'preview_digest':'p','red_pixels':0,'y':y} for n,y in [('b',15),('c',79)]}
        self.probe_capture(lambda:state)
        await self.coordinator.sweep();await self.coordinator.sweep();self.visited()
        # c was detected in an earlier sweep but stopped before being served; b changes later.
        self.coordinator.pending['c']=(0.0,-79)
        state['b']={**state['b'],'preview_digest':'b2'}
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a','c','b'])

    async def test_invisible_session_is_checked_only_at_idle_interval(self):
        state={}
        self.probe_capture(lambda:state)
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a','b'])
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a'])
        self.coordinator.last_visit['b']-=self.coordinator.config['idle_check_seconds']
        await self.coordinator.sweep()
        self.assertEqual(self.visited(),['a','b'])

    async def test_probe_failure_falls_back_to_visiting_all(self):
        capture=self.probe_capture(lambda:{})
        capture.sessions=AsyncMock(side_effect=ValueError('private visual operation failed'))
        self.coordinator.close_connection=AsyncMock()
        await self.coordinator.sweep()
        self.assertEqual(sorted(self.visited()),['a','b'])
        self.coordinator.close_connection.assert_awaited()

    async def test_session_directory_is_contact_id_and_token_stable(self):
        child=self.coordinator.session(target('a'))
        cid=self.coordinator.history.resolve('fixture','a')
        self.assertEqual(child.root.name,cid)
        other=self.coordinator.session(target('b'))
        self.assertEqual(child.ownership,other.ownership)
        self.assertEqual(child.ownership,hashlib.sha256(str(self.coordinator.root).encode()).hexdigest())

    def test_sessions_parent_directory_is_private(self):
        self.coordinator.session(target('a'))
        self.assertEqual((self.coordinator.root/'sessions').stat().st_mode&0o777,0o700)

    async def test_merged_contacts_keep_separate_sessions_after_restart(self):
        self.coordinator.session(target('a'));self.coordinator.session(target('b'))
        h=self.coordinator.history;h.merge(h.resolve('fixture','a'),h.resolve('fixture','b'))
        restarted=BotCoordinator(self.path);self.coordinator=restarted
        self.assertNotEqual(restarted.sessions['a'].contact_id,restarted.sessions['b'].contact_id)
        self.probe_capture(lambda:{})
        await restarted.sweep()
        self.assertEqual(self.visited(),['b'])

    async def test_following_alias_suggestion_after_merge_starts(self):
        self.coordinator.session(target('a'));self.coordinator.session(target('b'))
        h=self.coordinator.history;a=h.resolve('fixture','a');h.merge(h.resolve('fixture','b'),a)
        self.config['allowlist']=[{'recipient':'a','kind':'private','aliases':['b']}];self.save()
        restarted=BotCoordinator(self.path);self.coordinator=restarted
        self.probe_capture(lambda:{})
        await restarted.sweep()
        self.assertEqual(restarted.sessions['a'].contact_id,a)
        self.assertEqual(self.visited(),['a'])

    async def rename_and_sweep(self,restart):
        child=self.coordinator.session(target('a'))
        self.config['allowlist']=[{'recipient':'a2','kind':'private','aliases':['a']},target('b')];self.save()
        if restart: self.coordinator=BotCoordinator(self.path)
        self.probe_capture(lambda:{})
        for _ in range(3): await self.coordinator.sweep()
        self.assertFalse((child.root/'routing-disabled.json').exists())
        self.assertIn('a2',[c.args[0]['recipient'] for c in self.coordinator.visit.await_args_list])
        self.assertEqual(self.coordinator.session({'recipient':'a2','kind':'private','aliases':['a']}).root,child.root)

    async def test_rename_with_alias_after_restart_keeps_serving(self):
        await self.rename_and_sweep(restart=True)

    async def test_rename_with_alias_without_restart_keeps_serving(self):
        await self.rename_and_sweep(restart=False)

    def test_unmigrated_legacy_directory_refuses(self):
        (self.root/'state'/'sessions'/('0'*64)).mkdir(parents=True)
        with self.assertRaisesRegex(ValueError,'migrate-history'): BotCoordinator(self.path)

    def test_known_contacts_restored_after_restart(self):
        self.coordinator.session(target('a'))
        self.config['allowlist']=[target('b')];self.save()
        restarted=BotCoordinator(self.path)
        self.assertIn('a',restarted.sessions)

    async def test_merged_entry_is_not_polled(self):
        self.coordinator.session(target('a'));self.coordinator.session(target('b'))
        h=self.coordinator.history;h.merge(h.resolve('fixture','b'),h.resolve('fixture','a'))
        self.probe_capture(lambda:{})
        await self.coordinator.sweep()
        self.assertNotIn('b',self.visited())

    async def test_worker_receives_alias_candidates(self):
        from scripts.wechat_bot.native_capture import NativeCapture
        capture=NativeCapture('fixture',None,'🍓a',self.root/'a',names=['🍓a','a'])
        class Pipe:
            def __init__(self): self.lines=[]
            def write(self,data): self.lines.append(json.loads(data))
            async def drain(self): pass
        class Out:
            async def readline(inner):
                return (json.dumps({'id':capture.counter,'protocol_version':1,'ok':True,'result':{}})+'\n').encode()
        pipe=Pipe();capture.process=type('P',(),{'stdin':pipe,'stdout':Out()})()
        await capture.call('capture')
        self.assertEqual(pipe.lines[-1]['params']['accepted_names'],['🍓a','a'])

    async def test_session_probe_maps_alias_row_to_entry(self):
        self.config['allowlist']=[target('a'),{'recipient':'🍓b','kind':'private','aliases':['b']}];self.save()
        self.coordinator=BotCoordinator(self.path)
        state={'b':{'preview_digest':'p1','red_pixels':0}}
        capture=self.probe_capture(lambda:state)
        await self.coordinator.sweep()
        self.assertIn('🍓b',self.visited())
        self.assertIn('b',capture.sessions.await_args.args[0])
        await self.coordinator.sweep();self.visited()
        state['b']={'preview_digest':'p2','red_pixels':0}
        await self.coordinator.sweep()
        self.assertIn('🍓b',self.visited())

    def media_fixture(self):
        capture=self.probe_capture(lambda:{})
        child=self.coordinator.session(target('a'))
        h=self.coordinator.history
        f={'bubbles':[{'text':'报价.pdf','digest':'d1','direction':'inbound_visual','kind':'file','media_id':'m'*64}],'snapshot_id':'s1'}
        [row]=h.record_frame(child.contact_id,{'bubbles':[{'text':'old','digest':'old','direction':'inbound_visual'}]}) or [None]
        h.record_frame(child.contact_id,{'bubbles':[{'text':'old','digest':'old','direction':'inbound_visual'},*f['bubbles']]})
        f['bubbles'].insert(0,{'text':'old','digest':'old','direction':'inbound_visual'})
        src=self.root/'dl.pdf';src.write_bytes(b'%PDF')
        capture.frame=AsyncMock(return_value=f)
        capture.call=AsyncMock(return_value={'filename':'报价.pdf','extension':'pdf'})
        capture.download=AsyncMock(return_value=src)
        return capture,child,h

    async def test_file_download_runs_only_without_pending_replies(self):
        capture,child,h=self.media_fixture()
        async def busy(t): self.coordinator.sessions[t['recipient']].had_incoming=True
        self.coordinator.visit=AsyncMock(side_effect=busy)
        await self.coordinator.sweep()
        capture.call.assert_not_awaited()
        async def idle(t): self.coordinator.sessions[t['recipient']].had_incoming=False
        self.coordinator.visit=AsyncMock(side_effect=idle)
        await self.coordinator.sweep()
        capture.call.assert_awaited_once_with('prepare_file',media_id='m'*64,snapshot_id='s1')
        self.assertFalse(h.has_pending_media())
        self.assertIsNotNone(h.conn.execute("select media_id from messages where kind='file'").fetchone()[0])

    async def test_file_scrolled_away_is_marked_gone(self):
        capture,child,h=self.media_fixture()
        capture.frame=AsyncMock(return_value={'bubbles':[],'snapshot_id':'s2'})
        await self.coordinator.download_pending_media()
        capture.call.assert_not_awaited()
        self.assertEqual(h.conn.execute('select status from media_jobs').fetchone()[0],'gone')

    async def test_reconcile_backfills_missing_and_stale_sends(self):
        child=self.coordinator.session(target('a'))
        from scripts.agent_bridge.codex import write_private
        write_private(child.root/'send-k1.json',{'idempotency_key':'k1','status':'triggered','reply':'答','input':[],'client_echo_observed':True})
        write_private(child.root/'send-k2.json',{'idempotency_key':'k2','status':'unknown','reply':'答2','input':[]})
        h=self.coordinator.history
        h.record_bot_reply(child.contact_id,'k2','答2','attempting',{},[])
        self.coordinator.reconcile_journals()
        self.assertEqual(h.send('k1')['status'],'triggered');self.assertEqual(h.send('k1')['echo_observed'],1)
        self.assertEqual(h.send('k2')['status'],'unknown')

    async def test_invalid_hot_config_blocks_operations(self):
        self.path.write_text('{broken')
        self.coordinator.connect=AsyncMock()
        with self.assertRaises(ValueError): await self.coordinator.sweep()
        self.coordinator.connect.assert_not_awaited()


if __name__=='__main__': unittest.main()
