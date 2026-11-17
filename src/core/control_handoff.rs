//! Session-scoped handoff uses the same durable marker as brain-requested handoff.
//! No writer DB / migration, recipient-name guessing, or raw error output.
use super::emit_brain_error;
use crate::core::{config::BotConfig, state::write_private};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};
use std::{fs, io::Write, os::unix::fs::DirBuilderExt, path::Path};

fn private_directory(path: &Path) -> std::io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(std::io::Error::other("invalid session directory"));
    }
    Ok(())
}

pub(super) fn execute<W: Write>(cfg: &BotConfig, req: &Value, out: &mut W) -> i32 {
    let Some(id) = req.get("session_id").and_then(Value::as_str).filter(|id| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    }) else {
        return emit_brain_error(
            out,
            "invalid_arguments",
            "invalid session_id",
            false,
            None,
            2,
        );
    };
    let human = match req.get("to").and_then(Value::as_str) {
        Some("human") => true,
        Some("bot") => false,
        _ => {
            return emit_brain_error(
                out,
                "invalid_arguments",
                "to must be human or bot",
                false,
                None,
                2,
            );
        }
    };
    let db = cfg.state_dir.join("history/history.sqlite3");
    if !db.exists() {
        return emit_brain_error(out, "not_found", "session not found", false, None, 2);
    }
    let result = (|| -> anyhow::Result<Option<Value>> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let name: Option<String> = conn.query_row(
            "SELECT display_name FROM contacts WHERE account_id=?1 AND contact_id=?2 AND merged_into IS NULL",
            params![cfg.account_id, id], |r| r.get(0),
        ).optional()?;
        let Some(name) = name else { return Ok(None) };
        // Resolve identity before any filesystem mutation; IDs are account scoped.
        let sessions = cfg.state_dir.join("sessions");
        private_directory(&sessions)?;
        let dir = sessions.join(id);
        private_directory(&dir)?;
        let marker = dir.join("handoff.json");
        if human {
            write_private(
                &marker,
                &json!({"recipient": name, "account_id": cfg.account_id,
                "session_id": id, "source": "operator", "at": super::now_secs()}),
            )?;
        } else {
            match fs::remove_file(&marker) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        // Ensure directory entry survives a restart, including marker deletion.
        fs::File::open(&dir)?.sync_all()?;
        Ok(Some(
            json!({"account_id": cfg.account_id, "session_id": id, "contact_id": id,
            "handoff": human, "reply_mode": if human {"human"} else {"bot"}}),
        ))
    })();
    match result {
        Ok(Some(data)) => {
            let _ = writeln!(out, "{}", json!({"ok": true, "data": data}));
            0
        }
        Ok(None) => emit_brain_error(out, "not_found", "session not found", false, None, 2),
        Err(_) => emit_brain_error(
            out,
            "store_unavailable",
            "session change unavailable",
            true,
            None,
            1,
        ),
    }
}
