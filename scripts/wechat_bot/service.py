"""Persistent, single-private-contact customer-service pilot. No public MCP/schema changes."""
import argparse
import asyncio
import fcntl
import hashlib
import json
import logging
import sys
from datetime import datetime, timezone
import os
from pathlib import Path
import signal
import time

from scripts.agent_bridge.codex import private_directory, write_private
from scripts.agent_bridge.mcp import MCP
from .capture import RemoteCapture
from .history.names import candidate_names, clean_title
from .understanding import understand
from .history.store import similar
from .policy import reconcile, relocate, stitch, AmbiguousAnchor, ContinuityGap, Policy, echo_matches, fingerprint, new_bubbles, validate_reply
from .reply import generate
from .native_capture import NativeCapture


logger = logging.getLogger('wechat_bot')


def log_event(event, **fields):
    # Values are metadata only: message bodies, replies and credential config are excluded by callers.
    logger.info(json.dumps({'time':datetime.now(timezone.utc).isoformat(), 'event':event, **fields},ensure_ascii=False))


# model_api refuses requests whose images add up to more than this.
MAX_MODEL_IMAGE_BYTES = 30*1024*1024


def ensure_unreadable_named(reply, failed):
    """Every unreadable item is named with a resend request, even when the model left it out (spec §5.6)."""
    missing=[f['label'] for f in failed if f['label'] not in reply]
    if not missing:
        return reply
    suffix='、'.join(missing)+'打不开，麻烦重发一下。'
    if len(suffix)>200:
        suffix='有%d个文件或图片打不开，麻烦重发一下。'%len(missing)
    return validate_reply(reply[:200-len(suffix)].rstrip()+suffix)


def model_view(document):
    # Local paths and screen geometry never go to the model.
    return {k:v for k,v in document.items() if k not in ('path','bubble')}


def load_config(path):
    value = json.loads(Path(path).read_text())
    allowed = {'schema_version', 'host', 'account_id', 'allowlist', 'state_dir', 'poll_seconds',
               'cooldown_seconds', 'max_replies_per_hour','capture_backend','model_config','ignorelist','idle_check_seconds','context_messages','history_http','manage_window',
               'turn_quiet_seconds','turn_max_seconds','images_per_model_call'}
    if set(value)-allowed or value.get('schema_version') != 1:
        raise ValueError('unsupported bot config schema or field')
    from .routing import validate_rules
    validate_rules(value)
    if not isinstance(value.get('account_id'),str) or not value['account_id']:
        raise ValueError('explicit account required')
    host = value.get('host','')
    if not host or host.startswith('-') or any(c.isspace() for c in host):
        raise ValueError('invalid SSH host')
    if not Path(value.get('state_dir','')).is_absolute():
        raise ValueError('absolute private state_dir required')
    if value.setdefault('capture_backend','native') not in ['native','legacy']:
        raise ValueError('invalid capture_backend')
    for key, default, low, high in [('poll_seconds',2,1,60), ('cooldown_seconds',15,0,300),
                                    ('max_replies_per_hour',20,1,100), ('idle_check_seconds',60,10,3600),
                                    ('turn_quiet_seconds',10,0,120), ('turn_max_seconds',60,10,600),
                                    ('images_per_model_call',4,1,16)]:
        number = value.setdefault(key,default)
        if type(number) is not int or not low <= number <= high:
            raise ValueError('invalid ' + key)
    if type(value.setdefault('manage_window',True)) is not bool:
        raise ValueError('invalid manage_window')
    number = value.setdefault('context_messages',5)
    if type(number) is not int or not 0 <= number <= 100:
        raise ValueError('invalid context_messages')
    http = value.setdefault('history_http',{'bind':'127.0.0.1','port':8765})
    if (not isinstance(http,dict) or set(http)-{'bind','port'} or not isinstance(http.setdefault('bind','127.0.0.1'),str)
            or type(http.setdefault('port',8765)) is not int or not 1 <= http['port'] <= 65535):
        raise ValueError('invalid history_http')
    from .model_api import validate_config
    validate_config(value.get('model_config', {'backend':'codex-exec'}))
    return value


class BotService:
    def __init__(self, config, *, routing_check=None, history=None, contact_id=None, ownership=None, names=None):
        self.config = config
        self.routing_check = routing_check
        self.root = private_directory(config['state_dir'])
        self.recipient = config['allowlist'][0]['recipient']
        self.account = config['account_id']
        self.history, self.contact_id = history, contact_id
        self.names = names or candidate_names(config['allowlist'][0])
        self.policy = Policy(self.account,self.recipient,'service',names=self.names)
        binding = {'host':config['host'],'account_id':self.account,'recipient':self.recipient,'kind':'private'}
        if contact_id:
            # Renaming a contact (new recipient, old name as alias) must keep its state directory.
            binding = {'host':config['host'],'account_id':self.account,'contact_id':contact_id,'kind':'private'}
        binding_path = self.root/'binding.json'
        if binding_path.exists() and json.loads(binding_path.read_text()) != binding:
            raise ValueError('state belongs to a different account or contact; use a separate state directory')
        write_private(binding_path,binding)
        self.stop = asyncio.Event()
        self.cursor_path = self.root/'cursor.json'
        self.cursor = json.loads(self.cursor_path.read_text()) if self.cursor_path.exists() else None
        if self.cursor is not None and self.cursor.get('profile') == 'wechat-4-dark-1x':
            # Saved before theme profiles existed: that name meant dark theme at 1x.
            self.cursor = {**self.cursor, 'profile':'wechat-4-dark', 'scale':1}
        if self.cursor is not None:
            self.policy.check_frame(self.cursor)
        self.paused = any(json.loads(p.read_text()).get('status') in ['preparing_media','generating','generated','attempting','unknown']
                          for p in self.root.glob('send-*.json'))
        self.pause_reason = 'unfinished model/send journal requires review' if self.paused else None
        self.last_health_log = None
        self.last_health_log_at = 0
        self.ownership = ownership or hashlib.sha256(str(self.root).encode()).hexdigest()
        self.clock = time.time
        self.recovery_unconfirmed = False
        self.turn_path = self.root/'turn.json'
        self.turn = json.loads(self.turn_path.read_text()) if self.turn_path.exists() else None

    def target_allowed(self):
        from .routing import decision
        verdict=self.routing_check() if self.routing_check else decision(self.config,self.recipient)
        if verdict!='allowed':
            self.health('ignored',rule=verdict)
            return False
        return True

    def health(self, state, **details):
        signature = (state, json.dumps(details,sort_keys=True,ensure_ascii=False))
        now = time.monotonic()
        if signature != self.last_health_log or now-self.last_health_log_at >= 60:
            log_event('state',state=state,account_id=self.account,**details)
            self.last_health_log=signature; self.last_health_log_at=now
        write_private(self.root/'health.json', {'pid':os.getpid(),'state':state,'updated_at':time.time(),
            'account_id':self.account,'allowlist':[self.recipient],'groups_enabled':False,
            'model_backend':self.config.get('model_config',{}).get('backend','codex-exec'),'platform_message_verified':False,**details})

    def advance(self, frame):
        write_private(self.cursor_path,frame)
        self.cursor = frame

    def batch_key(self, previous, incoming):
        identity = [self.account,self.recipient,list(map(fingerprint,previous['bubbles'])),list(map(fingerprint,incoming))]
        return 'bot-service-' + hashlib.sha256(json.dumps(identity,ensure_ascii=False).encode()).hexdigest()

    def limited(self):
        attempts = [json.loads(p.read_text()).get('attempted_at',0) for p in self.root.glob('send-*.json')]
        recent = [t for t in attempts if t and time.time()-t < 3600]
        return (len(recent) >= self.config['max_replies_per_hour'] or
                bool(recent and time.time()-max(recent) < self.config['cooldown_seconds']))

    def clean_recipient(self, frame):
        return clean_title(frame['title_lines'][0])

    def history_failed(self, error):
        log_event('history_write_failed',account_id=self.account,error_type=type(error).__name__)
        self.health('history_write_failed',error=str(error)[:200])

    async def record(self, frame, capture):
        """Append the visible frame to history before any reply decision. False stops this cycle."""
        if self.history is None:
            return True
        try:
            rows = self.history.record_frame(self.contact_id,frame)
            self.history.observe_name(self.contact_id,frame['title_lines'][0])
        except Exception as error:
            self.history_failed(error)
            return False
        for row in rows:
            media_id = (row.get('bubble') or {}).get('media_id')
            if row['kind']=='image' and media_id and hasattr(capture,'download'):
                try:
                    path = await capture.download(media_id)
                    self.history.attach_media(row['message_id'],path,kind='image',filename=None,source='chat_thumbnail')
                except Exception as error:
                    log_event('history_thumbnail_failed',error_type=type(error).__name__)
        return True

    def write_journal(self, journal, record):
        write_private(journal,record)
        if self.history is not None:
            try:
                self.history.update_send(record['idempotency_key'],record['status'],record.get('client_echo_observed'))
            except Exception as error:
                # The journal stays authoritative; startup reconciliation repairs the mirror.
                log_event('history_send_mirror_failed',error_type=type(error).__name__)

    async def recover_gap(self, capture, frame, max_pages=6):
        """Changes after the cursor found by scrolling up (tall media pushed every anchor away).

        None when no page overlaps or the chat did not return to the same bottom frame; then
        `self.recovery_unconfirmed` tells the caller whether to wait (True) or rebaseline (False)."""
        self.recovery_unconfirmed = False
        if not hasattr(capture,'scroll') or not self.cursor['bubbles']:
            return None
        pages=[frame['bubbles']]
        changes=None
        for attempt in range(max_pages):
            try:
                page=await capture.scroll('up')
            except ValueError as error:
                # e.g. a worker without scroll_chat: nothing moved on the first attempt.
                log_event('gap_scroll_failed',account_id=self.account,error=str(error)[:200])
                if attempt==0: return None
                break
            pages.insert(0,page['bubbles'])
            # Each page takes its predecessor's image identities: dHash may jitter between scroll positions.
            aligned=[reconcile(self.cursor['bubbles'],pages[0])]
            for newer in pages[1:]:
                aligned.append(reconcile(aligned[-1],newer))
            try:
                changes=stitch(self.cursor['bubbles'],aligned)
                break
            except (ContinuityGap,AmbiguousAnchor):
                continue
        bottom=await self.scroll_to_bottom(capture,len(pages)+2)
        if bottom is None or list(map(fingerprint,reconcile(frame['bubbles'],bottom['bubbles']))) != list(map(fingerprint,frame['bubbles'])):
            # A lost wheel event can leave the chat scrolled up: overshoot once more before deciding.
            bottom=await self.scroll_to_bottom(capture,5)
            if changes is not None and list(map(fingerprint,reconcile(frame['bubbles'],bottom['bubbles']))) != list(map(fingerprint,frame['bubbles'])):
                self.recovery_unconfirmed = True
                return None
        if changes is None:
            return None
        for page in aligned:
            # The history store gets the same pages, so recovered messages are recorded too (spec G4).
            if not await self.record({**frame,'bubbles':page},capture):
                self.recovery_unconfirmed = True
                return None
        log_event('continuity_gap_recovered',account_id=self.account,pages=len(pages)-1,recovered=len(changes))
        return changes

    async def scroll_to_bottom(self, capture, pages):
        while pages>0:
            await capture.scroll('down',pages=min(5,pages))
            pages-=5
        bottom=await capture.frame()
        self.policy.check_frame(bottom)
        return bottom

    async def cycle(self, client, capture):
        self.had_incoming = False
        if not self.target_allowed(): return
        timings={}
        began=time.perf_counter()
        frame = await capture.frame()
        timings['capture_seconds']=time.perf_counter()-began
        self.policy.check_frame(frame)
        if not await self.record(frame, capture):
            return
        legacy = self.cursor is not None and any(b.get('kind')=='image' and not b.get('phash') for b in self.cursor['bubbles'])
        if legacy:
            log_event('legacy_cursor_rebaseline',account_id=self.account,visible_bubbles=len(frame['bubbles']))
        if self.cursor is None or not self.cursor['bubbles'] or legacy:
            # An empty baseline (no chat open after a restart or theme switch) anchors nothing:
            # whatever is visible next is history. Cost: a brand-new contact's first message is
            # only recorded; replies start from the second one.
            self.advance(frame)
            self.health('watching',historical_messages_ignored=True)
            return
        ambiguous = None
        # The same image loads from a blurred preview to full pixels; keep the cursor's identity for it.
        frame['bubbles'] = reconcile(self.cursor['bubbles'],frame['bubbles'])
        try:
            changes = new_bubbles(self.cursor['bubbles'],frame['bubbles'])
        except AmbiguousAnchor as error:
            ambiguous = str(error)
            # Identical repeated bubbles hide the exact overlap. Bubbles after the latest plausible
            # overlap are new in every reading and are handled normally; the ambiguous ones are skipped.
            changes = error.certain
            log_event('ambiguous_bubbles_skipped',account_id=self.account,reason=ambiguous,
                      certain_new=len(changes))
        except ContinuityGap:
            recovered = await self.recover_gap(capture, frame)
            if recovered is not None:
                changes = recovered
            elif self.recovery_unconfirmed:
                self.health('waiting_for_stable_frame',reason='scrolled back but bottom frame changed')
                return
            else:
                # Visible history no longer overlaps the cursor (cleared chat, long absence): which
                # messages were handled is unknowable, so rebaseline without replying.
                ambiguous,changes='visual continuity gap',[]
                log_event('continuity_gap_rebaseline',account_id=self.account,visible_bubbles=len(frame['bubbles']))
        if ambiguous and not changes:
            # Nothing was journaled, sent or given to the model: rebaseline once two frames agree.
            confirmed = await capture.frame()
            self.policy.check_frame(confirmed)
            if list(map(fingerprint,frame['bubbles'])) != list(map(fingerprint,confirmed['bubbles'])):
                self.health('waiting_for_stable_frame',reason=ambiguous)
                return
            self.advance(confirmed)
            self.health('watching',reason=ambiguous+': visible history rebaselined without reply')
            return
        arrivals = []
        for item in changes:
            if item['direction']=='inbound_visual':
                arrivals.append(item)
            elif not self.own_echo(item) and (self.turn or arrivals):
                # A person answered from the phone or desktop: this turn is handled (spec §5.9).
                self.drop_turn('screen')
                arrivals = []
        if arrivals:
            began=time.perf_counter()
            confirmed = await capture.frame()
            timings['confirmation_seconds']=time.perf_counter()-began
            self.policy.check_frame(confirmed)
            confirmed['bubbles'] = reconcile(frame['bubbles'],confirmed['bubbles'])
            if list(map(fingerprint,frame['bubbles'])) != list(map(fingerprint,confirmed['bubbles'])):
                self.health('waiting_for_stable_frame')
                return
            now = self.clock()
            if self.turn is None:
                self.turn = {'opened_at':now,'baseline':self.cursor,'bubbles':[]}
                log_event('turn_opened',account_id=self.account)
            self.turn['bubbles'] += arrivals
            self.turn.update(last_inbound_at=now,frame=confirmed)
            write_private(self.turn_path,self.turn)
            log_event('incoming_candidate',count=len(arrivals),capture_seconds=timings['capture_seconds'])
            self.advance(confirmed)
        elif changes or self.turn is None:
            self.advance(frame)
        if self.turn is None:
            self.health('watching')
            return
        self.had_incoming = True
        if not self.turn_due():
            self.health('waiting_for_turn',pending_visual_messages=len(self.turn['bubbles']))
            return
        await self.process_turn(client, capture, timings)

    def turn_due(self):
        now = self.clock()
        return (now-self.turn['last_inbound_at'] >= self.config.get('turn_quiet_seconds',10) or
                now-self.turn['opened_at'] >= self.config.get('turn_max_seconds',60))

    def keep_turn(self, turn):
        """Put a closed turn back (stopped before sending) so the next start answers it."""
        self.turn = turn
        write_private(self.turn_path,turn)

    def drop_turn(self, source):
        self.turn = None
        self.turn_path.unlink(missing_ok=True)
        log_event('already_replied',account_id=self.account,source=source)
        self.health('watching',reason='already replied ('+source+')')

    def replied_meanwhile(self, confirmed, latest, input_ids):
        """Pre-send check (spec §5.9): a person or an earlier send already answered this turn."""
        source=None
        if self.history is not None and self.history.replied_after(self.contact_id,input_ids):
            source='history'
        else:
            try:
                later=new_bubbles(confirmed['bubbles'],reconcile(confirmed['bubbles'],latest['bubbles']))
            except (ContinuityGap,AmbiguousAnchor):
                later=[]
            if any(b['direction']=='outbound_visual' and not self.own_echo(b) for b in later):
                source='screen'
        if source:
            log_event('already_replied',account_id=self.account,source=source)
            self.health('watching',reason='already replied ('+source+')')
        return source is not None

    def archive_media(self, incoming, input_ids, images, files):
        """Attach each fetched image/file to its history message (spec G4); failures only log."""
        if self.history is None:
            return
        if len(input_ids)!=len(incoming):
            log_event('media_archive_skipped',account_id=self.account,reason='messages not matched')
            return
        pairs=list(zip(incoming,input_ids))
        for item,kind in [(i,'image') for i in images]+[(f,'file') for f in files]:
            message_id=next((m for bubble,m in pairs if bubble==item['bubble']),None)
            if message_id is None:
                continue
            try:
                item['media_id']=self.history.attach_media(message_id,item['path'],kind=kind,filename=item.get('filename'),
                                                           source=item['source'] if kind=='image' else 'original')
                if kind=='file':
                    self.history.finish_media_job(message_id,'done')
            except Exception as error:
                log_event('history_media_attach_failed',error_type=type(error).__name__)

    async def understand_images(self, text, images, failed, context):
        """(notes, images for the reply call). Over one batch, images are understood first (spec §5.4)."""
        batch=self.config.get('images_per_model_call',4)
        config=self.config.get('model_config') or {}
        notes,model_images=[],[i['path'] for i in images]
        if len(images)>batch and config.get('backend')=='openai-compatible':
            found=await understand(images,text=text,config=config,batch_size=batch)
            for item,note in zip(images,found):
                if note is None:
                    failed.append({'label':item['label'],'kind':'image','reason':'model_failed'})
                    continue
                notes.append({'label':item['label'],'notes':note})
                if self.history is not None and item.get('media_id'):
                    try: self.history.set_understanding(item['media_id'],note)
                    except Exception as error: log_event('history_understanding_failed',error_type=type(error).__name__)
            model_images=[]
        else:
            # The API refuses a request over MAX_MODEL_IMAGE_BYTES: later images become unreadable instead.
            model_images,total=[],0
            for item in images:
                size=Path(item['path']).stat().st_size
                if total+size>MAX_MODEL_IMAGE_BYTES:
                    failed.append({'label':item['label'],'kind':'image','reason':'too_large'})
                    continue
                total+=size
                model_images.append(item['path'])
        total=sum(Path(p).stat().st_size for p in model_images)
        # Earlier images without a stored understanding are shown again while the batch has room.
        for item in context:
            media=item.get('media') or {}
            if (media.get('kind')=='image' and not media.get('understanding') and media.get('path')
                    and len(model_images)<batch and Path(media['path']).is_file()
                    and total+Path(media['path']).stat().st_size<=MAX_MODEL_IMAGE_BYTES):
                model_images.append(media['path'])
                total+=Path(media['path']).stat().st_size
        return notes,model_images

    def own_echo(self, bubble):
        """An outbound bubble matching a reply this contact's service sent in the last 10 minutes."""
        for path in self.root.glob('send-*.json'):
            record = json.loads(path.read_text())
            if (record.get('reply') and time.time()-record.get('attempted_at',0) < 600
                    and (echo_matches(bubble.get('text',''),record['reply']) or similar(bubble.get('text',''),record['reply']))):
                return True
        return False

    async def process_turn(self, client, capture, timings):
        incoming = self.turn['bubbles']
        if self.limited():
            self.health('rate_limited',pending_visual_messages=len(incoming))
            return
        confirmed = self.turn['frame']
        log_event('turn_closed',account_id=self.account,texts=sum(b.get('kind','text')=='text' for b in incoming),
                  images=sum(b.get('kind')=='image' for b in incoming),files=sum(b.get('kind')=='file' for b in incoming),
                  wait_seconds=round(self.clock()-self.turn['opened_at'],1))
        # Read history before the journal commit: a read failure must leave the turn pending.
        context = [{'role':'user' if b['direction']=='inbound_visual' else 'assistant','text':b['text']}
                   for b in confirmed['bubbles'][-12:]]
        input_ids = []
        if self.history is not None:
            try:
                input_ids = self.history.match_recent_inbound(self.contact_id,incoming)
                context = self.history.recent_context(self.contact_id,self.config.get('context_messages',5),
                                                      exclude_ids=frozenset(input_ids))
                context += [{'role':'user','text':b['text']} for b in incoming]
            except Exception as error:
                self.history_failed(error)
                return
        key = self.batch_key(self.turn['baseline'],incoming)
        journal = self.root/('send-'+key+'.json')
        if journal.exists():
            if json.loads(journal.read_text()).get('status')=='triggered':
                self.drop_turn('journal')  # sent before a restart; never send it twice
                return
            if json.loads(journal.read_text()).get('status')=='stopped_before_send':
                journal.unlink()  # stopped before any send attempt: answer the restored turn now
            else:
                self.paused = True
                self.health('paused',reason='existing batch requires review',idempotency_key=key)
                return
        record = {'status':'preparing_media','account_id':self.account,'recipient':self.recipient,
                  'idempotency_key':key,'input':incoming,'direction_evidence':'visual_candidate',
                  'created_at':time.time(),'platform_message_verified':False,'timings':timings}
        self.write_journal(journal,record)
        # The journal now owns this turn; a restart never blindly repeats an uncertain attempt.
        pending = self.turn
        self.turn = None
        self.turn_path.unlink(missing_ok=True)
        self.health('preparing_media',idempotency_key=key)
        try:
            began=time.perf_counter()
            images,files,failed=[],[],[]
            if any(b.get('kind','text')!='text' for b in incoming):
                if not hasattr(capture,'attachments'):
                    raise ValueError('media-capable capture backend required')
                # Media IDs are bound to the frame they were seen in: fetch from what is on screen now.
                snapshot=await capture.frame()
                self.policy.check_frame(snapshot)
                fetch=relocate(incoming,snapshot['bubbles'])
                images,files,failed=await capture.attachments(fetch,snapshot)
            else:
                fetch=incoming
            for item in failed:
                log_event('media_failed',account_id=self.account,kind=item['kind'],reason=item['reason'])
            self.archive_media(fetch,input_ids,images,files)
            timings['media_seconds']=time.perf_counter()-began
            record.update(status='generating',media=[{'kind':f.get('kind','document'),'filename':f.get('filename'),
                'original_bytes':f.get('original_bytes'),'truncated':f.get('truncated',False)} for f in files],
                images=[{'label':i['label'],'source':i['source']} for i in images],failed=failed)
            self.write_journal(journal,record)
            self.health('generating',idempotency_key=key,image_count=len(images),file_count=len(files))
            if not self.target_allowed():
                record['status']='skipped_by_rule'; self.write_journal(journal,record); return
            model={}
            text='\n'.join(b['text'] for b in incoming)
            notes,model_images=await self.understand_images(text,images,failed,context)
            reply = validate_reply(await generate(text,mode='service',context=context,images=model_images,
                                                  files=[model_view(f) for f in files],notes=notes,failed=[f['label'] for f in failed],
                                                  metadata=model,model_config=self.config.get('model_config')))
            reply = ensure_unreadable_named(reply,failed)
            timings['model_seconds']=model.get('seconds')
            log_event('model_completed',**model)
            record['model']=model
            record.update(status='generated',reply=reply)
            self.write_journal(journal,record)
            if self.stop.is_set():
                record['status']='stopped_before_send'; self.write_journal(journal,record); self.keep_turn(pending); return
            began=time.perf_counter()
            latest = await capture.frame()
            timings['pre_send_seconds']=time.perf_counter()-began
            # Title/profile only: new arrivals during generation become the next turn (spec §5.8).
            self.policy.check_frame(latest)
            if self.replied_meanwhile(confirmed,latest,input_ids):
                record['status']='already_replied'; self.write_journal(journal,record)
                return
            if self.stop.is_set():
                record['status']='stopped_before_send'; self.write_journal(journal,record); self.keep_turn(pending); return
            if not self.target_allowed():
                record['status']='skipped_by_rule'; self.write_journal(journal,record); return
            if self.history is not None:
                self.history.record_bot_reply(self.contact_id,key,reply,'generated',record.get('model',{}),input_ids)
            record.update(status='attempting',attempted_at=time.time())
            self.write_journal(journal,record)
            self.health('sending',idempotency_key=key)
            began=time.perf_counter()
            # Send to the title actually verified on screen (an alias when OCR drops emoji/symbols).
            result = await client.tool('send_message',recipient=self.clean_recipient(confirmed),message=reply,idempotency_key=key)
            receipt = await client.tool('get_send_status',idempotency_key=key)
            timings['send_seconds']=time.perf_counter()-began
            record.update(status=receipt.get('status','unknown'),send_result=result,durable_status=receipt,delivery='unknown')
            self.write_journal(journal,record)
            if record['status'] != 'triggered':
                raise RuntimeError('send outcome requires review')
        except Exception as error:
            log_event('processing_failed',phase=record['status'],error_type=type(error).__name__,idempotency_key=key)
            record['error']=str(error)[:400]
            self.write_journal(journal,record)
            self.paused = True
            self.pause_reason = 'model or send attempt requires review: '+type(error).__name__
            self.health('paused',reason=self.pause_reason,idempotency_key=key)
            raise
        # A readback error after a durable triggered receipt must not trigger another send.
        self.health('verifying_send',idempotency_key=key)
        began=time.perf_counter()
        after = await capture.frame()
        timings['echo_seconds']=time.perf_counter()-began
        self.policy.check_frame(after)
        echoed = any(b['direction']=='outbound_visual' and echo_matches(b['text'],reply) for b in after['bubbles'])
        record['client_echo_observed']=echoed
        self.write_journal(journal,record)
        try:
            unseen = new_bubbles(confirmed['bubbles'],after['bubbles'])
        except (ContinuityGap, AmbiguousAnchor) as error:
            # The send has a durable receipt. Tall media can scroll every remembered bubble away;
            # keep the pre-send cursor so the next cycle applies the usual gap rule (two equal
            # frames, rebaseline without reply) instead of pausing the contact.
            record['postflight_error']=str(error)
            self.write_journal(journal,record)
            log_event('postflight_gap',idempotency_key=key,durable_send_status=record['status'])
            unseen = None
        except ValueError as error:
            record['postflight_error']=str(error)
            self.write_journal(journal,record)
            log_event('postflight_failed',idempotency_key=key,durable_send_status=record['status'],error=str(error))
            raise
        if unseen is not None and not any(b['direction']=='inbound_visual' for b in unseen):
            self.advance(after)
        log_event('batch_completed',idempotency_key=key,status=record['status'],client_echo_observed=echoed,timings=timings)
        self.health('watching',last_send_id=receipt.get('send_id'),last_client_echo_observed=echoed,delivery='unknown')

    async def run(self):
        log_event('service_started',pid=os.getpid(),capture_backend=self.config.get('capture_backend','native'),
                  model_backend=self.config.get('model_config',{}).get('backend','codex-exec'),
                  model=self.config.get('model_config',{}).get('model'),state_dir=str(self.root))
        for path in sorted(self.root.glob('send-*.json')):
            pending=json.loads(path.read_text())
            if pending.get('status') in ['preparing_media','generating','generated','attempting','unknown']:
                # Journal errors produced by model/transport code are safe status messages, never HTTP bodies.
                log_event('unfinished_journal',journal=path.name,phase=pending['status'],error=pending.get('error'))
        for sig in (signal.SIGTERM,signal.SIGINT):
            asyncio.get_running_loop().add_signal_handler(sig,self.stop.set)
        lock_path = self.root/'service.lock'
        with open(lock_path,'a') as lock:
            lock_path.chmod(0o600)
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            try:
                while not self.stop.is_set():
                    if self.paused:
                        self.health('paused',reason=self.pause_reason or 'review persisted bot journal before restarting')
                        await self.wait(5)
                        continue
                    client = MCP(['ssh','-o','BatchMode=yes','-o','LogLevel=ERROR',self.config['host'],
                                  '/usr/bin/nc -U "$HOME/.wechat-rpa/worker.sock"'],
                                 {'platform':'wechat','account_id':self.account})
                    capture_class=NativeCapture if self.config.get('capture_backend','native')=='native' else RemoteCapture
                    capture = capture_class(self.config['host'],client,self.recipient,self.root,self.ownership)
                    try:
                        self.health('connecting')
                        await client.start()
                        status = await client.tool('get_listener_status')
                        if status.get('enabled'):
                            raise ValueError('another observer is enabled')
                        await capture.start()
                        while not self.stop.is_set() and not self.paused:
                            await self.cycle(client,capture)
                            await self.wait(self.config['poll_seconds'])
                    except ValueError as error:
                        self.paused=True
                        self.pause_reason=str(error)[:400]
                        log_event('observer_failed',error_type=type(error).__name__)
                        self.health('paused',reason=self.pause_reason)
                    except Exception as error:
                        log_event('observer_failed',error_type=type(error).__name__)
                        self.health('paused' if self.paused else 'reconnecting',reason=str(error)[:400])
                    finally:
                        try:
                            await capture.close()
                        except Exception:
                            pass  # Stable ownership allows recovery of our own diagnostic flag after reconnect.
                        await client.close()
                    if not self.stop.is_set():
                        await self.wait(5)
            finally:
                self.health('stopped')

    async def wait(self, seconds):
        try:
            await asyncio.wait_for(self.stop.wait(),seconds)
        except asyncio.TimeoutError:
            pass


def main():
    parser=argparse.ArgumentParser(description='Allowlisted personal-WeChat customer-service pilot')
    parser.add_argument('--config',required=True)
    args=parser.parse_args()
    logging.basicConfig(level=logging.INFO,format='%(message)s',stream=sys.stdout)
    from .coordinator import BotCoordinator
    asyncio.run(BotCoordinator(args.config).run())


if __name__=='__main__':
    main()
