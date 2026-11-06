"""Single-writer SQLite history store used by the bot coordinator (design §7–§9)."""
import difflib
import hashlib
import json
import os
from pathlib import Path
import secrets
import sqlite3
import time
import unicodedata

from scripts.agent_bridge.codex import private_directory
from scripts.wechat_bot.policy import AmbiguousAnchor, ContinuityGap, echo_matches, fingerprint, new_bubbles, phash_distance, PHASH_TOLERANCE, reconcile
from .names import candidate_names, name_key
from .schema import MIGRATIONS


# A bot reply's on-screen echo appears within seconds; older replies never absorb new outbound bubbles.
ECHO_WINDOW_MS = 10*60*1000


class NameConflict(ValueError):
    """A normalized name already belongs to a different, unmerged contact."""


def now_ms():
    return int(time.time()*1000)


def new_id(prefix):
    return prefix + secrets.token_hex(8)


def connect(path):
    conn = sqlite3.connect(path, isolation_level=None, check_same_thread=False)
    conn.row_factory = sqlite3.Row
    conn.execute('PRAGMA journal_mode=WAL')
    conn.execute('PRAGMA foreign_keys=ON')
    conn.execute('PRAGMA busy_timeout=5000')
    return conn


class HistoryStore:
    def __init__(self, root, conn):
        self.root, self.conn = root, conn
        # contact_id -> fingerprints of the last gap/ambiguous frame awaiting a second identical frame.
        self.unstable = {}

    @classmethod
    def open(cls, state_dir):
        root = private_directory(Path(state_dir)/'history')
        path = root/'history.sqlite3'
        if not path.exists():
            os.close(os.open(path, os.O_CREAT | os.O_WRONLY, 0o600))
        os.chmod(path, 0o600)
        store = cls(root, connect(path))
        store.migrate()
        return store

    def migrate(self):
        self.conn.execute('CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)')
        done = {r[0] for r in self.conn.execute('SELECT version FROM schema_migrations')}
        for version in sorted(MIGRATIONS):
            if version in done:
                continue
            with self.transaction():
                for statement in split_sql(MIGRATIONS[version]):
                    self.conn.execute(statement)
                self.conn.execute('INSERT INTO schema_migrations VALUES(?,?)', (version, now_ms()))

    def transaction(self):
        return Transaction(self.conn)

    # Contacts -------------------------------------------------------------

    def canonical(self, contact_id):
        seen = set()
        while contact_id not in seen:
            seen.add(contact_id)
            row = self.conn.execute('SELECT merged_into FROM contacts WHERE contact_id=?', (contact_id,)).fetchone()
            if row is None or row[0] is None:
                return contact_id
            contact_id = row[0]
        raise ValueError('merge cycle')

    def _owners(self, account_id, key, sources=('config', 'alias'), follow_merge=True):
        rows = self.conn.execute(
            f"SELECT DISTINCT contact_id FROM contact_names WHERE account_id=? AND name_key=? AND source IN ({','.join('?'*len(sources))})",
            (account_id, key, *sources))
        return {self.canonical(r[0]) if follow_merge else r[0] for r in rows}

    def ensure_contact(self, account_id, entry, follow_merge=True):
        """Contact for an allowlist entry. Session state uses follow_merge=False: a merged contact keeps
        its own directory/cursor; only history reads follow the merge."""
        names = candidate_names(entry)
        with self.transaction():
            owners = set().union(*(self._owners(account_id, name_key(n), follow_merge=follow_merge) for n in names))
            if len(owners) > 1 and len({self.canonical(o) for o in owners}) == 1:
                # The entry names a merge target together with contacts merged into it: use the target.
                owners = {self.canonical(owners.pop())}
            if len(owners) > 1:
                raise NameConflict('names of %r belong to different contacts' % entry['recipient'])
            stamp = now_ms()
            if owners:
                contact_id = owners.pop()
                self.conn.execute('UPDATE contacts SET display_name=?, updated_at=? WHERE contact_id=?',
                                  (entry['recipient'], stamp, contact_id))
            else:
                contact_id = new_id('c_')
                self.conn.execute('INSERT INTO contacts VALUES(?,?,?,?,NULL,?,?)',
                                  (contact_id, account_id, entry.get('kind', 'private'), entry['recipient'], stamp, stamp))
            for index, raw in enumerate(names):
                self._name(contact_id, account_id, raw, 'config' if index == 0 else 'alias', stamp)
            return contact_id

    def _name(self, contact_id, account_id, raw, source, stamp):
        row = self.conn.execute('SELECT source FROM contact_names WHERE contact_id=? AND raw_name=?', (contact_id, raw)).fetchone()
        if row is None:
            self.conn.execute('INSERT INTO contact_names(contact_id,account_id,raw_name,name_key,source,first_seen_at,last_seen_at) VALUES(?,?,?,?,?,?,?)',
                              (contact_id, account_id, raw, name_key(raw), source, stamp, stamp))
        else:
            # A configured/alias name is never downgraded to merely observed.
            keep = row[0] if source == 'observed' else source
            self.conn.execute('UPDATE contact_names SET source=?, last_seen_at=? WHERE contact_id=? AND raw_name=?',
                              (keep, stamp, contact_id, raw))

    def resolve(self, account_id, raw_name):
        owners = self._owners(account_id, name_key(raw_name))
        return owners.pop() if len(owners) == 1 else None

    def account_of(self, contact_id):
        return self.conn.execute('SELECT account_id FROM contacts WHERE contact_id=?', (contact_id,)).fetchone()[0]

    def observe_name(self, contact_id, raw_name):
        account_id = self.account_of(contact_id)
        with self.transaction():
            key = name_key(raw_name)
            known = self.conn.execute(
                "SELECT 1 FROM contact_names WHERE contact_id=? AND name_key=? AND source IN ('config','alias')",
                (contact_id, key)).fetchone() is not None
            self._name(contact_id, account_id, raw_name, 'observed', now_ms())
            return known

    def merge(self, from_id, into_id, note=''):
        if self.canonical(into_id) == from_id or from_id == into_id:
            raise ValueError('cannot merge a contact into itself')
        with self.transaction():
            stamp = now_ms()
            self.conn.execute('UPDATE contacts SET merged_into=?, updated_at=? WHERE contact_id=?', (into_id, stamp, from_id))
            return self.conn.execute('INSERT INTO merges(from_contact,into_contact,operator_note,created_at) VALUES(?,?,?,?)',
                                     (from_id, into_id, note, stamp)).lastrowid

    def unmerge(self, merge_id):
        with self.transaction():
            row = self.conn.execute('SELECT from_contact FROM merges WHERE id=? AND undone_at IS NULL', (merge_id,)).fetchone()
            if row is None:
                raise ValueError('unknown or already undone merge')
            stamp = now_ms()
            self.conn.execute('UPDATE contacts SET merged_into=NULL, updated_at=? WHERE contact_id=?', (stamp, row[0]))
            self.conn.execute('UPDATE merges SET undone_at=? WHERE id=?', (stamp, merge_id))


    # Messages -------------------------------------------------------------

    # Replies that certainly never reached the chat stay out of model context (attempting/unknown may have).
    EXCLUDED_SEND = ('superseded', 'stopped_before_send', 'skipped_by_rule', 'generated', 'preparing_media', 'generating',
                     'already_replied')

    def record_frame(self, contact_id, frame, *, batch_key=None, baseline_meta=None, pair_imported=False):
        """Append bubbles not yet recorded for this contact; one transaction with the watermark."""
        current = frame['bubbles']
        with self.transaction():
            row = self.conn.execute('SELECT frame_fingerprints FROM contact_cursor WHERE contact_id=?', (contact_id,)).fetchone()
            plan = []
            stored = json.loads(row[0]) if row is not None else None
            if stored is not None and any(len(x) == 2 for x in stored):
                # Saved before fingerprint v2 (pixel image digests): anchors are not comparable.
                # Reset the watermark only; the visible frame was already recorded under v1.
                self.conn.execute('UPDATE contact_cursor SET frame_fingerprints=?, updated_at=? WHERE contact_id=?',
                                  (json.dumps(list(map(fingerprint, current)), ensure_ascii=False), now_ms(), contact_id))
                from scripts.wechat_bot.service import log_event  # Lazy: service imports this module.
                log_event('history_cursor_reset', contact_id=contact_id, visible_bubbles=len(current))
                return []
            if stored is None:
                plan = [(b, 'baseline', dict(baseline_meta or {})) for b in current]
            else:
                previous = [{'direction': d, 'digest': g, 'phash': h} for d, g, h in stored]
                current = frame['bubbles'] = reconcile(previous, current)
                old = [tuple(x) for x in stored]
                seen = list(map(fingerprint, current))
                try:
                    plan = [(b, 'live', {}) for b in new_bubbles(previous, current)]
                except AmbiguousAnchor as error:
                    if self.unstable.get(contact_id) != seen:
                        self.unstable[contact_id] = seen  # one jittery frame must not rewrite history
                        return []
                    new = seen
                    size = max(k for k in range(1, min(len(old), len(new))+1) if old[-k:] == new[:k])
                    end = len(current) - len(error.certain)
                    plan = [(b, 'ambiguous', {'note': 'repeat count uncertain'}) for b in current[size:end]]
                    plan += [(b, 'live', {}) for b in error.certain]
                except ContinuityGap:
                    if self.unstable.get(contact_id) != seen:
                        self.unstable[contact_id] = seen
                        return []
                    plan = [(b, 'rebaseline_gap', {'may_duplicate': True}) for b in current]
            self.unstable.pop(contact_id, None)
            rows = [self._append(contact_id, bubble, capture, meta, batch_key, pair_imported) for bubble, capture, meta in plan]
            rows = [r for r in rows if r is not None]
            self.conn.execute('INSERT INTO contact_cursor VALUES(?,?,?) ON CONFLICT(contact_id) DO UPDATE SET frame_fingerprints=excluded.frame_fingerprints, updated_at=excluded.updated_at',
                              (contact_id, json.dumps(list(map(fingerprint, current)), ensure_ascii=False), now_ms()))
            return rows

    def _append(self, contact_id, bubble, capture, meta, batch_key, pair_imported=False):
        inbound = bubble['direction'] == 'inbound_visual'
        if not inbound and self._pair_echo(contact_id, bubble, pair_imported):
            return None
        extra = {k: v for k, v in bubble.items() if k not in ('text', 'digest', 'direction')}
        row = self._insert(contact_id, direction='inbound' if inbound else 'outbound',
                           sender='customer' if inbound else 'self_other', kind=bubble.get('kind', 'text'),
                           text=bubble.get('text', ''), capture=capture, bubble_digest=bubble.get('digest'),
                           batch_key=batch_key, meta={**extra, **meta}, bubble=bubble)
        if row['kind'] == 'file':
            # Every file original is archived; downloads run later when no reply is pending.
            self.conn.execute("INSERT INTO media_jobs(message_id,contact_id,status,attempts,updated_at) VALUES(?,?,'pending',0,0)",
                              (row['message_id'], contact_id))
        return row

    def _insert(self, contact_id, *, direction, sender, kind, text, capture, bubble_digest=None,
                send_key=None, batch_key=None, meta=None, observed_at=None, bubble=None):
        seq = self.conn.execute('SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE contact_id=?', (contact_id,)).fetchone()[0]
        message_id = new_id('m_')
        self.conn.execute('INSERT INTO messages(message_id,contact_id,account_id,seq,direction,sender,kind,text,observed_at,capture,bubble_digest,send_key,batch_key,meta) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
                          (message_id, contact_id, self.account_of(contact_id), seq, direction, sender, kind, text,
                           observed_at or now_ms(), capture, bubble_digest, send_key, batch_key,
                           json.dumps(meta or {}, ensure_ascii=False)))
        return {'message_id': message_id, 'seq': seq, 'direction': direction, 'sender': sender,
                'capture': capture, 'text': text, 'kind': kind, 'bubble': bubble}

    def _pair_echo(self, contact_id, bubble, pair_imported=False):
        # Live frames pair only with recent live replies; the migration baseline pairs with imported ones.
        window = "(m.capture='imported')" if pair_imported else "(m.capture != 'imported' AND s.created_at >= ?)"
        rows = self.conn.execute(
            "SELECT m.message_id, s.send_key, s.reply_text FROM messages m JOIN sends s ON s.send_key=m.send_key "
            "WHERE m.contact_id=? AND m.sender='bot' AND m.bubble_digest IS NULL AND s.status IN ('triggered','unknown') "
            f"AND {window} ORDER BY m.seq DESC", (contact_id,) if pair_imported else (contact_id, now_ms() - ECHO_WINDOW_MS))
        rows = rows.fetchall()
        for index, (message_id, send_key, reply) in enumerate(rows):
            # Exact (normalized) match against any pending reply, or an OCR-noisy match ('AI'→'Al',
            # truncated last line) against the newest pending reply only.
            if echo_matches(bubble.get('text', ''), reply) or (index == 0 and similar(bubble.get('text', ''), reply)):
                self.conn.execute('UPDATE messages SET bubble_digest=? WHERE message_id=?', (bubble.get('digest'), message_id))
                self.conn.execute('UPDATE sends SET echo_observed=1, updated_at=? WHERE send_key=?', (now_ms(), send_key))
                return True
        return False

    def import_message(self, contact_id, bubble, *, observed_at, batch_key):
        """Append a message reconstructed from an old send journal (capture='imported')."""
        with self.transaction():
            inbound = bubble.get('direction', 'inbound_visual') == 'inbound_visual'
            return self._insert(contact_id, direction='inbound' if inbound else 'outbound',
                                sender='customer' if inbound else 'self_other', kind=bubble.get('kind', 'text'),
                                text=bubble.get('text', ''), capture='imported', bubble_digest=bubble.get('digest'),
                                batch_key=batch_key, observed_at=observed_at, bubble=bubble)['message_id']

    def record_bot_reply(self, contact_id, send_key, reply_text, status, model, input_message_ids, observed_at=None, capture='live'):
        with self.transaction():
            existing = self.conn.execute('SELECT message_id FROM messages WHERE send_key=? AND sender=?', (send_key, 'bot')).fetchone()
            if existing:
                return existing[0]
            stamp = now_ms()
            self.conn.execute('INSERT INTO sends VALUES(?,?,?,?,?,0,?,?,?)',
                              (send_key, contact_id, status, reply_text, json.dumps(model or {}, ensure_ascii=False),
                               json.dumps(list(input_message_ids)), stamp, stamp))
            return self._insert(contact_id, direction='outbound', sender='bot', kind='text', text=reply_text,
                                capture=capture, send_key=send_key, observed_at=observed_at)['message_id']

    def match_recent_inbound(self, contact_id, bubbles):
        """Message IDs of the newest recorded inbound messages matching these bubbles, in order.

        Images also need a phash within tolerance: same-size screenshots share one size digest."""
        rows = list(self.conn.execute("SELECT message_id, bubble_digest, meta FROM messages WHERE contact_id=? AND direction='inbound' ORDER BY seq DESC LIMIT 200", (contact_id,)))
        found, used = [], set()
        for bubble in reversed(bubbles):
            for message_id, digest, meta in rows:
                if message_id in used or digest != bubble.get('digest'):
                    continue
                stored = json.loads(meta or '{}').get('phash')
                if bubble.get('phash') and stored and phash_distance(stored, bubble['phash']) > PHASH_TOLERANCE:
                    continue
                used.add(message_id)
                found.append(message_id)
                break
        return found[::-1]

    def replied_after(self, contact_id, message_ids):
        """True when our side (bot send that went out, or a person) wrote after the newest of these messages."""
        if not message_ids:
            return False
        marks = ','.join('?'*len(message_ids))
        last = self.conn.execute(f'SELECT MAX(seq) FROM messages WHERE message_id IN ({marks})', tuple(message_ids)).fetchone()[0]
        if last is None:
            return False
        row = self.conn.execute(
            "SELECT 1 FROM messages m LEFT JOIN sends s ON s.send_key=m.send_key "
            "WHERE m.contact_id=? AND m.seq>? AND m.direction='outbound' "
            f"AND NOT (m.sender='bot' AND (s.status IS NULL OR s.status IN ({','.join('?'*len(self.EXCLUDED_SEND))}))) LIMIT 1",
            (contact_id, last, *self.EXCLUDED_SEND)).fetchone()
        return row is not None

    def update_send(self, send_key, status, echo_observed=None):
        with self.transaction():
            self.conn.execute('UPDATE sends SET status=?, updated_at=? WHERE send_key=?', (status, now_ms(), send_key))
            if echo_observed is not None:
                self.conn.execute('UPDATE sends SET echo_observed=? WHERE send_key=?', (int(bool(echo_observed)), send_key))

    def send(self, send_key):
        row = self.conn.execute('SELECT * FROM sends WHERE send_key=?', (send_key,)).fetchone()
        return dict(row) if row else None

    # Media ----------------------------------------------------------------

    MAX_MEDIA = 50*1024*1024

    def attach_media(self, message_id, source_path, *, kind, filename, source, max_bytes=None):
        limit = self.MAX_MEDIA if max_bytes is None else int(max_bytes)
        data = Path(source_path).read_bytes()
        if len(data) > limit:
            mb = limit // (1024 * 1024)
            unit = f'{mb}MB' if limit % (1024 * 1024) == 0 else f'{limit} bytes'
            raise ValueError(f'media larger than {unit}')
        digest = hashlib.sha256(data).hexdigest()
        extension = (Path(filename or source_path).suffix.lstrip('.') or 'bin').lower()[:10]
        directory = private_directory(private_directory(self.root/'media')/digest[:2])
        target = directory/(digest+'.'+extension)
        if not target.exists():
            temporary = target.with_name(target.name+'.'+secrets.token_hex(4)+'.tmp')
            descriptor = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            with os.fdopen(descriptor, 'wb') as handle:
                handle.write(data)
            os.replace(temporary, target)
        with self.transaction():
            self.conn.execute('INSERT OR IGNORE INTO media(media_id,kind,filename,extension,size,source,path,created_at) VALUES(?,?,?,?,?,?,?,?)',
                              (digest, kind, filename, extension, len(data), source,
                               str(target.relative_to(self.root/'media')), now_ms()))
            self.conn.execute('UPDATE messages SET media_id=? WHERE message_id=?', (digest, message_id))
        return digest

    def next_media_job(self, contact_id):
        stamp = now_ms()
        for row in self.conn.execute("SELECT * FROM media_jobs WHERE contact_id=? AND status='pending' ORDER BY updated_at", (contact_id,)):
            if row['attempts'] == 0 or stamp - row['updated_at'] >= 30_000 * 2**row['attempts']:
                return dict(row)
        return None

    def finish_media_job(self, message_id, status, error=None):
        if status not in ('done', 'failed', 'gone'):
            raise ValueError('invalid media job status')
        with self.transaction():
            if status == 'failed':
                attempts = self.conn.execute('SELECT attempts FROM media_jobs WHERE message_id=?', (message_id,)).fetchone()[0] + 1
                status = 'failed' if attempts >= 3 else 'pending'
                self.conn.execute('UPDATE media_jobs SET status=?, attempts=?, last_error=?, updated_at=? WHERE message_id=?',
                                  (status, attempts, error, now_ms(), message_id))
            else:
                self.conn.execute('UPDATE media_jobs SET status=?, last_error=?, updated_at=? WHERE message_id=?',
                                  (status, error, now_ms(), message_id))

    def has_pending_media(self):
        return self.conn.execute("SELECT 1 FROM media_jobs WHERE status='pending' LIMIT 1").fetchone() is not None

    def family(self, contact_id):
        """The contact plus every contact merged into it (directly or transitively)."""
        ids = [r[0] for r in self.conn.execute('SELECT contact_id FROM contacts')]
        return [c for c in ids if self.canonical(c) == self.canonical(contact_id)]

    def recent_context(self, contact_id, limit, exclude_ids=frozenset()):
        if limit <= 0:
            return []
        family = self.family(contact_id)
        rows = self.conn.execute(
            f"SELECT m.message_id, m.sender, m.text, d.kind, d.filename, d.understanding, d.path FROM messages m "
            f"LEFT JOIN sends s ON s.send_key=m.send_key LEFT JOIN media d ON d.media_id=m.media_id "
            f"WHERE m.contact_id IN ({','.join('?'*len(family))}) "
            f"AND NOT (m.sender='bot' AND s.status IN ({','.join('?'*len(self.EXCLUDED_SEND))})) "
            f"ORDER BY m.observed_at DESC, m.rowid DESC", (*family, *self.EXCLUDED_SEND))
        picked = []
        for message_id, sender, text, kind, filename, understanding, path in rows:
            if message_id in exclude_ids:
                continue
            item = {'role': 'user' if sender == 'customer' else 'assistant', 'text': text}
            if kind:
                item['media'] = {'kind': kind, 'filename': filename, 'understanding': understanding,
                                 'path': str(self.root/'media'/path) if path else None}
            picked.append(item)
            if len(picked) == limit:
                break
        return picked[::-1]

    def set_understanding(self, media_id, text):
        with self.transaction():
            self.conn.execute('UPDATE media SET understanding=?, understood_at=? WHERE media_id=?',
                              (text[:1000], now_ms(), media_id))


def compact(text):
    return ''.join(unicodedata.normalize('NFKC', text).split())


def similar(observed, expected, threshold=0.85):
    a, b = compact(observed), compact(expected)
    if not a or not b:
        return False
    # The visible bubble may be truncated at the end; compare against the same-length prefix too.
    ratio = max(difflib.SequenceMatcher(None, a, b).ratio(), difflib.SequenceMatcher(None, a, b[:len(a)]).ratio())
    return ratio >= threshold and len(a) >= 0.6 * len(b)


class Transaction:
    """BEGIN IMMEDIATE ... COMMIT; nested use joins the outer transaction."""

    def __init__(self, conn):
        self.conn, self.outer = conn, False

    def __enter__(self):
        self.outer = not self.conn.in_transaction
        if self.outer:
            self.conn.execute('BEGIN IMMEDIATE')
        return self.conn

    def __exit__(self, kind, value, trace):
        if self.outer:
            self.conn.execute('ROLLBACK' if kind else 'COMMIT')
        return False


def split_sql(script):
    """Split a migration into statements, keeping trigger bodies intact."""
    statements, buffer = [], ''
    for line in script.strip().splitlines():
        buffer += line + '\n'
        if sqlite3.complete_statement(buffer):
            statements.append(buffer.strip())
            buffer = ''
    return statements
