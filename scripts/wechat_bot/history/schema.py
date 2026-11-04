"""SQLite schema for conversation history (design §7). Times are UTC Unix milliseconds."""

MIGRATIONS = {
    1: """
CREATE TABLE contacts(
  contact_id TEXT PRIMARY KEY, account_id TEXT NOT NULL, kind TEXT NOT NULL,
  display_name TEXT NOT NULL, merged_into TEXT REFERENCES contacts(contact_id),
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE contact_names(
  id INTEGER PRIMARY KEY, contact_id TEXT NOT NULL REFERENCES contacts(contact_id),
  account_id TEXT NOT NULL, raw_name TEXT NOT NULL, name_key TEXT NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('config','alias','observed')),
  first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
  UNIQUE(contact_id, raw_name));
CREATE INDEX contact_names_key ON contact_names(account_id, name_key);
CREATE TABLE messages(
  message_id TEXT PRIMARY KEY, contact_id TEXT NOT NULL REFERENCES contacts(contact_id),
  account_id TEXT NOT NULL, seq INTEGER NOT NULL,
  direction TEXT NOT NULL CHECK(direction IN ('inbound','outbound')),
  sender TEXT NOT NULL CHECK(sender IN ('customer','bot','self_other')),
  kind TEXT NOT NULL, text TEXT NOT NULL, observed_at INTEGER NOT NULL,
  capture TEXT NOT NULL CHECK(capture IN ('live','baseline','rebaseline_gap','ambiguous','imported')),
  bubble_digest TEXT, send_key TEXT, media_id TEXT, batch_key TEXT, meta TEXT NOT NULL DEFAULT '{}');
CREATE UNIQUE INDEX messages_contact_seq ON messages(contact_id, seq);
CREATE INDEX messages_account_time ON messages(account_id, observed_at);
CREATE INDEX messages_send_key ON messages(send_key);
CREATE TABLE sends(
  send_key TEXT PRIMARY KEY, contact_id TEXT NOT NULL REFERENCES contacts(contact_id),
  status TEXT NOT NULL, reply_text TEXT NOT NULL, model TEXT NOT NULL DEFAULT '{}',
  echo_observed INTEGER NOT NULL DEFAULT 0, input_message_ids TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE media(
  media_id TEXT PRIMARY KEY, kind TEXT NOT NULL, filename TEXT, extension TEXT NOT NULL,
  size INTEGER NOT NULL, source TEXT NOT NULL, path TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE media_jobs(
  message_id TEXT PRIMARY KEY REFERENCES messages(message_id), contact_id TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('pending','done','failed','gone')),
  attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT, updated_at INTEGER NOT NULL);
CREATE TABLE contact_cursor(
  contact_id TEXT PRIMARY KEY REFERENCES contacts(contact_id),
  frame_fingerprints TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE merges(
  id INTEGER PRIMARY KEY, from_contact TEXT NOT NULL, into_contact TEXT NOT NULL,
  operator_note TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, undone_at INTEGER);
CREATE VIRTUAL TABLE messages_fts USING fts5(text, content='messages', content_rowid='rowid', tokenize='trigram');
CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, text) VALUES (new.rowid, new.text); END;
CREATE TRIGGER messages_fts_update AFTER UPDATE OF text ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
  INSERT INTO messages_fts(rowid, text) VALUES (new.rowid, new.text); END;
""",
}
