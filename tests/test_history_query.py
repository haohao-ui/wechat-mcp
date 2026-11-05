from pathlib import Path
import sqlite3
import tempfile
import unittest

from scripts.wechat_bot.history.query import HistoryQuery, QueryError
from scripts.wechat_bot.history.store import HistoryStore


def bub(text, d='inbound_visual', kind='text'):
    return {'text': text, 'digest': text, 'direction': d, 'kind': kind}


def fr(*b):
    return {'bubbles': list(b)}


class QueryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.state = Path(self.temp.name)/'state'
        self.writer = HistoryStore.open(self.state)
        self.c = self.writer.ensure_contact('acct', {'recipient': '🍓小美', 'kind': 'private'})
        self.c2 = self.writer.ensure_contact('acct', {'recipient': '乙', 'kind': 'private'})
        self.visible = [bub('你好'), bub('还是上次那个款'), bub('已发顺丰', 'outbound_visual'), bub('收到')]
        self.writer.record_frame(self.c, fr(*self.visible))
        self.writer.record_bot_reply(self.c, 'k1', '好的，已为您登记', 'generated', {}, [])
        self.writer.update_send('k1', 'triggered')
        self.writer.record_frame(self.c, fr(*self.visible, bub('好的，已为您登记', 'outbound_visual')))
        self.writer.record_frame(self.c2, fr(bub('乙的消息')))
        self.expected_texts = ['你好', '还是上次那个款', '已发顺丰', '收到', '好的，已为您登记']
        self.q = HistoryQuery(self.state/'history'/'history.sqlite3')

    def tearDown(self):
        self.q.conn.close()
        self.writer.conn.close()
        self.temp.cleanup()

    def texts(self, page):
        return [m['text'] for m in page['items']]

    def test_read_only_connection_cannot_write(self):
        with self.assertRaises(sqlite3.OperationalError):
            self.q.conn.execute("insert into merges(from_contact,into_contact,created_at) values('x','y',1)")

    def test_recent_messages_default_limit_and_order(self):
        q = HistoryQuery(self.state/'history'/'history.sqlite3', default_recent=3)
        self.assertEqual(self.texts(q.get_recent_messages(self.c)), ['已发顺丰', '收到', '好的，已为您登记'])
        q.conn.close()

    def test_pagination_stable_while_writing(self):
        page = self.q.get_history(self.c, limit=3)
        seen = self.texts(page)
        self.writer.record_frame(self.c, fr(*self.visible, bub('好的，已为您登记', 'outbound_visual'), bub('新')))
        while page['next_cursor']:
            page = self.q.get_history(self.c, cursor=page['next_cursor'], limit=3)
            seen += self.texts(page)
        self.assertEqual(seen, self.expected_texts+['新'])

    def test_tail_cursor_follows_new_messages(self):
        page = self.q.get_history(self.c, limit=500)
        self.assertIsNone(page['next_cursor'])
        self.writer.record_frame(self.c, fr(*self.visible, bub('好的，已为您登记', 'outbound_visual'), bub('后来的')))
        newer = self.q.get_history(self.c, cursor=page['tail_cursor'])
        self.assertEqual(self.texts(newer), ['后来的'])

    def test_clock_rollback_does_not_skip_messages(self):
        page = self.q.get_history(self.c, limit=500)
        self.writer.record_frame(self.c, fr(*self.visible, bub('好的，已为您登记', 'outbound_visual'), bub('回拨后')))
        self.writer.conn.execute("update messages set observed_at=1 where text='回拨后'")
        self.assertEqual(self.texts(self.q.get_history(self.c, cursor=page['tail_cursor'])), ['回拨后'])

    def test_desc_order_and_time_range(self):
        page = self.q.get_history(self.c, order='desc', limit=2)
        self.assertEqual(self.texts(page), ['好的，已为您登记', '收到'])
        future = self.q.get_history(self.c, since=10**15)
        self.assertEqual(future['items'], [])

    def test_concurrent_read_during_open_write_transaction(self):
        self.writer.conn.execute('BEGIN IMMEDIATE')
        self.writer.conn.execute("insert into merges(from_contact,into_contact,created_at) values('a','b',1)")
        self.assertEqual(len(self.q.get_recent_messages(self.c)['items']), 5)
        self.writer.conn.execute('ROLLBACK')

    def test_search_trigram_and_short_term_fallback(self):
        self.assertEqual(self.texts(self.q.search_messages('上次那个')), ['还是上次那个款'])
        self.assertEqual(self.texts(self.q.search_messages('顺丰')), ['已发顺丰'])
        hit = self.q.search_messages('上次那个')['items'][0]
        self.assertEqual(len(hit['context_ids']), 3)  # 1 before (start of history) + 2 after

    def test_search_scoped_to_contact(self):
        self.assertEqual(self.q.search_messages('消息', contact_id=self.c)['items'], [])
        self.assertEqual(self.texts(self.q.search_messages('乙的消息')), ['乙的消息'])

    def test_merged_contact_history_is_included(self):
        self.writer.merge(self.c2, self.c)
        self.assertIn('乙的消息', self.texts(self.q.get_history(self.c)))
        listed = [x['contact_id'] for x in self.q.list_contacts()['items']]
        self.assertEqual(listed, [self.c])
        self.assertEqual(self.q.get_contact(contact_id=self.c2)['contact_id'], self.c)

    def test_bot_message_carries_send_status(self):
        m = self.q.get_recent_messages(self.c)['items'][-1]
        self.assertEqual((m['sender'], m['send']['status'], m['send']['echo_observed']), ('bot', 'triggered', True))
        self.assertIsNone(self.q.get_recent_messages(self.c)['items'][0]['send'])

    def test_limits_cursor_and_unknown_ids(self):
        with self.assertRaises(QueryError) as caught:
            self.q.get_history(self.c, limit=501)
        self.assertEqual(caught.exception.code, 'invalid_argument')
        with self.assertRaises(QueryError):
            self.q.get_history(self.c, cursor='garbage')
        with self.assertRaises(QueryError) as caught:
            self.q.get_message('m_missing')
        self.assertEqual(caught.exception.code, 'not_found')
        with self.assertRaises(QueryError):
            self.q.get_history('c_missing')

    def test_name_lookup_uses_name_key(self):
        a = self.q.get_contact(name='🍓小美')
        self.assertEqual(a['contact_id'], self.q.get_contact(name='小美')['contact_id'])
        self.assertEqual(a['message_count'], 5)
        self.assertIn('🍓小美', a['names'])

    def test_list_contacts_query_and_paging(self):
        self.assertEqual([x['display_name'] for x in self.q.list_contacts(query='小美')['items']], ['🍓小美'])
        first = self.q.list_contacts(limit=1)
        self.assertEqual(len(first['items']), 1)
        second = self.q.list_contacts(limit=1, cursor=first['next_cursor'])
        self.assertNotEqual(first['items'][0]['contact_id'], second['items'][0]['contact_id'])

    def test_get_message_and_media(self):
        mid = self.q.get_recent_messages(self.c)['items'][0]['message_id']
        self.assertIn('meta', self.q.get_message(mid))
        src = Path(self.temp.name)/'p.png'
        src.write_bytes(b'img')
        media_id = self.writer.attach_media(mid, src, kind='image', filename=None, source='chat_thumbnail')
        info, path = self.q.get_media(media_id)
        self.assertEqual((info['kind'], path.read_bytes()), ('image', b'img'))
        self.assertEqual(self.q.get_message(mid)['media']['media_id'], media_id)
