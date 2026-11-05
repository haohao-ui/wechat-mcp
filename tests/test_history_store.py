from pathlib import Path
import tempfile
import unittest

from scripts.wechat_bot.history.store import HistoryStore, NameConflict


def bub(text, d='inbound_visual', kind='text'):
    return {'text': text, 'digest': text, 'direction': d, 'kind': kind}


def fr(*b):
    return {'bubbles': list(b)}


class StoreBase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.tmp = Path(self.temp.name)
        self.state = self.tmp/'state'
        self.store = HistoryStore.open(self.state)
        self.c = self.store.ensure_contact('acct', {'recipient': '客户', 'kind': 'private'})

    def tearDown(self):
        self.store.conn.close()
        self.temp.cleanup()

    def rows(self, column):
        return [r[0] for r in self.store.conn.execute(
            f'select {column} from messages where contact_id=? order by seq', (self.c,))]

    def texts(self): return self.rows('text')
    def captures(self): return self.rows('capture')
    def senders(self): return self.rows('sender')


class FingerprintV2Tests(StoreBase):
    def test_legacy_history_cursor_resets_without_duplicates(self):
        import json
        frame = fr(bub('a'), bub('b'), bub('c'))
        self.store.conn.execute('INSERT INTO contact_cursor VALUES(?,?,?)',
                                (self.c, json.dumps([['inbound_visual', 'a'], ['inbound_visual', 'b']]), 0))
        self.assertEqual(self.store.record_frame(self.c, frame), [])
        self.assertEqual(self.texts(), [])
        stored = json.loads(self.store.conn.execute('SELECT frame_fingerprints FROM contact_cursor').fetchone()[0])
        self.assertEqual(len(stored[0]), 3)

    def test_same_size_older_image_is_not_matched_to_a_new_one(self):
        img = lambda h: {'text': '[图片]', 'digest': 'image:50:40', 'direction': 'inbound_visual',
                         'kind': 'image', 'phash': h}
        self.store.record_frame(self.c, fr(bub('a')))
        old = self.store.record_frame(self.c, fr(bub('a'), img('0000000000000000')))[0]['message_id']
        self.assertEqual(self.store.match_recent_inbound(self.c, [img('ffffffffffffffff')]), [])
        self.assertEqual(self.store.match_recent_inbound(self.c, [img('000000000000000f')]), [old])

    def test_reconciled_image_is_not_recorded_twice(self):
        img = lambda h: {'text': '[图片]', 'digest': 'image:50:40', 'direction': 'inbound_visual',
                         'kind': 'image', 'phash': h, 'media_id': h*4}
        self.store.record_frame(self.c, fr(bub('a')))
        self.store.record_frame(self.c, fr(bub('a'), img('0f0f0f0f0f0f0f0f')))
        self.store.record_frame(self.c, fr(bub('a'), img('0f0f0f0f0f0f0f00')))
        self.assertEqual(self.texts(), ['a', '[图片]'])


class UnderstandingTests(StoreBase):
    def test_migration_adds_understanding_columns(self):
        columns = {r[1] for r in self.store.conn.execute('PRAGMA table_info(media)')}
        self.assertTrue({'understanding', 'understood_at'} <= columns)

    def test_recent_context_includes_media_understanding(self):
        self.store.record_frame(self.c, fr(bub('在吗')))
        img = {'text': '[图片]', 'digest': 'image:1:1', 'phash': '0'*16, 'direction': 'inbound_visual', 'kind': 'image'}
        rows = self.store.record_frame(self.c, fr(bub('在吗'), img))
        source = self.tmp/'a.png'; source.write_bytes(b'\x89PNG\r\n\x1a\nx')
        media_id = self.store.attach_media(rows[0]['message_id'], source, kind='image', filename=None, source='original')
        context = self.store.recent_context(self.c, 5)
        self.assertEqual(context[-1]['media']['understanding'], None)
        self.assertTrue(Path(context[-1]['media']['path']).is_file())
        self.store.set_understanding(media_id, '一张快递单'*300)
        context = self.store.recent_context(self.c, 5)
        self.assertEqual(len(context[-1]['media']['understanding']), 1000)
        self.assertNotIn('media', context[0])


class ContactTests(StoreBase):
    def test_schema_created_private_and_wal(self):
        db = self.state/'history'/'history.sqlite3'
        self.assertEqual(db.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.state/'history').stat().st_mode & 0o777, 0o700)
        self.assertEqual(self.store.conn.execute('PRAGMA journal_mode').fetchone()[0], 'wal')
        tables = {r[0] for r in self.store.conn.execute("select name from sqlite_master")}
        self.assertTrue({'contacts', 'contact_names', 'messages', 'sends', 'media', 'media_jobs',
                         'contact_cursor', 'merges', 'schema_migrations', 'messages_fts'} <= tables)

    def test_reopen_keeps_data(self):
        self.store.conn.close()
        self.store = HistoryStore.open(self.state)
        self.assertEqual(self.store.resolve('acct', '客户'), self.c)

    def test_ensure_contact_is_stable_and_alias_aware(self):
        a = self.store.ensure_contact('acct', {'recipient': '🍓小美', 'kind': 'private', 'aliases': ['@小美']})
        self.assertTrue(a.startswith('c_') and len(a) == 18)
        self.assertEqual(self.store.ensure_contact('acct', {'recipient': '🍓小美', 'kind': 'private'}), a)
        self.assertEqual(self.store.resolve('acct', '小美'), a)
        self.assertEqual(self.store.resolve('acct', '@小美'), a)
        self.assertIsNone(self.store.resolve('other-acct', '小美'))

    def test_rename_keeps_contact_when_old_name_is_alias(self):
        a = self.store.ensure_contact('acct', {'recipient': '旧名', 'kind': 'private'})
        self.assertEqual(self.store.ensure_contact('acct', {'recipient': '新名', 'kind': 'private', 'aliases': ['旧名']}), a)
        self.assertEqual(self.store.resolve('acct', '新名'), a)
        name = self.store.conn.execute('select display_name from contacts where contact_id=?', (a,)).fetchone()[0]
        self.assertEqual(name, '新名')

    def test_merge_follows_and_unmerge_restores(self):
        a = self.store.ensure_contact('acct', {'recipient': '甲', 'kind': 'private'})
        b = self.store.ensure_contact('acct', {'recipient': '乙', 'kind': 'private'})
        mid = self.store.merge(b, a, '改名')
        self.assertEqual(self.store.resolve('acct', '乙'), a)
        self.assertEqual(self.store.canonical(b), a)
        self.store.unmerge(mid)
        self.assertEqual(self.store.resolve('acct', '乙'), b)

    def test_conflicting_name_key_rejected(self):
        self.store.ensure_contact('acct', {'recipient': '小美', 'kind': 'private'})
        self.store.ensure_contact('acct', {'recipient': '别人', 'kind': 'private'})
        with self.assertRaises(NameConflict):
            self.store.ensure_contact('acct', {'recipient': '别人', 'kind': 'private', 'aliases': ['🍓小美']})

    def test_observe_name_reports_known(self):
        self.assertTrue(self.store.observe_name(self.c, '客户'))
        self.assertFalse(self.store.observe_name(self.c, '客户✨新'))
        source = self.store.conn.execute("select source from contact_names where raw_name='客户✨新'").fetchone()[0]
        self.assertEqual(source, 'observed')


class MessageTests(StoreBase):
    def test_first_frame_is_baseline_then_live_increment(self):
        self.store.record_frame(self.c, fr(bub('a'), bub('b', 'outbound_visual')))
        rows = self.store.record_frame(self.c, fr(bub('a'), bub('b', 'outbound_visual'), bub('c')))
        self.assertEqual([(r['text'], r['capture'], r['sender']) for r in rows], [('c', 'live', 'customer')])
        self.assertEqual(self.captures(), ['baseline', 'baseline', 'live'])
        self.assertEqual(self.senders(), ['customer', 'self_other', 'customer'])

    def test_identical_repeats_are_two_messages(self):
        self.store.record_frame(self.c, fr(bub('x')))
        self.store.record_frame(self.c, fr(bub('x'), bub('好'), bub('好')))
        self.assertEqual(self.texts(), ['x', '好', '好'])

    def test_replaying_same_frame_after_crash_writes_nothing(self):
        self.store.record_frame(self.c, fr(bub('x')))
        f = fr(bub('x'), bub('y'))
        self.store.record_frame(self.c, f)
        self.assertEqual(self.store.record_frame(self.c, f), [])
        self.assertEqual(self.texts(), ['x', 'y'])

    def test_ambiguous_and_gap_are_kept_and_marked(self):
        self.store.record_frame(self.c, fr(bub('A')))
        for _ in range(2): self.store.record_frame(self.c, fr(bub('A'), bub('A')))
        self.assertEqual(self.captures()[-1], 'ambiguous')
        for _ in range(2): self.store.record_frame(self.c, fr(bub('完全不同')))
        self.assertEqual(self.captures()[-1], 'rebaseline_gap')
        self.assertEqual(self.texts(), ['A', 'A', '完全不同'])

    def test_single_jittery_gap_frame_writes_nothing(self):
        self.store.record_frame(self.c, fr(bub('x'), bub('y')))
        self.assertEqual(self.store.record_frame(self.c, fr(bub('滚动中'))), [])
        self.store.record_frame(self.c, fr(bub('x'), bub('y'), bub('z')))
        self.assertEqual(self.texts(), ['x', 'y', 'z'])
        self.assertEqual(self.captures(), ['baseline', 'baseline', 'live'])

    def test_ambiguous_certain_tail_is_live(self):
        self.store.record_frame(self.c, fr(bub('A'), bub('A')))
        for _ in range(2): self.store.record_frame(self.c, fr(bub('A'), bub('A'), bub('next')))
        self.assertEqual(self.captures()[-1], 'live')
        self.assertEqual(self.texts()[-1], 'next')

    def test_bot_reply_echo_is_paired_not_duplicated(self):
        self.store.record_frame(self.c, fr(bub('问')))
        self.store.record_bot_reply(self.c, 'bot-service-k', '你好～', 'attempting', {}, [])
        self.store.update_send('bot-service-k', 'triggered')
        self.store.record_frame(self.c, fr(bub('问'), bub('你好~', 'outbound_visual')))
        self.assertEqual(self.senders(), ['customer', 'bot'])
        self.assertEqual(self.store.send('bot-service-k')['echo_observed'], 1)

    def test_bot_reply_is_idempotent(self):
        a = self.store.record_bot_reply(self.c, 'k', '回复', 'attempting', {}, [])
        self.assertEqual(self.store.record_bot_reply(self.c, 'k', '回复', 'attempting', {}, []), a)
        self.assertEqual(self.senders(), ['bot'])

    def test_ocr_noisy_echo_still_pairs(self):
        self.store.record_frame(self.c, fr(bub('在吗')))
        self.store.record_bot_reply(self.c, 'k', '在的，我是AI客服助手。有什么可以帮您？我这边暂时没有订单信息。', 'attempting', {}, [])
        self.store.update_send('k', 'triggered')
        self.store.record_frame(self.c, fr(bub('在吗'), bub('在的，我是 Al客服助手。有什么可以帮您？我这边暂\n时没有订单信息', 'outbound_visual')))
        self.assertEqual(self.senders(), ['customer', 'bot'])
        self.assertEqual(self.store.send('k')['echo_observed'], 1)

    def test_different_outbound_after_bot_reply_is_self_other(self):
        self.store.record_frame(self.c, fr(bub('在吗')))
        self.store.record_bot_reply(self.c, 'k', '在的，我是AI客服助手。', 'attempting', {}, [])
        self.store.update_send('k', 'triggered')
        self.store.record_frame(self.c, fr(bub('在吗'), bub('人工：订单已经发顺丰了', 'outbound_visual')))
        self.assertEqual(self.senders(), ['customer', 'bot', 'self_other'])

    def test_replied_after_sees_manual_reply(self):
        rows = self.store.record_frame(self.c, fr(bub('在吗')))
        self.store.record_frame(self.c, fr(bub('在吗'), bub('图片呢')))
        ids = [m for m, in self.store.conn.execute("select message_id from messages where text='图片呢'")]
        self.assertFalse(self.store.replied_after(self.c, ids))
        self.store.record_frame(self.c, fr(bub('在吗'), bub('图片呢'), bub('人工：收到', 'outbound_visual')))
        self.assertTrue(self.store.replied_after(self.c, ids))
        self.assertFalse(self.store.replied_after(self.c, []))

    def test_replied_after_ignores_failed_bot_send(self):
        self.store.record_frame(self.c, fr(bub('在吗')))
        self.store.record_frame(self.c, fr(bub('在吗'), bub('图片呢')))
        ids = [m for m, in self.store.conn.execute("select message_id from messages where text='图片呢'")]
        self.store.record_bot_reply(self.c, 'k', '在的', 'generated', {}, ids)
        self.assertFalse(self.store.replied_after(self.c, ids))
        self.store.update_send('k', 'triggered')
        self.assertTrue(self.store.replied_after(self.c, ids))

    def test_old_or_imported_reply_is_not_echo_target(self):
        self.store.record_frame(self.c, fr(bub('问')))
        self.store.record_bot_reply(self.c, 'old', '好的，稍等', 'triggered', {}, [])
        self.store.conn.execute('update sends set created_at=created_at-11*60*1000 where send_key=?', ('old',))
        self.store.record_bot_reply(self.c, 'imp', '已发货', 'triggered', {}, [], capture='imported')
        self.store.record_frame(self.c, fr(bub('问'), bub('好的，稍等', 'outbound_visual'), bub('已发货', 'outbound_visual')))
        self.assertEqual(self.senders(), ['customer', 'bot', 'bot', 'self_other', 'self_other'])

    def test_unpaired_outbound_is_self_other(self):
        self.store.record_frame(self.c, fr(bub('问')))
        self.store.record_frame(self.c, fr(bub('问'), bub('人工回复', 'outbound_visual')))
        self.assertEqual(self.senders()[-1], 'self_other')

    def test_superseded_reply_is_not_echo_target(self):
        self.store.record_frame(self.c, fr(bub('问')))
        self.store.record_bot_reply(self.c, 'k', '好的', 'generated', {}, [])
        self.store.update_send('k', 'superseded')
        self.store.record_frame(self.c, fr(bub('问'), bub('好的', 'outbound_visual')))
        self.assertEqual(self.senders(), ['customer', 'bot', 'self_other'])

    def test_recent_context_limit_order_and_exclusions(self):
        self.store.record_frame(self.c, fr(*[bub(str(i)) for i in range(8)]))
        self.store.record_bot_reply(self.c, 'k1', '作废', 'generated', {}, [])
        self.store.update_send('k1', 'superseded')
        ctx = self.store.recent_context(self.c, 5)
        self.assertEqual([x['text'] for x in ctx], ['3', '4', '5', '6', '7'])
        self.assertTrue(all(x['role'] == 'user' for x in ctx))

    def test_recent_context_roles_exclude_ids_and_merged(self):
        other = self.store.ensure_contact('acct', {'recipient': '旧号', 'kind': 'private'})
        self.store.record_frame(other, fr(bub('旧问题')))
        self.store.merge(other, self.c)
        rows = self.store.record_frame(self.c, fr(bub('问'), bub('答', 'outbound_visual'), bub('新')))
        ctx = self.store.recent_context(self.c, 10, exclude_ids=frozenset({rows[-1]['message_id']}))
        self.assertEqual([(x['role'], x['text']) for x in ctx], [('user', '旧问题'), ('user', '问'), ('assistant', '答')])

    def test_recent_context_excludes_never_attempted_reply(self):
        self.store.record_frame(self.c, fr(bub('问')))
        self.store.record_bot_reply(self.c, 'g', '未发出', 'generated', {}, [])
        self.store.record_bot_reply(self.c, 'u', '可能已发出', 'unknown', {}, [])
        self.assertEqual([x['text'] for x in self.store.recent_context(self.c, 5)], ['问', '可能已发出'])

    def test_contact_time_index_exists(self):
        names = {r[1] for r in self.store.conn.execute("PRAGMA index_list('messages')")}
        self.assertIn('messages_contact_time', names)

    def test_fts_trigger_indexes_text(self):
        self.store.record_frame(self.c, fr(bub('还是上次那个款')))
        hit = self.store.conn.execute("select count(*) from messages_fts where messages_fts match '上次那'").fetchone()[0]
        self.assertEqual(hit, 1)


class MediaTests(StoreBase):
    def test_attach_media_content_addressed_and_private(self):
        [row] = self.store.record_frame(self.c, fr(bub('[图片]', kind='image')))
        src = self.tmp/'x.png'
        src.write_bytes(b'png')
        mid = self.store.attach_media(row['message_id'], src, kind='image', filename=None, source='chat_thumbnail')
        path = self.state/'history'/'media'/mid[:2]/(mid+'.png')
        self.assertEqual(path.read_bytes(), b'png')
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        linked = self.store.conn.execute('select media_id from messages where message_id=?', (row['message_id'],)).fetchone()[0]
        self.assertEqual(linked, mid)

    def test_oversized_media_rejected(self):
        [row] = self.store.record_frame(self.c, fr(bub('big.pdf', kind='file')))
        src = self.tmp/'big.pdf'
        src.write_bytes(b'0'*(10*1024*1024+1))
        with self.assertRaises(ValueError):
            self.store.attach_media(row['message_id'], src, kind='file', filename='big.pdf', source='original')

    def test_file_messages_queue_download_with_backoff(self):
        [row] = self.store.record_frame(self.c, fr(bub('报价.pdf', kind='file')))
        job = self.store.next_media_job(self.c)
        self.assertEqual(job['message_id'], row['message_id'])
        self.store.finish_media_job(row['message_id'], 'failed', 'timeout')
        self.assertIsNone(self.store.next_media_job(self.c))
        self.assertTrue(self.store.has_pending_media())

    def test_image_messages_are_not_queued(self):
        self.store.record_frame(self.c, fr(bub('[图片]', kind='image')))
        self.assertFalse(self.store.has_pending_media())

    def test_failed_three_times_is_terminal_and_gone_is_terminal(self):
        [row] = self.store.record_frame(self.c, fr(bub('a.pdf', kind='file')))
        for _ in range(3):
            self.store.finish_media_job(row['message_id'], 'failed', 'x')
        self.assertFalse(self.store.has_pending_media())
        [other] = self.store.record_frame(self.c, fr(bub('a.pdf', kind='file'), bub('b.pdf', kind='file')))
        self.store.finish_media_job(other['message_id'], 'gone')
        self.assertFalse(self.store.has_pending_media())
