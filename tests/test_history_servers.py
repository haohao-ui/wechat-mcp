import http.client
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.parse

from scripts.wechat_bot.history.http_server import make_server, ensure_token
from scripts.wechat_bot.history.query import HistoryQuery
from scripts.wechat_bot.history.store import HistoryStore
from scripts.wechat_bot.service import load_config

ROOT = Path(__file__).resolve().parents[1]


def bub(text, d='inbound_visual'):
    return {'text': text, 'digest': text, 'direction': d}


class ServerBase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        base = Path(self.temp.name)
        self.state = base/'state'
        self.path = base/'config.json'
        self.path.write_text(json.dumps({'schema_version': 1, 'host': 'fixture', 'account_id': 'acct',
                                         'allowlist': [{'recipient': '🍓小美', 'kind': 'private'}],
                                         'state_dir': str(self.state)}))
        store = HistoryStore.open(self.state)
        self.c = store.ensure_contact('acct', {'recipient': '🍓小美', 'kind': 'private'})
        store.record_frame(self.c, {'bubbles': [bub('还是上次那个款'), bub('已发顺丰', 'outbound_visual'), bub('最后一条')]})
        store.conn.close()

    def tearDown(self):
        self.temp.cleanup()


class McpTests(ServerBase):
    def rpc(self, requests):
        proc = subprocess.run([sys.executable, '-m', 'scripts.wechat_bot.history.mcp_server', '--config', str(self.path)],
                              input=''.join(json.dumps(r, ensure_ascii=False)+'\n' for r in requests),
                              capture_output=True, text=True, cwd=ROOT, timeout=30)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return [json.loads(line) for line in proc.stdout.splitlines()]

    def test_mcp_lists_tools_and_returns_history(self):
        out = self.rpc([{'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {}},
                        {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
                        {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
                        {'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call',
                         'params': {'name': 'history_get_recent_messages', 'arguments': {'contact_id': self.c}}}])
        self.assertEqual(len(out), 3)
        self.assertEqual(out[0]['result']['serverInfo']['name'], 'wechat-history')
        self.assertEqual(out[0]['result']['protocolVersion'], '2024-11-05')
        names = {t['name'] for t in out[1]['result']['tools']}
        self.assertEqual(names, {'history_list_contacts', 'history_get_contact', 'history_get_recent_messages',
                                 'history_get_history', 'history_search_messages', 'history_get_message', 'history_get_media'})
        self.assertEqual(json.loads(out[2]['result']['content'][0]['text'])['items'][-1]['text'], '最后一条')

    def test_mcp_errors(self):
        out = self.rpc([{'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                         'params': {'name': 'history_get_message', 'arguments': {'message_id': 'm_missing'}}},
                        {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
                         'params': {'name': 'history_get_history', 'arguments': {'contact_id': self.c, 'bogus': 1}}},
                        {'jsonrpc': '2.0', 'id': 3, 'method': 'nope'}])
        self.assertTrue(out[0]['result']['isError'])
        self.assertTrue(out[1]['result']['isError'])
        self.assertEqual(out[2]['error']['code'], -32601)


class HttpTests(ServerBase):
    def setUp(self):
        super().setUp()
        self.token = ensure_token(self.state/'history')
        self.server = make_server(HistoryQuery(self.state/'history'/'history.sqlite3'), self.token, '127.0.0.1', 0)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        super().tearDown()

    def request(self, method, path, token=True):
        conn = http.client.HTTPConnection('127.0.0.1', self.server.server_address[1], timeout=10)
        headers = {'Authorization': 'Bearer '+(self.token if token is True else token)} if token else {}
        conn.request(method, path, headers=headers)
        response = conn.getresponse()
        response.body = response.read()
        conn.close()
        return response

    def get(self, path, token=True):
        return self.request('GET', path, token)

    def test_http_requires_token_and_only_get(self):
        self.assertEqual(self.get('/v1/contacts', token=None).status, 401)
        self.assertEqual(self.get('/v1/contacts', token='wrong').status, 401)
        self.assertEqual(self.request('POST', '/v1/contacts').status, 405)
        self.assertEqual(self.request('DELETE', '/v1/contacts').status, 405)

    def test_http_history_and_search(self):
        r = self.get(f'/v1/contacts/{self.c}/messages?limit=2')
        self.assertEqual(r.status, 200)
        body = json.loads(r.body)
        self.assertEqual(len(body['items']), 2)
        self.assertIsNotNone(body['next_cursor'])
        r = self.get('/v1/search?q='+urllib.parse.quote('上次那个'))
        self.assertEqual([m['text'] for m in json.loads(r.body)['items']], ['还是上次那个款'])
        r = self.get(f'/v1/contacts/{self.c}/recent?limit=1')
        self.assertEqual(json.loads(r.body)['items'][0]['text'], '最后一条')

    def test_http_errors(self):
        self.assertEqual(self.get('/v1/messages/m_missing').status, 404)
        self.assertEqual(self.get(f'/v1/contacts/{self.c}/messages?limit=999').status, 400)
        self.assertEqual(self.get(f'/v1/contacts/{self.c}/messages?limit=abc').status, 400)
        self.assertEqual(self.get('/v1/unknown').status, 404)

    def test_http_emoji_query_and_traversal_are_safe(self):
        r = self.get('/v1/contacts?query='+urllib.parse.quote('🍓小美/../'))
        self.assertEqual(r.status, 200)
        self.assertEqual(self.get('/v1/media/..%2F..%2Fhistory.sqlite3').status, 404)
        r = self.get('/v1/contacts?name='+urllib.parse.quote('小美'))
        self.assertEqual(json.loads(r.body)['contact_id'], self.c)

    def test_http_unexpected_error_is_json_500(self):
        from unittest.mock import patch
        with patch.object(HistoryQuery, 'list_contacts', side_effect=RuntimeError('boom /secret/path')):
            r = self.get('/v1/contacts')
        self.assertEqual(r.status, 500)
        body = json.loads(r.body)
        self.assertEqual(body['error']['code'], 'internal')
        self.assertNotIn('secret', r.body.decode())

    def test_http_default_bind_is_loopback(self):
        self.assertEqual(load_config(self.path)['history_http']['bind'], '127.0.0.1')

    def test_token_file_private_and_stable(self):
        path = self.state/'history'/'http-token'
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(ensure_token(self.state/'history'), self.token)


class ConcurrencyAndControlTests(HttpTests):
    def test_parallel_requests_succeed(self):
        results = []
        def worker():
            results.append(self.get(f'/v1/contacts/{self.c}/messages?limit=3').status)
        threads = [threading.Thread(target=worker) for _ in range(16)]
        for t in threads: t.start()
        for t in threads: t.join()
        self.assertEqual(results, [200]*16)

    def test_history_http_launchd_plist(self):
        from scripts.wechat_bot import control
        plist = control.history_http_plist(self.path, Path(self.temp.name)/'logs')
        self.assertEqual(plist['Label'], 'local.wechat-rpa.history')
        self.assertEqual(plist['ProgramArguments'][-3:], ['scripts.wechat_bot.history.http_server', '--config', str(self.path)])
        self.assertTrue(plist['KeepAlive'])
