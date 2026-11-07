use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};

use crate::core::config::BotConfig;

pub const DEFAULT_BACKUP_RETENTION: usize = 7;
pub const AUTO_BACKUP_INTERVAL_SECS: f64 = 86400.0; // 24 hours

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
}

/// Formats a SystemTime as YYYYMMDD-HHMMSS using local time.
pub fn format_timestamp(ts: SystemTime) -> String {
    let dur = ts.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs() as i64;
    unsafe {
        let mut tm: Tm = std::mem::zeroed();
        localtime_r(&secs, &mut tm);
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

/// Backs up a single SQLite database using SQLite's online backup API.
/// Faithfully translates scripts/backup_db.py:
/// - Rejects existing destination file.
/// - Requires source file to exist.
/// - Writes to a temporary file in the destination directory with 0600 permissions.
/// - Verifies backup with PRAGMA integrity_check.
/// - Atomically replaces destination with verified backup.
pub fn backup_sqlite_db(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        bail!("Source database does not exist: {}", source.display());
    }
    if destination.exists() {
        bail!("Output already exists; choose a new backup path: {}", destination.display());
    }

    let dest_parent = destination.parent().context("destination has no parent dir")?;
    if !dest_parent.exists() {
        fs::create_dir_all(dest_parent)?;
        fs::set_permissions(dest_parent, fs::Permissions::from_mode(0o700))?;
    }

    let temp_name = format!(".service-backup-{}", uuid::Uuid::new_v4().simple());
    let temp_path = dest_parent.join(temp_name);

    // Create temp file with 0600 mode
    {
        let _ = OpenOptions::new()
            .create(true)
            .write(true)
            .mode(0o600)
            .open(&temp_path)
            .with_context(|| format!("failed to create temp file at {}", temp_path.display()))?;
    }

    let run_backup_internal = || -> Result<()> {
        let src_conn = Connection::open_with_flags(
            source,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI,
        ).with_context(|| format!("failed to open source database at {}", source.display()))?;

        let dst_conn = Connection::open(&temp_path)
            .with_context(|| format!("failed to open temp database at {}", temp_path.display()))?;

        unsafe {
            let p_backup = rusqlite::ffi::sqlite3_backup_init(
                dst_conn.handle(),
                b"main\0".as_ptr() as *const _,
                src_conn.handle(),
                b"main\0".as_ptr() as *const _,
            );
            if p_backup.is_null() {
                let p = rusqlite::ffi::sqlite3_errmsg(dst_conn.handle());
                let err_msg = if p.is_null() {
                    "unknown error".to_string()
                } else {
                    std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                };
                bail!("failed to initialize sqlite backup: {}", err_msg);
            }

            let rc = rusqlite::ffi::sqlite3_backup_step(p_backup, -1);
            let finish_rc = rusqlite::ffi::sqlite3_backup_finish(p_backup);

            if rc != rusqlite::ffi::SQLITE_DONE {
                bail!("sqlite backup execution failed: step_rc={}, finish_rc={}", rc, finish_rc);
            }
        }

        let integrity: String = dst_conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .context("failed to query integrity_check on backup")?;

        if integrity != "ok" {
            bail!("Backup integrity check failed: {}", integrity);
        }

        Ok(())
    };

    if let Err(e) = run_backup_internal() {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
    }

    let _ = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600));
    fs::rename(&temp_path, destination)
        .with_context(|| format!("failed to rename temp backup to {}", destination.display()))?;
    let _ = fs::set_permissions(destination, fs::Permissions::from_mode(0o600));

    Ok(())
}

/// Prunes old backup directories in `backups_dir`, keeping only the newest `keep` backups.
pub fn prune_old_backups(backups_dir: &Path, keep: usize) -> Result<usize> {
    if !backups_dir.exists() {
        return Ok(0);
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(backups_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if !file_name.starts_with('.') {
                    entries.push((file_name.to_string(), path));
                }
            }
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut removed = 0;
    if entries.len() > keep {
        let to_remove = entries.len() - keep;
        for (_, path) in entries.iter().take(to_remove) {
            fs::remove_dir_all(path)?;
            removed += 1;
        }
    }

    Ok(removed)
}

/// Resolves the message gateway database path.
/// Priority:
/// 1. $MESSAGE_GATEWAY_DB environment variable
/// 2. ~/.wechat-rpa/test.sqlite3
pub fn resolve_gateway_db_path() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("MESSAGE_GATEWAY_DB") {
        let p = PathBuf::from(env_path);
        return Some(p);
    }

    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join(".wechat-rpa").join("test.sqlite3");
        return Some(p);
    }

    None
}

/// Runs a backup for both history.sqlite3 and test.sqlite3 according to config.
/// Destination:
/// - If `custom_out` is provided, backs up directly into that directory.
/// - Otherwise backs up into `<config.state_dir>/backups/YYYYMMDD-HHMMSS/` and prunes old backups to 7.
pub fn run_backup(config: &BotConfig, custom_out: Option<&Path>) -> Result<PathBuf> {
    let out_dir = match custom_out {
        Some(dir) => dir.to_path_buf(),
        None => {
            let timestamp = format_timestamp(SystemTime::now());
            config.state_dir.join("backups").join(timestamp)
        }
    };

    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)
            .with_context(|| format!("failed to create backup dir {}", out_dir.display()))?;
        fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o700))?;
    } else {
        fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o700))?;
    }

    let history_db = config.state_dir.join("history").join("history.sqlite3");
    let gateway_db = resolve_gateway_db_path();

    let mut backed_up_count = 0;

    if history_db.exists() {
        let target = out_dir.join("history.sqlite3");
        backup_sqlite_db(&history_db, &target)
            .with_context(|| format!("failed to backup history db from {}", history_db.display()))?;
        backed_up_count += 1;
    }

    if let Some(gw_path) = gateway_db {
        if gw_path.exists() {
            let target = out_dir.join("test.sqlite3");
            backup_sqlite_db(&gw_path, &target)
                .with_context(|| format!("failed to backup gateway db from {}", gw_path.display()))?;
            backed_up_count += 1;
        }
    }

    if backed_up_count == 0 {
        bail!("No source databases found to backup (checked history at {} and gateway database)", history_db.display());
    }

    // If using the standard backups directory, prune older backups
    if custom_out.is_none() {
        let backups_root = config.state_dir.join("backups");
        let _ = prune_old_backups(&backups_root, DEFAULT_BACKUP_RETENTION);
    }

    Ok(out_dir)
}

/// Returns the timestamp in seconds of the newest backup in `backups_dir`, or 0.0 if none exist.
pub fn latest_backup_timestamp(backups_dir: &Path) -> f64 {
    if !backups_dir.exists() {
        return 0.0;
    }
    let mut newest: f64 = 0.0;
    if let Ok(entries) = fs::read_dir(backups_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(meta) = fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        if let Ok(dur) = modified.duration_since(UNIX_EPOCH) {
                            let secs = dur.as_secs_f64();
                            if secs > newest {
                                newest = secs;
                            }
                        }
                    }
                }
            }
        }
    }
    newest
}

/// Checks whether an automatic backup should be triggered in serve-core.
/// Condition:
/// - is_idle: no pending tasks and no sessions with active turn
/// - time elapsed since last_backup_at is >= AUTO_BACKUP_INTERVAL_SECS
pub fn should_auto_backup(is_idle: bool, now: f64, last_backup_at: f64) -> bool {
    is_idle && (now - last_backup_at >= AUTO_BACKUP_INTERVAL_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use std::thread;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("wechat-backup-test-{}-{}", name, uuid::Uuid::new_v4().simple()));
            fs::create_dir_all(&p).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_config(state_dir: &Path) -> BotConfig {
        let config_json = serde_json::json!({
            "schema_version": 1,
            "host": "test-host",
            "account_id": "test_acc",
            "allowlist": [{"recipient": "test_rec", "kind": "service"}],
            "state_dir": state_dir.to_str().unwrap(),
            "poll_seconds": 2,
            "cooldown_seconds": 1,
            "max_replies_per_hour": 10,
            "idle_check_seconds": 30,
            "turn_quiet_seconds": 3,
            "turn_max_seconds": 60,
            "images_per_model_call": 3,
            "max_attachment_mb": 25,
            "context_messages": 10,
            "manage_window": false,
            "capture_backend": "test",
            "model_config": {"backend": "mock"},
            "brain": {"mode": "builtin", "external": {"timeout_seconds": 120, "timeout_action": "skip", "queue_minutes": 30}},
            "history_http": {"bind": "127.0.0.1", "port": 0}
        });
        serde_json::from_value(config_json).unwrap()
    }

    #[test]
    fn test_format_timestamp() {
        let ts = SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000);
        let s = format_timestamp(ts);
        assert_eq!(s.len(), 15);
        assert_eq!(&s[8..9], "-");
    }

    #[test]
    fn test_backup_refuses_overwrite() {
        let temp = TempDir::new("refuse_overwrite");
        let src = temp.0.join("src.sqlite3");
        let dst = temp.0.join("dst.sqlite3");

        let conn = Connection::open(&src).unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", []).unwrap();
        drop(conn);

        fs::write(&dst, b"existing").unwrap();

        let err = backup_sqlite_db(&src, &dst).unwrap_err();
        assert!(err.to_string().contains("Output already exists"));
    }

    #[test]
    fn test_backup_missing_source_fails() {
        let temp = TempDir::new("missing_src");
        let src = temp.0.join("nonexistent.sqlite3");
        let dst = temp.0.join("dst.sqlite3");

        let err = backup_sqlite_db(&src, &dst).unwrap_err();
        assert!(err.to_string().contains("Source database does not exist"));
    }

    #[test]
    fn test_backup_single_db_wal_concurrent_write() {
        let temp = TempDir::new("wal_concurrent");
        let src = temp.0.join("source.sqlite3");
        let dst = temp.0.join("backup.sqlite3");

        // 1. 初始化源数据库并设置为 WAL 模式
        {
            let conn = Connection::open(&src).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE items (id INTEGER PRIMARY KEY, content TEXT);
                 INSERT INTO items (content) VALUES ('initial 1'), ('initial 2');",
            ).unwrap();
        }

        // 2. 并发写入线程：在备份执行期间持续写入
        let running = Arc::new(AtomicBool::new(true));
        let src_clone = src.clone();
        let running_clone = running.clone();

        let writer_handle = thread::spawn(move || {
            let conn = Connection::open(&src_clone).unwrap();
            let mut i = 0;
            while running_clone.load(Ordering::SeqCst) {
                let _ = conn.execute(
                    "INSERT INTO items (content) VALUES (?1)",
                    [format!("concurrent item {}", i)],
                );
                i += 1;
                thread::sleep(Duration::from_millis(5));
            }
        });

        // 3. 执行在线备份
        thread::sleep(Duration::from_millis(20));
        let backup_res = backup_sqlite_db(&src, &dst);
        running.store(false, Ordering::SeqCst);
        writer_handle.join().unwrap();

        assert!(backup_res.is_ok(), "Backup failed: {:?}", backup_res.err());

        // 4. 验证备份文件权限 0600
        let meta = fs::metadata(&dst).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "Backup file permission must be 0600");

        // 5. 验证备份数据库 integrity_check 和可用性
        let dst_conn = Connection::open(&dst).unwrap();
        let integrity: String = dst_conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(integrity, "ok");

        let count: i64 = dst_conn.query_row("SELECT count(*) FROM items", [], |r| r.get(0)).unwrap();
        assert!(count >= 2, "Expected at least initial items, got {}", count);
    }

    #[test]
    fn test_prune_old_backups() {
        let temp = TempDir::new("prune");
        let backups_dir = temp.0.join("backups");
        fs::create_dir_all(&backups_dir).unwrap();

        // 创建 10 个按时间排序的备份目录
        for i in 1..=10 {
            let dir_name = format!("20261108-{:02}0000", i);
            let sub = backups_dir.join(dir_name);
            fs::create_dir_all(&sub).unwrap();
            fs::set_permissions(&sub, fs::Permissions::from_mode(0o700)).unwrap();
            let file = sub.join("dummy.sqlite3");
            fs::write(&file, b"data").unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let removed = prune_old_backups(&backups_dir, 7).unwrap();
        assert_eq!(removed, 3);

        let mut remaining = Vec::new();
        for entry in fs::read_dir(&backups_dir).unwrap() {
            remaining.push(entry.unwrap().file_name().to_str().unwrap().to_string());
        }
        remaining.sort();
        assert_eq!(remaining.len(), 7);
        // 应该保留 04 到 10
        assert_eq!(remaining[0], "20261108-040000");
        assert_eq!(remaining[6], "20261108-100000");
    }

    #[test]
    fn test_run_backup_default_and_retention() {
        let temp = TempDir::new("run_backup");
        let state_dir = temp.0.join("state");
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();

        // 构造 history.sqlite3
        let history_db = history_dir.join("history.sqlite3");
        {
            let conn = Connection::open(&history_db).unwrap();
            conn.execute_batch(
                "CREATE TABLE test_h (id INT PRIMARY KEY); INSERT INTO test_h VALUES (1);",
            ).unwrap();
        }

        // 构造 gateway db 并通过环境变量指定
        let gw_db = temp.0.join("test.sqlite3");
        {
            let conn = Connection::open(&gw_db).unwrap();
            conn.execute_batch(
                "CREATE TABLE test_gw (id INT PRIMARY KEY); INSERT INTO test_gw VALUES (99);",
            ).unwrap();
        }
        unsafe {
            std::env::set_var("MESSAGE_GATEWAY_DB", gw_db.to_str().unwrap());
        }

        let config = sample_config(&state_dir);
        let out = run_backup(&config, None).unwrap();
        assert!(out.exists());

        // 检查目录权限 0700
        let dir_mode = fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);

        // 检查备份产物
        let h_bak = out.join("history.sqlite3");
        let g_bak = out.join("test.sqlite3");
        assert!(h_bak.exists());
        assert!(g_bak.exists());

        assert_eq!(fs::metadata(&h_bak).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&g_bak).unwrap().permissions().mode() & 0o777, 0o600);

        // 校验 integrity
        let conn_h = Connection::open(&h_bak).unwrap();
        assert_eq!(conn_h.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
        let conn_g = Connection::open(&g_bak).unwrap();
        assert_eq!(conn_g.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
    }

    #[test]
    fn test_run_backup_custom_out() {
        let temp = TempDir::new("custom_out");
        let state_dir = temp.0.join("state");
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();

        let history_db = history_dir.join("history.sqlite3");
        let conn = Connection::open(&history_db).unwrap();
        conn.execute("CREATE TABLE t (x TEXT)", []).unwrap();
        drop(conn);

        let config = sample_config(&state_dir);

        let custom_dir = temp.0.join("manual_export");
        let res = run_backup(&config, Some(&custom_dir)).unwrap();
        assert_eq!(res, custom_dir);
        assert!(custom_dir.join("history.sqlite3").exists());
    }

    #[test]
    fn test_should_auto_backup() {
        // 1. 空闲且满 24 小时 -> 触发
        assert!(should_auto_backup(true, 100000.0, 100000.0 - 86400.0));
        assert!(should_auto_backup(true, 100000.0, 100000.0 - 86401.0));

        // 2. 非空闲且满 24 小时 -> 不触发
        assert!(!should_auto_backup(false, 100000.0, 100000.0 - 86400.0));

        // 3. 空闲但未满 24 小时 -> 不触发
        assert!(!should_auto_backup(true, 100000.0, 100000.0 - 86399.0));

        // 4. 非空闲且未满 24 小时 -> 不触发
        assert!(!should_auto_backup(false, 100000.0, 100000.0 - 10.0));
    }
}
