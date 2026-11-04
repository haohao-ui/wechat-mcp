"""OpenAI-compatible Chat Completions adapter; credentials never enter model journals."""
import asyncio
import base64
import json
import os
import re
from pathlib import Path
import stat
import time
import urllib.error
import urllib.parse
import urllib.request

from .policy import validate_reply


def validate_config(config):
    if not isinstance(config, dict):
        raise ValueError('model_config must be an object')
    backend = config.get('backend', 'codex-exec')
    if backend == 'codex-exec':
        if set(config) - {'backend'}:
            raise ValueError('unsupported codex model setting')
        return
    allowed = {'backend', 'base_url', 'model', 'api_key_file', 'api_key_env', 'supports_images', 'timeout_seconds', 'json_output'}
    if backend != 'openai-compatible' or set(config) - allowed:
        raise ValueError('unsupported model backend or setting')
    url = urllib.parse.urlsplit(config.get('base_url', ''))
    local = url.hostname in ('localhost', '127.0.0.1', '::1')
    if (url.scheme != 'https' and not (url.scheme == 'http' and local)) or not url.hostname or url.username or url.password or url.query or url.fragment:
        raise ValueError('model base_url requires HTTPS, or local HTTP, without credentials/query')
    if not isinstance(config.get('model'), str) or not config['model'].strip():
        raise ValueError('explicit model name required')
    if 'api_key_file' in config and (not isinstance(config['api_key_file'], str) or not Path(config['api_key_file']).is_absolute()):
        raise ValueError('absolute api_key_file required')
    if 'api_key_env' in config and (not isinstance(config['api_key_env'], str) or not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', config['api_key_env'])):
        raise ValueError('invalid api_key_env name')
    for key in ('supports_images', 'json_output'):
        if type(config.get(key, False if key == 'supports_images' else True)) is not bool:
            raise ValueError('invalid ' + key)
    timeout = config.get('timeout_seconds', 60)
    if type(timeout) is not int or not 1 <= timeout <= 180:
        raise ValueError('model timeout must be 1..180 seconds')


def read_key(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600:
            raise ValueError('api_key_file must be an owned regular file with mode 600')
        key = os.read(descriptor, 4097).decode().strip()
        if not key or len(key) > 4096 or any(c.isspace() for c in key):
            raise ValueError('invalid API key file')
        return key
    finally:
        os.close(descriptor)


def resolve_key(config):
    if 'api_key_file' in config:
        return read_key(config['api_key_file'])
    host = urllib.parse.urlsplit(config['base_url']).hostname
    name = config.get('api_key_env', 'DEEPSEEK_API_KEY' if host == 'api.deepseek.com' else 'OPENAI_API_KEY')
    key = os.environ.get(name, '').strip()
    if not key:
        raise ValueError('model API key environment variable is missing: ' + name)
    if len(key) > 4096 or any(c.isspace() for c in key):
        raise ValueError('invalid model API key environment variable: ' + name)
    return key


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def request_completion(config, payload):
    key = resolve_key(config)
    request = urllib.request.Request(config['base_url'].rstrip('/') + '/chat/completions',
        data=json.dumps(payload, ensure_ascii=False).encode(),
        headers={'Authorization': 'Bearer ' + key, 'Content-Type': 'application/json'}, method='POST')
    try:
        with urllib.request.build_opener(NoRedirect()).open(request, timeout=config.get('timeout_seconds', 60)) as response:
            data = response.read(2 * 1024 * 1024 + 1)
    except urllib.error.HTTPError as error:
        raise RuntimeError('model HTTP error ' + str(error.code) + '; not automatically retried') from None
    except (OSError, urllib.error.URLError):
        raise RuntimeError('model connection failed; not automatically retried') from None
    if len(data) > 2 * 1024 * 1024:
        raise ValueError('model response too large')
    return json.loads(data)


def parse_json_object(content):
    """The JSON object in the model text (tolerating surrounding prose), or None if malformed."""
    try:
        return json.loads(content)
    except json.JSONDecodeError:
        start = content.find('{')
        if start < 0:
            return None
        try:
            return json.JSONDecoder().raw_decode(content, start)[0]
        except json.JSONDecodeError:
            return None


async def generate_api(prompt, *, images, config, metadata=None):
    validate_config(config)
    if images and not config.get('supports_images', False):
        raise ValueError('configured model has no enabled image support')
    content = [{'type': 'text', 'text': prompt}]
    total = 0
    for image in images:
        path = Path(image)
        if not path.is_file() or path.stat().st_size > 10 * 1024 * 1024:
            raise ValueError('invalid image or image exceeds 10MiB')
        data = path.read_bytes()
        mime = 'image/png' if data.startswith(b'\x89PNG\r\n\x1a\n') else 'image/jpeg' if data.startswith(b'\xff\xd8\xff') else None
        if mime is None:
            raise ValueError('unsupported image bytes')
        total += len(data)
        if total > 30 * 1024 * 1024:
            raise ValueError('combined images exceed 30MiB')
        content.append({'type': 'image_url', 'image_url': {'url': 'data:' + mime + ';base64,' + base64.b64encode(data).decode()}})
    payload = {'model': config['model'], 'messages': [{'role':'system','content':'只输出JSON对象，严格使用 {"reply":"回复正文"}，只允许reply一个字段，值为非空字符串。不要输出Markdown或其他字段。'}, {'role': 'user', 'content': content if images else prompt}],
               'stream': False, 'max_tokens': 1024}
    if config.get('json_output', True):
        payload['response_format'] = {'type': 'json_object'}
    began = time.perf_counter()
    # Transport errors are never retried; malformed JSON content (no side effects yet) is asked once more.
    for attempt in range(2):
        result = await asyncio.to_thread(request_completion, config, payload)
        choices = result.get('choices', [])
        if len(choices) != 1 or choices[0].get('finish_reason') != 'stop':
            raise ValueError('model response missing or incomplete')
        message = choices[0].get('message', {})
        if message.get('tool_calls') or not isinstance(message.get('content'), str):
            raise ValueError('expected model text without tool calls')
        parsed = parse_json_object(message['content'])
        if parsed is not None:
            break
    else:
        raise ValueError('model returned malformed JSON twice')
    if not isinstance(parsed, dict) or set(parsed) != {'reply'}:
        raise ValueError('expected JSON with reply only')
    reply = validate_reply(parsed['reply'])
    if metadata is not None:
        metadata.update({'backend': 'openai-compatible', 'provider': urllib.parse.urlsplit(config['base_url']).hostname,
                         'requested_model': config['model'], 'model': result.get('model'),
                         'seconds': time.perf_counter()-began, 'usage': result.get('usage')})
    return reply
