"""Read-only history queries shared by the MCP and HTTP servers (design §10)."""
import base64
import json
from pathlib import Path
import sqlite3

from .names import name_key


class QueryError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code, self.message = code, message


LIMITS = {'contacts': 200, 'recent': 200, 'history': 500, 'search': 100}


def encode_cursor(value):
    return base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip('=')


def decode_cursor(text, size):
    try:
        value = json.loads(base64.urlsafe_b64decode(text + '=' * (-len(text) % 4)))
        if not isinstance(value, list) or len(value) != size:
            raise ValueError
        return value
    except Exception:
        raise QueryError('invalid_argument', 'invalid cursor') from None


def check_limit(limit, kind):
    if type(limit) is not int or not 1 <= limit <= LIMITS[kind]:
        raise QueryError('invalid_argument', f'limit must be 1..{LIMITS[kind]}')
    return limit


MESSAGE_SELECT = (
    "SELECT m.rowid AS rid, m.message_id, m.contact_id, m.seq, m.direction, m.sender, m.kind, m.text, "
    "m.observed_at, m.capture, m.meta, m.send_key, s.status AS send_status, s.echo_observed, "
    "m.media_id, md.kind AS media_kind, md.filename AS media_filename, md.source AS media_source, "
    "md.understanding AS media_understanding "
    "FROM messages m LEFT JOIN sends s ON s.send_key=m.send_key LEFT JOIN media md ON md.media_id=m.media_id ")


class HistoryQuery:
    def __init__(self, db_path, *, default_recent=5):
        self.path = Path(db_path)
        if not self.path.exists():
            raise QueryError('not_found', 'history database does not exist')
        self.conn = sqlite3.connect(f'file:{self.path}?mode=ro', uri=True, check_same_thread=False)
        self.conn.row_factory = sqlite3.Row
        self.conn.execute('PRAGMA busy_timeout=5000')
        self.default_recent = default_recent

    # Contacts -------------------------------------------------------------

    def _merged(self):
        return {r['contact_id']: r['merged_into'] for r in self.conn.execute('SELECT contact_id, merged_into FROM contacts')}

    def _canonical(self, contact_id, merged=None):
        merged = merged or self._merged()
        if contact_id not in merged:
            raise QueryError('not_found', 'unknown contact')
        seen = set()
        while merged.get(contact_id) and contact_id not in seen:
            seen.add(contact_id)
            contact_id = merged[contact_id]
        return contact_id

    def _family(self, contact_id):
        merged = self._merged()
        root = self._canonical(contact_id, merged)
        return root, [c for c in merged if self._canonical(c, merged) == root]

    def _contact(self, root, family):
        row = self.conn.execute('SELECT contact_id, account_id, display_name FROM contacts WHERE contact_id=?', (root,)).fetchone()
        marks = ','.join('?' * len(family))
        names = [r[0] for r in self.conn.execute(f'SELECT DISTINCT raw_name FROM contact_names WHERE contact_id IN ({marks}) ORDER BY id', family)]
        stats = self.conn.execute(f'SELECT COUNT(*), MAX(observed_at) FROM messages WHERE contact_id IN ({marks})', family).fetchone()
        return {'contact_id': root, 'account_id': row['account_id'], 'display_name': row['display_name'], 'names': names,
                'merged_from': [c for c in family if c != root], 'message_count': stats[0], 'last_observed_at': stats[1]}

    def list_contacts(self, query=None, cursor=None, limit=50):
        check_limit(limit, 'contacts')
        after = decode_cursor(cursor, 1)[0] if cursor else 0
        params, where = [after], 'c.rowid > ? AND c.merged_into IS NULL'
        if query:
            where += ' AND EXISTS (SELECT 1 FROM contact_names n WHERE n.contact_id=c.contact_id AND instr(n.name_key, ?) > 0)'
            params.append(name_key(query))
        rows = self.conn.execute(f'SELECT c.rowid AS rid, c.contact_id FROM contacts c WHERE {where} ORDER BY c.rowid LIMIT ?',
                                 (*params, limit + 1)).fetchall()
        items = [self._contact(*self._family(r['contact_id'])) for r in rows[:limit]]
        return {'items': items, 'next_cursor': encode_cursor([rows[limit - 1]['rid']]) if len(rows) > limit else None}

    def get_contact(self, contact_id=None, name=None):
        if bool(contact_id) == bool(name):
            raise QueryError('invalid_argument', 'give exactly one of contact_id or name')
        if name:
            merged = self._merged()
            owners = {self._canonical(r[0], merged) for r in self.conn.execute(
                "SELECT contact_id FROM contact_names WHERE name_key=? AND source IN ('config','alias')", (name_key(name),))}
            if not owners:
                raise QueryError('not_found', 'unknown contact name')
            if len(owners) > 1:
                raise QueryError('invalid_argument', 'name matches several contacts; use contact_id')
            contact_id = owners.pop()
        return self._contact(*self._family(contact_id))

    # Messages -------------------------------------------------------------

    def _message(self, row, with_meta=False):
        item = {'message_id': row['message_id'], 'contact_id': row['contact_id'], 'seq': row['seq'],
                'direction': row['direction'], 'sender': row['sender'], 'kind': row['kind'], 'text': row['text'],
                'observed_at': row['observed_at'], 'capture': row['capture'],
                'send': ({'send_key': row['send_key'], 'status': row['send_status'], 'echo_observed': bool(row['echo_observed'])}
                         if row['send_key'] else None),
                'media': ({'media_id': row['media_id'], 'kind': row['media_kind'], 'filename': row['media_filename'],
                           'source': row['media_source'], 'understanding': row['media_understanding']}
                          if row['media_id'] else None)}
        if with_meta:
            item['meta'] = json.loads(row['meta'] or '{}')
        return item

    def _range(self, family, since, until, cursor, order):
        if order not in ('asc', 'desc'):
            raise QueryError('invalid_argument', 'order must be asc or desc')
        marks = ','.join('?' * len(family))
        where, params = [f'm.contact_id IN ({marks})'], list(family)
        for value, clause in ((since, 'm.observed_at >= ?'), (until, 'm.observed_at < ?')):
            if value is not None:
                if type(value) is not int:
                    raise QueryError('invalid_argument', 'since/until must be integer milliseconds')
                where.append(clause)
                params.append(value)
        if cursor:
            # Write order (rowid), not wall-clock time: a clock rollback cannot hide later messages.
            rid, cursor_order = decode_cursor(cursor, 2)
            if cursor_order != order or type(rid) is not int:
                raise QueryError('invalid_argument', 'cursor order mismatch')
            where.append('m.rowid > ?' if order == 'asc' else 'm.rowid < ?')
            params.append(rid)
        return ' AND '.join(where), params

    def _page(self, sql, params, limit, order, with_context=False, tail=None):
        rows = self.conn.execute(sql, (*params, limit + 1)).fetchall()
        items = [self._message(r) for r in rows[:limit]]
        if with_context:
            for item in items:
                item['context_ids'] = [r[0] for r in self.conn.execute(
                    'SELECT message_id FROM messages WHERE contact_id=? AND seq BETWEEN ? AND ? AND seq != ? ORDER BY seq',
                    (item['contact_id'], item['seq'] - 2, item['seq'] + 2, item['seq']))]
        last = rows[limit - 1] if len(rows) > limit else None
        page = {'items': items, 'next_cursor': encode_cursor([last['rid'], order]) if last else None}
        if order == 'asc':
            # Always present: poll with it later to receive only messages written afterwards.
            end = rows[min(len(rows), limit) - 1]['rid'] if rows else (decode_cursor(tail, 2)[0] if tail else 0)
            page['tail_cursor'] = encode_cursor([end, 'asc'])
        return page

    def get_history(self, contact_id, since=None, until=None, cursor=None, limit=100, order='asc'):
        check_limit(limit, 'history')
        _, family = self._family(contact_id)
        where, params = self._range(family, since, until, cursor, order)
        direction = 'ASC' if order == 'asc' else 'DESC'
        return self._page(MESSAGE_SELECT + f'WHERE {where} ORDER BY m.rowid {direction} LIMIT ?',
                          params, limit, order, tail=cursor)

    def get_recent_messages(self, contact_id, limit=None):
        limit = check_limit(self.default_recent if limit is None else limit, 'recent')
        page = self.get_history(contact_id, order='desc', limit=limit)
        return {'items': page['items'][::-1]}

    def search_messages(self, q, contact_id=None, since=None, until=None, cursor=None, limit=20):
        check_limit(limit, 'search')
        if not isinstance(q, str) or not q.strip():
            raise QueryError('invalid_argument', 'q is required')
        q = q.strip()
        family = self._family(contact_id)[1] if contact_id else [r[0] for r in self.conn.execute('SELECT contact_id FROM contacts')]
        where, params = self._range(family, since, until, cursor, 'desc')
        like = '%' + q.replace('\\', '\\\\').replace('%', '\\%').replace('_', '\\_') + '%'
        # Images and files are also found by what the model understood from them (media.understanding).
        understood = " OR md.understanding LIKE ? ESCAPE '\\'"
        if len(q) >= 3:
            # Trigram FTS needs >=3 characters; the query is passed as one quoted phrase.
            where += ' AND (m.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)' + understood + ')'
            params += ['"' + q.replace('"', '""') + '"', like]
        else:
            where += " AND (m.text LIKE ? ESCAPE '\\'" + understood + ")"
            params += [like, like]
        return self._page(MESSAGE_SELECT + f'WHERE {where} ORDER BY m.rowid DESC LIMIT ?',
                          params, limit, 'desc', with_context=True)

    def get_message(self, message_id):
        row = self.conn.execute(MESSAGE_SELECT + 'WHERE m.message_id=?', (message_id,)).fetchone()
        if row is None:
            raise QueryError('not_found', 'unknown message')
        return self._message(row, with_meta=True)

    def get_media(self, media_id):
        row = self.conn.execute('SELECT * FROM media WHERE media_id=?', (media_id,)).fetchone()
        if row is None:
            raise QueryError('not_found', 'unknown media')
        root = (self.path.parent/'media').resolve()
        path = (root/row['path']).resolve()
        if root not in path.parents or not path.is_file():
            raise QueryError('not_found', 'media file missing')
        return ({'media_id': row['media_id'], 'kind': row['kind'], 'filename': row['filename'],
                 'extension': row['extension'], 'size': row['size'], 'source': row['source'],
                 'understanding': row['understanding'], 'understood_at': row['understood_at']}, path)
