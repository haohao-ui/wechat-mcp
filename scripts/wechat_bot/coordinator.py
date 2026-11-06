"""Serialize one desktop connection across isolated private-chat sessions."""
import asyncio
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import time

from scripts.agent_bridge.codex import private_directory, write_private
from scripts.agent_bridge.mcp import MCP
from .capture import RemoteCapture
from .native_capture import must_wait, ACTION_INTERRUPTED, NativeCapture, layout_unavailable
from .history.names import candidate_names
from .history.store import HistoryStore
from .routing import decision, match_entry
from .service import BotService, load_config, log_event


# A real unread badge adds a filled red circle (well over 20 px) on top of the avatar's own red.
BADGE_PIXELS=20
# Worker refusals raised before any message is read: the WeChat window is unusable right now
# (light appearance, resized, hidden, desktop busy). These retry each sweep instead of pausing.
# Consecutive wrong-title switches tolerated before a contact pauses for review.
TITLE_MISMATCH_LIMIT=3
RECALIBRATION_LIMIT=5
WINDOW_UNAVAILABLE=('uncalibrated_theme','unsupported_image_geometry','wechat_not_visible',
                    'window_capture_failed','desktop_busy','vision_timeout',
                    'layout_missing','layout_unreadable','screen_not_main')+ACTION_INTERRUPTED


def extract_mismatch_failures(error):
    text=str(error)
    start=text.find('invalid("layout_mismatch')
    if start<0: return []
    end=text.find('")',start)
    token=text[start+9:end if end>=0 else len(text)]
    if ':' in token:
        payload=token.split(':',1)[1]
        return [f.strip() for f in payload.split(',') if f.strip()]
    return []


def ocr_noise(a, b):
    """True when two previews differ by at most one character (an OCR misread, not a new message)."""
    if a == b: return True
    if abs(len(a)-len(b)) > 1: return False
    if len(a) == len(b): return sum(x != y for x, y in zip(a, b)) <= 1
    short, long_ = (a, b) if len(a) < len(b) else (b, a)
    return any(long_[:i]+long_[i+1:] == short for i in range(len(long_)))


def preview_changed(prior, row):
    # Whole-list OCR reruns whenever any row changes and can misread one letter of another row
    # (acceptance 2026-11-05: "pdf" read as "pof"). The time must match exactly (review I4: a
    # repeated "[图片]" a minute later differs only there); only the preview text gets the tolerance.
    if prior.get('time') is not None and row.get('preview_time') is not None and prior['time'] != row['preview_time']:
        return True
    if prior.get('key') and row.get('preview_key'):
        return not ocr_noise(prior['key'], row['preview_key'])
    return row['preview_digest'] != prior['digest']


def letters(text):
    """Letters and digits only, like the worker's preview_key."""
    return ''.join(c for c in text if c.isalnum())


def shows_last_seen(key, child):
    """True when a row's preview is the last message the bot read in that chat (its own reply, usually)."""
    bubbles=((getattr(child,'cursor',None) or {}).get('bubbles') or [])
    if not key or not bubbles:
        return False
    last=bubbles[-1]
    for text in (last.get('text',''), (last.get('sender') or '')+last.get('text','')):
        seen=letters(text)
        if seen and (ocr_noise(key, seen[:len(key)]) or ocr_noise(key, seen)):
            return True
    return False


class BotCoordinator:
    def __init__(self, config_path):
        self.config_path=Path(config_path)
        self.config=load_config(self.config_path)
        self.root=private_directory(self.config['state_dir'])
        self.sessions={}
        self.stop=asyncio.Event()
        self.client=None
        self.capture=None
        self.active_name=None
        self.last_health=None
        self.last_health_at=0
        # Session-list observations: switch chats only when an allowlisted row changes.
        self.list_state={}
        self.last_visit={}
        # name -> (first detected, -row y): older arrivals first; lower rows changed earlier.
        self.pending={}
        self.layout_mismatches=0
        binding={k:self.config[k] for k in ('host','account_id')}
        path=self.root/'coordinator-binding.json'
        if path.exists() and json.loads(path.read_text())!=binding:
            raise ValueError('coordinator state belongs to a different account/host')
        write_private(path,binding)
        sessions=self.root/'sessions'
        if sessions.exists() and any(len(p.name)==64 and all(c in '0123456789abcdef' for c in p.name)
                                     for p in sessions.iterdir() if p.is_dir()):
            raise ValueError('legacy session directories found; stop the bot and run control migrate-history first')
        self.history=HistoryStore.open(self.root)
        # One stable worker-ownership token for every chat, independent of session directory names.
        self.ownership=hashlib.sha256(str(self.root).encode()).hexdigest()
        # Restore known contacts even when removed from today's allowlist, so a restart cannot
        # erase the exclusion interval and later replay its history.
        current={}
        for entry in self.config['allowlist']:
            try: current[self.history.ensure_contact(self.config['account_id'],entry,follow_merge=False)]=entry
            except ValueError: pass
        for row in self.history.conn.execute(
                "SELECT contact_id, display_name, kind FROM contacts WHERE account_id=? AND kind IN ('private','group')",(self.config['account_id'],)).fetchall():
            if (self.root/'sessions'/row[0]/'binding.json').exists():
                # Prefer today's allowlist entry: after a rename the old display name is only an alias.
                self.session(current.get(row[0],{'recipient':row[1],'kind':row[2]}))

    def rules(self):
        value=load_config(self.config_path)
        if any(value[k]!=self.config[k] for k in ('host','account_id','state_dir')):
            raise ValueError('account/host/state changes require a separate state directory and restart')
        return value

    def session(self, target):
        name=target['recipient']
        if name not in self.sessions:
            contact_id=self.history.ensure_contact(self.config['account_id'],target,follow_merge=False)
            directory=private_directory(private_directory(self.root/'sessions')/contact_id)
            config={**self.config,'allowlist':[target],'state_dir':str(directory)}
            child=BotService(config,routing_check=lambda:decision(self.rules(),name),history=self.history,
                             contact_id=contact_id,ownership=self.ownership,names=candidate_names(target)[:20])
            child.stop=self.stop
            child.had_incoming=False
            self.sessions[name]=child
        return self.sessions[name]

    def health(self, state, **details):
        value={'pid':os.getpid(),'state':state,'account_id':self.config['account_id'],
               'allowlist':[x['recipient'] for x in self.rules()['allowlist']],
               'groups_enabled':any(x['kind']=='group' for x in self.rules()['allowlist']),'model_backend':self.config.get('model_config',{}).get('backend','codex-exec'),
               'platform_message_verified':False,'sessions':{},**details}
        if isinstance(self.capture,NativeCapture) and self.capture.ocr_status:
            value['ocr']=self.capture.ocr_status
        for name,child in self.sessions.items():
            path=child.root/'health.json'
            value['sessions'][name]=json.loads(path.read_text()) if path.exists() else {'state':'pending'}
        signature=json.dumps(value,sort_keys=True,ensure_ascii=False)
        now=time.monotonic()
        if signature!=self.last_health or now-self.last_health_at>=60:
            # Per-session heartbeat timestamps are omitted from the log summary to avoid poll spam.
            summary={name:item['state'] for name,item in value['sessions'].items()}
            compact=(state,tuple(summary.items()),json.dumps(details,sort_keys=True))
            if compact!=getattr(self,'last_summary',None) or now-self.last_health_at>=60:
                log_event('coordinator_state',state=state,sessions=summary,**details)
                self.last_summary=compact; self.last_health_at=now
            self.last_health=signature
        write_private(self.root/'health.json',{**value,'updated_at':time.time()})

    def disable_session(self, target, verdict):
        child=self.session(target)
        write_private(child.root/'routing-disabled.json',{'rule':verdict,'updated_at':time.time()})
        child.health('ignored',rule=verdict)

    async def close_capture(self):
        if self.capture:
            capture,self.capture=self.capture,None
            try: await capture.close()
            except Exception as error: log_event('capture_cleanup_failed',error_type=type(error).__name__)
        self.active_name=None

    async def close_connection(self):
        await self.close_capture()
        if self.client:
            client,self.client=self.client,None
            await client.close()

    async def connect(self, child):
        if self.active_name==child.recipient and self.capture is not None:
            return
        if isinstance(self.capture,NativeCapture):
            if not child.target_allowed(): return
            child.health('connecting')
            await self.capture.retarget(child.recipient,child.root,child.names,group=child.group)
            self.active_name=child.recipient
            switch=self.capture.last_switch or {}
            log_event('chat_switched',recipient=child.recipient,clicked=switch.get('clicked'),
                      seconds=switch.get('seconds'),fallback=switch.get('fallback'))
            return
        await self.close_capture()
        if not self.client:
            self.client=MCP(['ssh','-o','BatchMode=yes','-o','LogLevel=ERROR',self.config['host'],
                '/usr/bin/nc -U "$HOME/.wechat-rpa/worker.sock"'],
                {'platform':'wechat','account_id':self.config['account_id']})
            await self.client.start()
            status=await self.client.tool('get_listener_status')
            if status.get('enabled'): raise ValueError('another observer is enabled')
        if not child.target_allowed(): return
        capture_class=NativeCapture if self.config.get('capture_backend','native')=='native' else RemoteCapture
        self.capture=capture_class(self.config['host'],self.client,child.recipient,child.root,child.ownership,names=child.names,
                                   manage_window=self.config.get('manage_window',True),
                                   **({'group':child.group} if capture_class is NativeCapture else {}))
        child.health('connecting')
        await self.capture.start()
        self.active_name=child.recipient

    def clear_layout_waits(self, verified):
        """The screen matches the map again: idle chats (not visited until a message arrives) leave
        their layout-mismatch wait too, so status stops reporting needs_recalibration."""
        for other in self.sessions.values():
            if other is verified or other.paused: continue
            try: state=json.loads((other.root/'health.json').read_text())
            except (OSError,ValueError): continue
            if state.get('state')=='needs_recalibration' or 'layout_mismatch' in str(state.get('reason','')):
                other.health('watching',reason='layout verified on another chat')

    async def visit(self, target):
        child=self.session(target)
        verdict=decision(self.rules(),child.recipient)
        if verdict!='allowed':
            self.disable_session(target,verdict)
            if self.active_name==child.recipient: await self.close_capture()
            return
        if child.paused:
            child.health('paused',reason=child.pause_reason or 'review unfinished session journal')
            return
        marker=child.root/'routing-disabled.json'
        if marker.exists():
            # Re-enabling a conversation baselines its history, skipping messages from the excluded period.
            child.cursor=None
        try:
            await self.connect(child)
            if self.stop.is_set() or not child.target_allowed(): return
            await child.cycle(self.client,self.capture)
            child.title_mismatches=0
            if self.layout_mismatches: self.clear_layout_waits(child)
            self.layout_mismatches=0
            if child.cursor is not None and marker.exists(): marker.unlink()
        except ValueError as error:
            if str(error)=='incomplete bottom bubble; no automatic reply' and not child.paused:
                child.health('waiting_for_stable_frame',reason='bottom bubble is incomplete; retry capture without model/send')
                return
            if not child.paused and 'invalid("allowlisted_title_mismatch")' in str(error):
                # A switch can land on another chat (e.g. after a manual send). Nothing was read or sent,
                # so retry; a persistent mismatch (renamed contact, wrong config) still pauses.
                child.title_mismatches=getattr(child,'title_mismatches',0)+1
                if child.title_mismatches<TITLE_MISMATCH_LIMIT:
                    child.health('waiting_for_wechat_window',reason=str(error)[:400],attempt=child.title_mismatches)
                    await self.close_connection()
                    return
            if not child.paused and 'invalid("layout_mismatch' in str(error):
                self.layout_mismatches=getattr(self,'layout_mismatches',0)+1
                failures=extract_mismatch_failures(error)
                state='needs_recalibration' if self.layout_mismatches>=RECALIBRATION_LIMIT else 'waiting_for_wechat_window'
                child.health(state,reason=str(error)[:400],failures=failures,attempt=self.layout_mismatches)
                return
            if not child.paused and (layout_unavailable(error) or any(f'invalid("{code}")' in str(error) or f'invalid("{code}:' in str(error) for code in WINDOW_UNAVAILABLE)):
                # Nothing was read or sent; the window comes back (e.g. appearance back to dark) without a restart.
                child.health('waiting_for_wechat_window',reason=str(error)[:400])
                if 'screen_not_main' not in str(error):
                    await self.close_connection()
                return
            child.paused=True; child.pause_reason=str(error)[:400]
            child.health('paused',reason=child.pause_reason)
            await self.close_connection()
        except Exception as error:
            child.health('paused' if child.paused else 'reconnecting',reason=str(error)[:400])
            await self.close_connection()
        if child.paused: await self.close_connection()

    async def download_pending_media(self):
        """Download at most one archived file original from the open chat (design §7.5.1)."""
        child=self.sessions.get(self.active_name)
        if child is None or not isinstance(self.capture,NativeCapture) or self.capture.process is None:
            return
        job=self.history.next_media_job(child.contact_id)
        if job is None:
            return
        digest=self.history.conn.execute('SELECT bubble_digest FROM messages WHERE message_id=?',(job['message_id'],)).fetchone()[0]
        try:
            frame=await self.capture.frame()
            bubble=next((b for b in frame.get('bubbles',[]) if b.get('kind')=='file' and b.get('digest')==digest),None)
            if bubble is None:
                self.history.finish_media_job(job['message_id'],'gone','not visible')
                return
            info=await self.capture.call('prepare_file',media_id=bubble['media_id'],snapshot_id=frame['snapshot_id'])
            path=await self.capture.download(bubble['media_id'],kind='file')
            self.history.attach_media(job['message_id'],path,kind='file',filename=info.get('filename'),source='original')
            self.history.finish_media_job(job['message_id'],'done')
            log_event('history_file_archived',recipient=child.recipient)
        except ValueError as error:
            gone='file_snapshot_changed' in str(error)
            self.history.finish_media_job(job['message_id'],'gone' if gone else 'failed',str(error)[:200])
            log_event('history_file_download_failed',recipient=child.recipient,gone=gone,error=str(error)[:200])
        except Exception as error:
            self.history.finish_media_job(job['message_id'],'failed',str(error)[:200])
            log_event('history_file_download_failed',recipient=child.recipient,error_type=type(error).__name__)

    def reconcile_journals(self):
        """Startup repair: the send journals are authoritative for the history mirror (design §9.2)."""
        for child in self.sessions.values():
            for path in child.root.glob('send-*.json'):
                record=json.loads(path.read_text())
                key=record.get('idempotency_key')
                if not key: continue
                existing=self.history.send(key)
                if existing is None and record.get('reply') and record.get('status') in ('attempting','triggered','unknown'):
                    # Only replies that were (or may have been) sent; placed at their journal time.
                    at=record.get('attempted_at',record.get('created_at'))
                    self.history.record_bot_reply(child.contact_id,key,record['reply'],record['status'],record.get('model',{}),[],
                                                  observed_at=int(at*1000) if at else None)
                    existing=self.history.send(key)
                if existing and (existing['status']!=record['status'] or
                                 ('client_echo_observed' in record and existing['echo_observed']!=int(bool(record['client_echo_observed'])))):
                    self.history.update_send(key,record['status'],record.get('client_echo_observed'))

    async def due_sessions(self, names):
        """Names whose chat should be opened this sweep, decided from one session-list probe."""
        if not names or not isinstance(self.capture,NativeCapture) or self.capture.process is None:
            return set(names)
        rules=self.rules()
        entries={name:(match_entry(rules,name) or {'recipient':name,'kind':'private'}) for name in names}
        try:
            probe=await self.capture.sessions([n for name in names for n in candidate_names(entries[name])])
        except Exception as error:
            log_event('session_probe_failed',error_type=type(error).__name__,error=str(error)[:200])
            # A wait keeps the worker: closing ends it, and restarting it needs a GUI action.
            if not must_wait(error): await self.close_connection()
            return set(names)
        rows={}
        for row in probe.get('rows',[]):
            entry=match_entry(rules,row['name'])
            if entry: rows[entry['recipient']]=row
        now=time.monotonic()
        due=set()
        for name in names:
            row,prior=rows.get(name),self.list_state.get(name)
            reason=None
            if name==self.active_name: reason='active'
            elif name not in self.last_visit: reason='initial'
            elif now-self.last_visit[name]>=self.config['idle_check_seconds']: reason='idle_check'
            elif getattr(self.sessions.get(name),'turn',None) is not None: reason='turn_open'
            elif row is not None:
                # First unselected sighting: check once more so the baseline cannot hide a message.
                if prior is None or prior['digest'] is None: reason='initial'
                elif prior.get('left'):
                    # First sighting after leaving: a reference only if it shows what the bot last read;
                    # a message that arrived while switching is marked read (no badge) but shows here.
                    if preview_changed(prior,row) and not shows_last_seen(row.get('preview_key'),self.sessions.get(name)):
                        reason='preview_changed'
                elif preview_changed(prior,row): reason='preview_changed'
                elif row['red_pixels']-prior['min_red']>=BADGE_PIXELS: reason='unread_badge'
            if row is not None:
                # Avatar art may contain red; the badge is measured against the lowest count seen.
                low=row['red_pixels'] if prior is None else min(prior['min_red'],row['red_pixels'])
                # The selected row's highlight changes OCR (客服/容服), so previews are only compared
                # in the unselected state; the open chat is captured every sweep regardless.
                if name==self.active_name:
                    # Its preview changes while open (our own reply); the chat itself was just read, so the
                    # first sighting after leaving is a new reference, not a change (the badge still counts).
                    self.list_state[name]={'digest':prior['digest'] if prior else None,'key':prior and prior.get('key'),
                                           'time':prior and prior.get('time'),'min_red':low,'left':True}
                else:
                    self.list_state[name]={'digest':row['preview_digest'],'key':row.get('preview_key'),
                                           'time':row.get('preview_time'),'min_red':low}
            if reason is None and name in self.pending: reason='pending'
            if reason:
                due.add(name)
                if reason in ('preview_changed','unread_badge'):
                    self.pending.setdefault(name,(now,-row.get('y',0)))
                if reason not in ('active','idle_check','pending','turn_open'):
                    log_event('session_switch_due',recipient=name,reason=reason,visible=row is not None,
                              red_pixels=row and row['red_pixels'],baseline_red=prior and prior['min_red'])
        return due

    async def sweep(self):
        rules=self.rules()
        targets=rules['allowlist']
        # Removed contacts are also disabled, so restoring them cannot replay the excluded interval.
        allowed_ids={self.history.ensure_contact(self.config['account_id'],t,follow_merge=False) for t in targets}
        for name in list(self.sessions):
            if not any(x['recipient']==name for x in targets) and self.sessions[name].contact_id in allowed_ids:
                # Renamed: the same contact is now served under its new allowlist name.
                if self.active_name==name: await self.close_capture()
                del self.sessions[name]
                continue
            if not any(x['recipient']==name for x in targets):
                self.disable_session({'recipient':name,'kind':'private'},'not_allowlisted')
                if self.active_name==name: await self.close_capture()
        active=[]
        allowed=[]
        for target in targets:
            verdict=decision(rules,target['recipient'])
            if verdict!='allowed':
                self.disable_session(target,verdict)
                if self.active_name==target['recipient']: await self.close_capture()
                continue
            child=self.session(target)
            if self.history.canonical(child.contact_id)!=child.contact_id:
                # Merged into another contact: only the target contact is polled.
                self.disable_session(target,'merged')
                if self.active_name==target['recipient']: await self.close_capture()
                continue
            active.append(target['recipient'])
            allowed.append(target)
        due=await self.due_sessions(active)
        # The open chat is checked first: its new messages show no unread badge before we switch away.
        # Then chats with detected activity in arrival order, then initial/idle checks.
        infinity=(float('inf'),0)
        allowed.sort(key=lambda t:(t['recipient']!=self.active_name,self.pending.get(t['recipient'],infinity)))
        visited=[]
        for target in allowed:
            if self.stop.is_set(): break
            if target['recipient'] not in due: continue
            visited.append(target['recipient'])
            if decision(self.rules(),target['recipient'])!='allowed': continue
            await self.visit(target)
            self.last_visit[target['recipient']]=time.monotonic()
            self.pending.pop(target['recipient'],None)
        # File originals are archived only when no chat has a reply in progress or waiting.
        if (not self.stop.is_set() and not self.pending and not any(getattr(self.sessions[n],'had_incoming',False) for n in visited)
                and not any(getattr(child,'turn',None) is not None for child in self.sessions.values())):
            await self.download_pending_media()
        state='ignored' if not active else 'paused' if all(self.sessions[n].paused for n in active) else 'watching'
        self.health(state,active_chat=self.active_name)

    async def serve_calibration(self):
        req_path = self.root / 'calibrate-request.json'
        if not req_path.exists():
            return
        res_path = self.root / 'calibrate-result.json'
        try:
            req = json.loads(req_path.read_text())
        except Exception:
            return
        req_id = req.get('id')
        action = req.get('action')
        created_at = req.get('created_at', 0)
        try:
            if time.time() - created_at > 600:
                write_private(res_path, {'id': req_id, 'ok': False, 'error': 'expired'})
                return
            if self.capture is None:
                write_private(res_path, {'id': req_id, 'ok': False, 'error': 'worker_not_connected'})
                return
            result = await self.capture.call('calibrate')
            if action == 'accept':
                from .control import calibrate_accept
                res_data = calibrate_accept(config=self.config, worker_func=lambda: result)
                write_private(res_path, {'id': req_id, 'ok': True, 'result': res_data})
            else:
                from .control import run_calibrate, REMOTE_LAYOUT_DIR
                diffs, missing, candidate = run_calibrate(self.config, worker_result=result)
                version = candidate.get('wechat_version')
                out = {
                    'candidate_json': f"{REMOTE_LAYOUT_DIR}/candidate-wechat-{version}.json",
                    'candidate_png': f"{REMOTE_LAYOUT_DIR}/candidate-wechat-{version}.png",
                    'differences': diffs,
                    'missing': missing,
                }
                write_private(res_path, {'id': req_id, 'ok': True, 'result': out})
        except Exception as error:
            write_private(res_path, {'id': req_id, 'ok': False, 'error': str(error)})
        finally:
            req_path.unlink(missing_ok=True)

    async def run(self):
        for sig in (signal.SIGTERM,signal.SIGINT):
            asyncio.get_running_loop().add_signal_handler(sig,self.stop.set)
        path=self.root/'service.lock'
        with path.open('a') as lock:
            path.chmod(0o600);fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.reconcile_journals()
            log_event('service_started',pid=os.getpid(),mode='multi-private',model_backend=self.config.get('model_config',{}).get('backend','codex-exec'))
            self.health('connecting')
            try:
                while not self.stop.is_set():
                    try: await self.sweep()
                    except Exception as error:
                        await self.close_connection()
                        # Invalid config blocks all subsequent operations until corrected; no fallback rules.
                        log_event('routing_config_failed',error_type=type(error).__name__)
                        write_private(self.root/'health.json',{'pid':os.getpid(),'state':'paused','reason':str(error)[:400],'updated_at':time.time()})
                    try: await self.serve_calibration()
                    except Exception as error:
                        log_event('serve_calibration_failed',error_type=type(error).__name__,error=str(error)[:200])
                    try: await asyncio.wait_for(self.stop.wait(),self.config['poll_seconds'])
                    except asyncio.TimeoutError: pass
            finally:
                await self.close_connection()
                for child in self.sessions.values(): child.health('stopped')
                write_private(self.root/'health.json',{'pid':os.getpid(),'state':'stopped','updated_at':time.time()})
