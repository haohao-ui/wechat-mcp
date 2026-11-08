//! 当前登录的微信账号。
//!
//! 实测（2026-11-08，专用 Mac，微信 4.1.15）：微信为每个登录过的账号在
//! `~/Library/Containers/com.tencent.xinWeChat/Data/Documents/xwechat_files/<wxid>_<4位>/` 建一个数据目录，
//! 只有当前登录账号的目录被微信进程打开（170 个文件），其他账号目录一个都没打开。
//! `all_users`、`Backup` 是公共目录，不是账号。
//! 所以不点界面：看微信进程打开的是哪个账号目录，就知道当前登录的是谁。用 wxid（不会变）作账号标识。
//!
//! 聊天记录和联系人都挂在这个账号下：换号各自保留，换回来不用重新初始化。

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::core::state::{read_json, write_private};

const ACCOUNT_STATE_FILE: &str = "current-account.json";
const ADOPT_MARKER_FILE: &str = "account-adopted.json";

/// 从 `lsof -Fn` 的输出里找当前登录账号：取 xwechat_files 下被打开文件最多的账号目录，去掉末尾 `_xxxx` 后缀。
pub fn account_from_open_files(lsof_output: &str) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in lsof_output.lines() {
        let path = line.strip_prefix('n').unwrap_or(line);
        let Some(idx) = path.find("/xwechat_files/") else { continue };
        let rest = &path[idx + "/xwechat_files/".len()..];
        let dir = rest.split('/').next().unwrap_or("");
        if dir.is_empty() || dir == "all_users" || dir == "Backup" || !rest.contains('/') {
            continue;
        }
        *counts.entry(dir).or_default() += 1;
    }
    let (dir, _) = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))?;
    let account = match dir.rsplit_once('_') {
        Some((head, tail)) if !head.is_empty() && tail.len() == 4 && tail.chars().all(|c| c.is_ascii_hexdigit()) => head,
        _ => dir,
    };
    Some(account.to_string())
}

/// 读本机当前登录的微信账号；微信没运行或没登录（没有打开任何账号目录）返回 None。
pub fn detect_logged_in_account() -> Option<String> {
    let pids = Command::new("/usr/bin/pgrep").args(["-x", "WeChat"]).output().ok()?;
    let pids = String::from_utf8_lossy(&pids.stdout).to_string();
    for pid in pids.split_whitespace() {
        let Ok(out) = Command::new("/usr/sbin/lsof").args(["-n", "-P", "-Fn", "-p", pid]).output() else { continue };
        if let Some(acc) = account_from_open_files(&String::from_utf8_lossy(&out.stdout)) {
            return Some(acc);
        }
    }
    None
}

/// 记下当前账号，供控制命令（联系人列表、合并联系人等）使用
pub fn record_current_account(state_dir: &Path, account_id: &str, now: f64) -> Result<()> {
    write_private(&state_dir.join(ACCOUNT_STATE_FILE), &json!({"account_id": account_id, "detected_at": now}))
}

/// 控制命令用：核心记下的当前登录账号；还没有记录时用配置里的账号名
pub fn effective_account_id(state_dir: &Path, configured: &str) -> String {
    read_json::<Value>(&state_dir.join(ACCOUNT_STATE_FILE))
        .ok()
        .and_then(|v| v.get("account_id").and_then(|a| a.as_str()).map(str::to_string))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| configured.to_string())
}

/// 第一次识别到登录账号时（用户 2026-11-08 选择"清空重来"）：以前挂在配置账号名下的数据分不清属于哪个号
/// （这台机器登录过两个号），不迁移。先把数据库完整备份到 backups/before-account-<时间>/，
/// 再移走原库文件（含 -wal/-shm），并清掉旧的通讯录同步结果，当前号从零开始。
/// 只做一次（做完写标记）；之后再换号不会动任何已有数据。备份失败则不做任何修改。
pub fn start_fresh_once(state_dir: &Path, legacy: &str, account: &str, now: f64) -> Result<bool> {
    let marker = state_dir.join(ADOPT_MARKER_FILE);
    if marker.exists() {
        return Ok(false);
    }
    let hist = state_dir.join("history");
    let db = hist.join("history.sqlite3");
    let backup_dir = state_dir.join("backups").join(format!("before-account-{}", now as i64));
    if db.exists() {
        fs::create_dir_all(&backup_dir)?;
        crate::core::backup::backup_sqlite_db(&db, &backup_dir.join("history.sqlite3"))
            .context("清空前备份数据库失败，未做任何修改")?;
        for suffix in ["", "-wal", "-shm"] {
            let f = hist.join(format!("history.sqlite3{suffix}"));
            if f.exists() {
                fs::remove_file(&f)?;
            }
        }
    }
    for f in ["contacts-sync-result.json", "contacts-sync-checkpoint.json"] {
        let _ = fs::remove_file(state_dir.join(f));
    }
    write_private(
        &marker,
        &json!({"from": legacy, "to": account, "mode": "fresh", "backup": backup_dir.display().to_string(), "at": now}),
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 实测结构：微信同时打开公共目录和当前账号目录；另一个登录过的账号目录没有打开
    #[test]
    fn test_account_from_open_files_picks_open_account_dir() {
        let base = "n/path/to/Library/Containers/com.tencent.xinWeChat/Data/Documents/xwechat_files";
        let out = format!(
            "p123\n{base}/all_users/login/wxid_other0x12/x\n{base}/all_users/config/global_config\n\
             {base}/wxid_abcdefa329_32ec/db_storage/message/message_0.db\n{base}/wxid_abcdefa329_32ec/db_storage/contact/contact.db\n\
             n/System/Library/Fonts/PingFang.ttc\n"
        );
        assert_eq!(account_from_open_files(&out).as_deref(), Some("wxid_abcdefa329"));
    }

    #[test]
    fn test_account_from_open_files_none_when_not_logged_in() {
        let out = "p1\nn/path/to/Library/Containers/com.tencent.xinWeChat/Data/Documents/xwechat_files/all_users/config/a\n";
        assert_eq!(account_from_open_files(out), None);
        assert_eq!(account_from_open_files(""), None);
    }

    /// 老账号的目录名不带 wxid_ 前缀（用自己设的微信号），同样去掉末尾 4 位
    #[test]
    fn test_account_from_open_files_custom_id_dir() {
        let out = "n/a/xwechat_files/my_old_id_9f3a/db_storage/x.db\n";
        assert_eq!(account_from_open_files(out).as_deref(), Some("my_old_id"));
    }

    #[test]
    fn test_start_fresh_backs_up_then_empties_once() {
        let dir = std::env::temp_dir().join(format!("acct-fresh-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = crate::core::history::store::HistoryStore::open(&dir).unwrap();
        store.ensure_contact("test-desktop", &json!({"recipient": "张三", "kind": "private"}), false).unwrap();
        drop(store);
        fs::write(dir.join("contacts-sync-result.json"), "{}").unwrap();

        assert!(start_fresh_once(&dir, "test-desktop", "wxid_a", 1000.0).unwrap());
        // 备份里有原数据
        let backup = dir.join("backups").join("before-account-1000").join("history.sqlite3");
        let conn = rusqlite::Connection::open(&backup).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "备份必须包含原数据");
        // 当前库是空的
        let store = crate::core::history::store::HistoryStore::open(&dir).unwrap();
        let conn = store.connection();
        let n: i64 = conn.lock().unwrap().query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert!(!dir.join("contacts-sync-result.json").exists());

        // 只做一次：之后（含换号）不再动数据
        store.ensure_contact("wxid_a", &json!({"recipient": "李四", "kind": "private"}), false).unwrap();
        drop(conn);
        drop(store);
        assert!(!start_fresh_once(&dir, "test-desktop", "wxid_b", 2000.0).unwrap());
        let store = crate::core::history::store::HistoryStore::open(&dir).unwrap();
        let n: i64 = store.connection().lock().unwrap().query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_effective_account_falls_back_to_config() {
        let dir = std::env::temp_dir().join(format!("acct-eff-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(effective_account_id(&dir, "test-desktop"), "test-desktop");
        record_current_account(&dir, "wxid_a", 1.0).unwrap();
        assert_eq!(effective_account_id(&dir, "test-desktop"), "wxid_a");
        let _ = fs::remove_dir_all(&dir);
    }
}
