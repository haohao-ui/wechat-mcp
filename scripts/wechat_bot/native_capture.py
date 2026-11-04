"""Persistent SSH/private worker transport. Captures never invoke the public read workflow."""
import asyncio
import base64
import hashlib
import json
import os
from pathlib import Path

from .capture import RemoteCapture


class NativeCapture(RemoteCapture):
    def __init__(self,*args,names=None,**kwargs):
        super().__init__(*args,**kwargs)
        self.names=list(names or [])
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
        # This one admitted GUI operation focuses the allowed chat and starts the helper in its TCC context.
        await self.client.tool('get_messages',recipient=self.recipient)
        await self.connect()

    async def retarget(self, recipient, directory, names=None):
        self.names=list(names or [recipient])
        # Keep the GUI worker and its status indicator alive while selecting the next authorized chat.
        try:
            # Fast path: click the verified session-list row; the worker checks the exact title after.
            self.last_switch=await self.call('open_session',target=recipient,target_names=self.names)
        except ValueError as error:
            self.last_switch={'opened':False,'fallback':str(error)[:200]}
            await self.client.tool('get_messages',recipient=recipient)
        self.recipient=recipient
        self.directory=Path(directory)
        await self.frame()  # The native worker verifies the full title before any new media is returned.

    async def connect(self):
        for _ in range(20):
            try:
                self.process=await asyncio.create_subprocess_exec(
                    'ssh','-o','BatchMode=yes','-o','LogLevel=ERROR',self.host,
                    '/usr/bin/nc -U "$HOME/.wechat-rpa/bot-worker.sock"',
                    stdin=asyncio.subprocess.PIPE,stdout=asyncio.subprocess.PIPE,stderr=asyncio.subprocess.DEVNULL,
                    limit=4*1024*1024)
                await self.call('capture')
                return
            except (RuntimeError,ConnectionError,asyncio.TimeoutError):
                await self.close_transport()
                await asyncio.sleep(0.2)
        raise RuntimeError('private GUI bot worker did not become ready')

    async def call(self,operation,**params):
        async with self.rpc_lock:
            self.counter+=1
            value={'protocol_version':1,'id':self.counter,'operation':operation,
                   'params':{'recipient':self.recipient,'accepted_names':self.names or [self.recipient],**params}}
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
        if not response.get('ok'):
            # A refused operation is a complete, in-sync reply; the transport stays usable.
            raise ValueError('private visual operation failed: '+str(response.get('error')))
        return response['result']

    async def frame(self):
        return await self.call('capture')

    async def sessions(self, names, debug=False):
        # Read-only probe of the left session list; only allowlisted rows are returned.
        return await self.call('sessions',names=list(names),debug=debug)

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

    async def attachments(self,incoming,frame):
        from .documents import extract
        images,files=[],[]
        for item in incoming:
            kind=item.get('kind','text')
            if kind=='image':
                if len(images)>=4: raise ValueError('at most four images per reply')
                path=await self.download(item['media_id'])
                images.append(path)
                files.append({'kind':'image','source':item.get('image_source','chat_thumbnail'),'original_bytes':False})
            elif kind=='file':
                info=await self.call('prepare_file',media_id=item['media_id'],snapshot_id=frame['snapshot_id'])
                path=await self.download(item['media_id'],kind='file')
                if info['extension'] in ['png','jpg','jpeg']:
                    images.append(path)
                    files.append({'filename':info['filename'],'kind':'image','original_bytes':True})
                else:
                    content=extract(path,info['extension'],info.get('extraction'))
                    for preview in info.get('extraction',{}).get('previews',[]):
                        if len(images)>=4: raise ValueError('too many PDF/image previews')
                        images.append(await self.download(preview))
                    files.append({'filename':info['filename'],'original_bytes':True,**content})
        if len(json.dumps(files,ensure_ascii=False))>24000:
            raise ValueError('combined extracted content too large')
        return images,files

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
