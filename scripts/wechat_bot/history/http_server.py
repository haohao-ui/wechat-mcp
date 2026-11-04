"""Read-only HTTP API over the conversation history (design §10.4). Loopback + bearer token by default."""
import argparse
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import mimetypes
import os
from pathlib import Path
import re
import secrets
import threading
import urllib.parse

from .mcp_server import default_config, open_query
from .query import QueryError

INT_PARAMS = {'limit', 'since', 'until'}
ROUTES = [
    (re.compile(r'^/v1/contacts$'), 'contacts'),
    (re.compile(r'^/v1/contacts/(c_[0-9a-f]{16})$'), 'get_contact'),
    (re.compile(r'^/v1/contacts/(c_[0-9a-f]{16})/recent$'), 'get_recent_messages'),
    (re.compile(r'^/v1/contacts/(c_[0-9a-f]{16})/messages$'), 'get_history'),
    (re.compile(r'^/v1/search$'), 'search_messages'),
    (re.compile(r'^/v1/messages/(m_[0-9a-f]{16})$'), 'get_message'),
    (re.compile(r'^/v1/media/([0-9a-f]{64})$'), 'get_media'),
]
ALLOWED = {'contacts': {'query', 'cursor', 'limit', 'name'}, 'get_contact': set(), 'get_recent_messages': {'limit'},
           'get_history': {'since', 'until', 'cursor', 'limit', 'order'},
           'search_messages': {'q', 'contact_id', 'since', 'until', 'cursor', 'limit'},
           'get_message': set(), 'get_media': set()}
ID_ARG = {'get_contact': 'contact_id', 'get_recent_messages': 'contact_id', 'get_history': 'contact_id',
          'get_message': 'message_id', 'get_media': 'media_id'}


def ensure_token(history_dir):
    path = Path(history_dir)/'http-token'
    if not path.exists():
        descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, 'w') as handle:
            handle.write(secrets.token_urlsafe(32))
    os.chmod(path, 0o600)
    return path.read_text().strip()


def make_server(query, token, bind, port):
    # sqlite3.threadsafety is 1 here: one connection must not be used by threads concurrently.
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        server_version = 'wechat-history/1'

        def log_message(self, *args):
            pass  # never log query strings (customer names / search terms)

        def reply(self, status, body, content_type='application/json; charset=utf-8'):
            data = body if isinstance(body, bytes) else json.dumps(body, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(data)))
            self.send_header('Cache-Control', 'no-store')
            self.end_headers()
            self.wfile.write(data)

        def error(self, status, code, message):
            self.reply(status, {'error': {'code': code, 'message': message}})

        def refuse(self):
            self.error(405, 'method_not_allowed', 'read-only API: GET only')

        do_POST = do_PUT = do_DELETE = do_PATCH = refuse

        def do_GET(self):
            supplied = self.headers.get('Authorization', '')
            if not hmac.compare_digest(supplied.encode(), ('Bearer ' + token).encode()):
                return self.error(401, 'unauthorized', 'bearer token required')
            url = urllib.parse.urlsplit(self.path)
            for pattern, action in ROUTES:
                match = pattern.match(url.path)
                if match:
                    break
            else:
                return self.error(404, 'not_found', 'unknown route')
            try:
                with lock:
                    status, body, kind = self.dispatch(action, match, url)
                self.reply(status, body, kind)
            except QueryError as error:
                self.error(404 if error.code == 'not_found' else 400, error.code, error.message)

        def dispatch(self, action, match, url):
            pairs = urllib.parse.parse_qs(url.query, keep_blank_values=True, strict_parsing=False)
            if set(pairs) - ALLOWED[action] or any(len(v) != 1 for v in pairs.values()):
                raise QueryError('invalid_argument', 'unsupported or repeated query parameter')
            args = {}
            for key, [value] in pairs.items():
                if key in INT_PARAMS:
                    if not re.fullmatch(r'-?\d{1,16}', value):
                        raise QueryError('invalid_argument', f'{key} must be an integer')
                    value = int(value)
                args[key] = value
            if action in ID_ARG:
                args[ID_ARG[action]] = match.group(1)
            if action == 'contacts':
                result = query.get_contact(name=args['name']) if 'name' in args else query.list_contacts(**args)
            elif action == 'get_media':
                info, path = query.get_media(**args)
                return 200, path.read_bytes(), mimetypes.guess_type(path.name)[0] or 'application/octet-stream'
            else:
                result = getattr(query, action)(**args)
            return 200, result, 'application/json; charset=utf-8'

    class Server(ThreadingHTTPServer):
        request_queue_size = 128  # default 5 resets bursts of concurrent agent requests
        daemon_threads = True

    return Server((bind, port), Handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', default=str(default_config()))
    query, config = open_query(parser.parse_args().config)
    token = ensure_token(query.path.parent)
    server = make_server(query, token, config['history_http']['bind'], config['history_http']['port'])
    print(json.dumps({'event': 'history_http_started', 'bind': config['history_http']['bind'],
                      'port': server.server_address[1]}), flush=True)
    server.serve_forever()


if __name__ == '__main__':
    main()
