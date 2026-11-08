use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::core::model::Bubble;
use super::names::{candidate_names, name_key};

use crate::core::policy;

pub const ECHO_WINDOW_MS: i64 = 10 * 60 * 1000;
pub const MAX_MEDIA: usize = 50 * 1024 * 1024;

pub const EXCLUDED_SEND: &[&str] = &[
    "superseded",
    "stopped_before_send",
    "skipped_by_rule",
    "generated",
    "preparing_media",
    "generating",
    "already_replied",
    "not_sent",
];

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_id(prefix: &str) -> String {
    format!("{}{}", prefix, uuid::Uuid::new_v4().simple().to_string()[..16].to_lowercase())
}

pub fn ensure_private_dir(path: impl AsRef<Path>) -> Result<PathBuf> {
    let p = path.as_ref();
    fs::create_dir_all(p)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    Ok(p.to_path_buf())
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let k: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() + 8) % 64 != 0 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, w_val) in w.iter_mut().take(16).enumerate() {
            *w_val = u32::from_be_bytes(chunk[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut h_val = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h_val.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h_val = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(h_val);
    }
    h.iter().map(|val| format!("{:08x}", val)).collect::<String>()
}

pub struct HistoryStore {
    root: PathBuf,
    conn: Arc<Mutex<Connection>>,
    unstable: Arc<Mutex<std::collections::HashMap<String, Vec<policy::Fingerprint>>>>,
}

impl Clone for HistoryStore {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            conn: Arc::clone(&self.conn),
            unstable: Arc::clone(&self.unstable),
        }
    }
}

pub const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
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
"#,
    ),
    (
        2,
        r#"
CREATE INDEX messages_contact_time ON messages(contact_id, observed_at);
"#,
    ),
    (
        3,
        r#"
ALTER TABLE media ADD COLUMN understanding TEXT;
ALTER TABLE media ADD COLUMN understood_at INTEGER;
"#,
    ),
    (
        4,
        r#"
CREATE TABLE contact_names_new(
  id INTEGER PRIMARY KEY, contact_id TEXT NOT NULL REFERENCES contacts(contact_id),
  account_id TEXT NOT NULL, raw_name TEXT NOT NULL, name_key TEXT NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('config','alias','observed','addressbook')),
  first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
  UNIQUE(contact_id, raw_name));
INSERT INTO contact_names_new(id, contact_id, account_id, raw_name, name_key, source, first_seen_at, last_seen_at)
  SELECT id, contact_id, account_id, raw_name, name_key, source, first_seen_at, last_seen_at FROM contact_names;
DROP TABLE contact_names;
ALTER TABLE contact_names_new RENAME TO contact_names;
CREATE INDEX contact_names_key ON contact_names(account_id, name_key);

CREATE TABLE contact_profiles(
  contact_id TEXT PRIMARY KEY REFERENCES contacts(contact_id),
  account_id TEXT NOT NULL,
  wechat_id TEXT,
  category TEXT NOT NULL CHECK(category IN ('official','work','personal','group')),
  nickname TEXT NOT NULL,
  remark TEXT,
  region TEXT,
  source TEXT,
  added_on TEXT,
  common_groups INTEGER,
  enterprise TEXT,
  real_name TEXT,
  tags TEXT NOT NULL DEFAULT '[]',
  friend_permission TEXT,
  identity TEXT NOT NULL CHECK(identity IN ('strong','weak')),
  first_synced_at INTEGER NOT NULL,
  last_synced_at INTEGER NOT NULL,
  removed_at INTEGER
);
CREATE UNIQUE INDEX contact_profiles_account_wechat ON contact_profiles(account_id, wechat_id) WHERE wechat_id IS NOT NULL AND wechat_id != '';
CREATE INDEX contact_profiles_account_category ON contact_profiles(account_id, category);

CREATE TABLE contact_reviews(
  id INTEGER PRIMARY KEY,
  account_id TEXT NOT NULL,
  reason TEXT NOT NULL,
  candidates TEXT NOT NULL DEFAULT '[]',
  profile TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL,
  resolved_at INTEGER
);
CREATE INDEX contact_reviews_account ON contact_reviews(account_id);
"#,
    ),
];

fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )?;
    Ok(conn)
}

pub(crate) fn canonical_conn(conn: &Connection, mut contact_id: String) -> Result<String> {
    let mut seen = HashSet::new();
    while !seen.contains(&contact_id) {
        seen.insert(contact_id.clone());
        let row: Option<Option<String>> = conn
            .query_row(
                "SELECT merged_into FROM contacts WHERE contact_id = ?",
                [&contact_id],
                |r| r.get(0),
            )
            .optional()?;
        match row {
            Some(Some(merged)) => {
                contact_id = merged;
            }
            _ => return Ok(contact_id),
        }
    }
    bail!("merge cycle")
}

fn account_of_conn(conn: &Connection, contact_id: &str) -> Result<String> {
    let account: String = conn.query_row(
        "SELECT account_id FROM contacts WHERE contact_id=?",
        [contact_id],
        |r| r.get(0),
    )?;
    Ok(account)
}

fn family_conn(conn: &Connection, contact_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT contact_id FROM contacts")?;
    let ids: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    let canonical_target = canonical_conn(conn, contact_id.to_string())?;
    let mut result = Vec::new();
    for id in ids {
        if canonical_conn(conn, id.clone())? == canonical_target {
            result.push(id);
        }
    }
    Ok(result)
}

fn _owners_conn(
    conn: &Connection,
    account_id: &str,
    key: &str,
    sources: &[&str],
    follow_merge: bool,
) -> Result<HashSet<String>> {
    let marks = sources.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT DISTINCT contact_id FROM contact_names WHERE account_id=? AND name_key=? AND source IN ({})",
        marks
    );
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
    params_vec.push(&account_id);
    params_vec.push(&key);
    for s in sources {
        params_vec.push(s);
    }
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params_vec), |r| r.get::<_, String>(0))?;
    let mut raw_owners = HashSet::new();
    for r in rows {
        raw_owners.insert(r?);
    }
    drop(stmt);

    if follow_merge {
        let mut resolved = HashSet::new();
        for o in raw_owners {
            resolved.insert(canonical_conn(conn, o)?);
        }
        Ok(resolved)
    } else {
        Ok(raw_owners)
    }
}

fn _name_with_conn(
    conn: &Connection,
    contact_id: &str,
    account_id: &str,
    raw: &str,
    source: &str,
    stamp: i64,
) -> Result<()> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT source FROM contact_names WHERE contact_id=? AND raw_name=?",
            params![contact_id, raw],
            |r| r.get(0),
        )
        .optional()?;

    if let Some(existing_source) = existing {
        let keep = if source == "observed" {
            existing_source
        } else {
            source.to_string()
        };
        conn.execute(
            "UPDATE contact_names SET source=?, last_seen_at=? WHERE contact_id=? AND raw_name=?",
            params![keep, stamp, contact_id, raw],
        )?;
    } else {
        conn.execute(
            "INSERT INTO contact_names(contact_id,account_id,raw_name,name_key,source,first_seen_at,last_seen_at) VALUES(?,?,?,?,?,?,?)",
            params![contact_id, account_id, raw, name_key(raw), source, stamp, stamp],
        )?;
    }
    Ok(())
}

impl HistoryStore {
    pub fn open(state_dir: impl AsRef<Path>) -> Result<Self> {
        let root = ensure_private_dir(state_dir.as_ref().join("history"))?;
        let db_path = root.join("history.sqlite3");
        if !db_path.exists() {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .mode(0o600)
                .open(&db_path)?;
            drop(file);
        }
        fs::set_permissions(&db_path, fs::Permissions::from_mode(0o600))?;

        let conn = connect(&db_path)?;
        let store = Self {
            root,
            conn: Arc::new(Mutex::new(conn)),
            unstable: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn connection(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    pub fn root_dir(&self) -> &Path {
        &self.root
    }

    pub fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)",
            [],
        )?;
        let mut stmt = conn.prepare("SELECT version FROM schema_migrations")?;
        let applied: HashSet<i64> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);

        for &(version, sql) in MIGRATIONS {
            if applied.contains(&version) {
                continue;
            }
            conn.execute("BEGIN IMMEDIATE", [])?;
            let res: Result<()> = (|| {
                conn.execute_batch(sql)?;
                conn.execute(
                    "INSERT INTO schema_migrations(version, applied_at) VALUES (?, ?)",
                    params![version, now_ms()],
                )?;
                Ok(())
            })();
            match res {
                Ok(_) => {
                    conn.execute("COMMIT", [])?;
                }
                Err(e) => {
                    let _ = conn.execute("ROLLBACK", []);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    // Contacts -----------------------------------------------------------------

    pub fn canonical(&self, contact_id: String) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        canonical_conn(&conn, contact_id)
    }

    pub fn ensure_contact(&self, account_id: &str, entry: &Value, follow_merge: bool) -> Result<String> {
        let names = candidate_names(entry);
        let recipient = entry
            .get("recipient")
            .and_then(|v| v.as_str())
            .context("missing recipient in entry")?;
        let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("private");

        let conn = self.conn.lock().unwrap();
        let mut owners = HashSet::new();
        for n in &names {
            let set = _owners_conn(&conn, account_id, &name_key(n), &["config", "alias"], follow_merge)?;
            owners.extend(set);
        }

        if owners.len() > 1 {
            let mut canonical_owners = HashSet::new();
            for o in &owners {
                canonical_owners.insert(canonical_conn(&conn, o.clone())?);
            }
            if canonical_owners.len() == 1 {
                owners = canonical_owners;
            } else {
                bail!("names of {:?} belong to different contacts", recipient);
            }
        }

        let stamp = now_ms();
        let contact_id = if let Some(existing) = owners.into_iter().next() {
            conn.execute(
                "UPDATE contacts SET display_name=?, updated_at=? WHERE contact_id=?",
                params![recipient, stamp, existing],
            )?;
            existing
        } else {
            let cid = new_id("c_");
            conn.execute(
                "INSERT INTO contacts VALUES(?,?,?,?,NULL,?,?)",
                params![cid, account_id, kind, recipient, stamp, stamp],
            )?;
            cid
        };

        for (index, raw) in names.iter().enumerate() {
            let source = if index == 0 { "config" } else { "alias" };
            _name_with_conn(&conn, &contact_id, account_id, raw, source, stamp)?;
        }

        Ok(contact_id)
    }

    pub fn resolve(&self, account_id: &str, raw_name: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let owners = _owners_conn(&conn, account_id, &name_key(raw_name), &["config", "alias", "addressbook"], true)?;
        if owners.len() == 1 {
            Ok(owners.into_iter().next())
        } else {
            Ok(None)
        }
    }

    pub fn account_of(&self, contact_id: &str) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        account_of_conn(&conn, contact_id)
    }

    pub fn observe_name(&self, contact_id: &str, raw_name: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let account_id = account_of_conn(&conn, contact_id)?;
        let stamp = now_ms();
        let key = name_key(raw_name);
        let known: bool = conn
            .query_row(
                "SELECT 1 FROM contact_names WHERE contact_id=? AND name_key=? AND source IN ('config','alias')",
                params![contact_id, key],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);

        _name_with_conn(&conn, contact_id, &account_id, raw_name, "observed", stamp)?;
        Ok(known)
    }

    pub fn merge(&self, from_id: &str, into_id: &str, note: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        if canonical_conn(&conn, into_id.to_string())? == from_id || from_id == into_id {
            bail!("cannot merge a contact into itself");
        }
        let stamp = now_ms();
        conn.execute(
            "UPDATE contacts SET merged_into=?, updated_at=? WHERE contact_id=?",
            params![into_id, stamp, from_id],
        )?;
        conn.execute(
            "INSERT INTO merges(from_contact,into_contact,operator_note,created_at) VALUES(?,?,?,?)",
            params![from_id, into_id, note, stamp],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn unmerge(&self, merge_id: i64) -> Result<()> {
        let stamp = now_ms();
        let conn = self.conn.lock().unwrap();
        let from_contact: Option<String> = conn
            .query_row(
                "SELECT from_contact FROM merges WHERE id=? AND undone_at IS NULL",
                params![merge_id],
                |r| r.get(0),
            )
            .optional()?;
        let from = match from_contact {
            Some(f) => f,
            None => bail!("unknown or already undone merge"),
        };
        conn.execute(
            "UPDATE contacts SET merged_into=NULL, updated_at=? WHERE contact_id=?",
            params![stamp, from],
        )?;
        conn.execute(
            "UPDATE merges SET undone_at=? WHERE id=?",
            params![stamp, merge_id],
        )?;
        Ok(())
    }

    // Messages -----------------------------------------------------------------

    pub fn record_bot_reply(
        &self,
        contact_id: &str,
        send_key: &str,
        reply_text: &str,
        status: &str,
        model: &Value,
        input_message_ids: &[String],
        observed_at: Option<i64>,
        capture: Option<&str>,
    ) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<String> = conn
            .query_row(
                "SELECT message_id FROM messages WHERE send_key=? AND sender='bot'",
                params![send_key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(mid) = existing {
            return Ok(mid);
        }

        let stamp = now_ms();
        let model_json = serde_json::to_string(model)?;
        let inputs_json = serde_json::to_string(input_message_ids)?;
        conn.execute(
            "INSERT INTO sends VALUES(?,?,?,?,?,0,?,?,?)",
            params![send_key, contact_id, status, reply_text, model_json, inputs_json, stamp, stamp],
        )?;

        let cap = capture.unwrap_or("live");
        let account_id = account_of_conn(&conn, contact_id)?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE contact_id=?",
            params![contact_id],
            |r| r.get(0),
        )?;
        let message_id = new_id("m_");
        conn.execute(
            "INSERT INTO messages(message_id,contact_id,account_id,seq,direction,sender,kind,text,observed_at,capture,send_key,meta)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                message_id,
                contact_id,
                account_id,
                seq,
                "outbound",
                "bot",
                "text",
                reply_text,
                observed_at.unwrap_or(stamp),
                cap,
                send_key,
                "{}",
            ],
        )?;
        Ok(message_id)
    }

    pub fn update_send(
        &self,
        send_key: &str,
        status: &str,
        echo_observed: Option<bool>,
    ) -> Result<()> {
        let stamp = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sends SET status=?, updated_at=? WHERE send_key=?",
            params![status, stamp, send_key],
        )?;
        if let Some(echo) = echo_observed {
            conn.execute(
                "UPDATE sends SET echo_observed=? WHERE send_key=?",
                params![if echo { 1 } else { 0 }, send_key],
            )?;
        }
        Ok(())
    }

    pub fn send(&self, send_key: &str) -> Result<Option<Value>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT send_key, contact_id, status, reply_text, model, echo_observed, input_message_ids, created_at, updated_at FROM sends WHERE send_key=?",
                params![send_key],
                |r| {
                    Ok(json!({
                        "send_key": r.get::<_, String>(0)?,
                        "contact_id": r.get::<_, String>(1)?,
                        "status": r.get::<_, String>(2)?,
                        "reply_text": r.get::<_, String>(3)?,
                        "model": serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
                        "echo_observed": r.get::<_, i64>(5)? != 0,
                        "input_message_ids": serde_json::from_str::<Value>(&r.get::<_, String>(6)?).unwrap_or(Value::Null),
                        "created_at": r.get::<_, i64>(7)?,
                        "updated_at": r.get::<_, i64>(8)?,
                    }))
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn import_message(
        &self,
        contact_id: &str,
        bubble: &Bubble,
        observed_at: i64,
        batch_key: &str,
    ) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let inbound = bubble.direction == "inbound_visual";
        let direction = if inbound { "inbound" } else { "outbound" };
        let sender = if inbound { "customer" } else { "self_other" };
        let kind = bubble.kind.as_deref().unwrap_or("text");
        let account_id = account_of_conn(&conn, contact_id)?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE contact_id=?",
            params![contact_id],
            |r| r.get(0),
        )?;
        let message_id = new_id("m_");
        let meta_json = serde_json::to_string(&bubble.extra)?;
        conn.execute(
            "INSERT INTO messages(message_id,contact_id,account_id,seq,direction,sender,kind,text,observed_at,capture,bubble_digest,batch_key,meta)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                message_id,
                contact_id,
                account_id,
                seq,
                direction,
                sender,
                kind,
                bubble.text,
                observed_at,
                "imported",
                bubble.digest,
                batch_key,
                meta_json,
            ],
        )?;
        Ok(message_id)
    }

    pub fn replied_after(
        &self,
        contact_id: &str,
        message_ids: &[String],
        exclude_send_key: Option<&str>,
    ) -> Result<bool> {
        if message_ids.is_empty() {
            return Ok(false);
        }
        let conn = self.conn.lock().unwrap();
        let marks = message_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql_max = format!("SELECT MAX(seq) FROM messages WHERE message_id IN ({})", marks);
        let params_ids: Vec<&dyn rusqlite::ToSql> = message_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let last: Option<i64> = conn
            .query_row(&sql_max, rusqlite::params_from_iter(params_ids), |r| r.get(0))
            .optional()?
            .flatten();

        let last_seq = match last {
            Some(s) => s,
            None => return Ok(false),
        };

        let exc_marks = EXCLUDED_SEND.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT 1 FROM messages m LEFT JOIN sends s ON s.send_key=m.send_key
             WHERE m.contact_id=? AND m.seq>? AND m.direction='outbound'
             AND NOT (m.sender='bot' AND (s.status IS NULL OR s.status IN ({})))
             AND (m.send_key IS NULL OR m.send_key IS NOT ?) LIMIT 1",
            exc_marks
        );
        let mut q_params: Vec<&dyn rusqlite::ToSql> = Vec::new();
        q_params.push(&contact_id);
        q_params.push(&last_seq);
        for s in EXCLUDED_SEND {
            q_params.push(s);
        }
        q_params.push(&exclude_send_key);

        let row: Option<i64> = conn
            .query_row(&sql, rusqlite::params_from_iter(q_params), |r| r.get(0))
            .optional()?;
        Ok(row.is_some())
    }

    pub fn family(&self, contact_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        family_conn(&conn, contact_id)
    }

    pub fn recent_context(
        &self,
        contact_id: &str,
        limit: usize,
        exclude_ids: &HashSet<String>,
    ) -> Result<Vec<Value>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let conn = self.conn.lock().unwrap();
        let family = family_conn(&conn, contact_id)?;
        let family_marks = family.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let exc_marks = EXCLUDED_SEND.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT m.message_id, m.sender, m.text, d.kind, d.filename, d.understanding, d.path
             FROM messages m
             LEFT JOIN sends s ON s.send_key=m.send_key
             LEFT JOIN media d ON d.media_id=m.media_id
             WHERE m.contact_id IN ({})
             AND NOT (m.sender='bot' AND s.status IN ({}))
             ORDER BY m.observed_at DESC, m.rowid DESC",
            family_marks, exc_marks
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
        for f in &family {
            params_vec.push(f);
        }
        for e in EXCLUDED_SEND {
            params_vec.push(e);
        }

        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(params_vec))?;

        let mut picked = Vec::new();
        while let Some(row) = rows.next()? {
            let message_id: String = row.get(0)?;
            if exclude_ids.contains(&message_id) {
                continue;
            }
            let sender: String = row.get(1)?;
            let text: String = row.get(2)?;
            let kind: Option<String> = row.get(3)?;
            let filename: Option<String> = row.get(4)?;
            let understanding: Option<String> = row.get(5)?;
            let path: Option<String> = row.get(6)?;

            let role = if sender == "customer" { "user" } else { "assistant" };
            let mut item = json!({
                "role": role,
                "text": text,
            });
            if let Some(k) = kind {
                let abs_path = path.map(|p| self.root.join("media").join(p).to_string_lossy().to_string());
                item["media"] = json!({
                    "kind": k,
                    "filename": filename,
                    "understanding": understanding,
                    "path": abs_path,
                });
            }
            picked.push(item);
            if picked.len() == limit {
                break;
            }
        }
        picked.reverse();
        Ok(picked)
    }

    pub fn set_understanding(&self, media_id: &str, text: &str) -> Result<()> {
        let stamp = now_ms();
        let truncated: String = text.chars().take(1000).collect();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE media SET understanding=?, understood_at=? WHERE media_id=?",
            params![truncated, stamp, media_id],
        )?;
        Ok(())
    }

    pub fn attach_media(
        &self,
        message_id: &str,
        source_path: impl AsRef<Path>,
        kind: &str,
        filename: Option<&str>,
        source: &str,
        max_bytes: Option<usize>,
    ) -> Result<String> {
        let limit = max_bytes.unwrap_or(MAX_MEDIA);
        let data = fs::read(source_path.as_ref())?;
        if data.len() > limit {
            let mb = limit / (1024 * 1024);
            let unit = if limit % (1024 * 1024) == 0 {
                format!("{}MB", mb)
            } else {
                format!("{} bytes", limit)
            };
            bail!("media larger than {}", unit);
        }

        let digest = sha256_hex(&data);

        let raw_ext = filename
            .or_else(|| source_path.as_ref().file_name().and_then(|f| f.to_str()))
            .and_then(|n| Path::new(n).extension().and_then(|e| e.to_str()))
            .unwrap_or("bin")
            .trim_start_matches('.');
        let extension = if raw_ext.is_empty() {
            "bin".to_string()
        } else {
            raw_ext.to_lowercase().chars().take(10).collect::<String>()
        };

        let media_root = ensure_private_dir(self.root.join("media"))?;
        let sub_dir = ensure_private_dir(media_root.join(&digest[..2]))?;
        let target = sub_dir.join(format!("{}.{}", digest, extension));
        if !target.exists() {
            let tmp_name = format!("{}.{}.tmp", target.file_name().unwrap().to_string_lossy(), uuid::Uuid::new_v4().simple());
            let tmp_path = sub_dir.join(tmp_name);
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp_path)?;
            use std::io::Write;
            file.write_all(&data)?;
            drop(file);
            fs::rename(tmp_path, &target)?;
        }

        let rel_path = target
            .strip_prefix(&media_root)
            .unwrap_or(&target)
            .to_string_lossy()
            .to_string();

        let stamp = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO media(media_id,kind,filename,extension,size,source,path,created_at) VALUES(?,?,?,?,?,?,?,?)",
            params![digest, kind, filename, extension, data.len() as i64, source, rel_path, stamp],
        )?;
        conn.execute(
            "UPDATE messages SET media_id=? WHERE message_id=?",
            params![digest, message_id],
        )?;
        Ok(digest)
    }

    pub fn next_media_job(&self, contact_id: &str) -> Result<Option<Value>> {
        let stamp = now_ms();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT message_id, contact_id, status, attempts, last_error, updated_at
             FROM media_jobs WHERE contact_id=? AND status='pending' ORDER BY updated_at",
        )?;
        let mut rows = stmt.query(params![contact_id])?;
        while let Some(row) = rows.next()? {
            let attempts: i64 = row.get(3)?;
            let updated_at: i64 = row.get(5)?;
            let backoff = 30_000i64 * (1i64 << attempts.min(30));
            if attempts == 0 || stamp - updated_at >= backoff {
                return Ok(Some(json!({
                    "message_id": row.get::<_, String>(0)?,
                    "contact_id": row.get::<_, String>(1)?,
                    "status": row.get::<_, String>(2)?,
                    "attempts": attempts,
                    "last_error": row.get::<_, Option<String>>(4)?,
                    "updated_at": updated_at,
                })));
            }
        }
        Ok(None)
    }

    pub fn finish_media_job(&self, message_id: &str, status: &str, error: Option<&str>) -> Result<()> {
        if !matches!(status, "done" | "failed" | "gone") {
            bail!("invalid media job status");
        }
        let stamp = now_ms();
        let conn = self.conn.lock().unwrap();
        if status == "failed" {
            let prev_attempts: i64 = conn.query_row(
                "SELECT attempts FROM media_jobs WHERE message_id=?",
                params![message_id],
                |r| r.get::<_, i64>(0),
            )?;
            let attempts = prev_attempts + 1;
            let final_status = if attempts >= 3 { "failed" } else { "pending" };
            conn.execute(
                "UPDATE media_jobs SET status=?, attempts=?, last_error=?, updated_at=? WHERE message_id=?",
                params![final_status, attempts, error, stamp, message_id],
            )?;
        } else {
            conn.execute(
                "UPDATE media_jobs SET status=?, last_error=?, updated_at=? WHERE message_id=?",
                params![status, error, stamp, message_id],
            )?;
        }
        Ok(())
    }

    pub fn has_pending_media(&self) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let found: Option<i64> = conn
            .query_row("SELECT 1 FROM media_jobs WHERE status='pending' LIMIT 1", [], |r| r.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    // Record Frame -------------------------------------------------------------

    pub fn record_frame(
        &self,
        contact_id: &str,
        frame: &Value,
        batch_key: Option<&str>,
        baseline_meta: Option<&Value>,
        pair_imported: bool,
    ) -> Result<Vec<Value>> {
        let raw_bubbles = frame
            .get("bubbles")
            .and_then(|v| v.as_array())
            .context("frame missing bubbles")?;
        let current_bubbles: Vec<Bubble> = serde_json::from_value(Value::Array(raw_bubbles.clone()))?;

        let conn = self.conn.lock().unwrap();
        let cursor_row: Option<String> = conn
            .query_row(
                "SELECT frame_fingerprints FROM contact_cursor WHERE contact_id=?",
                params![contact_id],
                |r| r.get(0),
            )
            .optional()?;

        let mut stored_fps: Option<Vec<Value>> = None;
        if let Some(ref text) = cursor_row {
            stored_fps = serde_json::from_str(text).ok();
        }

        if let Some(ref stored) = stored_fps {
            if stored.iter().any(|x| x.as_array().map(|a| a.len() == 2).unwrap_or(false)) {
                // v1 fingerprint reset
                let current_fps: Vec<policy::Fingerprint> = current_bubbles.iter().map(policy::fingerprint).collect();
                let fps_json = serde_json::to_string(&current_fps)?;
                conn.execute(
                    "UPDATE contact_cursor SET frame_fingerprints=?, updated_at=? WHERE contact_id=?",
                    params![fps_json, now_ms(), contact_id],
                )?;
                return Ok(vec![]);
            }
        }

        let plan: Vec<(Bubble, &'static str, Value)>;
        let mut current_reconciled = current_bubbles.clone();

        if stored_fps.is_none() {
            let meta = baseline_meta.cloned().unwrap_or(json!({}));
            plan = current_bubbles.into_iter().map(|b| (b, "baseline", meta.clone())).collect();
        } else {
            let stored = stored_fps.unwrap();
            let mut previous: Vec<Bubble> = Vec::new();
            for item in &stored {
                if let Some(arr) = item.as_array() {
                    let dir = arr.get(0).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let dig = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let ph = arr.get(2).and_then(|v| v.as_str()).map(|s| s.to_string());
                    previous.push(Bubble {
                        text: "".to_string(),
                        digest: dig,
                        direction: dir,
                        kind: None,
                        media_id: None,
                        phash: ph,
                        filename: None,
                        sender: None,
                        transfer_pending: None,
                        image_source: None,
                        x: None,
                        y: None,
                        width: None,
                        height: None,
                        extra: serde_json::Map::new(),
                    });
                }
            }
            current_reconciled = policy::reconcile(&previous, &current_bubbles);
            let seen: Vec<policy::Fingerprint> = current_reconciled.iter().map(policy::fingerprint).collect();

            match policy::new_bubbles(&previous, &current_reconciled) {
                Ok(new_b) => {
                    self.unstable.lock().unwrap().remove(contact_id);
                    plan = new_b.into_iter().map(|b| (b, "live", json!({}))).collect();
                }
                Err(policy::ContinuityError::AmbiguousAnchor { certain }) => {
                    let mut unstable_guard = self.unstable.lock().unwrap();
                    if unstable_guard.get(contact_id) != Some(&seen) {
                        unstable_guard.insert(contact_id.to_string(), seen);
                        return Ok(vec![]);
                    }
                    unstable_guard.remove(contact_id);

                    let old_fps: Vec<policy::Fingerprint> = previous.iter().map(policy::fingerprint).collect();
                    let mut max_k = 1;
                    for k in 1..=old_fps.len().min(seen.len()) {
                        if old_fps[old_fps.len() - k..] == seen[..k] {
                            max_k = k;
                        }
                    }
                    let end = current_reconciled.len() - certain.len();
                    let mut p = Vec::new();
                    if max_k <= end && end <= current_reconciled.len() {
                        for b in &current_reconciled[max_k..end] {
                            p.push((b.clone(), "ambiguous", json!({"note": "repeat count uncertain"})));
                        }
                    }
                    for b in certain {
                        p.push((b, "live", json!({})));
                    }
                    plan = p;
                }
                Err(policy::ContinuityError::ContinuityGap) => {
                    let mut unstable_guard = self.unstable.lock().unwrap();
                    if unstable_guard.get(contact_id) != Some(&seen) {
                        unstable_guard.insert(contact_id.to_string(), seen);
                        return Ok(vec![]);
                    }
                    unstable_guard.remove(contact_id);
                    plan = current_reconciled
                        .iter()
                        .map(|b| (b.clone(), "rebaseline_gap", json!({"may_duplicate": true})))
                        .collect();
                }
            }
        }

        let mut rows = Vec::new();
        for (bubble, capture, meta) in plan {
            if let Some(row) = self._append(&conn, contact_id, &bubble, capture, meta, batch_key, pair_imported)? {
                rows.push(row);
            }
        }

        let final_fps: Vec<policy::Fingerprint> = current_reconciled.iter().map(policy::fingerprint).collect();
        let fps_json = serde_json::to_string(&final_fps)?;
        let stamp = now_ms();
        conn.execute(
            "INSERT INTO contact_cursor VALUES(?,?,?)
             ON CONFLICT(contact_id) DO UPDATE SET frame_fingerprints=excluded.frame_fingerprints, updated_at=excluded.updated_at",
            params![contact_id, fps_json, stamp],
        )?;
        Ok(rows)
    }

    fn _append(
        &self,
        conn: &Connection,
        contact_id: &str,
        bubble: &Bubble,
        capture: &str,
        meta: Value,
        batch_key: Option<&str>,
        pair_imported: bool,
    ) -> Result<Option<Value>> {
        let inbound = bubble.direction == "inbound_visual";
        if !inbound && self._pair_echo_conn(conn, contact_id, bubble, pair_imported)? {
            return Ok(None);
        }

        let direction = if inbound { "inbound" } else { "outbound" };
        let sender = if inbound { "customer" } else { "self_other" };
        let kind = bubble.kind.as_deref().unwrap_or("text");

        let account_id = account_of_conn(conn, contact_id)?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE contact_id=?",
            params![contact_id],
            |r| r.get(0),
        )?;
        let message_id = new_id("m_");

        let mut meta_map = match meta {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        if let Ok(Value::Object(b_obj)) = serde_json::to_value(bubble) {
            for (k, v) in b_obj {
                if !matches!(k.as_str(), "text" | "digest" | "direction") && !v.is_null() {
                    meta_map.insert(k, v);
                }
            }
        }
        let meta_json = serde_json::to_string(&meta_map)?;

        conn.execute(
            "INSERT INTO messages(message_id,contact_id,account_id,seq,direction,sender,kind,text,observed_at,capture,bubble_digest,batch_key,meta)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                message_id,
                contact_id,
                account_id,
                seq,
                direction,
                sender,
                kind,
                bubble.text,
                now_ms(),
                capture,
                bubble.digest,
                batch_key,
                meta_json,
            ],
        )?;

        if kind == "file" {
            conn.execute(
                "INSERT INTO media_jobs(message_id,contact_id,status,attempts,updated_at) VALUES(?,?,'pending',0,0)",
                params![message_id, contact_id],
            )?;
        }

        Ok(Some(json!({
            "message_id": message_id,
            "seq": seq,
            "direction": direction,
            "sender": sender,
            "capture": capture,
            "text": bubble.text,
            "kind": kind,
            "bubble": serde_json::to_value(bubble)?,
        })))
    }

    fn _pair_echo_conn(
        &self,
        conn: &Connection,
        contact_id: &str,
        bubble: &Bubble,
        pair_imported: bool,
    ) -> Result<bool> {
        let (window_clause, cutoff): (&str, i64) = if pair_imported {
            ("(m.capture='imported')", 0)
        } else {
            ("(m.capture != 'imported' AND s.created_at >= ?)", now_ms() - ECHO_WINDOW_MS)
        };
        let sql = format!(
            "SELECT m.message_id, s.send_key, s.reply_text FROM messages m JOIN sends s ON s.send_key=m.send_key
             WHERE m.contact_id=? AND m.sender='bot' AND m.bubble_digest IS NULL AND s.status IN ('triggered','unknown')
             AND {} ORDER BY m.seq DESC",
            window_clause
        );

        let mut stmt = conn.prepare(&sql)?;
        let mut rows: Vec<(String, String, String)> = Vec::new();
        if pair_imported {
            let mapped = stmt.query_map(params![contact_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })?;
            for r in mapped {
                rows.push(r?);
            }
        } else {
            let mapped = stmt.query_map(params![contact_id, cutoff], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })?;
            for r in mapped {
                rows.push(r?);
            }
        }
        drop(stmt);

        for (index, (message_id, send_key, reply)) in rows.into_iter().enumerate() {
            if policy::echo_matches(&bubble.text, &reply) || (index == 0 && similar(&bubble.text, &reply, 0.85)) {
                conn.execute(
                    "UPDATE messages SET bubble_digest=? WHERE message_id=?",
                    params![bubble.digest, message_id],
                )?;
                conn.execute(
                    "UPDATE sends SET echo_observed=1, updated_at=? WHERE send_key=?",
                    params![now_ms(), send_key],
                )?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn match_recent_inbound(
        &self,
        contact_id: &str,
        bubbles: &[Bubble],
    ) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT message_id, bubble_digest, meta FROM messages WHERE contact_id=? AND direction='inbound' ORDER BY seq DESC LIMIT 200",
        )?;
        let rows: Vec<(String, Option<String>, String)> = stmt
            .query_map(params![contact_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);

        let mut found = Vec::new();
        let mut used = HashSet::new();

        for bubble in bubbles.iter().rev() {
            for (message_id, digest, meta_str) in &rows {
                if used.contains(message_id) || digest.as_deref() != Some(&bubble.digest) {
                    continue;
                }
                let meta_val: Value = serde_json::from_str(meta_str).unwrap_or(Value::Null);
                let stored_phash = meta_val.get("phash").and_then(|v| v.as_str());
                if let (Some(b_phash), Some(s_phash)) = (bubble.phash.as_deref(), stored_phash) {
                    if let Ok(dist) = policy::phash_distance(s_phash, b_phash) {
                        if dist > policy::PHASH_TOLERANCE {
                            continue;
                        }
                    }
                }
                used.insert(message_id.clone());
                found.push(message_id.clone());
                break;
            }
        }
        found.reverse();
        Ok(found)
    }

    pub fn upsert_profile(&self, account_id: &str, p: &ProfileInput, now_ms: i64) -> Result<UpsertOutcome> {
        let clean_wechat_id = p.wechat_id.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
        let clean_nickname = p.nickname.trim();
        let clean_remark = p.remark.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
        let category = &p.category;
        let identity = if clean_wechat_id.is_some() { "strong" } else { "weak" };
        let tags_json = serde_json::to_string(&p.tags).unwrap_or_else(|_| "[]".to_string());
        let disp_name = clean_remark.unwrap_or(clean_nickname);
        let kind = if category == "group" { "group" } else { "private" };

        let conn = self.conn.lock().unwrap();

        let record_review = |reason: &str, candidates: &[String]| -> Result<()> {
            let cand_json = serde_json::to_string(candidates).unwrap_or_else(|_| "[]".to_string());
            let prof_json = serde_json::to_string(p).unwrap_or_else(|_| "{}".to_string());
            conn.execute(
                "INSERT INTO contact_reviews(account_id, reason, candidates, profile, created_at, resolved_at) VALUES(?, ?, ?, ?, ?, NULL)",
                params![account_id, reason, cand_json, prof_json, now_ms],
            )?;
            Ok(())
        };

        let write_names = |target_cid: &str| -> Result<()> {
            _name_with_conn(&conn, target_cid, account_id, clean_nickname, "addressbook", now_ms)?;
            if let Some(r) = clean_remark {
                if r != clean_nickname {
                    _name_with_conn(&conn, target_cid, account_id, r, "addressbook", now_ms)?;
                }
            }
            Ok(())
        };

        let update_profile_row = |target_cid: &str| -> Result<()> {
            conn.execute(
                "UPDATE contacts SET display_name=?, updated_at=? WHERE contact_id=?",
                params![disp_name, now_ms, target_cid],
            )?;
            conn.execute(
                r#"UPDATE contact_profiles SET
                    category=?, wechat_id=?, nickname=?, remark=?, region=?, source=?,
                    added_on=?, common_groups=?, enterprise=?, real_name=?, tags=?,
                    friend_permission=?, identity=?, last_synced_at=?, removed_at=NULL
                   WHERE contact_id=?"#,
                params![
                    category,
                    clean_wechat_id,
                    clean_nickname,
                    clean_remark,
                    p.region.as_deref(),
                    p.source.as_deref(),
                    p.added_on.as_deref(),
                    p.common_groups,
                    p.enterprise.as_deref(),
                    p.real_name.as_deref(),
                    tags_json,
                    p.friend_permission.as_deref(),
                    identity,
                    now_ms,
                    target_cid,
                ],
            )?;
            write_names(target_cid)?;
            Ok(())
        };

        let insert_profile_row = |target_cid: &str, is_new_contact: bool| -> Result<()> {
            if is_new_contact {
                conn.execute(
                    "INSERT INTO contacts(contact_id, account_id, kind, display_name, merged_into, created_at, updated_at) VALUES(?,?,?,?,NULL,?,?)",
                    params![target_cid, account_id, kind, disp_name, now_ms, now_ms],
                )?;
            } else {
                conn.execute(
                    "UPDATE contacts SET display_name=?, updated_at=? WHERE contact_id=?",
                    params![disp_name, now_ms, target_cid],
                )?;
            }
            conn.execute(
                r#"INSERT INTO contact_profiles(
                    contact_id, account_id, wechat_id, category, nickname, remark, region, source,
                    added_on, common_groups, enterprise, real_name, tags, friend_permission,
                    identity, first_synced_at, last_synced_at, removed_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)"#,
                params![
                    target_cid,
                    account_id,
                    clean_wechat_id,
                    category,
                    clean_nickname,
                    clean_remark,
                    p.region.as_deref(),
                    p.source.as_deref(),
                    p.added_on.as_deref(),
                    p.common_groups,
                    p.enterprise.as_deref(),
                    p.real_name.as_deref(),
                    tags_json,
                    p.friend_permission.as_deref(),
                    identity,
                    now_ms,
                    now_ms,
                ],
            )?;
            write_names(target_cid)?;
            Ok(())
        };

        if let Some(wid) = clean_wechat_id {
            let existing_by_wxid: Option<String> = conn
                .query_row(
                    "SELECT contact_id FROM contact_profiles WHERE account_id=? AND wechat_id=?",
                    params![account_id, wid],
                    |r| r.get(0),
                )
                .optional()?;

            if let Some(cid) = existing_by_wxid {
                update_profile_row(&cid)?;
                return Ok(UpsertOutcome::Updated(cid));
            }

            let mut candidate_cids = HashSet::new();
            for name in [Some(clean_nickname), clean_remark].into_iter().flatten() {
                let owners = _owners_conn(&conn, account_id, &name_key(name), &["config", "alias", "observed", "addressbook"], true)?;
                candidate_cids.extend(owners);
            }

            if candidate_cids.len() > 1 {
                let mut candidates: Vec<String> = candidate_cids.into_iter().collect();
                candidates.sort();
                record_review("ambiguous_name", &candidates)?;
                return Ok(UpsertOutcome::NeedsReview {
                    reason: "ambiguous_name".to_string(),
                    candidates,
                });
            }

            if candidate_cids.len() == 1 {
                let target_cid = candidate_cids.into_iter().next().unwrap();
                let existing_prof_wxid: Option<Option<String>> = conn
                    .query_row(
                        "SELECT wechat_id FROM contact_profiles WHERE contact_id=?",
                        params![&target_cid],
                        |r| r.get(0),
                    )
                    .optional()?;

                match existing_prof_wxid {
                    Some(Some(old_wxid)) if old_wxid != wid => {
                        record_review("wechat_id_conflict", &[target_cid.clone()])?;
                        return Ok(UpsertOutcome::NeedsReview {
                            reason: "wechat_id_conflict".to_string(),
                            candidates: vec![target_cid],
                        });
                    }
                    Some(_) => {
                        update_profile_row(&target_cid)?;
                        return Ok(UpsertOutcome::Updated(target_cid));
                    }
                    None => {
                        insert_profile_row(&target_cid, false)?;
                        return Ok(UpsertOutcome::Updated(target_cid));
                    }
                }
            }

            let cid = new_id("c_");
            insert_profile_row(&cid, true)?;
            Ok(UpsertOutcome::Created(cid))
        } else {
            let mut stmt = conn.prepare(
                "SELECT contact_id, enterprise FROM contact_profiles WHERE account_id=? AND category=? AND nickname=? AND (wechat_id IS NULL OR wechat_id='')"
            )?;
            let weak_rows = stmt.query_map(params![account_id, category, clean_nickname], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?.filter_map(Result::ok).collect::<Vec<_>>();
            drop(stmt);

            let matched_weak = weak_rows.into_iter().find(|(_, ent)| {
                match (&p.enterprise, ent) {
                    (Some(e1), Some(e2)) => e1 == e2,
                    (None, None) => true,
                    _ => false,
                }
            });

            if let Some((cid, _)) = matched_weak {
                update_profile_row(&cid)?;
                return Ok(UpsertOutcome::Updated(cid));
            }

            let mut candidate_cids = HashSet::new();
            for name in [Some(clean_nickname), clean_remark].into_iter().flatten() {
                let owners = _owners_conn(&conn, account_id, &name_key(name), &["config", "alias", "observed", "addressbook"], true)?;
                candidate_cids.extend(owners);
            }

            if candidate_cids.len() > 1 {
                let mut candidates: Vec<String> = candidate_cids.into_iter().collect();
                candidates.sort();
                record_review("ambiguous_name", &candidates)?;
                return Ok(UpsertOutcome::NeedsReview {
                    reason: "ambiguous_name".to_string(),
                    candidates,
                });
            }

            if candidate_cids.len() == 1 {
                let target_cid = candidate_cids.into_iter().next().unwrap();
                let has_profile: bool = conn
                    .query_row(
                        "SELECT 1 FROM contact_profiles WHERE contact_id=?",
                        params![&target_cid],
                        |_| Ok(true),
                    )
                    .optional()?
                    .unwrap_or(false);

                if has_profile {
                    update_profile_row(&target_cid)?;
                } else {
                    insert_profile_row(&target_cid, false)?;
                }
                return Ok(UpsertOutcome::Updated(target_cid));
            }

            let cid = new_id("c_");
            insert_profile_row(&cid, true)?;
            Ok(UpsertOutcome::Created(cid))
        }
    }

    pub fn mark_missing(&self, account_id: &str, category: &str, seen_contact_ids: &[String], now_ms: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        if seen_contact_ids.is_empty() {
            let count = conn.execute(
                "UPDATE contact_profiles SET removed_at=? WHERE account_id=? AND category=? AND removed_at IS NULL",
                params![now_ms, account_id, category],
            )?;
            return Ok(count);
        }
        let marks = seen_contact_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE contact_profiles SET removed_at=? WHERE account_id=? AND category=? AND removed_at IS NULL AND contact_id NOT IN ({})",
            marks
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
        params_vec.push(&now_ms);
        params_vec.push(&account_id);
        params_vec.push(&category);
        for id in seen_contact_ids {
            params_vec.push(id);
        }
        let count = conn.execute(&sql, rusqlite::params_from_iter(params_vec))?;
        Ok(count)
    }

    pub fn list_profiles(&self, account_id: &str, category: Option<&str>) -> Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let map_row = |r: &rusqlite::Row| -> rusqlite::Result<Value> {
            let tags_str: String = r.get(12)?;
            let tags_val: Value = serde_json::from_str(&tags_str).unwrap_or_else(|_| json!([]));
            Ok(json!({
                "contact_id": r.get::<_, String>(0)?,
                "account_id": r.get::<_, String>(1)?,
                "wechat_id": r.get::<_, Option<String>>(2)?,
                "category": r.get::<_, String>(3)?,
                "nickname": r.get::<_, String>(4)?,
                "remark": r.get::<_, Option<String>>(5)?,
                "region": r.get::<_, Option<String>>(6)?,
                "source": r.get::<_, Option<String>>(7)?,
                "added_on": r.get::<_, Option<String>>(8)?,
                "common_groups": r.get::<_, Option<i64>>(9)?,
                "enterprise": r.get::<_, Option<String>>(10)?,
                "real_name": r.get::<_, Option<String>>(11)?,
                "tags": tags_val,
                "friend_permission": r.get::<_, Option<String>>(13)?,
                "identity": r.get::<_, String>(14)?,
                "first_synced_at": r.get::<_, i64>(15)?,
                "last_synced_at": r.get::<_, i64>(16)?,
                "removed_at": r.get::<_, Option<i64>>(17)?,
            }))
        };

        let mut list = Vec::new();
        if let Some(cat) = category {
            let mut stmt = conn.prepare(
                "SELECT contact_id, account_id, wechat_id, category, nickname, remark, region, source, added_on, common_groups, enterprise, real_name, tags, friend_permission, identity, first_synced_at, last_synced_at, removed_at FROM contact_profiles WHERE account_id=? AND category=? ORDER BY first_synced_at ASC"
            )?;
            let rows = stmt.query_map(params![account_id, cat], map_row)?;
            for row in rows {
                list.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT contact_id, account_id, wechat_id, category, nickname, remark, region, source, added_on, common_groups, enterprise, real_name, tags, friend_permission, identity, first_synced_at, last_synced_at, removed_at FROM contact_profiles WHERE account_id=? ORDER BY first_synced_at ASC"
            )?;
            let rows = stmt.query_map(params![account_id], map_row)?;
            for row in rows {
                list.push(row?);
            }
        }
        Ok(list)
    }

    pub fn pending_reviews(&self, account_id: &str) -> Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, reason, candidates, profile, created_at FROM contact_reviews WHERE account_id=? AND resolved_at IS NULL ORDER BY id ASC"
        )?;
        let rows = stmt.query_map([account_id], |r| {
            let cands_str: String = r.get(3)?;
            let prof_str: String = r.get(4)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "account_id": r.get::<_, String>(1)?,
                "reason": r.get::<_, String>(2)?,
                "candidates": serde_json::from_str::<Value>(&cands_str).unwrap_or_else(|_| json!([])),
                "profile": serde_json::from_str::<Value>(&prof_str).unwrap_or_else(|_| json!({})),
                "created_at": r.get::<_, i64>(5)?,
            }))
        })?;
        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }
}

pub fn compact(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let nfkc: String = text.nfkc().collect();
    nfkc.chars().filter(|c| !c.is_whitespace()).collect()
}

pub fn similar(observed: &str, expected: &str, threshold: f64) -> bool {
    let a: Vec<char> = compact(observed).chars().collect();
    let b: Vec<char> = compact(expected).chars().collect();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if (a.len() as f64) < 0.6 * (b.len() as f64) {
        return false;
    }
    let b_prefix: &[char] = if a.len() < b.len() { &b[..a.len()] } else { &b[..] };
    let r1 = sequence_matcher_ratio(&a, &b);
    let r2 = sequence_matcher_ratio(&a, b_prefix);
    r1.max(r2) >= threshold
}

fn sequence_matcher_ratio(a: &[char], b: &[char]) -> f64 {
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    fn find_blocks(a: &[char], a_lo: usize, a_hi: usize, b: &[char], b_lo: usize, b_hi: usize) -> usize {
        let mut best_i = a_lo;
        let mut best_j = b_lo;
        let mut best_size = 0;
        for i in a_lo..a_hi {
            for j in b_lo..b_hi {
                let mut k = 0;
                while i + k < a_hi && j + k < b_hi && a[i + k] == b[j + k] {
                    k += 1;
                }
                if k > best_size {
                    best_i = i;
                    best_j = j;
                    best_size = k;
                }
            }
        }
        if best_size == 0 {
            return 0;
        }
        best_size
            + find_blocks(a, a_lo, best_i, b, b_lo, best_j)
            + find_blocks(a, best_i + best_size, a_hi, b, best_j + best_size, b_hi)
    }
    let matches = find_blocks(a, 0, a.len(), b, 0, b.len());
    2.0 * (matches as f64) / (total as f64)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileInput {
    pub category: String,
    pub wechat_id: Option<String>,
    pub nickname: String,
    pub remark: Option<String>,
    pub region: Option<String>,
    pub source: Option<String>,
    pub added_on: Option<String>,
    pub common_groups: Option<i64>,
    pub enterprise: Option<String>,
    pub real_name: Option<String>,
    pub tags: Vec<String>,
    pub friend_permission: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpsertOutcome {
    Created(String),
    Updated(String),
    NeedsReview {
        reason: String,
        candidates: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestTempDir(PathBuf);
    impl TestTempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("wechat-store-test-{}", uuid::Uuid::new_v4().simple()));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct StoreFixture {
        _temp: TestTempDir,
        state: PathBuf,
        store: HistoryStore,
        c: String,
    }

    fn bub(text: &str, direction: &str, kind: &str) -> Bubble {
        Bubble {
            text: text.to_string(),
            digest: text.to_string(),
            direction: direction.to_string(),
            kind: Some(kind.to_string()),
            media_id: None,
            phash: None,
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: serde_json::Map::new(),
        }
    }

    fn fr(bubbles: Vec<Bubble>) -> Value {
        json!({
            "bubbles": bubbles
        })
    }

    fn setup_fixture() -> StoreFixture {
        let temp = TestTempDir::new();
        let state = temp.path().join("state");
        let store = HistoryStore::open(&state).unwrap();
        let c = store
            .ensure_contact("acct", &json!({"recipient": "客户", "kind": "private"}), true)
            .unwrap();
        StoreFixture {
            _temp: temp,
            state,
            store,
            c,
        }
    }

    fn rows(f: &StoreFixture, col: &str) -> Vec<String> {
        let sql = format!("SELECT {} FROM messages WHERE contact_id=? ORDER BY seq", col);
        let conn = f.store.connection();
        let guard = conn.lock().unwrap();
        let mut stmt = guard.prepare(&sql).unwrap();
        let mapped = stmt.query_map([&f.c], |r| r.get::<_, String>(0)).unwrap();
        mapped.filter_map(Result::ok).collect()
    }

    fn texts(f: &StoreFixture) -> Vec<String> { rows(f, "text") }
    fn captures(f: &StoreFixture) -> Vec<String> { rows(f, "capture") }
    fn senders(f: &StoreFixture) -> Vec<String> { rows(f, "sender") }

    #[test]
    fn test_store_legacy_history_cursor_resets_without_duplicates() {
        let f = setup_fixture();
        let frame = fr(vec![bub("a", "inbound_visual", "text"), bub("b", "inbound_visual", "text"), bub("c", "inbound_visual", "text")]);
        f.store.connection().lock().unwrap().execute(
            "INSERT INTO contact_cursor VALUES(?,?,?)",
            params![f.c, json!([["inbound_visual", "a"], ["inbound_visual", "b"]]).to_string(), 0],
        ).unwrap();
        let res = f.store.record_frame(&f.c, &frame, None, None, false).unwrap();
        assert_eq!(res.len(), 0);
        assert_eq!(texts(&f).len(), 0);
        let stored_str: String = f.store.connection().lock().unwrap().query_row(
            "SELECT frame_fingerprints FROM contact_cursor",
            [],
            |r| r.get(0),
        ).unwrap();
        let stored: Vec<Value> = serde_json::from_str(&stored_str).unwrap();
        assert_eq!(stored[0].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_store_same_size_older_image_is_not_matched_to_a_new_one() {
        let f = setup_fixture();
        let img = |h: &str| Bubble {
            text: "[图片]".to_string(),
            digest: "image:50:40".to_string(),
            direction: "inbound_visual".to_string(),
            kind: Some("image".to_string()),
            media_id: None,
            phash: Some(h.to_string()),
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: serde_json::Map::new(),
        };
        f.store.record_frame(&f.c, &fr(vec![bub("a", "inbound_visual", "text")]), None, None, false).unwrap();
        let old = f.store.record_frame(&f.c, &fr(vec![bub("a", "inbound_visual", "text"), img("0000000000000000")]), None, None, false).unwrap()[0]["message_id"].as_str().unwrap().to_string();
        assert_eq!(f.store.match_recent_inbound(&f.c, &[img("ffffffffffffffff")]).unwrap(), Vec::<String>::new());
        assert_eq!(f.store.match_recent_inbound(&f.c, &[img("000000000000000f")]).unwrap(), vec![old]);
    }

    #[test]
    fn test_store_reconciled_image_is_not_recorded_twice() {
        let f = setup_fixture();
        let img = |h: &str| Bubble {
            text: "[图片]".to_string(),
            digest: "image:50:40".to_string(),
            direction: "inbound_visual".to_string(),
            kind: Some("image".to_string()),
            media_id: Some(h.repeat(4)),
            phash: Some(h.to_string()),
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: serde_json::Map::new(),
        };
        f.store.record_frame(&f.c, &fr(vec![bub("a", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("a", "inbound_visual", "text"), img("0f0f0f0f0f0f0f0f")]), None, None, false).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("a", "inbound_visual", "text"), img("0f0f0f0f0f0f0f00")]), None, None, false).unwrap();
        assert_eq!(texts(&f), vec!["a", "[图片]"]);
    }

    #[test]
    fn test_store_migration_adds_understanding_columns() {
        let f = setup_fixture();
        let conn = f.store.connection();
        let guard = conn.lock().unwrap();
        let mut stmt = guard.prepare("PRAGMA table_info(media)").unwrap();
        let cols: HashSet<String> = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap().filter_map(Result::ok).collect();
        assert!(cols.contains("understanding"));
        assert!(cols.contains("understood_at"));
    }

    #[test]
    fn test_store_recent_context_includes_media_understanding() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text")]), None, None, false).unwrap();
        let img = Bubble {
            text: "[图片]".to_string(),
            digest: "image:1:1".to_string(),
            phash: Some("0".repeat(16)),
            direction: "inbound_visual".to_string(),
            kind: Some("image".to_string()),
            media_id: None,
            filename: None,
            sender: None,
            transfer_pending: None,
            image_source: None,
            x: None,
            y: None,
            width: None,
            height: None,
            extra: serde_json::Map::new(),
        };
        let rows = f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text"), img]), None, None, false).unwrap();
        let source = f._temp.path().join("a.png");
        fs::write(&source, b"\x89PNG\r\n\x1a\nx").unwrap();
        let mid = rows[0]["message_id"].as_str().unwrap();
        let media_id = f.store.attach_media(mid, &source, "image", None, "original", None).unwrap();

        let context = f.store.recent_context(&f.c, 5, &HashSet::new()).unwrap();
        let last_media = &context.last().unwrap()["media"];
        assert!(last_media["understanding"].is_null());
        let media_path = last_media["path"].as_str().unwrap();
        assert!(Path::new(media_path).is_file());

        let long_text = "一张快递单".repeat(300);
        f.store.set_understanding(&media_id, &long_text).unwrap();
        let context2 = f.store.recent_context(&f.c, 5, &HashSet::new()).unwrap();
        let understanding = context2.last().unwrap()["media"]["understanding"].as_str().unwrap();
        assert_eq!(understanding.chars().count(), 1000);
        assert!(context2[0].get("media").is_none());
    }

    #[test]
    fn test_store_schema_created_private_and_wal() {
        let f = setup_fixture();
        let db = f.state.join("history").join("history.sqlite3");
        let meta = fs::metadata(&db).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let dir_meta = fs::metadata(f.state.join("history")).unwrap();
        assert_eq!(dir_meta.permissions().mode() & 0o777, 0o700);

        let journal_mode: String = f.store.connection().lock().unwrap().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let conn = f.store.connection();
        let guard = conn.lock().unwrap();
        let mut stmt = guard.prepare("SELECT name FROM sqlite_master").unwrap();
        let tables: HashSet<String> = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().filter_map(Result::ok).collect();
        for t in &["contacts", "contact_names", "messages", "sends", "media", "media_jobs", "contact_cursor", "merges", "schema_migrations", "messages_fts"] {
            assert!(tables.contains(*t), "missing table {}", t);
        }
    }

    #[test]
    fn test_store_reopen_keeps_data() {
        let f = setup_fixture();
        drop(f.store);
        let store2 = HistoryStore::open(&f.state).unwrap();
        assert_eq!(store2.resolve("acct", "客户").unwrap(), Some(f.c));
    }

    #[test]
    fn test_store_ensure_contact_is_stable_and_alias_aware() {
        let f = setup_fixture();
        let a = f.store.ensure_contact("acct", &json!({"recipient": "🍓小美", "kind": "private", "aliases": ["@小美"]}), true).unwrap();
        assert!(a.starts_with("c_") && a.len() == 18);
        assert_eq!(f.store.ensure_contact("acct", &json!({"recipient": "🍓小美", "kind": "private"}), true).unwrap(), a);
        assert_eq!(f.store.resolve("acct", "小美").unwrap(), Some(a.clone()));
        assert_eq!(f.store.resolve("acct", "@小美").unwrap(), Some(a.clone()));
        assert_eq!(f.store.resolve("other-acct", "小美").unwrap(), None);
    }

    #[test]
    fn test_store_rename_keeps_contact_when_old_name_is_alias() {
        let f = setup_fixture();
        let a = f.store.ensure_contact("acct", &json!({"recipient": "旧名", "kind": "private"}), true).unwrap();
        assert_eq!(f.store.ensure_contact("acct", &json!({"recipient": "新名", "kind": "private", "aliases": ["旧名"]}), true).unwrap(), a);
        assert_eq!(f.store.resolve("acct", "新名").unwrap(), Some(a.clone()));
        let display: String = f.store.connection().lock().unwrap().query_row("SELECT display_name FROM contacts WHERE contact_id=?", [&a], |r| r.get(0)).unwrap();
        assert_eq!(display, "新名");
    }

    #[test]
    fn test_store_merge_follows_and_unmerge_restores() {
        let f = setup_fixture();
        let a = f.store.ensure_contact("acct", &json!({"recipient": "甲", "kind": "private"}), true).unwrap();
        let b = f.store.ensure_contact("acct", &json!({"recipient": "乙", "kind": "private"}), true).unwrap();
        let mid = f.store.merge(&b, &a, "改名").unwrap();
        assert_eq!(f.store.resolve("acct", "乙").unwrap(), Some(a.clone()));
        assert_eq!(f.store.canonical(b.clone()).unwrap(), a);
        f.store.unmerge(mid).unwrap();
        assert_eq!(f.store.resolve("acct", "乙").unwrap(), Some(b));
    }

    #[test]
    fn test_store_conflicting_name_key_rejected() {
        let f = setup_fixture();
        f.store.ensure_contact("acct", &json!({"recipient": "小美", "kind": "private"}), true).unwrap();
        f.store.ensure_contact("acct", &json!({"recipient": "别人", "kind": "private"}), true).unwrap();
        let res = f.store.ensure_contact("acct", &json!({"recipient": "别人", "kind": "private", "aliases": ["🍓小美"]}), true);
        assert!(res.is_err());
    }

    #[test]
    fn test_store_observe_name_reports_known() {
        let f = setup_fixture();
        assert_eq!(f.store.observe_name(&f.c, "客户").unwrap(), true);
        assert_eq!(f.store.observe_name(&f.c, "客户✨新").unwrap(), false);
        let src: String = f.store.connection().lock().unwrap().query_row("SELECT source FROM contact_names WHERE raw_name='客户✨新'", [], |r| r.get(0)).unwrap();
        assert_eq!(src, "observed");
    }

    #[test]
    fn test_store_first_frame_is_baseline_then_live_increment() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("a", "inbound_visual", "text"), bub("b", "outbound_visual", "text")]), None, None, false).unwrap();
        let rows = f.store.record_frame(&f.c, &fr(vec![bub("a", "inbound_visual", "text"), bub("b", "outbound_visual", "text"), bub("c", "inbound_visual", "text")]), None, None, false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["text"].as_str().unwrap(), "c");
        assert_eq!(rows[0]["capture"].as_str().unwrap(), "live");
        assert_eq!(rows[0]["sender"].as_str().unwrap(), "customer");
        assert_eq!(captures(&f), vec!["baseline", "baseline", "live"]);
        assert_eq!(senders(&f), vec!["customer", "self_other", "customer"]);
    }

    #[test]
    fn test_store_identical_repeats_are_two_messages() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("x", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("x", "inbound_visual", "text"), bub("好", "inbound_visual", "text"), bub("好", "inbound_visual", "text")]), None, None, false).unwrap();
        assert_eq!(texts(&f), vec!["x", "好", "好"]);
    }

    #[test]
    fn test_store_replaying_same_frame_after_crash_writes_nothing() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("x", "inbound_visual", "text")]), None, None, false).unwrap();
        let frame = fr(vec![bub("x", "inbound_visual", "text"), bub("y", "inbound_visual", "text")]);
        f.store.record_frame(&f.c, &frame, None, None, false).unwrap();
        let empty = f.store.record_frame(&f.c, &frame, None, None, false).unwrap();
        assert_eq!(empty.len(), 0);
        assert_eq!(texts(&f), vec!["x", "y"]);
    }

    #[test]
    fn test_store_ambiguous_and_gap_are_kept_and_marked() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("A", "inbound_visual", "text")]), None, None, false).unwrap();
        for _ in 0..2 {
            f.store.record_frame(&f.c, &fr(vec![bub("A", "inbound_visual", "text"), bub("A", "inbound_visual", "text")]), None, None, false).unwrap();
        }
        assert_eq!(captures(&f).last().unwrap(), "ambiguous");
        for _ in 0..2 {
            f.store.record_frame(&f.c, &fr(vec![bub("完全不同", "inbound_visual", "text")]), None, None, false).unwrap();
        }
        assert_eq!(captures(&f).last().unwrap(), "rebaseline_gap");
        assert_eq!(texts(&f), vec!["A", "A", "完全不同"]);
    }

    #[test]
    fn test_store_single_jittery_gap_frame_writes_nothing() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("x", "inbound_visual", "text"), bub("y", "inbound_visual", "text")]), None, None, false).unwrap();
        let res = f.store.record_frame(&f.c, &fr(vec![bub("滚动中", "inbound_visual", "text")]), None, None, false).unwrap();
        assert_eq!(res.len(), 0);
        f.store.record_frame(&f.c, &fr(vec![bub("x", "inbound_visual", "text"), bub("y", "inbound_visual", "text"), bub("z", "inbound_visual", "text")]), None, None, false).unwrap();
        assert_eq!(texts(&f), vec!["x", "y", "z"]);
        assert_eq!(captures(&f), vec!["baseline", "baseline", "live"]);
    }

    #[test]
    fn test_store_ambiguous_certain_tail_is_live() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("A", "inbound_visual", "text"), bub("A", "inbound_visual", "text")]), None, None, false).unwrap();
        for _ in 0..2 {
            f.store.record_frame(&f.c, &fr(vec![bub("A", "inbound_visual", "text"), bub("A", "inbound_visual", "text"), bub("next", "inbound_visual", "text")]), None, None, false).unwrap();
        }
        assert_eq!(captures(&f).last().unwrap(), "live");
        assert_eq!(texts(&f).last().unwrap(), "next");
    }

    #[test]
    fn test_store_bot_reply_echo_is_paired_not_duplicated() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_bot_reply(&f.c, "bot-service-k", "你好～", "attempting", &json!({}), &[], None, None).unwrap();
        f.store.update_send("bot-service-k", "triggered", None).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text"), bub("你好~", "outbound_visual", "text")]), None, None, false).unwrap();
        assert_eq!(senders(&f), vec!["customer", "bot"]);
        assert_eq!(f.store.send("bot-service-k").unwrap().unwrap()["echo_observed"].as_bool().unwrap(), true);
    }

    #[test]
    fn test_store_bot_reply_is_idempotent() {
        let f = setup_fixture();
        let a = f.store.record_bot_reply(&f.c, "k", "回复", "attempting", &json!({}), &[], None, None).unwrap();
        assert_eq!(f.store.record_bot_reply(&f.c, "k", "回复", "attempting", &json!({}), &[], None, None).unwrap(), a);
        assert_eq!(senders(&f), vec!["bot"]);
    }

    #[test]
    fn test_store_ocr_noisy_echo_still_pairs() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_bot_reply(&f.c, "k", "在的，我是AI客服助手。有什么可以帮您？我这边暂时没有订单信息。", "attempting", &json!({}), &[], None, None).unwrap();
        f.store.update_send("k", "triggered", None).unwrap();
        f.store.record_frame(&f.c, &fr(vec![
            bub("在吗", "inbound_visual", "text"),
            bub("在的，我是 Al客服助手。有什么可以帮您？我这边暂\n时没有订单信息", "outbound_visual", "text")
        ]), None, None, false).unwrap();
        assert_eq!(senders(&f), vec!["customer", "bot"]);
        assert_eq!(f.store.send("k").unwrap().unwrap()["echo_observed"].as_bool().unwrap(), true);
    }

    #[test]
    fn test_store_different_outbound_after_bot_reply_is_self_other() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_bot_reply(&f.c, "k", "在的，我是AI客服助手。", "attempting", &json!({}), &[], None, None).unwrap();
        f.store.update_send("k", "triggered", None).unwrap();
        f.store.record_frame(&f.c, &fr(vec![
            bub("在吗", "inbound_visual", "text"),
            bub("人工：订单已经发顺丰了", "outbound_visual", "text")
        ]), None, None, false).unwrap();
        assert_eq!(senders(&f), vec!["customer", "bot", "self_other"]);
    }

    #[test]
    fn test_store_replied_after_sees_manual_reply() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text"), bub("图片呢", "inbound_visual", "text")]), None, None, false).unwrap();
        let ids: Vec<String> = f.store.connection().lock().unwrap().prepare("SELECT message_id FROM messages WHERE text='图片呢'").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect();
        assert_eq!(f.store.replied_after(&f.c, &ids, None).unwrap(), false);
        f.store.record_frame(&f.c, &fr(vec![
            bub("在吗", "inbound_visual", "text"),
            bub("图片呢", "inbound_visual", "text"),
            bub("人工：收到", "outbound_visual", "text"),
        ]), None, None, false).unwrap();
        assert_eq!(f.store.replied_after(&f.c, &ids, None).unwrap(), true);
        assert_eq!(f.store.replied_after(&f.c, &[], None).unwrap(), false);
    }

    #[test]
    fn test_store_replied_after_ignores_failed_bot_send() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text"), bub("图片呢", "inbound_visual", "text")]), None, None, false).unwrap();
        let ids: Vec<String> = f.store.connection().lock().unwrap().prepare("SELECT message_id FROM messages WHERE text='图片呢'").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect();
        f.store.record_bot_reply(&f.c, "k", "在的", "generated", &json!({}), &ids, None, None).unwrap();
        assert_eq!(f.store.replied_after(&f.c, &ids, None).unwrap(), false);
        f.store.update_send("k", "triggered", None).unwrap();
        assert_eq!(f.store.replied_after(&f.c, &ids, None).unwrap(), true);
    }

    #[test]
    fn test_store_replied_after_ignores_own_send_and_not_sent() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("在吗", "inbound_visual", "text")]), None, None, false).unwrap();
        let ids: Vec<String> = f.store.connection().lock().unwrap().prepare("SELECT message_id FROM messages WHERE text='在吗'").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect();
        f.store.record_bot_reply(&f.c, "k", "在的", "generated", &json!({}), &ids, None, None).unwrap();
        f.store.update_send("k", "attempting", None).unwrap();
        assert_eq!(f.store.replied_after(&f.c, &ids, None).unwrap(), true);
        assert_eq!(f.store.replied_after(&f.c, &ids, Some("k")).unwrap(), false);
        f.store.update_send("k", "not_sent", None).unwrap();
        assert_eq!(f.store.replied_after(&f.c, &ids, None).unwrap(), false);
    }

    #[test]
    fn test_store_old_or_imported_reply_is_not_echo_target() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_bot_reply(&f.c, "old", "好的，稍等", "triggered", &json!({}), &[], None, None).unwrap();
        f.store.connection().lock().unwrap().execute("UPDATE sends SET created_at=created_at-11*60*1000 WHERE send_key=?", ["old"]).unwrap();
        f.store.record_bot_reply(&f.c, "imp", "已发货", "triggered", &json!({}), &[], None, Some("imported")).unwrap();
        f.store.record_frame(&f.c, &fr(vec![
            bub("问", "inbound_visual", "text"),
            bub("好的，稍等", "outbound_visual", "text"),
            bub("已发货", "outbound_visual", "text")
        ]), None, None, false).unwrap();
        assert_eq!(senders(&f), vec!["customer", "bot", "bot", "self_other", "self_other"]);
    }

    #[test]
    fn test_store_unpaired_outbound_is_self_other() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text"), bub("人工回复", "outbound_visual", "text")]), None, None, false).unwrap();
        assert_eq!(senders(&f).last().unwrap(), "self_other");
    }

    #[test]
    fn test_store_superseded_reply_is_not_echo_target() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_bot_reply(&f.c, "k", "好的", "generated", &json!({}), &[], None, None).unwrap();
        f.store.update_send("k", "superseded", None).unwrap();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text"), bub("好的", "outbound_visual", "text")]), None, None, false).unwrap();
        assert_eq!(senders(&f), vec!["customer", "bot", "self_other"]);
    }

    #[test]
    fn test_store_recent_context_limit_order_and_exclusions() {
        let f = setup_fixture();
        let bubbles: Vec<Bubble> = (0..8).map(|i| bub(&i.to_string(), "inbound_visual", "text")).collect();
        f.store.record_frame(&f.c, &fr(bubbles), None, None, false).unwrap();
        f.store.record_bot_reply(&f.c, "k1", "作废", "generated", &json!({}), &[], None, None).unwrap();
        f.store.update_send("k1", "superseded", None).unwrap();
        let ctx = f.store.recent_context(&f.c, 5, &HashSet::new()).unwrap();
        let txts: Vec<&str> = ctx.iter().map(|x| x["text"].as_str().unwrap()).collect();
        assert_eq!(txts, vec!["3", "4", "5", "6", "7"]);
        assert!(ctx.iter().all(|x| x["role"].as_str().unwrap() == "user"));
    }

    #[test]
    fn test_store_recent_context_roles_exclude_ids_and_merged() {
        let f = setup_fixture();
        let other = f.store.ensure_contact("acct", &json!({"recipient": "旧号", "kind": "private"}), true).unwrap();
        f.store.record_frame(&other, &fr(vec![bub("旧问题", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.merge(&other, &f.c, "").unwrap();
        let rows = f.store.record_frame(&f.c, &fr(vec![
            bub("问", "inbound_visual", "text"),
            bub("答", "outbound_visual", "text"),
            bub("新", "inbound_visual", "text"),
        ]), None, None, false).unwrap();
        let mut excluded = HashSet::new();
        excluded.insert(rows.last().unwrap()["message_id"].as_str().unwrap().to_string());
        let ctx = f.store.recent_context(&f.c, 10, &excluded).unwrap();
        let roles_texts: Vec<(&str, &str)> = ctx.iter().map(|x| (x["role"].as_str().unwrap(), x["text"].as_str().unwrap())).collect();
        assert_eq!(roles_texts, vec![("user", "旧问题"), ("user", "问"), ("assistant", "答")]);
    }

    #[test]
    fn test_store_recent_context_excludes_never_attempted_reply() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("问", "inbound_visual", "text")]), None, None, false).unwrap();
        f.store.record_bot_reply(&f.c, "g", "未发出", "generated", &json!({}), &[], None, None).unwrap();
        f.store.record_bot_reply(&f.c, "u", "可能已发出", "unknown", &json!({}), &[], None, None).unwrap();
        let ctx = f.store.recent_context(&f.c, 5, &HashSet::new()).unwrap();
        let txts: Vec<&str> = ctx.iter().map(|x| x["text"].as_str().unwrap()).collect();
        assert_eq!(txts, vec!["问", "可能已发出"]);
    }

    #[test]
    fn test_store_contact_time_index_exists() {
        let f = setup_fixture();
        let conn = f.store.connection();
        let guard = conn.lock().unwrap();
        let mut stmt = guard.prepare("PRAGMA index_list('messages')").unwrap();
        let names: HashSet<String> = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap().filter_map(Result::ok).collect();
        assert!(names.contains("messages_contact_time"));
    }

    #[test]
    fn test_store_fts_trigger_indexes_text() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("还是上次那个款", "inbound_visual", "text")]), None, None, false).unwrap();
        let hit: i64 = f.store.connection().lock().unwrap().query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts match ?",
            ["\"上次那\""],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(hit, 1);
    }

    #[test]
    fn test_store_attach_media_content_addressed_and_private() {
        let f = setup_fixture();
        let rows = f.store.record_frame(&f.c, &fr(vec![bub("[图片]", "inbound_visual", "image")]), None, None, false).unwrap();
        let row = &rows[0];
        let src = f._temp.path().join("x.png");
        fs::write(&src, b"png").unwrap();
        let mid = f.store.attach_media(row["message_id"].as_str().unwrap(), &src, "image", None, "chat_thumbnail", None).unwrap();
        let path = f.state.join("history").join("media").join(&mid[..2]).join(format!("{}.png", mid));
        assert_eq!(fs::read(&path).unwrap(), b"png");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let linked: String = f.store.connection().lock().unwrap().query_row("SELECT media_id FROM messages WHERE message_id=?", [row["message_id"].as_str().unwrap()], |r| r.get(0)).unwrap();
        assert_eq!(linked, mid);
    }

    #[test]
    fn test_store_oversized_media_rejected() {
        let f = setup_fixture();
        let rows = f.store.record_frame(&f.c, &fr(vec![bub("big.pdf", "inbound_visual", "file")]), None, None, false).unwrap();
        let row = &rows[0];
        let src = f._temp.path().join("big.pdf");
        fs::write(&src, vec![b'0'; 10 * 1024 * 1024 + 1]).unwrap();
        let res = f.store.attach_media(row["message_id"].as_str().unwrap(), &src, "file", Some("big.pdf"), "original", Some(10 * 1024 * 1024));
        assert!(res.is_err());
    }

    #[test]
    fn test_store_attach_media_follows_configurable_limit_and_names_it() {
        let f = setup_fixture();
        let rows = f.store.record_frame(&f.c, &fr(vec![bub("file.bin", "inbound_visual", "file")]), None, None, false).unwrap();
        let row = &rows[0];
        let message_id = row["message_id"].as_str().unwrap();

        let src = f._temp.path().join("file.bin");
        fs::write(&src, vec![b'x'; 200]).unwrap();

        let err100 = f.store.attach_media(message_id, &src, "file", Some("file.bin"), "original", Some(100)).unwrap_err();
        assert!(err100.to_string().contains("media larger than 100 bytes"), "expected 'media larger than 100 bytes', got: {}", err100);

        let src_big = f._temp.path().join("big.bin");
        fs::write(&src_big, vec![b'0'; 50 * 1024 * 1024 + 1]).unwrap();
        let err50mb = f.store.attach_media(message_id, &src_big, "file", Some("big.bin"), "original", Some(50 * 1024 * 1024)).unwrap_err();
        assert!(err50mb.to_string().contains("media larger than 50MB"), "expected 'media larger than 50MB', got: {}", err50mb);
    }

    #[test]
    fn test_store_file_messages_queue_download_with_backoff() {
        let f = setup_fixture();
        let rows = f.store.record_frame(&f.c, &fr(vec![bub("报价.pdf", "inbound_visual", "file")]), None, None, false).unwrap();
        let row = &rows[0];
        let job = f.store.next_media_job(&f.c).unwrap().unwrap();
        assert_eq!(job["message_id"].as_str().unwrap(), row["message_id"].as_str().unwrap());
        f.store.finish_media_job(row["message_id"].as_str().unwrap(), "failed", Some("timeout")).unwrap();
        assert!(f.store.next_media_job(&f.c).unwrap().is_none());
        assert_eq!(f.store.has_pending_media().unwrap(), true);
    }

    #[test]
    fn test_store_image_messages_are_not_queued() {
        let f = setup_fixture();
        f.store.record_frame(&f.c, &fr(vec![bub("[图片]", "inbound_visual", "image")]), None, None, false).unwrap();
        assert_eq!(f.store.has_pending_media().unwrap(), false);
    }

    #[test]
    fn test_store_failed_three_times_is_terminal_and_gone_is_terminal() {
        let f = setup_fixture();
        let rows = f.store.record_frame(&f.c, &fr(vec![bub("a.pdf", "inbound_visual", "file")]), None, None, false).unwrap();
        let row = &rows[0];
        for _ in 0..3 {
            f.store.finish_media_job(row["message_id"].as_str().unwrap(), "failed", Some("x")).unwrap();
        }
        assert_eq!(f.store.has_pending_media().unwrap(), false);
        let rows2 = f.store.record_frame(&f.c, &fr(vec![bub("a.pdf", "inbound_visual", "file"), bub("b.pdf", "inbound_visual", "file")]), None, None, false).unwrap();
        let other = &rows2[0];
        f.store.finish_media_job(other["message_id"].as_str().unwrap(), "gone", None).unwrap();
        assert_eq!(f.store.has_pending_media().unwrap(), false);
    }

    #[test]
    fn test_store_interoperability_python_rust() {
        let py_check = std::process::Command::new("python3")
            .arg("-c")
            .arg("import sys; sys.exit(0)")
            .status();
        if py_check.is_err() || !py_check.unwrap().success() {
            println!("skipping python interoperability test: python3 not available");
            return;
        }

        let temp = TestTempDir::new();
        let state = temp.path().join("state");

        // 1. Python writes, Rust reads
        let py_writer = r#"
import sys
from pathlib import Path
from scripts.wechat_bot.history.store import HistoryStore
state = Path(sys.argv[1])
store = HistoryStore.open(state)
c = store.ensure_contact('acct', {'recipient': '测试互通', 'kind': 'private', 'aliases': ['别名']})
store.record_frame(c, {'bubbles': [{'text': 'Python写入的消息', 'digest': 'd1', 'direction': 'inbound_visual', 'kind': 'text'}]})
store.conn.close()
"#;
        let status = std::process::Command::new("python3")
            .arg("-c")
            .arg(py_writer)
            .arg(state.to_str().unwrap())
            .status()
            .unwrap();
        assert!(status.success(), "Python writer script failed");

        let rust_store = HistoryStore::open(&state).unwrap();
        let cid = rust_store.resolve("acct", "测试互通").unwrap().unwrap();
        let ctx = rust_store.recent_context(&cid, 5, &HashSet::new()).unwrap();
        assert_eq!(ctx.len(), 1);
        assert_eq!(ctx[0]["text"].as_str().unwrap(), "Python写入的消息");

        // 2. Rust writes, Python reads
        rust_store.record_bot_reply(&cid, "send_key_1", "Rust回复的消息", "triggered", &json!({}), &[], None, None).unwrap();
        drop(rust_store);

        let py_reader = r#"
import sys
from pathlib import Path
from scripts.wechat_bot.history.store import HistoryStore
state = Path(sys.argv[1])
store = HistoryStore.open(state)
c = store.resolve('acct', '测试互通')
ctx = store.recent_context(c, 5)
texts = [x['text'] for x in ctx]
assert 'Rust回复的消息' in texts, f'unexpected texts: {texts}'
store.conn.close()
"#;
        let status2 = std::process::Command::new("python3")
            .arg("-c")
            .arg(py_reader)
            .arg(state.to_str().unwrap())
            .status()
            .unwrap();
        assert!(status2.success(), "Python reader script failed");
    }

    #[test]
    fn test_profile_migration_creates_table_and_addressbook_source() {
        let f = setup_fixture();
        let conn = f.store.connection();
        let guard = conn.lock().unwrap();

        // 1. 验证 contact_profiles 表存在
        let exists: bool = guard
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='contact_profiles'",
                [],
                |_| Ok(true),
            )
            .optional()
            .unwrap()
            .unwrap_or(false);
        assert!(exists, "contact_profiles table must exist after migration");

        // 2. 验证 contact_names 可插入 source='addressbook'
        let initial_count: i64 = guard
            .query_row("SELECT COUNT(*) FROM contact_names", [], |r| r.get(0))
            .unwrap();
        guard
            .execute(
                "INSERT INTO contact_names(contact_id, account_id, raw_name, name_key, source, first_seen_at, last_seen_at) VALUES(?,?,?,?,?,?,?)",
                params![&f.c, "acct", "通讯录好友", "通讯录好友", "addressbook", 1000, 1000],
            )
            .expect("should allow inserting source='addressbook'");
        let count_after: i64 = guard
            .query_row("SELECT COUNT(*) FROM contact_names", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_after, initial_count + 1);
    }

    #[test]
    fn test_upsert_new_personal_creates_contact_and_names() {
        let f = setup_fixture();
        let input = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_t1".to_string()),
            nickname: "测试甲".to_string(),
            remark: Some("甲备注".to_string()),
            region: Some("北京 海淀".to_string()),
            source: Some("通过搜索添加".to_string()),
            added_on: Some("2024-01-01".to_string()),
            common_groups: Some(2),
            enterprise: None,
            real_name: None,
            tags: vec!["朋友".to_string()],
            friend_permission: Some("聊天、朋友圈".to_string()),
        };
        let outcome = f.store.upsert_profile("acct", &input, 1000).unwrap();
        match outcome {
            UpsertOutcome::Created(cid) => {
                assert_eq!(f.store.resolve("acct", "甲备注").unwrap(), Some(cid.clone()));
                assert_eq!(f.store.resolve("acct", "测试甲").unwrap(), Some(cid));
            }
            other => panic!("expected Created, got {:?}", other),
        }
    }

    #[test]
    fn test_upsert_same_wechat_id_updates_and_keeps_contact() {
        let f = setup_fixture();
        let input1 = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_t1".to_string()),
            nickname: "测试甲".to_string(),
            remark: Some("甲备注".to_string()),
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let cid = match f.store.upsert_profile("acct", &input1, 1000).unwrap() {
            UpsertOutcome::Created(cid) => cid,
            other => panic!("expected Created, got {:?}", other),
        };

        let input2 = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_t1".to_string()),
            nickname: "测试甲".to_string(),
            remark: Some("新甲备注".to_string()),
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let outcome2 = f.store.upsert_profile("acct", &input2, 2000).unwrap();
        assert_eq!(outcome2, UpsertOutcome::Updated(cid.clone()));
        assert_eq!(f.store.resolve("acct", "新甲备注").unwrap(), Some(cid));
    }

    #[test]
    fn test_upsert_matches_existing_history_contact_by_unique_name() {
        let f = setup_fixture();
        // 已有聊过天的联系人"测试乙"（无资料）
        let orig_cid = f.store.ensure_contact("acct", &json!({"recipient": "测试乙", "kind": "private"}), true).unwrap();
        f.store.record_frame(&orig_cid, &fr(vec![bub("你好测试乙", "inbound_visual", "text")]), None, None, false).unwrap();
        let msg_count_before: i64 = f.store.connection().lock().unwrap()
            .query_row("SELECT COUNT(*) FROM messages WHERE contact_id=?", [&orig_cid], |r| r.get(0)).unwrap();
        assert_eq!(msg_count_before, 1);

        let input = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_t2".to_string()),
            nickname: "测试乙".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let outcome = f.store.upsert_profile("acct", &input, 3000).unwrap();
        assert_eq!(outcome, UpsertOutcome::Updated(orig_cid.clone()));

        let msg_count_after: i64 = f.store.connection().lock().unwrap()
            .query_row("SELECT COUNT(*) FROM messages WHERE contact_id=?", [&orig_cid], |r| r.get(0)).unwrap();
        assert_eq!(msg_count_after, 1, "messages count must remain unchanged");
    }

    #[test]
    fn test_upsert_ambiguous_name_needs_review() {
        let f = setup_fixture();
        // 创建两个不同联系人，都叫"重名丙"
        let c1 = f.store.ensure_contact("acct", &json!({"recipient": "c1_display", "kind": "private"}), true).unwrap();
        let c2 = f.store.ensure_contact("acct", &json!({"recipient": "c2_display", "kind": "private"}), true).unwrap();
        f.store.observe_name(&c1, "重名丙").unwrap();
        f.store.observe_name(&c2, "重名丙").unwrap();

        let input = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_bing".to_string()),
            nickname: "重名丙".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let outcome = f.store.upsert_profile("acct", &input, 4000).unwrap();
        match outcome {
            UpsertOutcome::NeedsReview { reason, mut candidates } => {
                assert!(reason.contains("ambiguous") || reason.contains("conflict") || !reason.is_empty());
                candidates.sort();
                let mut expected = vec![c1, c2];
                expected.sort();
                assert_eq!(candidates, expected);
            }
            other => panic!("expected NeedsReview, got {:?}", other),
        }

        let reviews = f.store.pending_reviews("acct").unwrap();
        assert!(!reviews.is_empty(), "pending_reviews must list ambiguous profile review");
    }

    #[test]
    fn test_upsert_wechat_id_conflict_needs_review() {
        let f = setup_fixture();
        // 先建立一个已有微信号 wxid_a 的联系人，名字为冲突丁
        let input_a = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_a".to_string()),
            nickname: "冲突丁".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let outcome_a = f.store.upsert_profile("acct", &input_a, 1000).unwrap();
        let cid_a = match outcome_a {
            UpsertOutcome::Created(cid) => cid,
            other => panic!("expected Created, got {:?}", other),
        };

        // 再来同名但是微信号不同的 wxid_b
        let input_b = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_b".to_string()),
            nickname: "冲突丁".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let outcome_b = f.store.upsert_profile("acct", &input_b, 2000).unwrap();
        match outcome_b {
            UpsertOutcome::NeedsReview { reason, candidates } => {
                assert!(reason.contains("wechat_id") || reason.contains("conflict") || !reason.is_empty());
                assert!(candidates.contains(&cid_a));
            }
            other => panic!("expected NeedsReview, got {:?}", other),
        }

        // 验证原微信号未被覆盖
        let profiles = f.store.list_profiles("acct", None).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0]["wechat_id"].as_str().unwrap(), "wxid_a");
    }

    #[test]
    fn test_weak_identity_for_work_contact() {
        let f = setup_fixture();
        let input1 = ProfileInput {
            category: "work".to_string(),
            wechat_id: None,
            nickname: "企微员工戊".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: Some("某某企业".to_string()),
            real_name: Some("员工戊".to_string()),
            tags: vec![],
            friend_permission: None,
        };
        let outcome1 = f.store.upsert_profile("acct", &input1, 1000).unwrap();
        let cid = match outcome1 {
            UpsertOutcome::Created(cid) => cid,
            other => panic!("expected Created, got {:?}", other),
        };

        let profiles = f.store.list_profiles("acct", Some("work")).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0]["identity"].as_str().unwrap(), "weak");

        // 再次同步同名同企业
        let input2 = ProfileInput {
            category: "work".to_string(),
            wechat_id: None,
            nickname: "企微员工戊".to_string(),
            remark: Some("备注戊".to_string()),
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: Some("某某企业".to_string()),
            real_name: Some("员工戊".to_string()),
            tags: vec!["客户组".to_string()],
            friend_permission: None,
        };
        let outcome2 = f.store.upsert_profile("acct", &input2, 2000).unwrap();
        assert_eq!(outcome2, UpsertOutcome::Updated(cid));
    }

    #[test]
    fn test_official_category_recorded() {
        let f = setup_fixture();
        let input = ProfileInput {
            category: "official".to_string(),
            wechat_id: Some("gh_test_official".to_string()),
            nickname: "测试公众号".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        f.store.upsert_profile("acct", &input, 1000).unwrap();
        let officials = f.store.list_profiles("acct", Some("official")).unwrap();
        assert_eq!(officials.len(), 1);
        assert_eq!(officials[0]["category"].as_str().unwrap(), "official");
        assert_eq!(officials[0]["wechat_id"].as_str().unwrap(), "gh_test_official");
    }

    #[test]
    fn test_mark_missing_sets_removed_at_not_deleting() {
        let f = setup_fixture();
        let p1 = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_p1".to_string()),
            nickname: "联系人一".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let p2 = ProfileInput {
            category: "personal".to_string(),
            wechat_id: Some("wxid_p2".to_string()),
            nickname: "联系人二".to_string(),
            remark: None,
            region: None,
            source: None,
            added_on: None,
            common_groups: None,
            enterprise: None,
            real_name: None,
            tags: vec![],
            friend_permission: None,
        };
        let cid1 = match f.store.upsert_profile("acct", &p1, 1000).unwrap() {
            UpsertOutcome::Created(cid) => cid,
            other => panic!("expected Created, got {:?}", other),
        };
        let cid2 = match f.store.upsert_profile("acct", &p2, 1000).unwrap() {
            UpsertOutcome::Created(cid) => cid,
            other => panic!("expected Created, got {:?}", other),
        };

        // 记录 p1 聊天消息
        f.store.record_frame(&cid1, &fr(vec![bub("你好p1", "inbound_visual", "text")]), None, None, false).unwrap();

        // mark_missing: 只有 cid1 出现，cid2 缺失
        let missing_count = f.store.mark_missing("acct", "personal", &[cid1.clone()], 2000).unwrap();
        assert_eq!(missing_count, 1);

        // 验证 cid2 设置了 removed_at，但聊天记录和联系人未删除
        let profiles = f.store.list_profiles("acct", Some("personal")).unwrap();
        let prof1 = profiles.iter().find(|p| p["contact_id"] == cid1).unwrap();
        let prof2 = profiles.iter().find(|p| p["contact_id"] == cid2).unwrap();
        assert!(prof1["removed_at"].is_null());
        assert_eq!(prof2["removed_at"].as_i64().unwrap(), 2000);

        let msg_count: i64 = f.store.connection().lock().unwrap()
            .query_row("SELECT COUNT(*) FROM messages WHERE contact_id=?", [&cid1], |r| r.get(0)).unwrap();
        assert_eq!(msg_count, 1);

        // 再次出现时清空 removed_at
        let outcome2 = f.store.upsert_profile("acct", &p2, 3000).unwrap();
        assert_eq!(outcome2, UpsertOutcome::Updated(cid2.clone()));
        let profiles_after = f.store.list_profiles("acct", Some("personal")).unwrap();
        let prof2_after = profiles_after.iter().find(|p| p["contact_id"] == cid2).unwrap();
        assert!(prof2_after["removed_at"].is_null());
    }
}
