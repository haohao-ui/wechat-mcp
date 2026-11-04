import contextlib
import fcntl
import hashlib
import io
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from scripts.wechat_bot.history.migrate import migrate
from scripts.wechat_bot.history.store import HistoryStore


def bub(text):
    return {'text': text, 'digest': text, 'direction': 'outbound_visual' if text.startswith('答') else 'inbound_visual'}


class MigrateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.base = Path(self.temp.name)
        self.state = self.base/'state'
        (self.state/'sessions').mkdir(parents=True, mode=0o700)
        self.state.chmod(0o700)
        self.config = {'schema_version': 1, 'host': 'fixture', 'account_id': 'acct', 'state_dir': str(self.state),
                       'allowlist': [{'recipient': '午夜咖啡', 'kind': 'private'}, {'recipient': 'a', 'kind': 'private'}]}
        self.path = self.base/'config.json'
        self.path.write_text(json.dumps(self.config))
        patcher = patch('scripts.wechat_bot.history.migrate.service_loaded', return_value=False)
        patcher.start()
        self.addCleanup(patcher.stop)

    def tearDown(self):
        self.temp.cleanup()

    def make_legacy_session(self, name, sends, cursor):
        target = {'recipient': name, 'kind': 'private'}
        digest = hashlib.sha256(json.dumps(target, sort_keys=True, ensure_ascii=False).encode()).hexdigest()
        directory = self.state/'sessions'/digest
        directory.mkdir(mode=0o700)
        (directory/'binding.json').write_text(json.dumps({'host': 'fixture', 'account_id': 'acct', **target}))
        (directory/'cursor.json').write_text(json.dumps({'bubbles': [bub(t) for t in cursor]}))
        for key, question, answer, created in sends:
            (directory/f'send-{key}.json').write_text(json.dumps(
                {'idempotency_key': key, 'status': 'triggered', 'input': [bub(question)], 'reply': answer,
                 'created_at': created, 'client_echo_observed': True}))
        return directory

    def store(self):
        return HistoryStore.open(self.state)

    def column(self, cid, column):
        with contextlib.closing(sqlite3.connect(self.state/'history'/'history.sqlite3')) as conn:
            return [r[0] for r in conn.execute(f'select {column} from messages where contact_id=? order by seq', (cid,))]

    def test_migrate_moves_dirs_imports_and_baselines(self):
        legacy = self.make_legacy_session('午夜咖啡', sends=[('k1', '问1', '答1', 1.0), ('k2', '问2', '答2', 2.0)],
                                          cursor=['问2', '答2', '问3'])
        stats = migrate(self.config)
        store = self.store()
        cid = store.resolve('acct', '午夜咖啡')
        self.assertFalse(legacy.exists())
        self.assertTrue((self.state/'sessions'/cid/'cursor.json').exists())
        self.assertEqual(json.loads((self.state/'sessions'/cid/'binding.json').read_text()),
                         {'host': 'fixture', 'account_id': 'acct', 'contact_id': cid, 'kind': 'private'})
        self.assertTrue((self.state/'sessions'/f'MOVED-{legacy.name}-TO-{cid}.txt').exists())
        # The baseline '答2' is the visible echo of imported reply k2, so it pairs instead of duplicating.
        self.assertEqual(self.column(cid, 'text'), ['问1', '答1', '问2', '答2', '问2', '问3'])
        self.assertEqual(self.column(cid, 'capture'), ['imported']*4+['baseline']*2)
        self.assertEqual(self.column(cid, 'sender'), ['customer', 'bot', 'customer', 'bot', 'customer', 'customer'])
        self.assertEqual(store.send('k2')['echo_observed'], 1)
        self.assertEqual(stats, {'contacts': 1, 'moved_dirs': 1, 'imported_sends': 2, 'baseline_messages': 2})
        store.conn.close()

    def test_world_readable_sessions_dir_is_tightened(self):
        (self.state/'sessions').chmod(0o755)
        self.make_legacy_session('a', sends=[], cursor=['q'])
        migrate(self.config)
        self.assertEqual((self.state/'sessions').stat().st_mode & 0o777, 0o700)

    def test_migrate_is_idempotent(self):
        self.make_legacy_session('a', sends=[('k1', 'q', '答', 1.0)], cursor=['q'])
        migrate(self.config)
        store = self.store()
        cid = store.resolve('acct', 'a')
        before = self.column(cid, 'text')
        store.conn.close()
        stats = migrate(self.config)
        self.assertEqual(self.column(cid, 'text'), before)
        self.assertEqual(stats['imported_sends'], 0)

    def test_root_level_legacy_session_is_imported_once(self):
        (self.state/'binding.json').write_text(json.dumps({'host': 'fixture', 'account_id': 'acct', 'recipient': 'a', 'kind': 'private'}))
        (self.state/'cursor.json').write_text(json.dumps({'bubbles': [bub('q')]}))
        (self.state/'send-k9.json').write_text(json.dumps({'idempotency_key': 'k9', 'status': 'attempting', 'input': [bub('q')], 'created_at': 1.0}))
        migrate(self.config)
        store = self.store()
        cid = store.resolve('acct', 'a')
        self.assertEqual(json.loads((self.state/'sessions'/cid/'send-k9.json').read_text())['status'], 'attempting')
        store.conn.close()
        migrate(self.config)
        self.assertEqual(len(list((self.state/'sessions').glob('c_*'))), 1)

    def test_root_legacy_skipped_when_session_dir_already_has_it(self):
        self.make_legacy_session('a', sends=[], cursor=['q'])
        (self.state/'binding.json').write_text(json.dumps({'host': 'fixture', 'account_id': 'acct', 'recipient': 'a', 'kind': 'private'}))
        (self.state/'cursor.json').write_text(json.dumps({'bubbles': [bub('old')]}))
        migrate(self.config)
        store = self.store()
        cid = store.resolve('acct', 'a')
        self.assertEqual(json.loads((self.state/'sessions'/cid/'cursor.json').read_text())['bubbles'][0]['text'], 'q')
        store.conn.close()

    def test_migrate_refuses_while_service_running(self):
        with open(self.state/'service.lock', 'a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            with self.assertRaisesRegex(RuntimeError, 'stop the bot'):
                migrate(self.config)

    def test_merge_unmerge_status_and_backup_commands(self):
        from scripts.wechat_bot import control
        store = self.store()
        a = store.ensure_contact('acct', {'recipient': '甲', 'kind': 'private'})
        b = store.ensure_contact('acct', {'recipient': '乙', 'kind': 'private'})
        store.conn.close()
        run = lambda *argv: control.main([*argv, '--config', str(self.path)])
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            run('merge-contact', '--from', '乙', '--into', a, '--note', '改名')
        merge_id = json.loads(out.getvalue())['merge_id']
        store = self.store()
        self.assertEqual(store.resolve('acct', '乙'), a)
        store.conn.close()
        with contextlib.redirect_stdout(io.StringIO()):
            run('unmerge-contact', '--merge-id', str(merge_id))
        store = self.store()
        self.assertEqual(store.resolve('acct', '乙'), b)
        store.conn.close()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            run('history-status')
        status = json.loads(out.getvalue())
        self.assertEqual(status['messages'], 0)
        self.assertEqual(len(status['contacts']), 2)
        backup = self.base/'backup.sqlite3'
        with contextlib.redirect_stdout(io.StringIO()):
            run('history-backup', '--to', str(backup))
        self.assertEqual(backup.stat().st_mode & 0o777, 0o600)
        with contextlib.closing(sqlite3.connect(backup)) as conn:
            self.assertEqual(conn.execute('select count(*) from contacts').fetchone()[0], 2)
        with self.assertRaises(SystemExit):
            with contextlib.redirect_stderr(io.StringIO()):
                run('history-backup', '--to', str(backup))
