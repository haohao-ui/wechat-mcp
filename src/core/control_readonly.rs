//! Read-only projections for the native UI. Never open the migrating writer store
//! and never return provider/model metadata or raw storage errors.
//!
//! Requests follow `brain-*`: `--config <path> --request-json <object>`.
//! Both session commands accept optional `account_id` (must match the effective
//! account), limit (1..500, default 100), offset (`0..1_000_000`, default 0).
//! session-detail requires `contact_id`. Lists sort by latest observed message;
//! detail pages select newest messages first, then present each page in seq order.
//! `next_offset` is null at the end. Unread/delivery are unknown; `send_status` is
//! only "已发送" for a triggered bot send, otherwise null. `auto_reply_enabled` is
//! configuration eligibility; `auto_replying` additionally requires a live core
//! socket and recent, account-matching health. Missing/stale health stays null.
use super::{ControlPaths, emit_brain_error};
use crate::core::config::BotConfig;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::{fs, io::Write};

pub(super) fn emit_sessions<W: Write>(
    action: &str,
    cfg: &BotConfig,
    req: &Value,
    paths: &ControlPaths,
    stdout: &mut W,
) -> i32 {
    let limit = match req.get("limit") {
        None => 100,
        Some(v) => match v.as_i64() {
            Some(n @ 1..=500) => n,
            _ => {
                return emit_brain_error(
                    stdout,
                    "invalid_arguments",
                    "limit must be 1..500",
                    false,
                    None,
                    2,
                );
            }
        },
    };
    let offset = match req.get("offset") {
        None => 0,
        Some(v) => match v.as_i64() {
            Some(n @ 0..=1_000_000) => n,
            _ => {
                return emit_brain_error(
                    stdout,
                    "invalid_arguments",
                    "invalid offset",
                    false,
                    None,
                    2,
                );
            }
        },
    };
    let contact_id = if action == "session-detail" {
        match req.get("contact_id").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => Some(s),
            _ => {
                return emit_brain_error(
                    stdout,
                    "invalid_arguments",
                    "missing contact_id",
                    false,
                    None,
                    2,
                );
            }
        }
    } else {
        None
    };
    let path = cfg.state_dir.join("history/history.sqlite3");
    let exists = match path.try_exists() {
        Ok(exists) => exists,
        Err(_) => {
            return emit_brain_error(
                stdout,
                "store_unavailable",
                "session data unavailable",
                true,
                None,
                1,
            );
        }
    };
    if !exists {
        if contact_id.is_some() {
            return emit_brain_error(stdout, "not_found", "session not found", false, None, 2);
        }
        let _ = writeln!(
            stdout,
            "{}",
            json!({"ok": true, "data": {"account_id": cfg.account_id, "sessions": [], "next_offset": null}})
        );
        return 0;
    }
    let page_size = usize::try_from(limit).expect("validated positive page size");
    let page_offset = usize::try_from(offset).expect("validated nonnegative offset");
    let result = (|| -> rusqlite::Result<Option<Value>> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Keep list counts and detail messages on a single SQLite snapshot.
        conn.execute_batch("BEGIN DEFERRED")?;
        let mut stmt = conn.prepare(
            "SELECT c.contact_id, c.display_name, c.kind, COUNT(m.message_id), MAX(m.observed_at)
             FROM contacts c LEFT JOIN messages m ON m.contact_id=c.contact_id AND m.account_id=c.account_id
             WHERE c.account_id=?1 AND c.merged_into IS NULL AND (?2 IS NULL OR c.contact_id=?2)
             GROUP BY c.contact_id ORDER BY MAX(m.observed_at) DESC, c.contact_id")?;
        let rows = stmt.query_map(params![cfg.account_id, contact_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?;
        let core_running =
            std::os::unix::net::UnixStream::connect(paths.core_socket_file()).is_ok();
        let mut sessions = Vec::new();
        for row in rows {
            let (id, name, kind, count, last_at) = row?;
            let state = session_state(&conn, cfg, &id, &name, &kind, core_running)?;
            if contact_id.is_none()
                && count == 0
                && state["auto_reply_enabled"] != true
                && state["handoff"] != true
            {
                continue;
            }
            let mut session = json!({"session_id": id, "contact_id": id, "display_name": name, "kind": kind,
                "unread": null, "message_count": count, "last_message_at_ms": last_at});
            session
                .as_object_mut()
                .unwrap()
                .extend(state.as_object().unwrap().clone());
            sessions.push(session);
        }
        if let Some(id) = contact_id {
            let Some(session) = sessions.pop() else {
                return Ok(None);
            };
            // Only expose message content and fixed semantic fields. A bot row
            // alone is not evidence of a send: pending/unknown attempts stay null.
            let mut stmt = conn.prepare(
                "SELECT m.message_id, m.seq, m.direction, m.sender, m.kind, m.text, m.observed_at,
                        CASE WHEN m.sender='bot' AND m.direction='outbound' AND s.status='triggered' THEN 1 ELSE 0 END
                 FROM messages m LEFT JOIN sends s ON s.send_key=m.send_key AND s.contact_id=m.contact_id
                 WHERE m.account_id=?1 AND m.contact_id=?2 ORDER BY m.seq DESC LIMIT ?3 OFFSET ?4")?;
            let mut messages = stmt
                .query_map(params![cfg.account_id, id, limit + 1, offset], |r| {
                    let automatic: bool = r.get::<_, String>(3)? == "bot";
                    let sent: bool = r.get(7)?;
                    Ok(
                        json!({"message_id": r.get::<_, String>(0)?, "seq": r.get::<_, i64>(1)?,
                    "direction": r.get::<_, String>(2)?, "sender": r.get::<_, String>(3)?,
                    "kind": r.get::<_, String>(4)?, "text": r.get::<_, String>(5)?,
                    "observed_at_ms": r.get::<_, i64>(6)?, "auto_reply": automatic,
                    "send_status": if sent { Some("已发送") } else { None }, "delivery": null}),
                    )
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let has_more = messages.len() > page_size;
            messages.truncate(page_size);
            messages.reverse();
            Ok(Some(
                json!({"account_id": cfg.account_id, "session": session, "messages": messages,
                "next_offset": if has_more {Some(offset + limit)} else {None}, "history_scope": "recorded_messages"}),
            ))
        } else {
            let mut page: Vec<_> = sessions
                .into_iter()
                .skip(page_offset)
                .take(page_size + 1)
                .collect();
            let has_more = page.len() > page_size;
            page.truncate(page_size);
            Ok(Some(json!({"account_id": cfg.account_id, "sessions": page,
                "next_offset": if has_more {Some(offset + limit)} else {None}})))
        }
    })();
    match result {
        Ok(Some(data)) => {
            let _ = writeln!(stdout, "{}", json!({"ok": true, "data": data}));
            0
        }
        Ok(None) => emit_brain_error(stdout, "not_found", "session not found", false, None, 2),
        Err(_) => emit_brain_error(
            stdout,
            "store_unavailable",
            "session data unavailable",
            true,
            None,
            1,
        ),
    }
}

fn session_state(
    conn: &Connection,
    cfg: &BotConfig,
    id: &str,
    name: &str,
    kind: &str,
    core_running: bool,
) -> rusqlite::Result<Value> {
    let mut names = vec![name.to_owned()];
    let mut stmt =
        conn.prepare("SELECT raw_name FROM contact_names WHERE account_id=?1 AND contact_id=?2")?;
    names.extend(
        stmt.query_map(params![cfg.account_id, id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    let ignored = names
        .iter()
        .any(|n| crate::core::scheduler::decision(cfg, n, Some(kind)) == "ignored");
    let enabled = !ignored
        && names
            .iter()
            .any(|n| crate::core::scheduler::decision(cfg, n, Some(kind)) == "allowed");
    // IDs originate in SQLite, but must still be one safe path component.
    let safe_id = !id.is_empty()
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
    let dir = cfg.state_dir.join("sessions").join(id);
    let read_json = |path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    };
    let handoff = if safe_id {
        match fs::symlink_metadata(dir.join("handoff.json")) {
            Ok(_) => Some(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
            Err(_) => None,
        }
    } else {
        None
    };
    let health = if safe_id {
        read_json(dir.join("health.json"))
    } else {
        None
    };
    let state = health
        .as_ref()
        .filter(|h| h["account_id"] == cfg.account_id)
        .and_then(|h| h.get("state"))
        .and_then(Value::as_str);
    let updated_at = health.as_ref().and_then(|h| h["updated_at"].as_f64());
    let fresh =
        updated_at.is_some_and(|at| at <= super::now_secs() && super::now_secs() - at < 120.0);
    let paused = cfg.state_dir.join("admin-pause.json").try_exists().ok();
    let auto_replying =
        if !enabled || handoff == Some(true) || paused == Some(true) || !core_running {
            Some(false)
        } else if fresh && handoff == Some(false) && paused == Some(false) {
            match state {
                Some("watching" | "generating" | "sending" | "waiting_for_stable_frame") => {
                    Some(true)
                }
                Some("handoff" | "needs_recalibration" | "paused" | "ignored") => Some(false),
                _ => None,
            }
        } else {
            None
        };
    // Don't pass health details through: they can contain raw upstream errors.
    Ok(
        json!({"auto_reply_enabled": enabled, "auto_replying": auto_replying, "handoff": handoff, "reply_mode": handoff.map(|v| if v {"human"} else {"bot"}),
        "handling": if handoff == Some(true) {"handoff"} else if ignored {"ignored"} else if enabled {"auto_reply"} else {"record_only"}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{control::execute_control_full_with_paths, history::store::HistoryStore};
    use std::{
        collections::BTreeMap,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    struct Fixture {
        root: PathBuf,
        config: PathBuf,
        paths: ControlPaths,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("t2-readonly-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let config = root.join("config.json");
            fs::write(&config, json!({"schema_version": 1, "host": "local", "account_id": "account_a",
                "state_dir": root, "allowlist": [{"recipient": "Alice", "kind": "private", "aliases": ["Alias"]}],
                "ignorelist": [{"recipient": "Ignored", "kind": "private"}]}).to_string()).unwrap();
            let paths = ControlPaths::isolated(&root);
            Self {
                root,
                config,
                paths,
            }
        }
        fn run(&self, action: &str, req: Value) -> (i32, Value) {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = execute_control_full_with_paths(
                &[
                    action.into(),
                    "--config".into(),
                    self.config.to_string_lossy().into(),
                    "--request-json".into(),
                    req.to_string(),
                ],
                &mut out,
                &mut err,
                None,
                None,
                &self.paths,
            );
            assert!(err.is_empty());
            (code, serde_json::from_slice(&out).unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn files(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut result = BTreeMap::new();
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            // SQLite read transactions update shared-memory reader marks; these
            // are lock bookkeeping, not persisted application state.
            if path.to_string_lossy().ends_with("-shm") {
                continue;
            }

            if path.is_dir() {
                result.extend(files(&path));
            } else {
                result.insert(path.clone(), fs::read(path).unwrap());
            }
        }
        result
    }

    #[test]
    fn readonly_empty_and_invalid_requests_do_not_create_storage() {
        let f = Fixture::new();
        let before = files(&f.root);
        let (code, val) = f.run("sessions-list", json!({}));
        assert_eq!(code, 0);
        assert_eq!(val["data"]["sessions"], json!([]));
        assert_eq!(
            f.run("session-detail", json!({"contact_id": "missing"})).1["error"]["code"],
            "not_found"
        );
        for req in [
            json!({"limit": 0}),
            json!({"limit": "100"}),
            json!({"offset": -1}),
            json!([]),
            json!({"account_id": 3}),
        ] {
            assert_eq!(f.run("sessions-list", req).0, 2);
        }
        assert!(
            files(&f.root) == before,
            "read-only command changed persisted state"
        );
    }

    #[test]
    fn readonly_sessions_account_isolation_pagination_and_sent_semantics() {
        let f = Fixture::new();
        let store = HistoryStore::open(&f.root).unwrap();
        let alice = store
            .ensure_contact(
                "account_a",
                &json!({"recipient": "Alias", "kind": "private"}),
                false,
            )
            .unwrap();
        let group = store
            .ensure_contact(
                "account_a",
                &json!({"recipient": "Group", "kind": "group"}),
                false,
            )
            .unwrap();
        let foreign = store
            .ensure_contact(
                "account_b",
                &json!({"recipient": "Foreign", "kind": "private"}),
                false,
            )
            .unwrap();
        for (id, key, status, at) in [
            (&alice, "s1", "triggered", 1),
            (&alice, "s2", "unknown", 2),
            (&alice, "s3", "attempting", 3),
            (&group, "s4", "triggered", 4),
            (&foreign, "s5", "triggered", 5),
        ] {
            store
                .record_bot_reply(
                    id,
                    key,
                    "message",
                    status,
                    &json!({"error": "upstream-private-fixture"}),
                    &[],
                    Some(at),
                    None,
                )
                .unwrap();
        }
        let before = files(&f.root);
        let (code, val) = f.run("sessions-list", json!({"limit": 1}));
        assert_eq!(code, 0);
        assert_eq!(val["data"]["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(val["data"]["next_offset"], 1);
        assert_eq!(val["data"]["sessions"][0]["kind"], "group");
        assert_eq!(val["data"]["sessions"][0]["unread"], Value::Null);
        let val = f.run("sessions-list", json!({"offset": 1})).1;
        assert_eq!(val["data"]["sessions"][0]["auto_reply_enabled"], true);
        assert_eq!(val["data"]["sessions"][0]["auto_replying"], false);
        let (code, val) = f.run("session-detail", json!({"contact_id": alice, "limit": 2}));
        assert_eq!(code, 0);
        assert_eq!(val["data"]["next_offset"], 2);
        assert!(
            val["data"]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["auto_reply"] == true && m["send_status"].is_null())
        );
        let val = f
            .run(
                "session-detail",
                json!({"contact_id": alice, "limit": 2, "offset": 2}),
            )
            .1;
        assert_eq!(val["data"]["messages"][0]["send_status"], "已发送");
        assert!(val["data"]["messages"][0]["delivery"].is_null());
        assert!(!val.to_string().contains("upstream-private-fixture"));
        assert_eq!(
            f.run("session-detail", json!({"contact_id": foreign})).1["error"]["code"],
            "not_found"
        );
        assert_eq!(
            f.run("sessions-list", json!({"account_id": "account_b"})).0,
            2
        );
        assert!(
            files(&f.root) == before,
            "read-only command changed persisted state"
        );
        let dir = f.root.join("sessions").join(&alice);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("handoff.json"),
            json!({"recipient": "Alias"}).to_string(),
        )
        .unwrap();
        let val = f.run("session-detail", json!({"contact_id": alice})).1;
        assert_eq!(val["data"]["session"]["handling"], "handoff");
        assert_eq!(val["data"]["session"]["handoff"], true);
        assert_eq!(val["data"]["session"]["auto_replying"], false);
    }

    #[test]
    fn readonly_errors_are_fixed_and_storage_is_not_migrated() {
        let f = Fixture::new();
        fs::create_dir(f.root.join("history")).unwrap();
        fs::write(
            f.root.join("history/history.sqlite3"),
            b"private-upstream-error-fixture",
        )
        .unwrap();
        let before = files(&f.root);
        let (code, val) = f.run("sessions-list", json!({}));
        assert_eq!(code, 1);
        assert_eq!(val["error"]["message"], "session data unavailable");
        assert!(!val.to_string().contains("private-upstream-error-fixture"));
        assert!(
            files(&f.root) == before,
            "read-only command changed persisted state"
        );
        fs::write(&f.config, b"{private-config-fixture").unwrap();
        let val = f.run("sessions-list", json!({})).1;
        assert_eq!(val["error"]["message"], "config unavailable or invalid");
    }

    #[test]
    fn session_runtime_requires_live_core_and_fresh_account_health() {
        let f = Fixture::new();
        let store = HistoryStore::open(&f.root).unwrap();
        let id = store
            .ensure_contact(
                "account_a",
                &json!({"recipient": "Alice", "kind": "private"}),
                false,
            )
            .unwrap();
        let dir = f.root.join("sessions").join(&id);
        fs::create_dir_all(&dir).unwrap();
        let cfg = BotConfig::load(&f.config).unwrap();
        let conn = store.connection();
        let conn = conn.lock().unwrap();
        let health = |account: &str, at: f64| {
            fs::write(
                dir.join("health.json"),
                json!({"state": "watching", "account_id": account, "updated_at": at,
                "error": "private-upstream-fixture"})
                .to_string(),
            )
            .unwrap();
        };
        // Inject liveness to test health freshness independently of socket setup.
        health("account_a", super::super::now_secs());
        assert_eq!(
            session_state(&conn, &cfg, &id, "Alice", "private", true).unwrap()["auto_replying"],
            true
        );
        health("other", super::super::now_secs());
        assert!(
            session_state(&conn, &cfg, &id, "Alice", "private", true).unwrap()["auto_replying"]
                .is_null()
        );
        health("account_a", super::super::now_secs() - 180.0);
        assert!(
            session_state(&conn, &cfg, &id, "Alice", "private", true).unwrap()["auto_replying"]
                .is_null()
        );
        fs::write(f.root.join("admin-pause.json"), "{}").unwrap();
        assert_eq!(
            session_state(&conn, &cfg, &id, "Alice", "private", true).unwrap()["auto_replying"],
            false
        );
        // Identical names of different conversation kinds don't inherit access.
        assert_eq!(
            session_state(&conn, &cfg, &id, "Alice", "group", true).unwrap()["auto_reply_enabled"],
            false
        );
    }
    #[test]
    fn session_handoff_switch_repeat_unknown_restart_and_isolation() {
        let f = Fixture::new();
        let store = HistoryStore::open(&f.root).unwrap();
        let id = store
            .ensure_contact(
                "account_a",
                &json!({"recipient":"Alice", "kind":"private"}),
                false,
            )
            .unwrap();
        let foreign = store
            .ensure_contact(
                "account_b",
                &json!({"recipient":"Alice", "kind":"private"}),
                false,
            )
            .unwrap();
        drop(store);
        for to in ["human", "human", "bot", "bot", "human"] {
            let (code, state) = f.run("session-handoff", json!({"session_id":id, "to":to}));
            assert_eq!(code, 0, "{state}");
            assert_eq!(state["data"]["handoff"], to == "human");
            assert_eq!(state["data"]["reply_mode"], to);
            // Every invocation reloads config / DB from disk, with no retained state.
            let detail = f.run("session-detail", json!({"contact_id": id})).1;
            assert_eq!(detail["data"]["session"]["handoff"], to == "human");
            assert_eq!(detail["data"]["session"]["reply_mode"], to);
            let list = f.run("sessions-list", json!({})).1;
            assert_eq!(list["data"]["sessions"][0]["session_id"], id);
            if to == "human" {
                assert_eq!(detail["data"]["session"]["auto_replying"], false);
            }
        }
        let marker = f.root.join("sessions").join(&id).join("handoff.json");
        assert_eq!(
            fs::metadata(&marker).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let before = files(&f.root);
        for req in [
            json!({"session_id":"missing", "to":"human"}),
            json!({"session_id":foreign, "to":"human"}),
            json!({"session_id":id, "to":"human", "account_id":"account_b"}),
        ] {
            assert_eq!(
                f.run("session-handoff", req).1["error"]["code"],
                "not_found"
            );
        }
        for req in [
            json!({}),
            json!({"session_id":"../escape", "to":"human"}),
            json!({"session_id":id,"to":"invalid"}),
            json!({"session_id":id,"to":"human","account_id":1}),
        ] {
            assert_eq!(f.run("session-handoff", req).0, 2);
        }
        assert_eq!(files(&f.root), before);
        // Simulated application restart: reconstruct both store and BotService.
        let cfg = BotConfig::load(&f.config).unwrap();
        let mut child_cfg = cfg.clone();
        child_cfg.state_dir = f.root.join("sessions").join(&id);
        let _child =
            crate::core::turns::BotService::new(child_cfg, None, None, None, None, None).unwrap();
        assert!(marker.exists());
        assert_eq!(
            f.run("session-detail", json!({"contact_id":id})).1["data"]["session"]["handoff"],
            true
        );
    }

    #[test]
    fn session_handoff_rejects_symlink_and_hides_storage_error() {
        let f = Fixture::new();
        let store = HistoryStore::open(&f.root).unwrap();
        let id = store
            .ensure_contact(
                "account_a",
                &json!({"recipient":"Alice", "kind":"private"}),
                false,
            )
            .unwrap();
        let outside = f.root.join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, f.root.join("sessions")).unwrap();
        let result = f.run("session-handoff", json!({"session_id":id,"to":"human"}));
        assert_eq!(result.0, 1);
        assert_eq!(result.1["error"]["message"], "session change unavailable");
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}
