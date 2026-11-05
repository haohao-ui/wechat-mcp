import asyncio
import json
import os
from pathlib import Path
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import unittest
from unittest.mock import patch

from scripts.wechat_bot.model_api import generate_api, read_key, validate_config, NoRedirect, request_completion, resolve_key


class ModelApiTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.root=Path(self.temp.name)
        self.key=self.root/'key'; self.key.write_text('fixture-key'); self.key.chmod(0o600)
        self.config={'backend':'openai-compatible','base_url':'https://api.deepseek.com',
                     'model':'fixture-model','api_key_file':str(self.key)}
        self.result={'model':'fixture-actual','choices':[{'finish_reason':'stop','message':{'content':json.dumps({'reply':'你好，我是AI客服助手。'})}}]}

    def tearDown(self): self.temp.cleanup()

    async def test_dispatch_and_model_metadata(self):
        from scripts.wechat_bot.reply import generate
        meta={}
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=self.result) as request:
            value=await generate('你好',mode='service',model_config=self.config,metadata=meta)
        self.assertIn('AI',value)
        payload=request.call_args.args[1]
        self.assertEqual(payload['model'],'fixture-model')
        self.assertEqual(payload['messages'][0]['role'],'system')
        self.assertIn('{"reply":',payload['messages'][0]['content'])
        self.assertFalse(payload['stream'])
        self.assertNotIn('fixture-key',json.dumps(payload))
        self.assertEqual(meta['model'],'fixture-actual')
        self.assertNotIn('api_key_file',meta)

    async def test_earlier_images_are_not_counted_as_this_turn(self):
        # Acceptance 2026-11-05 (group): one image was sent, the reply said two — an earlier image in
        # recent_context was counted. The prompt must say only image_count images arrived this turn.
        from scripts.wechat_bot.reply import generate
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=self.result) as request:
            await generate('示例联系人甲：[图片]',mode='service',model_config=self.config,chat_kind='group',
                           context=[{'role':'customer','text':'[图片]','media':{'kind':'image','understanding':'一张截图'}}])
        prompt=json.dumps(request.call_args.args[1]['messages'],ensure_ascii=False)
        self.assertIn('recent_context里的图片和文件是之前发的',prompt)
        self.assertIn('本轮收到的图片数量以image_count为准',prompt)

    async def test_generate_api_custom_field(self):
        def check(value):
            if not isinstance(value,list): raise ValueError('notes must be a list')
            return value
        notes={**self.result,'choices':[{'finish_reason':'stop','message':{'content':json.dumps({'notes':['一张收据']})}}]}
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=notes) as request:
            value=await generate_api('看图',images=[],config=self.config,field='notes',check=check)
        self.assertEqual(value,['一张收据'])
        self.assertIn('{"notes":',request.call_args.args[1]['messages'][0]['content'])
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=self.result):
            with self.assertRaises(ValueError):
                await generate_api('看图',images=[],config=self.config,field='notes',check=check)

    async def test_malformed_output_is_a_model_output_error_with_shape_only(self):
        from scripts.wechat_bot.model_api import ModelOutputError
        bad={**self.result,'choices':[{'finish_reason':'stop','message':{'content':'好的，这是回复'}}]}
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=bad):
            with self.assertRaises(ModelOutputError) as caught:
                await generate_api('hi',images=[],config=self.config)
        self.assertIn('length=7',str(caught.exception))
        self.assertNotIn('好的',str(caught.exception))  # never the body

    async def test_actual_image_bytes_and_capability_gate(self):
        path=self.root/'image.png'; path.write_bytes(b'\x89PNG\r\n\x1a\nfixture')
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=self.result) as request:
            with self.assertRaisesRegex(ValueError,'image support'):
                await generate_api('test',images=[path],config=self.config)
            request.assert_not_called()
            await generate_api('test',images=[path],config={**self.config,'supports_images':True})
            content=request.call_args.args[1]['messages'][-1]['content']
            self.assertTrue(content[1]['image_url']['url'].startswith('data:image/png;base64,'))

    async def test_incomplete_output_never_accepted(self):
        for reason in ('length','tool_calls',None):
            result={**self.result,'choices':[{'finish_reason':reason,'message':{'content':'{}'}}]}
            with patch('scripts.wechat_bot.model_api.request_completion',return_value=result):
                with self.assertRaises(ValueError):
                    await generate_api('test',images=[],config=self.config)

    async def test_invalid_reply_rejected(self):
        result={**self.result,'choices':[{'finish_reason':'stop','message':{'content':'{"reply":"ok","recipient":"other"}'}}]}
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=result):
            with self.assertRaises(ValueError): await generate_api('test',images=[],config=self.config)

    def test_key_file_ownership_mode_and_symlink(self):
        self.assertEqual(read_key(self.key),'fixture-key')
        self.key.chmod(0o644)
        with self.assertRaises(ValueError): read_key(self.key)
        self.key.chmod(0o600)
        link=self.root/'link'; link.symlink_to(self.key)
        with self.assertRaises(OSError): read_key(link)

    def test_config_transport_and_fields(self):
        validate_config(self.config)
        validate_config({**self.config,'base_url':'http://localhost:8000/v1'})
        for delta in ({'base_url':'http://api.deepseek.com'},{'base_url':'https://user:pass@example.com'},
                      {'api_key':'secret'},{'timeout_seconds':0},{'supports_images':'yes'}):
            with self.assertRaises(ValueError): validate_config({**self.config,**delta})

    def test_deepseek_defaults_to_environment_key(self):
        config={k:v for k,v in self.config.items() if k!='api_key_file'}
        validate_config(config)
        with patch.dict(os.environ,{'DEEPSEEK_API_KEY':'fixture-env-key'},clear=True):
            self.assertEqual(resolve_key(config),'fixture-env-key')

    def test_custom_environment_and_other_provider_default(self):
        config={k:v for k,v in self.config.items() if k!='api_key_file'}
        with patch.dict(os.environ,{'OTHER_MODEL_KEY':'fixture-custom','OPENAI_API_KEY':'fixture-openai'},clear=True):
            self.assertEqual(resolve_key({**config,'api_key_env':'OTHER_MODEL_KEY'}),'fixture-custom')
            self.assertEqual(resolve_key({**config,'base_url':'https://example.com/v1'}),'fixture-openai')

    def test_missing_or_invalid_environment_key_fails_without_exposing_value(self):
        config={k:v for k,v in self.config.items() if k!='api_key_file'}
        for env in ({},{'DEEPSEEK_API_KEY':'secret invalid key'}):
            with patch.dict(os.environ,env,clear=True):
                with self.assertRaises(ValueError) as error: resolve_key(config)
                self.assertNotIn('secret invalid key',str(error.exception))
        for name in ('', 'BAD-NAME', 'key=value', None):
            with self.assertRaises(ValueError): validate_config({**config,'api_key_env':name})

    def test_explicit_file_has_priority_over_environment(self):
        with patch.dict(os.environ,{'DEEPSEEK_API_KEY':'fixture-env-key'},clear=True):
            self.assertEqual(resolve_key(self.config),'fixture-key')

    def test_redirect_never_forwards_key(self):
        self.assertIsNone(NoRedirect().redirect_request(None,None,302,'',{},'https://other.example'))

    def test_http_transport_sends_auth_and_json(self):
        observed = {}
        result = self.result
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                observed['path'] = self.path
                observed['authorization'] = self.headers.get('Authorization')
                observed['body'] = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                body = json.dumps(result).encode()
                self.send_response(200)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers(); self.wfile.write(body)
            def log_message(self, *_): pass
        server = HTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.handle_request, daemon=True); thread.start()
        try:
            config = {**self.config, 'base_url': 'http://127.0.0.1:' + str(server.server_port) + '/v1'}
            response = request_completion(config, {'model':'fixture', 'messages':[]})
            self.assertEqual(response['model'], 'fixture-actual')
            self.assertEqual(observed['path'], '/v1/chat/completions')
            self.assertEqual(observed['authorization'], 'Bearer fixture-key')
            self.assertEqual(observed['body']['model'], 'fixture')
        finally:
            thread.join(timeout=2); server.server_close()

    def reply_result(self, content):
        return {**self.result,'choices':[{'finish_reason':'stop','message':{'content':content}}]}

    async def test_json_object_wrapped_in_text_is_extracted(self):
        with patch('scripts.wechat_bot.model_api.request_completion',return_value=self.reply_result('好的：{"reply":"收到两个表情～"} 以上')):
            self.assertEqual(await generate_api('test',images=[],config=self.config),'收到两个表情～')

    async def test_malformed_model_json_is_requested_again_once(self):
        bad=self.reply_result('{"reply":"图上是"可爱"的表情"}')
        with patch('scripts.wechat_bot.model_api.request_completion',side_effect=[bad,self.result]) as request:
            self.assertIn('AI',await generate_api('test',images=[],config=self.config))
        self.assertEqual(request.call_count,2)

    async def test_malformed_model_json_twice_fails(self):
        bad=self.reply_result('{"reply":"图上是"可爱"的表情"}')
        with patch('scripts.wechat_bot.model_api.request_completion',side_effect=[bad,bad]) as request:
            with self.assertRaises(ValueError): await generate_api('test',images=[],config=self.config)
        self.assertEqual(request.call_count,2)

    async def test_no_automatic_retry(self):
        with patch('scripts.wechat_bot.model_api.request_completion',side_effect=RuntimeError('HTTP 429')) as request:
            with self.assertRaises(RuntimeError): await generate_api('test',images=[],config=self.config)
            self.assertEqual(request.call_count,1)


if __name__=='__main__': unittest.main()
