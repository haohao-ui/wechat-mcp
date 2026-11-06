import asyncio
import base64
import hashlib
import json
from pathlib import Path
import tempfile
from unittest.mock import AsyncMock, patch
import unittest

ROOT = Path(__file__).resolve().parents[1]
from scripts.wechat_bot.service import load_config
from scripts.wechat_bot.native_capture import NativeCapture
from scripts.wechat_bot.documents import extract, MAX_BYTES
from scripts.wechat_bot.history.store import HistoryStore, connect
from scripts.wechat_bot import reply
from scripts.agent_bridge.codex import private_directory


def source(path):
    return (ROOT / path).read_text()


class AttachmentLimitConfigTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / 'config.json'
        self.base = {
            'schema_version': 1,
            'host': 'testhost',
            'account_id': 'testacc',
            'state_dir': self.temp.name,
            'allowlist': [{'recipient': 'testuser', 'kind': 'private'}],
            'poll_seconds': 2,
        }

    def tearDown(self):
        self.temp.cleanup()

    def test_load_config_default_max_attachment_mb(self):
        self.path.write_text(json.dumps(self.base))
        cfg = load_config(self.path)
        self.assertEqual(cfg['max_attachment_mb'], 50)

    def test_load_config_accepts_valid_range(self):
        self.path.write_text(json.dumps({**self.base, 'max_attachment_mb': 120}))
        cfg = load_config(self.path)
        self.assertEqual(cfg['max_attachment_mb'], 120)

    def test_load_config_rejects_out_of_range_or_wrong_type(self):
        for bad in (0, 201, "50", 50.0):
            self.path.write_text(json.dumps({**self.base, 'max_attachment_mb': bad}))
            with self.assertRaises(ValueError):
                load_config(self.path)


class NativeCaptureLimitTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = private_directory(Path(self.temp.name) / 'private')
        self.capture = NativeCapture('host', None, 'user', self.root, '0' * 64, max_bytes=50 * 1024 * 1024)

    def tearDown(self):
        self.temp.cleanup()

    async def test_download_accepts_30mb_and_rejects_above_limit(self):
        size_30mb = 30 * 1024 * 1024
        # Fake a 30MB payload in chunks
        chunk = b'x' * 65536
        num_chunks = size_30mb // len(chunk)

        async def fake_call_30mb(op, **params):
            self.assertEqual(op, 'read_media')
            self.assertEqual(params['max_bytes'], 50 * 1024 * 1024)
            offset = params['offset']
            eof = (offset + len(chunk) >= size_30mb)
            return {
                'offset': offset,
                'size': size_30mb,
                'bytes': base64.b64encode(chunk).decode(),
                'eof': eof,
                'sha256': 'dummy',
                'extension': 'txt',
            }

        # Mock sha256 check
        self.capture.call = fake_call_30mb
        with patch('hashlib.sha256') as mock_sha:
            mock_sha.return_value.hexdigest.return_value = 'dummy'
            path = await self.capture.download('a' * 64, kind='file')
            self.assertTrue(path.exists())
            self.assertEqual(path.stat().st_size, size_30mb)

        # Above configured limit
        async def fake_call_oversized(op, **params):
            return {
                'offset': 0,
                'size': 50 * 1024 * 1024 + 1,
                'bytes': base64.b64encode(b'x').decode(),
                'eof': True,
                'sha256': 'dummy',
                'extension': 'txt',
            }

        self.capture.call = fake_call_oversized
        with self.assertRaisesRegex(RuntimeError, 'invalid media size/offset'):
            await self.capture.download('b' * 64, kind='file')

    async def test_prepare_and_read_calls_carry_configured_max_bytes(self):
        recorded = []

        async def fake_call(op, **params):
            recorded.append((op, params))
            if op == 'prepare_file':
                return {'filename': 'test.txt', 'extension': 'txt'}
            if op == 'prepare_image':
                return {'media_id': params['media_id'], 'extension': 'png'}
            if op == 'read_media':
                return {
                    'offset': 0,
                    'size': 4,
                    'bytes': base64.b64encode(b'test').decode(),
                    'eof': True,
                    'sha256': hashlib.sha256(b'test').hexdigest(),
                    'extension': 'txt',
                }
            return {}

        self.capture.call = fake_call
        self.capture.frame = AsyncMock(return_value={'bubbles': [], 'snapshot_id': 'snap1'})

        # Test attachments flow
        items = [
            {'kind': 'image', 'media_id': '1' * 64},
            {'kind': 'file', 'media_id': '2' * 64, 'filename': 'test.txt'},
        ]
        await self.capture.attachments(items, {'snapshot_id': 'snap0'})

        ops = {op: params for op, params in recorded}
        self.assertIn('prepare_image', ops)
        self.assertEqual(ops['prepare_image']['max_bytes'], 50 * 1024 * 1024)
        self.assertIn('prepare_file', ops)
        self.assertEqual(ops['prepare_file']['max_bytes'], 50 * 1024 * 1024)
        self.assertIn('read_media', ops)
        self.assertEqual(ops['read_media']['max_bytes'], 50 * 1024 * 1024)


class DocumentsAndStoreLimitTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_documents_extract_follows_max_bytes_parameter(self):
        self.assertEqual(MAX_BYTES, 50 * 1024 * 1024)
        p = self.root / 'sample.txt'
        p.write_bytes(b'hello world')
        # Default allows
        self.assertEqual(extract(p, 'txt')['text'], 'hello world')
        # Lower limit rejects
        with self.assertRaisesRegex(ValueError, 'file size limit'):
            extract(p, 'txt', max_bytes=5)
        # Higher limit accepts
        self.assertEqual(extract(p, 'txt', max_bytes=100)['text'], 'hello world')

    def test_store_attach_media_follows_configurable_limit_and_names_it(self):
        db_path = self.root / 'test.sqlite3'
        conn = connect(db_path)
        store = HistoryStore(self.root, conn)
        store.migrate()
        conn.execute("INSERT INTO contacts(contact_id, account_id, kind, display_name, created_at, updated_at) VALUES('c1', 'acc1', 'private', 'u', 0, 0)")
        conn.execute("INSERT INTO messages(message_id, contact_id, account_id, seq, direction, sender, kind, text, observed_at, capture) VALUES('m1', 'c1', 'acc1', 1, 'inbound', 'customer', 'text', 'hi', 0, 'live')")

        src = self.root / 'file.bin'
        src.write_bytes(b'x' * 200)

        # Configured via max_bytes argument
        with self.assertRaisesRegex(ValueError, 'media larger than 100 bytes'):
            store.attach_media('m1', src, kind='file', filename='file.bin', source='original', max_bytes=100)

        with self.assertRaisesRegex(ValueError, 'media larger than 50MB'):
            src_big = self.root / 'big.bin'
            src_big.write_bytes(b'0' * (50 * 1024 * 1024 + 1))
            store.attach_media('m1', src_big, kind='file', filename='big.bin', source='original', max_bytes=50 * 1024 * 1024)


class ReplyHintTests(unittest.TestCase):
    def test_too_large_hint_mentions_configured_mb(self):
        seen = {}
        async def fake_api(prompt, **kwargs):
            seen['prompt'] = prompt
            return {'reply': '好的'}

        with patch('scripts.wechat_bot.model_api.generate_api', fake_api):
            asyncio.run(reply.generate('[文件]', mode='service',
                                       failed=[{'label': '大文件.pdf', 'kind': 'file', 'reason': 'too_large'}],
                                       model_config={'backend': 'openai-compatible'},
                                       max_attachment_mb=50))
        data = json.loads(seen['prompt'][seen['prompt'].index('\n{') + 1:])
        hint = data['unreadable'][0]['hint']
        self.assertIn('50MB', hint)
        self.assertNotIn('10MB', hint)

        # Configured as 120MB
        with patch('scripts.wechat_bot.model_api.generate_api', fake_api):
            asyncio.run(reply.generate('[文件]', mode='service',
                                       failed=[{'label': '超大文件.pdf', 'kind': 'file', 'reason': 'too_large'}],
                                       model_config={'backend': 'openai-compatible'},
                                       max_attachment_mb=120))
        data120 = json.loads(seen['prompt'][seen['prompt'].index('\n{') + 1:])
        hint120 = data120['unreadable'][0]['hint']
        self.assertIn('120MB', hint120)


class SwiftSourceChecks(unittest.TestCase):
    def test_swift_sources_use_max_bytes_and_no_received_10mb_constants(self):
        worker = source('native/bot/Worker.swift')
        files = source('native/bot/Files.swift')

        # Files.swift saveAttachment must check maxBytes and not hard-code 10*1024*1024
        self.assertNotIn('10*1024*1024', files)
        self.assertIn('maxBytes', files)

        # Worker.swift must handle max_bytes in prepare_file, prepare_image, read_media
        self.assertNotIn('10*1024*1024', worker)
        self.assertIn('max_bytes', worker)
        self.assertIn('invalid_max_bytes', worker)


if __name__ == '__main__':
    unittest.main()
