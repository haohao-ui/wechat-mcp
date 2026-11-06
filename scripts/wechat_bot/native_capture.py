"""Persistent SSH/private worker transport. Captures never invoke the public read workflow."""
import asyncio
import base64
import hashlib
import json
import os
import time
from pathlib import Path

from .capture import RemoteCapture


# Combined extracted document content given to the model in one reply.
MAX_EXTRACTED=24000


# Action module refusals: a person moved the mouse, or the control is off screen. Nothing was done;
# callers retry on a later sweep instead of downgrading (thumbnail, "please resend", MCP search).
ACTION_INTERRUPTED=('user_input_detected','target_not_reached')


def action_interrupted(error):
    return any(f'invalid("{code}")' in str(error) for code in ACTION_INTERRUPTED)


def layout_unavailable(error):
    # The worker has no usable layout map (missing, unreadable, invalid): nothing may be clicked,
    # not even through the MCP search fallback, until the map is back (stage B acceptance).
    return 'invalid("layout_' in str(error)


ROW_NOT_VISIBLE='session_row_not_unique_or_missing'


def must_wait(error):
    # A WeChat menu, save panel or second window is open: wait, never fall back to MCP navigation.
    # A chat whose row is not visible has no new message to read (WeChat lifts it to the top when one
    # arrives): wait for the row instead of typing its name into search (acceptance 2026-11-06).
    return (action_interrupted(error) or layout_unavailable(error) or 'invalid("screen_not_main' in str(error)
            or ROW_NOT_VISIBLE in str(error))


def worker_code(error):
    # Worker refusals are fixed codes (invalid("...")), never message content; keep them for diagnosis.
    text=str(error)
    start=text.find('invalid("')
    return text[start+9:text.find('")',start)][:120] if start>=0 else type(error).__name__


def media_reason(error):
    text=str(error)
    for code,reason in [('unsupported_file_type','unsupported_type'),('file_size_limit','too_large'),
                        ('media_too_large','too_large'),('file_snapshot_changed','snapshot_changed')]:
        if code in text: return reason
    return 'download_failed'


class NativeCapture(RemoteCapture):
    def __init__(self,*args,names=None,manage_window=True,group=False,**kwargs):
        super().__init__(*args,**kwargs)
        self.names=list(names or [])
        # Group titles carry a member count ("测试群 (3)") that the worker ignores when this is set.
        self.group=bool(group)
        # Which text engine the worker really used (vision/rapidocr) and how often RapidOCR fell back.
        self.ocr_status=None
        # Dedicated-machine default: the worker may un-minimize, resize and move WeChat back on screen.
        self.manage_window=bool(manage_window)
        self.process=None
        self.counter=0
        self.last_switch=None
        self.rpc_lock=asyncio.Lock()

    async def start(self):
        if not self.owner_token:
            raise ValueError('persistent worker requires stable ownership token')
        script=b'''set -eu
dir="$HOME/.wechat-rpa"
test -x "$dir/bin/wechat_bot_worker"
test "$(stat -f %u "$dir")" = "$(id -u)"
test "$(stat -f %Lp "$dir")" = 700
owner='__OWNER__'
if test -e "$dir/bot-worker.enabled"; then
 test "$(cat "$dir/bot-worker.enabled")" = "$owner"
else
 umask 077
 set -C
 printf '%s' "$owner" > "$dir/bot-worker.enabled"
fi
'''
        await self.ssh('/bin/sh -s',script.replace(b'__OWNER__',self.owner_token.encode()))
        self.active=True
        try:
            # Worker still running (e.g. reconnect after the window was minimized): it can restore the
            # window (manage_window) and click the verified row, which MCP search cannot do.
            await self.connect(seconds=3)
            self.last_switch=await self.call('open_session',target=self.recipient,target_names=self.names or [self.recipient])
            return
        except (ValueError,RuntimeError,ConnectionError,asyncio.TimeoutError) as error:
            # The worker exits when its one connection closes and only a GUI action can start it again:
            # a wait (layout, screen, person moving the mouse) keeps the live connection.
            if must_wait(error): raise
            await self.close_transport()
        try:
            # Quiet relaunch (Stage C drill 2026-11-06): get_messages without a recipient only focuses
            # WeChat (Cmd+1, no click, no typing) and starts the worker in its TCC context; the
            # map-checked row click then opens the chat. Search navigation is the last resort.
            await self.client.tool('get_messages')
            await self.connect()
            self.last_switch=await self.call('open_session',target=self.recipient,target_names=self.names or [self.recipient])
            return
        except (ValueError,RuntimeError,ConnectionError,asyncio.TimeoutError) as error:
            if must_wait(error): raise
            from .service import log_event  # Lazy: service imports this module.
            log_event('quiet_relaunch_failed',recipient=self.recipient,error_type=type(error).__name__,error=str(error)[:200])
            await self.close_transport()
        # This one admitted GUI operation focuses the allowed chat and starts the helper in its TCC context.
        await self.navigate(self.names)
        await self.connect()

    async def navigate(self, names):
        # The MCP navigation matches the title exactly; OCR may drop emoji, so try each configured name.
        error=None
        for name in names or [self.recipient]:
            try:
                return await self.client.tool('get_messages',recipient=name)
            except Exception as failure:
                error=failure
        raise error

    async def retarget(self, recipient, directory, names=None, group=False):
        self.names=list(names or [recipient])
        self.group=bool(group)
        # Keep the GUI worker and its status indicator alive while selecting the next authorized chat.
        try:
            # Fast path: click the verified session-list row; the worker checks the exact title after.
            self.last_switch=await self.call('open_session',target=recipient,target_names=self.names)
        except ValueError as error:
            if must_wait(error): raise
            self.last_switch={'opened':False,'fallback':str(error)[:200]}
            from .service import log_event  # Lazy: service imports this module.
            log_event('row_click_failed',recipient=recipient,error=str(error)[:200])
            await self.navigate(self.names)
        self.recipient=recipient
        self.directory=Path(directory)
        await self.frame()  # The native worker verifies the full title before any new media is returned.

    async def connect(self, seconds=60):
        # Deadline, not attempt count: the worker warms Vision (~30s cold on macOS 26+) before listening.
        deadline=time.monotonic()+seconds
        while True:
            try:
                self.process=await asyncio.create_subprocess_exec(
                    'ssh','-o','BatchMode=yes','-o','LogLevel=ERROR',self.host,
                    '/usr/bin/nc -U "$HOME/.wechat-rpa/bot-worker.sock"',
                    stdin=asyncio.subprocess.PIPE,stdout=asyncio.subprocess.PIPE,stderr=asyncio.subprocess.DEVNULL,
                    limit=4*1024*1024)
                # Readiness probe without a title check, so a different open chat does not abort reuse.
                await self.call('sessions',names=self.names or [self.recipient])
                return
            except (RuntimeError,ConnectionError,asyncio.TimeoutError):
                await self.close_transport()
                if time.monotonic()>=deadline:
                    raise RuntimeError('private GUI bot worker did not become ready')
                await asyncio.sleep(0.2)

    async def call(self,operation,**params):
        async with self.rpc_lock:
            self.counter+=1
            value={'protocol_version':1,'id':self.counter,'operation':operation,
                   'params':{'recipient':self.recipient,'accepted_names':self.names or [self.recipient],
                             'manage_window':self.manage_window,'group_title':self.group,**params}}
            try:
                self.process.stdin.write(json.dumps(value).encode()+b'\n')
                await self.process.stdin.drain()
                line=await asyncio.wait_for(self.process.stdout.readline(),35)
                if not line:
                    raise RuntimeError('private bot worker disconnected')
                response=json.loads(line)
                if response.get('id')!=self.counter or response.get('protocol_version')!=1:
                    raise RuntimeError('private bot worker protocol mismatch')
            except BaseException:
                await self.close_transport()
                raise
        self.record_ocr(response.get('ocr'))
        if not response.get('ok'):
            # A refused operation is a complete, in-sync reply; the transport stays usable.
            raise ValueError('private visual operation failed: '+str(response.get('error')))
        return response['result']

    def record_ocr(self,report):
        if not isinstance(report,dict) or not report.get('engine_used'):
            return  # older worker: keep the last known status
        status=self.ocr_status or {'engine_used':None,'fallbacks':0,'last_fallback':None}
        status={**status,'engine_used':report['engine_used']}
        if report.get('fallback'):
            from .service import log_event  # Lazy: service imports this module.
            status['fallbacks']+=1; status['last_fallback']=str(report['fallback'])[:200]
            log_event('ocr_fallback',engine=report.get('engine'),reason=status['last_fallback'])
        self.ocr_status=status

    async def frame(self):
        return self.logged(await self.call('capture'))

    @staticmethod
    def logged(result):
        if result.get('window_resized'):
            from .service import log_event  # Lazy: service imports this module.
            log_event('window_resized',profile=result.get('profile'),scale=result.get('scale'))
        return result

    async def scroll(self, direction, pages=1):
        # Wheel-scrolls the open chat and returns the frame captured after it.
        if direction not in ('up','down') or not 1<=pages<=40:
            raise ValueError('invalid scroll request')
        return self.logged(await self.call('scroll_chat',direction=direction,pages=pages))

    async def sessions(self, names, debug=False):
        # Read-only probe of the left session list; only allowlisted rows are returned.
        return self.logged(await self.call('sessions',names=list(names),debug=debug))

    async def calibrate(self):
        # Read-only window capture and anchor calibration; never clicks, scrolls, or types.
        return self.logged(await self.call('calibrate'))

    async def download(self,media_id,kind='image'):
        if len(media_id)!=64 or any(x not in '0123456789abcdef' for x in media_id):
            raise ValueError('invalid media ID')
        data=bytearray()
        while True:
            result=await self.call('read_media',media_id=media_id,offset=len(data))
            if result['offset']!=len(data) or not 0<=result['size']<=10*1024*1024:
                raise RuntimeError('invalid media size/offset')
            chunk=base64.b64decode(result['bytes'],validate=True)
            if not chunk and not result['eof']:
                raise RuntimeError('empty nonterminal media chunk')
            data.extend(chunk)
            if len(data)>result['size']:
                raise RuntimeError('oversized media response')
            if result['eof']:
                if len(data)!=result['size'] or hashlib.sha256(data).hexdigest()!=result['sha256']:
                    raise RuntimeError('media digest mismatch')
                break
        extension=result.get('extension','png')
        if extension not in ['png','jpg','jpeg','pdf','txt','md','csv','json','docx','xlsx']:
            raise RuntimeError('unexpected media extension')
        suffix='.'+extension
        path=self.directory/(media_id+suffix)
        descriptor=os.open(path,os.O_CREAT|os.O_WRONLY|os.O_TRUNC,0o600)
        with os.fdopen(descriptor,'wb') as handle:
            handle.write(data)
        return path

    async def attachments(self,items,frame):
        """Fetch every image and file of a turn: (images, files, failed); one bad item never aborts the rest.

        Images are labelled 第N张图片 in turn order and prefer the saved original over the chat thumbnail.
        """
        from .documents import extract
        from .policy import relocate
        images,files,failed=[],[],[]
        used=0
        number=0
        items=list(items)
        for index,item in enumerate(items):
            kind=item.get('kind','text')
            if kind=='image':
                number+=1
                label='第%d张图片'%number
                source='original'
                try:
                    await self.call('prepare_image',media_id=item['media_id'],snapshot_id=frame['snapshot_id'])
                    # Save-as changes the screen; later items are looked up in a fresh frame.
                    frame=await self.frame()
                    items[index+1:]=relocate(items[index+1:],frame['bubbles'])
                except (ValueError,RuntimeError) as error:
                    if action_interrupted(error): raise
                    source='chat_thumbnail'
                    from .service import log_event  # Lazy: service imports this module.
                    log_event('image_original_unavailable',reason=media_reason(error),detail=worker_code(error))
                try:
                    path=await self.download(item['media_id'])
                except (ValueError,RuntimeError) as error:
                    failed.append({'label':label,'kind':'image','reason':'download_failed'})
                    continue
                images.append({'path':path,'label':label,'source':source,'bubble':item})
            elif kind=='file':
                label=item.get('filename') or '文件'
                try:
                    info=await self.call('prepare_file',media_id=item['media_id'],snapshot_id=frame['snapshot_id'])
                    path=await self.download(item['media_id'],kind='file')
                    frame=await self.frame()
                    items[index+1:]=relocate(items[index+1:],frame['bubbles'])
                except (ValueError,RuntimeError) as error:
                    if action_interrupted(error): raise
                    failed.append({'label':label,'kind':'file','reason':media_reason(error),'detail':worker_code(error)})
                    continue
                label=info.get('filename') or label
                if info['extension'] in ['png','jpg','jpeg']:
                    images.append({'path':path,'label':label,'source':'original','bubble':item})
                    continue
                try:
                    content=extract(path,info['extension'],info.get('extraction'))
                except (ValueError,RuntimeError,OSError):
                    failed.append({'label':label,'kind':'file','reason':'extract_failed'})
                    continue
                previews=info.get('extraction',{}).get('previews',[])
                if not (content.get('text') or content.get('rows') or content.get('sheets') or previews):
                    failed.append({'label':label,'kind':'file','reason':'empty'})
                    continue
                size=len(json.dumps(content,ensure_ascii=False))
                if used+size>MAX_EXTRACTED:
                    failed.append({'label':label,'kind':'file','reason':'too_large'})
                    continue
                used+=size
                for page,preview in enumerate(previews,1):
                    images.append({'path':await self.download(preview),'label':'%s第%d页'%(label,page),
                                   'source':'preview','bubble':item})
                files.append({'filename':info['filename'],'label':label,'original_bytes':True,'path':path,
                              'bubble':item,**content})
        return images,files,failed

    async def close_transport(self):
        if self.process:
            process,self.process=self.process,None
            if process.stdin: process.stdin.close()
            try: await asyncio.wait_for(process.wait(),2)
            except asyncio.TimeoutError:
                process.terminate();await process.wait()

    async def close(self):
        await self.close_transport()
        if self.active:
            script=b'''set -eu
flag="$HOME/.wechat-rpa/bot-worker.enabled"
test "$(cat "$flag")" = '__OWNER__'
rm -f "$flag"
'''
            await self.ssh('/bin/sh -s',script.replace(b'__OWNER__',self.owner_token.encode()))
            self.active=False
