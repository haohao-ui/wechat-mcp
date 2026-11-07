use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::store::HistoryStore;

pub const LEGACY_FILES: &[&str] = &["cursor.json", "recent-files.json"];

pub fn is_legacy_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(s) => s,
        None => return false,
    };
    name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn ensure_stopped(state: &Path) -> Result<()> {
    let lock_path = state.join("service.lock");
    if lock_path.exists() {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)?;
        match file.try_lock_exclusive() {
            Ok(_) => {
                let _ = file.unlock();
            }
            Err(_) => {
                bail!("stop the bot first (service.lock is held)");
            }
        }
    }
    Ok(())
}

fn write_private(path: &Path, value: &Value) -> Result<()> {
    crate::core::state::write_private(path, value)
}

pub fn entry_for(config: &Value, recipient: &str) -> Value {
    if let Some(allowlist) = config.get("allowlist").and_then(|v| v.as_array()) {
        for entry in allowlist {
            if entry.get("recipient").and_then(|v| v.as_str()) == Some(recipient) {
                return entry.clone();
            }
        }
    }
    json!({
        "recipient": recipient,
        "kind": "private"
    })
}

pub fn migrate(config: &Value) -> Result<HashMap<String, usize>> {
    let state_dir_str = config
        .get("state_dir")
        .and_then(|v| v.as_str())
        .context("missing state_dir")?;
    let state = Path::new(state_dir_str);
    ensure_stopped(state)?;

    let account_id = config
        .get("account_id")
        .and_then(|v| v.as_str())
        .context("missing account_id")?;
    let host = config
        .get("host")
        .and_then(|v| v.as_str())
        .context("missing host")?;

    let sessions = state.join("sessions");
    if sessions.is_dir() && !sessions.is_symlink() {
        let _ = fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700));
    }
    fs::create_dir_all(&sessions)?;
    let _ = fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700));

    let store = HistoryStore::open(state)?;
    let mut stats = HashMap::new();
    stats.insert("contacts".to_string(), 0);
    stats.insert("moved_dirs".to_string(), 0);
    stats.insert("imported_sends".to_string(), 0);
    stats.insert("baseline_messages".to_string(), 0);

    let bind = |dir: &Path, contact_id: &str| -> Result<()> {
        write_private(
            &dir.join("binding.json"),
            &json!({
                "host": host,
                "account_id": account_id,
                "contact_id": contact_id,
                "kind": "private",
            }),
        )
    };

    // 1. Hash-named session directories -> sessions/<contact_id>/
    let mut legacy_dirs: Vec<PathBuf> = Vec::new();
    if sessions.exists() {
        for entry in fs::read_dir(&sessions)? {
            let p = entry?.path();
            if is_legacy_dir(&p) {
                legacy_dirs.push(p);
            }
        }
    }
    legacy_dirs.sort();

    for legacy in legacy_dirs {
        let binding_path = legacy.join("binding.json");
        let binding_bytes = fs::read(&binding_path)?;
        let binding: Value = serde_json::from_slice(&binding_bytes)?;
        if binding.get("account_id").and_then(|v| v.as_str()) != Some(account_id)
            || binding.get("host").and_then(|v| v.as_str()) != Some(host)
        {
            bail!("{} belongs to another account/host", legacy.file_name().unwrap().to_string_lossy());
        }
        let recipient = binding
            .get("recipient")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let entry = entry_for(config, recipient);
        let contact_id = store.ensure_contact(account_id, &entry, true)?;
        let target = sessions.join(&contact_id);
        if target.exists() {
            bail!("both {} and {} exist; resolve manually", legacy.file_name().unwrap().to_string_lossy(), contact_id);
        }
        fs::rename(&legacy, &target)?;
        bind(&target, &contact_id)?;
        write_private(
            &sessions.join(format!("MOVED-{}-TO-{}.txt", legacy.file_name().unwrap().to_string_lossy(), contact_id)),
            &json!({
                "from": legacy.file_name().unwrap().to_string_lossy(),
                "to": contact_id,
            }),
        )?;
        *stats.get_mut("moved_dirs").unwrap() += 1;
    }

    // 2. Root-level single-session files
    let root_binding = state.join("binding.json");
    if root_binding.exists() {
        let binding_bytes = fs::read(&root_binding)?;
        if let Ok(binding) = serde_json::from_slice::<Value>(&binding_bytes) {
            if let (Some(rec), Some(acct)) = (
                binding.get("recipient").and_then(|v| v.as_str()),
                binding.get("account_id").and_then(|v| v.as_str()),
            ) {
                if acct == account_id {
                    let entry = entry_for(config, rec);
                    let contact_id = store.ensure_contact(account_id, &entry, true)?;
                    let target = sessions.join(&contact_id);
                    if !target.join("binding.json").exists() {
                        fs::create_dir_all(&target)?;
                        let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o700));
                        for leg in LEGACY_FILES {
                            let src = state.join(leg);
                            if src.exists() {
                                let v: Value = serde_json::from_slice(&fs::read(&src)?)?;
                                write_private(&target.join(leg), &v)?;
                            }
                        }
                        if let Ok(entries) = fs::read_dir(state) {
                            for e in entries {
                                if let Ok(entry) = e {
                                    let p = entry.path();
                                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                                        if name.starts_with("send-") && name.ends_with(".json") {
                                            let v: Value = serde_json::from_slice(&fs::read(&p)?)?;
                                            write_private(&target.join(name), &v)?;
                                        }
                                    }
                                }
                            }
                        }
                        bind(&target, &contact_id)?;
                    }
                }
            }
        }
    }

    // 3. Import journals in time order, baseline current cursor
    let mut session_dirs: Vec<PathBuf> = Vec::new();
    if sessions.exists() {
        for entry in fs::read_dir(&sessions)? {
            let p = entry?.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("c_") {
                        session_dirs.push(p);
                    }
                }
            }
        }
    }
    session_dirs.sort();

    for dir in session_dirs {
        let contact_id = dir.file_name().unwrap().to_str().unwrap().to_string();
        *stats.get_mut("contacts").unwrap() += 1;

        let mut journals: Vec<Value> = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let p = entry?.path();
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("send-") && name.ends_with(".json") {
                    if let Ok(v) = serde_json::from_slice::<Value>(&fs::read(&p)?) {
                        journals.push(v);
                    }
                }
            }
        }
        journals.sort_by_key(|j| (j.get("created_at").and_then(|v| v.as_f64()).unwrap_or(0.0) * 1000.0) as i64);

        for record in journals {
            let key = match record.get("idempotency_key").and_then(|v| v.as_str()) {
                Some(k) => k,
                None => continue,
            };
            if store.send(key)?.is_some() {
                continue;
            }
            let conn = store.connection();
            let already_imported: bool = conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT 1 FROM messages WHERE batch_key=? AND capture='imported'",
                    params![key],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if already_imported {
                continue;
            }

            let at = (record.get("created_at").and_then(|v| v.as_f64()).unwrap_or(0.0) * 1000.0) as i64;
            let mut ids = Vec::new();
            if let Some(inputs) = record.get("input").and_then(|v| v.as_array()) {
                for b_val in inputs {
                    let bubble: crate::core::model::Bubble = serde_json::from_value(b_val.clone())?;
                    let mid = store.import_message(&contact_id, &bubble, at, key)?;
                    ids.push(mid);
                }
            }
            if let Some(reply) = record.get("reply").and_then(|v| v.as_str()) {
                let sent_at = (record
                    .get("attempted_at")
                    .or_else(|| record.get("created_at"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    * 1000.0) as i64;
                let status = record.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
                let model = record.get("model").unwrap_or(&Value::Null);
                store.record_bot_reply(&contact_id, key, reply, status, model, &ids, Some(sent_at), Some("imported"))?;
                let echo_observed = record.get("client_echo_observed").and_then(|v| v.as_bool());
                store.update_send(key, status, echo_observed)?;
            }
            *stats.get_mut("imported_sends").unwrap() += 1;
        }

        let has_cursor: bool = store
            .connection()
            .lock()
            .unwrap()
            .query_row(
                "SELECT 1 FROM contact_cursor WHERE contact_id=?",
                params![contact_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);

        let cursor_path = dir.join("cursor.json");
        if !has_cursor && cursor_path.exists() {
            let cursor_val: Value = serde_json::from_slice(&fs::read(&cursor_path)?)?;
            let rows = store.record_frame(
                &contact_id,
                &cursor_val,
                None,
                Some(&json!({"may_duplicate_imported": true})),
                true,
            )?;
            *stats.get_mut("baseline_messages").unwrap() += rows.len();
        }
    }

    Ok(stats)
}

pub fn history_backup(store: &HistoryStore, target: &Path) -> Result<u64> {
    if target.exists() {
        bail!("backup target already exists: {}", target.display());
    }
    let target_str = target.to_str().context("invalid target path")?;
    {
        let conn_guard = store.connection();
        let src_conn = conn_guard.lock().unwrap();
        src_conn.execute("VACUUM INTO ?", [target_str])?;
    }
    fs::set_permissions(target, fs::Permissions::from_mode(0o600))?;
    let size = fs::metadata(target)?.len();
    Ok(size)
}

pub fn history_status(store: &HistoryStore) -> Result<Value> {
    let db = store.root_dir().join("history.sqlite3");
    let conn_guard = store.connection();
    let conn = conn_guard.lock().unwrap();

    let mut contacts = Vec::new();
    let mut stmt = conn.prepare("SELECT contact_id, display_name, merged_into FROM contacts ORDER BY rowid")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
    })?;

    for r in rows {
        let (cid, dname, merged) = r?;
        let (count, last): (i64, Option<i64>) = conn.query_row(
            "SELECT COUNT(*), MAX(observed_at) FROM messages WHERE contact_id=?",
            [&cid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut unmatched_stmt = conn.prepare(
            "SELECT raw_name FROM contact_names o WHERE contact_id=? AND source='observed' AND NOT EXISTS \
             (SELECT 1 FROM contact_names c WHERE c.contact_id=o.contact_id AND c.name_key=o.name_key AND c.source IN ('config','alias'))",
        )?;
        let unmatched: Vec<String> = unmatched_stmt
            .query_map([&cid], |r| r.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        contacts.push(json!({
            "contact_id": cid,
            "display_name": dname,
            "merged_into": merged,
            "messages": count,
            "last_observed_at": last,
            "unmatched_observed_names": unmatched,
        }));
    }
    drop(stmt);

    let mut suggestions = Vec::new();
    let mut merged_stmt = conn.prepare("SELECT contact_id FROM contacts WHERE merged_into IS NOT NULL ORDER BY rowid")?;
    let merged_rows: Vec<String> = merged_stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .filter_map(Result::ok)
        .collect();
    drop(merged_stmt);

    for cid in merged_rows {
        let into = super::store::canonical_conn(&conn, cid.clone())?;
        let mut n_stmt = conn.prepare("SELECT raw_name FROM contact_names WHERE contact_id=? AND source IN ('config','alias') ORDER BY id")?;
        let names: Vec<String> = n_stmt
            .query_map([&cid], |r| r.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        let into_name: String = conn.query_row(
            "SELECT display_name FROM contacts WHERE contact_id=?",
            [&into],
            |r| r.get(0),
        )?;
        suggestions.push(json!({
            "into": into,
            "into_name": into_name,
            "add_aliases": names,
        }));
    }

    let total_messages: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
    let pending_downloads: i64 = conn.query_row("SELECT COUNT(*) FROM media_jobs WHERE status='pending'", [], |r| r.get(0))?;
    let db_bytes = fs::metadata(&db)?.len();

    Ok(json!({
        "database": db.to_string_lossy(),
        "bytes": db_bytes,
        "alias_suggestions": suggestions,
        "messages": total_messages,
        "pending_file_downloads": pending_downloads,
        "contacts": contacts,
    }))
}

pub fn resolve_contact(store: &HistoryStore, account_id: &str, value: &str) -> Result<String> {
    if value.starts_with("c_") {
        let conn_guard = store.connection();
        let conn = conn_guard.lock().unwrap();
        let exists: bool = conn
            .query_row("SELECT 1 FROM contacts WHERE contact_id=?", [value], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if exists {
            return Ok(value.to_string());
        }
    } else if let Some(found) = store.resolve(account_id, value)? {
        let conn_guard = store.connection();
        let conn = conn_guard.lock().unwrap();
        let exists: bool = conn
            .query_row("SELECT 1 FROM contacts WHERE contact_id=?", [&found], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if exists {
            return Ok(found);
        }
    }
    bail!("unknown contact: {}", value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::history::store::sha256_hex;
    use crate::core::model::Bubble;
    use serde_json::json;

    struct TestTempDir(PathBuf);
    impl TestTempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("wechat-migrate-test-{}", uuid::Uuid::new_v4().simple()));
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

    struct MigrateFixture {
        _temp: TestTempDir,
        base: PathBuf,
        state: PathBuf,
        config: Value,
    }

    fn bub(text: &str) -> Bubble {
        let dir = if text.starts_with("答") {
            "outbound_visual"
        } else {
            "inbound_visual"
        };
        Bubble {
            text: text.to_string(),
            digest: text.to_string(),
            direction: dir.to_string(),
            kind: Some("text".to_string()),
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

    fn setup_fixture() -> MigrateFixture {
        let temp = TestTempDir::new();
        let base = temp.path().to_path_buf();
        let state = base.join("state");
        fs::create_dir_all(state.join("sessions")).unwrap();
        let _ = fs::set_permissions(&state, fs::Permissions::from_mode(0o700));

        let config = json!({
            "schema_version": 1,
            "host": "fixture",
            "account_id": "acct",
            "state_dir": state.to_string_lossy(),
            "allowlist": [
                {"recipient": "午夜咖啡", "kind": "private"},
                {"recipient": "a", "kind": "private"}
            ]
        });

        MigrateFixture {
            _temp: temp,
            base,
            state,
            config,
        }
    }

    fn make_legacy_session(
        f: &MigrateFixture,
        name: &str,
        sends: &[(&str, &str, &str, f64)],
        cursor: &[&str],
    ) -> PathBuf {
        let _target = json!({"recipient": name, "kind": "private"});
        let target_str = format!("{{\"kind\":\"private\",\"recipient\":\"{}\"}}", name);
        let digest = sha256_hex(target_str.as_bytes());
        let dir = f.state.join("sessions").join(&digest);
        fs::create_dir_all(&dir).unwrap();
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));

        let binding = json!({
            "host": "fixture",
            "account_id": "acct",
            "recipient": name,
            "kind": "private",
        });
        fs::write(dir.join("binding.json"), binding.to_string()).unwrap();

        let cursor_bubbles: Vec<Bubble> = cursor.iter().map(|&t| bub(t)).collect();
        fs::write(dir.join("cursor.json"), json!({"bubbles": cursor_bubbles}).to_string()).unwrap();

        for &(key, question, answer, created) in sends {
            let record = json!({
                "idempotency_key": key,
                "status": "triggered",
                "input": [bub(question)],
                "reply": answer,
                "created_at": created,
                "client_echo_observed": true,
            });
            fs::write(dir.join(format!("send-{}.json", key)), record.to_string()).unwrap();
        }

        dir
    }

    fn column(f: &MigrateFixture, cid: &str, col: &str) -> Vec<String> {
        let db_path = f.state.join("history").join("history.sqlite3");
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let sql = format!("SELECT {} FROM messages WHERE contact_id=? ORDER BY seq", col);
        let mut stmt = conn.prepare(&sql).unwrap();
        let mapped = stmt.query_map([cid], |r| r.get::<_, String>(0)).unwrap();
        mapped.filter_map(Result::ok).collect()
    }

    #[test]
    fn test_migrate_moves_dirs_imports_and_baselines() {
        let f = setup_fixture();
        let legacy = make_legacy_session(
            &f,
            "午夜咖啡",
            &[("k1", "问1", "答1", 1.0), ("k2", "问2", "答2", 2.0)],
            &["问2", "答2", "问3"],
        );
        let stats = migrate(&f.config).unwrap();
        let store = HistoryStore::open(&f.state).unwrap();
        let cid = store.resolve("acct", "午夜咖啡").unwrap().unwrap();

        assert!(!legacy.exists());
        assert!(f.state.join("sessions").join(&cid).join("cursor.json").exists());
        let binding: Value = serde_json::from_slice(&fs::read(f.state.join("sessions").join(&cid).join("binding.json")).unwrap()).unwrap();
        assert_eq!(binding, json!({"host": "fixture", "account_id": "acct", "contact_id": cid, "kind": "private"}));
        assert!(f.state.join("sessions").join(format!("MOVED-{}-TO-{}.txt", legacy.file_name().unwrap().to_string_lossy(), cid)).exists());

        assert_eq!(column(&f, &cid, "text"), vec!["问1", "答1", "问2", "答2", "问2", "问3"]);
        assert_eq!(column(&f, &cid, "capture"), vec!["imported", "imported", "imported", "imported", "baseline", "baseline"]);
        assert_eq!(column(&f, &cid, "sender"), vec!["customer", "bot", "customer", "bot", "customer", "customer"]);

        assert_eq!(store.send("k2").unwrap().unwrap()["echo_observed"].as_bool().unwrap(), true);
        assert_eq!(stats.get("contacts"), Some(&1));
        assert_eq!(stats.get("moved_dirs"), Some(&1));
        assert_eq!(stats.get("imported_sends"), Some(&2));
        assert_eq!(stats.get("baseline_messages"), Some(&2));
    }

    #[test]
    fn test_migrate_world_readable_sessions_dir_is_tightened() {
        let f = setup_fixture();
        fs::set_permissions(f.state.join("sessions"), fs::Permissions::from_mode(0o755)).unwrap();
        make_legacy_session(&f, "a", &[], &["q"]);
        migrate(&f.config).unwrap();
        let mode = fs::metadata(f.state.join("sessions")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn test_migrate_is_idempotent() {
        let f = setup_fixture();
        make_legacy_session(&f, "a", &[("k1", "q", "答", 1.0)], &["q"]);
        migrate(&f.config).unwrap();
        let store = HistoryStore::open(&f.state).unwrap();
        let cid = store.resolve("acct", "a").unwrap().unwrap();
        let before = column(&f, &cid, "text");
        drop(store);

        let stats = migrate(&f.config).unwrap();
        assert_eq!(column(&f, &cid, "text"), before);
        assert_eq!(stats.get("imported_sends"), Some(&0));
    }

    #[test]
    fn test_migrate_root_level_legacy_session_is_imported_once() {
        let f = setup_fixture();
        fs::write(f.state.join("binding.json"), json!({"host": "fixture", "account_id": "acct", "recipient": "a", "kind": "private"}).to_string()).unwrap();
        fs::write(f.state.join("cursor.json"), json!({"bubbles": [bub("q")]}).to_string()).unwrap();
        fs::write(f.state.join("send-k9.json"), json!({"idempotency_key": "k9", "status": "attempting", "input": [bub("q")], "created_at": 1.0}).to_string()).unwrap();

        migrate(&f.config).unwrap();
        let store = HistoryStore::open(&f.state).unwrap();
        let cid = store.resolve("acct", "a").unwrap().unwrap();
        let send_k9: Value = serde_json::from_slice(&fs::read(f.state.join("sessions").join(&cid).join("send-k9.json")).unwrap()).unwrap();
        assert_eq!(send_k9["status"].as_str().unwrap(), "attempting");
        drop(store);

        migrate(&f.config).unwrap();
        let mut count = 0;
        for entry in fs::read_dir(f.state.join("sessions")).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() && p.file_name().unwrap().to_str().unwrap().starts_with("c_") {
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn test_migrate_root_legacy_skipped_when_session_dir_already_has_it() {
        let f = setup_fixture();
        make_legacy_session(&f, "a", &[], &["q"]);
        fs::write(f.state.join("binding.json"), json!({"host": "fixture", "account_id": "acct", "recipient": "a", "kind": "private"}).to_string()).unwrap();
        fs::write(f.state.join("cursor.json"), json!({"bubbles": [bub("old")]}).to_string()).unwrap();

        migrate(&f.config).unwrap();
        let store = HistoryStore::open(&f.state).unwrap();
        let cid = store.resolve("acct", "a").unwrap().unwrap();
        let cursor: Value = serde_json::from_slice(&fs::read(f.state.join("sessions").join(&cid).join("cursor.json")).unwrap()).unwrap();
        assert_eq!(cursor["bubbles"][0]["text"].as_str().unwrap(), "q");
    }

    #[test]
    fn test_migrate_refuses_while_service_running() {
        let f = setup_fixture();
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(f.state.join("service.lock"))
            .unwrap();
        lock_file.lock_exclusive().unwrap();

        let res = migrate(&f.config);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("stop the bot"));
    }

    #[test]
    fn test_migrate_merge_unmerge_status_and_backup_commands() {
        let f = setup_fixture();
        let store = HistoryStore::open(&f.state).unwrap();
        let a = store.ensure_contact("acct", &json!({"recipient": "甲", "kind": "private"}), true).unwrap();
        let b = store.ensure_contact("acct", &json!({"recipient": "乙", "kind": "private"}), true).unwrap();

        // merge
        let src_id = resolve_contact(&store, "acct", "乙").unwrap();
        let dst_id = resolve_contact(&store, "acct", &a).unwrap();
        let merge_id = store.merge(&src_id, &dst_id, "改名").unwrap();
        assert_eq!(store.resolve("acct", "乙").unwrap(), Some(a.clone()));

        // unmerge
        store.unmerge(merge_id).unwrap();
        assert_eq!(store.resolve("acct", "乙").unwrap(), Some(b));

        // re-merge
        let src_id2 = resolve_contact(&store, "acct", "乙").unwrap();
        let dst_id2 = resolve_contact(&store, "acct", "甲").unwrap();
        store.merge(&src_id2, &dst_id2, "").unwrap();

        // status
        let status = history_status(&store).unwrap();
        assert_eq!(status["alias_suggestions"], json!([{"into": a, "into_name": "甲", "add_aliases": ["乙"]}]));
        assert_eq!(status["messages"], json!(0));
        assert_eq!(status["contacts"].as_array().unwrap().len(), 2);

        // backup
        let backup = f.base.join("backup.sqlite3");
        let bytes = history_backup(&store, &backup).unwrap();
        assert!(bytes > 0);
        assert_eq!(fs::metadata(&backup).unwrap().permissions().mode() & 0o777, 0o600);

        let b_conn = rusqlite::Connection::open(&backup).unwrap();
        let count: i64 = b_conn.query_row("SELECT count(*) FROM contacts", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);

        let err = history_backup(&store, &backup);
        assert!(err.is_err());
    }
}
