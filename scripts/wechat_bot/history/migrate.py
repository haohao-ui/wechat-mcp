"""One-time upgrade to per-contact history (design §9.3). Run only while the bot is stopped."""
import fcntl
import json
import os
from pathlib import Path

from scripts.agent_bridge.codex import private_directory, write_private
from scripts.wechat_bot.routing import match_entry
from .store import HistoryStore

LEGACY_FILES = ('cursor.json', 'recent-files.json')


def service_loaded():
    from scripts.wechat_bot.control import loaded
    return loaded()


def is_legacy_dir(path):
    return path.is_dir() and len(path.name) == 64 and all(c in '0123456789abcdef' for c in path.name)


def ensure_stopped(state):
    if service_loaded():
        raise RuntimeError('stop the bot first (control stop)')
    lock_path = state/'service.lock'
    if lock_path.exists():
        with open(lock_path, 'a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise RuntimeError('stop the bot first (service.lock is held)') from None
            fcntl.flock(lock, fcntl.LOCK_UN)


def entry_for(config, recipient):
    return match_entry(config, recipient) or {'recipient': recipient, 'kind': 'private'}


def migrate(config):
    state = Path(config['state_dir'])
    ensure_stopped(state)
    sessions = state/'sessions'
    if sessions.is_dir() and not sessions.is_symlink() and sessions.stat().st_uid == os.getuid():
        # Older versions created this parent with the default umask (0755); contents are already 0600.
        sessions.chmod(0o700)
    sessions = private_directory(sessions)
    store = HistoryStore.open(state)
    stats = {'contacts': 0, 'moved_dirs': 0, 'imported_sends': 0, 'baseline_messages': 0}
    try:
        def bind(directory, contact_id):
            write_private(directory/'binding.json', {'host': config['host'], 'account_id': config['account_id'],
                                                     'contact_id': contact_id, 'kind': 'private'})

        # 1. Hash-named session directories -> sessions/<contact_id>/ (contents kept intact).
        for legacy in sorted(p for p in sessions.iterdir() if is_legacy_dir(p)):
            binding = json.loads((legacy/'binding.json').read_text())
            if binding.get('account_id') != config['account_id'] or binding.get('host') != config['host']:
                raise RuntimeError(f'{legacy.name} belongs to another account/host')
            contact_id = store.ensure_contact(config['account_id'], entry_for(config, binding['recipient']))
            target = sessions/contact_id
            if target.exists():
                raise RuntimeError(f'both {legacy.name} and {contact_id} exist; resolve manually')
            os.rename(legacy, target)
            bind(target, contact_id)
            write_private(sessions/f'MOVED-{legacy.name}-TO-{contact_id}.txt', {'from': legacy.name, 'to': contact_id})
            stats['moved_dirs'] += 1

        # 2. Root-level single-session files (pre multi-chat), only if no session imported them yet.
        root_binding = state/'binding.json'
        if root_binding.exists():
            binding = json.loads(root_binding.read_text())
            if binding.get('recipient') and binding.get('account_id') == config['account_id']:
                contact_id = store.ensure_contact(config['account_id'], entry_for(config, binding['recipient']))
                target = sessions/contact_id
                if not (target/'binding.json').exists():
                    private_directory(target)
                    for source in [*(state/n for n in LEGACY_FILES), *state.glob('send-*.json')]:
                        if source.exists():
                            write_private(target/source.name, json.loads(source.read_text()))
                    bind(target, contact_id)

        # 3. Import old journals in time order, then baseline the current cursor.
        for directory in sorted(p for p in sessions.iterdir() if p.is_dir() and p.name.startswith('c_')):
            contact_id = directory.name
            stats['contacts'] += 1
            journals = sorted((json.loads(p.read_text()) for p in directory.glob('send-*.json')),
                              key=lambda r: r.get('created_at', 0))
            for record in journals:
                key = record.get('idempotency_key')
                if not key or store.send(key) or store.conn.execute(
                        "SELECT 1 FROM messages WHERE batch_key=? AND capture='imported'", (key,)).fetchone():
                    continue
                at = int(record.get('created_at', 0) * 1000)
                ids = [store.import_message(contact_id, b, observed_at=at, batch_key=key) for b in record.get('input', [])]
                if record.get('reply'):
                    sent_at = int(record.get('attempted_at', record.get('created_at', 0)) * 1000)
                    store.record_bot_reply(contact_id, key, record['reply'], record['status'], record.get('model', {}),
                                           ids, observed_at=sent_at, capture='imported')
                    store.update_send(key, record['status'], record.get('client_echo_observed'))
                stats['imported_sends'] += 1
            has_cursor = store.conn.execute('SELECT 1 FROM contact_cursor WHERE contact_id=?', (contact_id,)).fetchone()
            if not has_cursor and (directory/'cursor.json').exists():
                rows = store.record_frame(contact_id, json.loads((directory/'cursor.json').read_text()),
                                          baseline_meta={'may_duplicate_imported': True}, pair_imported=True)
                stats['baseline_messages'] += len(rows)
        return stats
    finally:
        store.conn.close()
