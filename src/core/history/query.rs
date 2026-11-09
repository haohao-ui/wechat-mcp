use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};

use super::names::name_key;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for QueryError {}

impl QueryError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new("invalid_argument", message)
    }
}

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn encode_cursor<T: serde::Serialize>(value: &T) -> String {
    let data = serde_json::to_vec(value).unwrap();
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(B64_CHARS[(b0 >> 2) as usize] as char);
        out.push(B64_CHARS[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_CHARS[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64_CHARS[(b2 & 63) as usize] as char);
        }
    }
    out
}

pub fn decode_cursor(text: &str, expected_size: usize) -> Result<Vec<Value>, QueryError> {
    let clean = text.trim_end_matches('=');
    let decode_val = |c: u8| -> Result<u8, QueryError> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(QueryError::invalid_argument("invalid cursor")),
        }
    };
    let bytes = clean.as_bytes();
    if bytes.is_empty() {
        return Err(QueryError::invalid_argument("invalid cursor"));
    }
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let c0 = decode_val(chunk[0])?;
        let c1 = if chunk.len() > 1 {
            decode_val(chunk[1])?
        } else {
            0
        };
        let c2 = if chunk.len() > 2 {
            decode_val(chunk[2])?
        } else {
            0
        };
        let c3 = if chunk.len() > 3 {
            decode_val(chunk[3])?
        } else {
            0
        };

        out.push((c0 << 2) | (c1 >> 4));
        if chunk.len() > 2 {
            out.push(((c1 & 15) << 4) | (c2 >> 2));
        }
        if chunk.len() > 3 {
            out.push(((c2 & 3) << 6) | c3);
        }
    }
    let val: Value =
        serde_json::from_slice(&out).map_err(|_| QueryError::invalid_argument("invalid cursor"))?;
    match val {
        Value::Array(arr) if arr.len() == expected_size => Ok(arr),
        _ => Err(QueryError::invalid_argument("invalid cursor")),
    }
}

pub fn check_limit(limit: i64, kind: &str) -> Result<i64, QueryError> {
    let max = match kind {
        "contacts" => 200,
        "recent" => 200,
        "history" => 500,
        "search" => 100,
        _ => 200,
    };
    if limit < 1 || limit > max {
        return Err(QueryError::invalid_argument(format!(
            "limit must be 1..{}",
            max
        )));
    }
    Ok(limit)
}

pub const MESSAGE_SELECT: &str = "SELECT m.rowid AS rid, m.message_id, m.contact_id, m.seq, m.direction, m.sender, m.kind, m.text, \
     m.observed_at, m.capture, m.meta, m.send_key, s.status AS send_status, s.echo_observed, \
     m.media_id, md.kind AS media_kind, md.filename AS media_filename, md.source AS media_source, \
     md.understanding AS media_understanding \
     FROM messages m LEFT JOIN sends s ON s.send_key=m.send_key LEFT JOIN media md ON md.media_id=m.media_id ";

pub struct HistoryQuery {
    path: PathBuf,
    conn: Connection,
    default_recent: i64,
    allowlist: Option<HashSet<String>>,
}

impl HistoryQuery {
    pub fn open(
        db_path: impl AsRef<Path>,
        default_recent: Option<i64>,
    ) -> Result<Self, QueryError> {
        let path = db_path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(QueryError::not_found("history database does not exist"));
        }
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|e| QueryError::new("open_failed", e.to_string()))?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        Ok(Self {
            path,
            conn,
            default_recent: default_recent.unwrap_or(5),
            allowlist: None,
        })
    }

    pub fn with_allowlist(mut self, allowed: HashSet<String>) -> Self {
        self.allowlist = Some(allowed);
        self
    }

    pub fn raw_connection(&self) -> &Connection {
        &self.conn
    }

    // Contacts -----------------------------------------------------------------

    fn _merged(&self) -> Result<HashMap<String, Option<String>>, QueryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT contact_id, merged_into FROM contacts")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let mut map = HashMap::new();
        for r in rows {
            let (cid, merged) = r.map_err(|e| QueryError::new("db_error", e.to_string()))?;
            map.insert(cid, merged);
        }
        Ok(map)
    }

    fn _canonical(
        &self,
        mut contact_id: String,
        merged: &HashMap<String, Option<String>>,
    ) -> Result<String, QueryError> {
        if !merged.contains_key(&contact_id) {
            return Err(QueryError::not_found("unknown contact"));
        }
        let mut seen = HashSet::new();
        while let Some(Some(next)) = merged.get(&contact_id) {
            if seen.contains(&contact_id) {
                break;
            }
            seen.insert(contact_id.clone());
            contact_id = next.clone();
        }
        Ok(contact_id)
    }

    fn _family(&self, contact_id: &str) -> Result<(String, Vec<String>), QueryError> {
        let merged = self._merged()?;
        let root = self._canonical(contact_id.to_string(), &merged)?;
        let mut fam = Vec::new();
        for c in merged.keys() {
            if self._canonical(c.clone(), &merged)? == root {
                fam.push(c.clone());
            }
        }
        fam.sort();
        Ok((root, fam))
    }

    fn _contact(&self, root: &str, family: &[String]) -> Result<Value, QueryError> {
        let row_opt = self
            .conn
            .query_row(
                "SELECT contact_id, account_id, display_name FROM contacts WHERE contact_id=?",
                params![root],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let (contact_id, account_id, display_name) = match row_opt {
            Some(v) => v,
            None => return Err(QueryError::not_found("unknown contact")),
        };

        let marks = family.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let names_sql = format!(
            "SELECT DISTINCT raw_name FROM contact_names WHERE contact_id IN ({}) ORDER BY id",
            marks
        );
        let params_vec: Vec<&dyn rusqlite::ToSql> =
            family.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let mut names_stmt = self
            .conn
            .prepare(&names_sql)
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let names_mapped = names_stmt
            .query_map(rusqlite::params_from_iter(params_vec.clone()), |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let mut names = Vec::new();
        for n in names_mapped {
            names.push(n.map_err(|e| QueryError::new("db_error", e.to_string()))?);
        }

        let stats_sql = format!(
            "SELECT COUNT(*), MAX(observed_at) FROM messages WHERE contact_id IN ({})",
            marks
        );
        let (count, max_observed): (i64, Option<i64>) = self
            .conn
            .query_row(&stats_sql, rusqlite::params_from_iter(params_vec), |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        let merged_from: Vec<String> = family.iter().filter(|&c| c != root).cloned().collect();

        let profile_opt: Option<Value> = self
            .conn
            .query_row(
                "SELECT category, wechat_id, nickname, remark, region, source, added_on, common_groups, enterprise, real_name, tags, friend_permission, identity, first_synced_at, last_synced_at, removed_at FROM contact_profiles WHERE contact_id = ?",
                params![root],
                |r| {
                    let tags_str: String = r.get(10)?;
                    let tags_val: Value = serde_json::from_str(&tags_str).unwrap_or_else(|_| json!([]));
                    Ok(json!({
                        "category": r.get::<_, String>(0)?,
                        "wechat_id": r.get::<_, Option<String>>(1)?,
                        "nickname": r.get::<_, String>(2)?,
                        "remark": r.get::<_, Option<String>>(3)?,
                        "region": r.get::<_, Option<String>>(4)?,
                        "source": r.get::<_, Option<String>>(5)?,
                        "added_on": r.get::<_, Option<String>>(6)?,
                        "common_groups": r.get::<_, Option<i64>>(7)?,
                        "enterprise": r.get::<_, Option<String>>(8)?,
                        "real_name": r.get::<_, Option<String>>(9)?,
                        "tags": tags_val,
                        "friend_permission": r.get::<_, Option<String>>(11)?,
                        "identity": r.get::<_, String>(12)?,
                        "first_synced_at": r.get::<_, i64>(13)?,
                        "last_synced_at": r.get::<_, i64>(14)?,
                        "removed_at": r.get::<_, Option<i64>>(15)?,
                    }))
                },
            )
            .optional()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        Ok(json!({
            "contact_id": contact_id,
            "account_id": account_id,
            "display_name": display_name,
            "names": names,
            "merged_from": merged_from,
            "message_count": count,
            "last_observed_at": max_observed,
            "profile": profile_opt,
        }))
    }

    pub fn list_contacts(
        &self,
        query: Option<&str>,
        cursor: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Value, QueryError> {
        self.list_contacts_with_category(query, cursor, limit, None)
    }

    pub fn list_contacts_with_category(
        &self,
        query: Option<&str>,
        cursor: Option<&str>,
        limit: Option<i64>,
        category: Option<&str>,
    ) -> Result<Value, QueryError> {
        let lim = check_limit(limit.unwrap_or(50), "contacts")?;
        let after: i64 = if let Some(cur) = cursor {
            let decoded = decode_cursor(cur, 1)?;
            decoded[0]
                .as_i64()
                .ok_or_else(|| QueryError::invalid_argument("invalid cursor"))?
        } else {
            0
        };

        let mut where_clause = "c.rowid > ? AND c.merged_into IS NULL".to_string();
        let mut params_vec: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Integer(after)];

        if let Some(q) = query {
            where_clause.push_str(" AND EXISTS (SELECT 1 FROM contact_names n WHERE n.contact_id=c.contact_id AND instr(n.name_key, ?) > 0)");
            params_vec.push(rusqlite::types::Value::Text(name_key(q)));
        }

        if let Some(cat) = category {
            where_clause.push_str(" AND EXISTS (SELECT 1 FROM contact_profiles cp WHERE cp.contact_id=c.contact_id AND cp.category=?)");
            params_vec.push(rusqlite::types::Value::Text(cat.to_string()));
        }

        if let Some(ref allowed) = self.allowlist {
            if allowed.is_empty() {
                return Ok(json!({
                    "items": [],
                    "next_cursor": None::<String>,
                }));
            }
            let marks = allowed.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            where_clause.push_str(&format!(" AND c.contact_id IN ({})", marks));
            for cid in allowed {
                params_vec.push(rusqlite::types::Value::Text(cid.clone()));
            }
        }

        let sql = format!(
            "SELECT c.rowid AS rid, c.contact_id FROM contacts c WHERE {} ORDER BY c.rowid LIMIT ?",
            where_clause
        );
        params_vec.push(rusqlite::types::Value::Integer(lim + 1));

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec
            .iter()
            .map(|p| p as &dyn rusqlite::ToSql)
            .collect();
        let mapped = stmt
            .query_map(rusqlite::params_from_iter(params_refs), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        let mut rows_vec = Vec::new();
        for r in mapped {
            rows_vec.push(r.map_err(|e| QueryError::new("db_error", e.to_string()))?);
        }

        let has_more = rows_vec.len() > lim as usize;
        let take_count = rows_vec.len().min(lim as usize);
        let mut items = Vec::new();
        for (_, cid) in &rows_vec[..take_count] {
            let (root, fam) = self._family(cid)?;
            items.push(self._contact(&root, &fam)?);
        }

        let next_cursor = if has_more {
            let last_rid = rows_vec[lim as usize - 1].0;
            Some(encode_cursor(&vec![last_rid]))
        } else {
            None
        };

        Ok(json!({
            "items": items,
            "next_cursor": next_cursor,
        }))
    }

    pub fn get_contact(
        &self,
        contact_id: Option<&str>,
        name: Option<&str>,
    ) -> Result<Value, QueryError> {
        if contact_id.is_some() == name.is_some() {
            return Err(QueryError::invalid_argument(
                "give exactly one of contact_id or name",
            ));
        }
        let target_cid = if let Some(n) = name {
            let merged = self._merged()?;
            let key = name_key(n);
            let mut stmt = self
                .conn
                .prepare("SELECT contact_id FROM contact_names WHERE name_key=? AND source IN ('config','alias','addressbook')")
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let mapped = stmt
                .query_map(params![key], |r| r.get::<_, String>(0))
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let mut owners = HashSet::new();
            for r in mapped {
                let cid = r.map_err(|e| QueryError::new("db_error", e.to_string()))?;
                owners.insert(self._canonical(cid, &merged)?);
            }
            if owners.is_empty() {
                return Err(QueryError::not_found("unknown contact name"));
            }
            if owners.len() > 1 {
                return Err(QueryError::invalid_argument(
                    "name matches several contacts; use contact_id",
                ));
            }
            owners.into_iter().next().unwrap()
        } else {
            contact_id.unwrap().to_string()
        };

        if let Some(ref allowed) = self.allowlist {
            if !allowed.contains(&target_cid) {
                return Err(QueryError::not_found("unknown contact"));
            }
        }

        let (root, fam) = self._family(&target_cid)?;
        self._contact(&root, &fam)
    }

    // Messages -----------------------------------------------------------------

    fn _message(&self, row: &rusqlite::Row, with_meta: bool) -> Result<Value, QueryError> {
        let message_id: String = row
            .get("message_id")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let contact_id: String = row
            .get("contact_id")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let seq: i64 = row
            .get("seq")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let direction: String = row
            .get("direction")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let sender: String = row
            .get("sender")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let kind: String = row
            .get("kind")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let text: String = row
            .get("text")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let observed_at: i64 = row
            .get("observed_at")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let capture: String = row
            .get("capture")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let send_key: Option<String> = row
            .get("send_key")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let send_status: Option<String> = row
            .get("send_status")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let echo_observed: Option<i64> = row
            .get("echo_observed")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let media_id: Option<String> = row
            .get("media_id")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let media_kind: Option<String> = row
            .get("media_kind")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let media_filename: Option<String> = row
            .get("media_filename")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let media_source: Option<String> = row
            .get("media_source")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let media_understanding: Option<String> = row
            .get("media_understanding")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        let send = if let Some(sk) = send_key {
            Some(json!({
                "send_key": sk,
                "status": send_status,
                "echo_observed": echo_observed.unwrap_or(0) != 0,
            }))
        } else {
            None
        };

        let media = if let Some(mid) = media_id {
            Some(json!({
                "media_id": mid,
                "kind": media_kind,
                "filename": media_filename,
                "source": media_source,
                "understanding": media_understanding,
            }))
        } else {
            None
        };

        let mut item = json!({
            "message_id": message_id,
            "contact_id": contact_id,
            "seq": seq,
            "direction": direction,
            "sender": sender,
            "kind": kind,
            "text": text,
            "observed_at": observed_at,
            "capture": capture,
            "send": send,
            "media": media,
        });

        if with_meta {
            let meta_str: String = row
                .get("meta")
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let meta_val: Value = serde_json::from_str(&meta_str).unwrap_or(json!({}));
            item["meta"] = meta_val;
        }

        Ok(item)
    }

    fn _range(
        &self,
        family: &[String],
        since: Option<i64>,
        until: Option<i64>,
        cursor: Option<&str>,
        order: &str,
    ) -> Result<(String, Vec<Box<dyn rusqlite::ToSql>>), QueryError> {
        if order != "asc" && order != "desc" {
            return Err(QueryError::invalid_argument("order must be asc or desc"));
        }
        let marks = family.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let mut where_clauses = vec![format!("m.contact_id IN ({})", marks)];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        for f in family {
            params.push(Box::new(f.clone()));
        }

        if let Some(s) = since {
            where_clauses.push("m.observed_at >= ?".to_string());
            params.push(Box::new(s));
        }
        if let Some(u) = until {
            where_clauses.push("m.observed_at < ?".to_string());
            params.push(Box::new(u));
        }

        if let Some(cur) = cursor {
            let decoded = decode_cursor(cur, 2)?;
            let rid = decoded[0]
                .as_i64()
                .ok_or_else(|| QueryError::invalid_argument("invalid cursor"))?;
            let cur_order = decoded[1]
                .as_str()
                .ok_or_else(|| QueryError::invalid_argument("invalid cursor"))?;
            if cur_order != order {
                return Err(QueryError::invalid_argument("cursor order mismatch"));
            }
            if order == "asc" {
                where_clauses.push("m.rowid > ?".to_string());
            } else {
                where_clauses.push("m.rowid < ?".to_string());
            }
            params.push(Box::new(rid));
        }

        Ok((where_clauses.join(" AND "), params))
    }

    pub fn get_history(
        &self,
        contact_id: &str,
        since: Option<i64>,
        until: Option<i64>,
        cursor: Option<&str>,
        limit: Option<i64>,
        order: Option<&str>,
    ) -> Result<Value, QueryError> {
        let lim = check_limit(limit.unwrap_or(100), "history")?;
        let ord = order.unwrap_or("asc");
        let (_, fam) = self._family(contact_id)?;
        let (where_clause, mut q_params) = self._range(&fam, since, until, cursor, ord)?;
        let direction = if ord == "asc" { "ASC" } else { "DESC" };
        let sql = format!(
            "{} WHERE {} ORDER BY m.rowid {} LIMIT ?",
            MESSAGE_SELECT, where_clause, direction
        );
        q_params.push(Box::new(lim + 1));

        let params_refs: Vec<&dyn rusqlite::ToSql> = q_params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let mut rows = stmt
            .query(rusqlite::params_from_iter(params_refs))
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        let mut items = Vec::new();
        let mut rids = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?
        {
            let rid: i64 = row
                .get("rid")
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            rids.push(rid);
            items.push(self._message(row, false)?);
        }

        let has_more = items.len() > lim as usize;
        if has_more {
            items.truncate(lim as usize);
            rids.truncate(lim as usize);
        }

        let next_cursor = if has_more {
            let last_rid = *rids.last().unwrap();
            Some(encode_cursor(&vec![json!(last_rid), json!(ord)]))
        } else {
            None
        };

        let mut page = json!({
            "items": items,
            "next_cursor": next_cursor,
        });

        if ord == "asc" {
            let end_rid = if !rids.is_empty() {
                *rids.last().unwrap()
            } else if let Some(cur) = cursor {
                decode_cursor(cur, 2)?[0].as_i64().unwrap_or(0)
            } else {
                0
            };
            page["tail_cursor"] = json!(encode_cursor(&vec![json!(end_rid), json!("asc")]));
        }

        Ok(page)
    }

    pub fn get_recent_messages(
        &self,
        contact_id: &str,
        limit: Option<i64>,
    ) -> Result<Value, QueryError> {
        let lim = check_limit(limit.unwrap_or(self.default_recent), "recent")?;
        let page = self.get_history(contact_id, None, None, None, Some(lim), Some("desc"))?;
        let mut items = page["items"].as_array().cloned().unwrap_or(vec![]);
        items.reverse();
        Ok(json!({ "items": items }))
    }

    pub fn search_messages(
        &self,
        q: &str,
        contact_id: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
        cursor: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Value, QueryError> {
        let lim = check_limit(limit.unwrap_or(20), "search")?;
        let trimmed_q = q.trim();
        if trimmed_q.is_empty() {
            return Err(QueryError::invalid_argument("q is required"));
        }

        let family = if let Some(cid) = contact_id {
            self._family(cid)?.1
        } else {
            let mut stmt = self
                .conn
                .prepare("SELECT contact_id FROM contacts")
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let mapped = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let mut all_c = Vec::new();
            for c in mapped {
                all_c.push(c.map_err(|e| QueryError::new("db_error", e.to_string()))?);
            }
            all_c
        };

        let (mut where_clause, mut params) = self._range(&family, since, until, cursor, "desc")?;
        let escaped_like = format!(
            "%{}%",
            trimmed_q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let understood = " OR md.understanding LIKE ? ESCAPE '\\'";

        if trimmed_q.chars().count() >= 3 {
            where_clause.push_str(&format!(
                " AND (m.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?){})",
                understood
            ));
            let fts_phrase = format!("\"{}\"", trimmed_q.replace('"', "\"\""));
            params.push(Box::new(fts_phrase));
            params.push(Box::new(escaped_like));
        } else {
            where_clause.push_str(&format!(" AND (m.text LIKE ? ESCAPE '\\'{})", understood));
            params.push(Box::new(escaped_like.clone()));
            params.push(Box::new(escaped_like));
        }

        let sql = format!(
            "{} WHERE {} ORDER BY m.rowid DESC LIMIT ?",
            MESSAGE_SELECT, where_clause
        );
        params.push(Box::new(lim + 1));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let mut rows = stmt
            .query(rusqlite::params_from_iter(params_refs))
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        let mut items = Vec::new();
        let mut rids = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?
        {
            let rid: i64 = row
                .get("rid")
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            rids.push(rid);
            let mut item = self._message(row, false)?;

            let cid: String = item["contact_id"].as_str().unwrap().to_string();
            let seq: i64 = item["seq"].as_i64().unwrap();
            let mut ctx_stmt = self.conn.prepare(
                "SELECT message_id FROM messages WHERE contact_id=? AND seq BETWEEN ? AND ? AND seq != ? ORDER BY seq",
            ).map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let ctx_rows = ctx_stmt
                .query_map(params![cid, seq - 2, seq + 2, seq], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let mut ctx_ids = Vec::new();
            for id in ctx_rows {
                ctx_ids.push(id.map_err(|e| QueryError::new("db_error", e.to_string()))?);
            }
            item["context_ids"] = json!(ctx_ids);

            items.push(item);
        }

        let has_more = items.len() > lim as usize;
        if has_more {
            items.truncate(lim as usize);
            rids.truncate(lim as usize);
        }

        let next_cursor = if has_more {
            let last_rid = *rids.last().unwrap();
            Some(encode_cursor(&vec![json!(last_rid), json!("desc")]))
        } else {
            None
        };

        Ok(json!({
            "items": items,
            "next_cursor": next_cursor,
        }))
    }

    /// Search an explicit contact set, filtering before limit and cursor selection.
    #[allow(dead_code, clippy::too_many_arguments)]
    pub fn search_messages_in(
        &self,
        q: &str,
        contacts: &HashSet<String>,
        since: Option<i64>,
        until: Option<i64>,
        cursor: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Value, QueryError> {
        let lim = check_limit(limit.unwrap_or(20), "search")?;
        let trimmed_q = q.trim();
        if trimmed_q.is_empty() {
            return Err(QueryError::invalid_argument("q is required"));
        }

        if contacts.is_empty() {
            return Ok(json!({"items": [], "next_cursor": null}));
        }
        let family: Vec<String> = contacts.iter().cloned().collect();

        let (mut where_clause, mut params) = self._range(&family, since, until, cursor, "desc")?;
        where_clause.push_str(
            " AND m.account_id = (SELECT account_id FROM contacts WHERE contact_id=m.contact_id)",
        );
        let escaped_like = format!(
            "%{}%",
            trimmed_q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let understood = " OR md.understanding LIKE ? ESCAPE '\\'";

        if trimmed_q.chars().count() >= 3 {
            where_clause.push_str(&format!(
                " AND (m.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?){})",
                understood
            ));
            let fts_phrase = format!("\"{}\"", trimmed_q.replace('"', "\"\""));
            params.push(Box::new(fts_phrase));
            params.push(Box::new(escaped_like));
        } else {
            where_clause.push_str(&format!(" AND (m.text LIKE ? ESCAPE '\\'{})", understood));
            params.push(Box::new(escaped_like.clone()));
            params.push(Box::new(escaped_like));
        }

        let sql = format!(
            "{} WHERE {} ORDER BY m.rowid DESC LIMIT ?",
            MESSAGE_SELECT, where_clause
        );
        params.push(Box::new(lim + 1));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let mut rows = stmt
            .query(rusqlite::params_from_iter(params_refs))
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        let mut items = Vec::new();
        let mut rids = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?
        {
            let rid: i64 = row
                .get("rid")
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            rids.push(rid);
            let mut item = self._message(row, false)?;

            let cid: String = item["contact_id"].as_str().unwrap().to_string();
            let seq: i64 = item["seq"].as_i64().unwrap();
            let mut ctx_stmt = self.conn.prepare(
                "SELECT message_id FROM messages WHERE contact_id=? AND account_id=(SELECT account_id FROM contacts WHERE contact_id=messages.contact_id) AND seq BETWEEN ? AND ? AND seq != ? ORDER BY seq",
            ).map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let ctx_rows = ctx_stmt
                .query_map(params![cid, seq - 2, seq + 2, seq], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| QueryError::new("db_error", e.to_string()))?;
            let mut ctx_ids = Vec::new();
            for id in ctx_rows {
                ctx_ids.push(id.map_err(|e| QueryError::new("db_error", e.to_string()))?);
            }
            item["context_ids"] = json!(ctx_ids);

            items.push(item);
        }

        let has_more = items.len() > lim as usize;
        if has_more {
            items.truncate(lim as usize);
            rids.truncate(lim as usize);
        }

        let next_cursor = if has_more {
            let last_rid = *rids.last().unwrap();
            Some(encode_cursor(&vec![json!(last_rid), json!("desc")]))
        } else {
            None
        };

        Ok(json!({
            "items": items,
            "next_cursor": next_cursor,
        }))
    }

    /// Media belongs to messages, rather than directly to a contact or account.
    #[allow(dead_code)]
    pub fn media_owners(
        &self,
        media_id: &str,
    ) -> Result<Vec<(String, String, String)>, QueryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT message_id, contact_id, account_id FROM messages WHERE media_id=?")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        stmt.query_map([media_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| QueryError::new("db_error", e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| QueryError::new("db_error", e.to_string()))
    }

    pub fn get_message(&self, message_id: &str) -> Result<Value, QueryError> {
        let sql = format!("{} WHERE m.message_id=?", MESSAGE_SELECT);
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let mut rows = stmt
            .query(params![message_id])
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        if let Some(r) = rows
            .next()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?
        {
            self._message(r, true)
        } else {
            Err(QueryError::not_found("unknown message"))
        }
    }

    pub fn get_media(&self, media_id: &str) -> Result<(Value, PathBuf), QueryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT media_id, kind, filename, extension, size, source, path, understanding, understood_at FROM media WHERE media_id=?")
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let row_opt = stmt
            .query_row(params![media_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                ))
            })
            .optional()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;

        let (mid, kind, filename, extension, size, source, rel_path, understanding, understood_at) =
            match row_opt {
                Some(v) => v,
                None => return Err(QueryError::not_found("unknown media")),
            };

        let media_root = self
            .path
            .parent()
            .unwrap_or(Path::new("."))
            .join("media")
            .canonicalize()
            .map_err(|_| QueryError::not_found("media file missing"))?;
        let full_path = media_root
            .join(&rel_path)
            .canonicalize()
            .map_err(|_| QueryError::not_found("media file missing"))?;

        if !full_path.starts_with(&media_root) || !full_path.is_file() {
            return Err(QueryError::not_found("media file missing"));
        }

        let info = json!({
            "media_id": mid,
            "kind": kind,
            "filename": filename,
            "extension": extension,
            "size": size,
            "source": source,
            "understanding": understanding,
            "understood_at": understood_at,
        });

        Ok((info, full_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::history::store::HistoryStore;
    use crate::core::model::Bubble;
    use serde_json::json;
    use std::fs;

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

    struct TestTempDir(PathBuf);
    impl TestTempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "wechat-history-test-{}",
                uuid::Uuid::new_v4().simple()
            ));
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

    struct TestFixture {
        _temp: TestTempDir,
        state: PathBuf,
        writer: HistoryStore,
        q: HistoryQuery,
        c: String,
        c2: String,
        visible: Vec<Bubble>,
        expected_texts: Vec<String>,
    }

    fn setup_fixture() -> TestFixture {
        let temp = TestTempDir::new();
        let state = temp.path().join("state");
        let writer = HistoryStore::open(&state).unwrap();
        let c = writer
            .ensure_contact(
                "acct",
                &json!({"recipient": "🍓小美", "kind": "private"}),
                true,
            )
            .unwrap();
        let c2 = writer
            .ensure_contact("acct", &json!({"recipient": "乙", "kind": "private"}), true)
            .unwrap();

        let visible = vec![
            bub("你好", "inbound_visual", "text"),
            bub("还是上次那个款", "inbound_visual", "text"),
            bub("已发顺丰", "outbound_visual", "text"),
            bub("收到", "inbound_visual", "text"),
        ];

        writer
            .record_frame(&c, &fr(visible.clone()), None, None, false)
            .unwrap();
        writer
            .record_bot_reply(
                &c,
                "k1",
                "好的，已为您登记",
                "generated",
                &json!({}),
                &[],
                None,
                None,
            )
            .unwrap();
        writer.update_send("k1", "triggered", None).unwrap();

        let mut frame2_bubbles = visible.clone();
        frame2_bubbles.push(bub("好的，已为您登记", "outbound_visual", "text"));
        writer
            .record_frame(&c, &fr(frame2_bubbles), None, None, false)
            .unwrap();

        writer
            .record_frame(
                &c2,
                &fr(vec![bub("乙的消息", "inbound_visual", "text")]),
                None,
                None,
                false,
            )
            .unwrap();

        let expected_texts = vec![
            "你好".to_string(),
            "还是上次那个款".to_string(),
            "已发顺丰".to_string(),
            "收到".to_string(),
            "好的，已为您登记".to_string(),
        ];

        let q = HistoryQuery::open(state.join("history").join("history.sqlite3"), None).unwrap();

        TestFixture {
            _temp: temp,
            state,
            writer,
            q,
            c,
            c2,
            visible,
            expected_texts,
        }
    }

    fn texts(page: &Value) -> Vec<String> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["text"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn test_query_read_only_connection_cannot_write() {
        let f = setup_fixture();
        let res = f.q.raw_connection().execute(
            "insert into merges(from_contact,into_contact,created_at) values('x','y',1)",
            [],
        );
        assert!(res.is_err());
    }

    #[test]
    fn test_query_recent_messages_default_limit_and_order() {
        let f = setup_fixture();
        let q =
            HistoryQuery::open(f.state.join("history").join("history.sqlite3"), Some(3)).unwrap();
        assert_eq!(
            texts(&q.get_recent_messages(&f.c, None).unwrap()),
            vec!["已发顺丰", "收到", "好的，已为您登记"]
        );
    }

    #[test]
    fn test_query_pagination_stable_while_writing() {
        let f = setup_fixture();
        let mut page =
            f.q.get_history(&f.c, None, None, None, Some(3), None)
                .unwrap();
        let mut seen = texts(&page);

        let mut frame_new = f.visible.clone();
        frame_new.push(bub("好的，已为您登记", "outbound_visual", "text"));
        frame_new.push(bub("新", "inbound_visual", "text"));
        f.writer
            .record_frame(&f.c, &fr(frame_new), None, None, false)
            .unwrap();

        while let Some(cur) = page["next_cursor"].as_str() {
            page =
                f.q.get_history(&f.c, None, None, Some(cur), Some(3), None)
                    .unwrap();
            seen.extend(texts(&page));
        }

        let mut expected = f.expected_texts.clone();
        expected.push("新".to_string());
        assert_eq!(seen, expected);
    }

    #[test]
    fn test_query_tail_cursor_follows_new_messages() {
        let f = setup_fixture();
        let page =
            f.q.get_history(&f.c, None, None, None, Some(500), None)
                .unwrap();
        assert!(page["next_cursor"].is_null());

        let mut frame_new = f.visible.clone();
        frame_new.push(bub("好的，已为您登记", "outbound_visual", "text"));
        frame_new.push(bub("后来的", "inbound_visual", "text"));
        f.writer
            .record_frame(&f.c, &fr(frame_new), None, None, false)
            .unwrap();

        let tail = page["tail_cursor"].as_str().unwrap();
        let newer =
            f.q.get_history(&f.c, None, None, Some(tail), None, None)
                .unwrap();
        assert_eq!(texts(&newer), vec!["后来的"]);
    }

    #[test]
    fn test_query_clock_rollback_does_not_skip_messages() {
        let f = setup_fixture();
        let page =
            f.q.get_history(&f.c, None, None, None, Some(500), None)
                .unwrap();

        let mut frame_new = f.visible.clone();
        frame_new.push(bub("好的，已为您登记", "outbound_visual", "text"));
        frame_new.push(bub("回拨后", "inbound_visual", "text"));
        f.writer
            .record_frame(&f.c, &fr(frame_new), None, None, false)
            .unwrap();

        f.writer
            .connection()
            .lock()
            .unwrap()
            .execute("update messages set observed_at=1 where text='回拨后'", [])
            .unwrap();

        let tail = page["tail_cursor"].as_str().unwrap();
        let res =
            f.q.get_history(&f.c, None, None, Some(tail), None, None)
                .unwrap();
        assert_eq!(texts(&res), vec!["回拨后"]);
    }

    #[test]
    fn test_query_desc_order_and_time_range() {
        let f = setup_fixture();
        let page =
            f.q.get_history(&f.c, None, None, None, Some(2), Some("desc"))
                .unwrap();
        assert_eq!(texts(&page), vec!["好的，已为您登记", "收到"]);

        let future =
            f.q.get_history(&f.c, Some(10i64.pow(15)), None, None, None, None)
                .unwrap();
        assert_eq!(future["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_query_concurrent_read_during_open_write_transaction() {
        let f = setup_fixture();
        f.writer
            .connection()
            .lock()
            .unwrap()
            .execute("BEGIN IMMEDIATE", [])
            .unwrap();
        f.writer
            .connection()
            .lock()
            .unwrap()
            .execute(
                "insert into merges(from_contact,into_contact,created_at) values('a','b',1)",
                [],
            )
            .unwrap();

        let recent = f.q.get_recent_messages(&f.c, None).unwrap();
        assert_eq!(recent["items"].as_array().unwrap().len(), 5);

        f.writer
            .connection()
            .lock()
            .unwrap()
            .execute("ROLLBACK", [])
            .unwrap();
    }

    #[test]
    fn test_query_search_trigram_and_short_term_fallback() {
        let f = setup_fixture();
        assert_eq!(
            texts(
                &f.q.search_messages("上次那个", None, None, None, None, None)
                    .unwrap()
            ),
            vec!["还是上次那个款"]
        );
        assert_eq!(
            texts(
                &f.q.search_messages("顺丰", None, None, None, None, None)
                    .unwrap()
            ),
            vec!["已发顺丰"]
        );
        let hits =
            f.q.search_messages("上次那个", None, None, None, None, None)
                .unwrap();
        let hit = &hits["items"][0];
        assert_eq!(hit["context_ids"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_query_search_scoped_to_contact() {
        let f = setup_fixture();
        let scoped =
            f.q.search_messages("消息", Some(&f.c), None, None, None, None)
                .unwrap();
        assert_eq!(scoped["items"].as_array().unwrap().len(), 0);

        let hit =
            f.q.search_messages("乙的消息", None, None, None, None, None)
                .unwrap();
        assert_eq!(texts(&hit), vec!["乙的消息"]);
    }

    #[test]
    fn test_query_merged_contact_history_is_included() {
        let f = setup_fixture();
        f.writer.merge(&f.c2, &f.c, "").unwrap();
        let history = texts(&f.q.get_history(&f.c, None, None, None, None, None).unwrap());
        assert!(history.contains(&"乙的消息".to_string()));

        let listed: Vec<String> = f.q.list_contacts(None, None, None).unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["contact_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(listed, vec![f.c.clone()]);
        assert_eq!(
            f.q.get_contact(Some(&f.c2), None).unwrap()["contact_id"]
                .as_str()
                .unwrap(),
            f.c
        );
    }

    #[test]
    fn test_query_bot_message_carries_send_status() {
        let f = setup_fixture();
        let recent = f.q.get_recent_messages(&f.c, None).unwrap();
        let items = recent["items"].as_array().unwrap();
        let last = &items[items.len() - 1];
        assert_eq!(last["sender"].as_str().unwrap(), "bot");
        assert_eq!(last["send"]["status"].as_str().unwrap(), "triggered");
        assert_eq!(last["send"]["echo_observed"].as_bool().unwrap(), true);

        assert!(items[0]["send"].is_null());
    }

    #[test]
    fn test_query_limits_cursor_and_unknown_ids() {
        let f = setup_fixture();
        let err1 =
            f.q.get_history(&f.c, None, None, None, Some(501), None)
                .unwrap_err();
        assert_eq!(err1.code, "invalid_argument");

        let err2 =
            f.q.get_history(&f.c, None, None, Some("garbage"), None, None)
                .unwrap_err();
        assert_eq!(err2.code, "invalid_argument");

        let err3 = f.q.get_message("m_missing").unwrap_err();
        assert_eq!(err3.code, "not_found");

        let err4 =
            f.q.get_history("c_missing", None, None, None, None, None)
                .unwrap_err();
        assert_eq!(err4.code, "not_found");
    }

    #[test]
    fn test_query_name_lookup_uses_name_key() {
        let f = setup_fixture();
        let a = f.q.get_contact(None, Some("🍓小美")).unwrap();
        let b = f.q.get_contact(None, Some("小美")).unwrap();
        assert_eq!(a["contact_id"], b["contact_id"]);
        assert_eq!(a["message_count"], 5);
        let names: Vec<String> = a["names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"🍓小美".to_string()));
    }

    #[test]
    fn test_query_list_contacts_query_and_paging() {
        let f = setup_fixture();
        let queried = f.q.list_contacts(Some("小美"), None, None).unwrap();
        let display_names: Vec<String> = queried["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["display_name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(display_names, vec!["🍓小美"]);

        let first = f.q.list_contacts(None, None, Some(1)).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        let cur = first["next_cursor"].as_str().unwrap();
        let second = f.q.list_contacts(None, Some(cur), Some(1)).unwrap();
        assert_ne!(
            first["items"][0]["contact_id"],
            second["items"][0]["contact_id"]
        );
    }

    #[test]
    fn test_query_media_understanding_is_exposed_and_searchable() {
        let f = setup_fixture();
        let mid = f.q.get_recent_messages(&f.c, None).unwrap()["items"][0]["message_id"]
            .as_str()
            .unwrap()
            .to_string();
        let src = f._temp.path().join("q.png");
        fs::write(&src, b"img2").unwrap();
        let media_id = f
            .writer
            .attach_media(&mid, &src, "image", None, "original", None)
            .unwrap();
        f.writer
            .set_understanding(&media_id, "顺丰快递面单，单号SF123")
            .unwrap();

        let message = f.q.get_message(&mid).unwrap();
        assert_eq!(
            message["media"]["understanding"].as_str().unwrap(),
            "顺丰快递面单，单号SF123"
        );
        assert_eq!(message["media"]["source"].as_str().unwrap(), "original");

        let (info, _) = f.q.get_media(&media_id).unwrap();
        assert_eq!(
            info["understanding"].as_str().unwrap(),
            "顺丰快递面单，单号SF123"
        );

        let hits =
            f.q.search_messages("快递面单", Some(&f.c), None, None, None, None)
                .unwrap();
        let hit_mids: Vec<String> = hits["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["message_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(hit_mids, vec![mid.clone()]);

        let hits2 =
            f.q.search_messages("SF", Some(&f.c), None, None, None, None)
                .unwrap();
        let hit2_mids: Vec<String> = hits2["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["message_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(hit2_mids, vec![mid]);
    }

    #[test]
    fn test_query_get_message_and_media() {
        let f = setup_fixture();
        let mid = f.q.get_recent_messages(&f.c, None).unwrap()["items"][0]["message_id"]
            .as_str()
            .unwrap()
            .to_string();
        let msg = f.q.get_message(&mid).unwrap();
        assert!(msg.get("meta").is_some());

        let src = f._temp.path().join("p.png");
        fs::write(&src, b"img").unwrap();
        let media_id = f
            .writer
            .attach_media(&mid, &src, "image", None, "chat_thumbnail", None)
            .unwrap();

        let (info, path) = f.q.get_media(&media_id).unwrap();
        assert_eq!(info["kind"].as_str().unwrap(), "image");
        assert_eq!(fs::read(path).unwrap(), b"img");
        assert_eq!(
            f.q.get_message(&mid).unwrap()["media"]["media_id"]
                .as_str()
                .unwrap(),
            media_id
        );
    }
}
