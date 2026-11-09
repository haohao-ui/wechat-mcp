use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

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

pub fn file_sha256(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)
            .with_context(|| format!("failed to read file for sha256: {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let result = hasher.finalize();
    Ok(result.iter().map(|b| format!("{:02x}", b)).collect())
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(u64, u64)> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
        fs::set_permissions(dst, fs::Permissions::from_mode(0o700))?;
    }
    let mut file_count = 0u64;
    let mut total_bytes = 0u64;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            let (sub_count, sub_bytes) = copy_dir_all(&src_path, &dst_path)?;
            file_count += sub_count;
            total_bytes += sub_bytes;
        } else if file_type.is_file() {
            let len = entry.metadata()?.len();
            fs::copy(&src_path, &dst_path)?;
            fs::set_permissions(&dst_path, fs::Permissions::from_mode(0o600))?;
            file_count += 1;
            total_bytes += len;
        }
    }

    Ok((file_count, total_bytes))
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct DatabaseManifestEntry {
    pub file: String,
    pub sha256: String,
    pub user_version: i64,
    pub size_bytes: u64,
    pub total_rows: i64,
    pub table_rows: HashMap<String, i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct MediaManifestEntry {
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct BackupManifest {
    pub version: u32,
    pub created_at: u64,
    pub database_count: usize,
    pub database_total_bytes: u64,
    pub databases: Vec<DatabaseManifestEntry>,
    pub media: MediaManifestEntry,
}

fn inspect_db(db_path: &Path, file_name: &str) -> Result<DatabaseManifestEntry> {
    let sha256 = file_sha256(db_path)?;
    let size_bytes = fs::metadata(db_path)?.len();

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut table_rows = HashMap::new();
    let mut total_rows = 0i64;
    for tbl in tables {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM \"{}\"", tbl.replace('"', "\"\"")), [], |r| r.get(0))
            .unwrap_or(0);
        table_rows.insert(tbl, count);
        total_rows += count;
    }

    Ok(DatabaseManifestEntry {
        file: file_name.to_string(),
        sha256,
        user_version,
        size_bytes,
        total_rows,
        table_rows,
    })
}

/// Backs up a single SQLite database using SQLite's online backup API.
/// - Opens source database as READ ONLY with busy_timeout.
/// - Rejects existing destination file.
/// - Requires source file to exist.
/// - Writes to a temporary file in the destination directory with 0600 permissions.
/// - Backs up iteratively via sqlite3_backup_step(128).
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
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        ).with_context(|| format!("failed to open source database at {}", source.display()))?;

        src_conn.busy_timeout(std::time::Duration::from_millis(5000))
            .context("failed to set busy_timeout on source database")?;

        let dst_conn = Connection::open(&temp_path)
            .with_context(|| format!("failed to open temp database at {}", temp_path.display()))?;

        dst_conn.busy_timeout(std::time::Duration::from_millis(5000))
            .context("failed to set busy_timeout on destination database")?;

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

            let mut rc;
            loop {
                rc = rusqlite::ffi::sqlite3_backup_step(p_backup, 128);
                if rc == rusqlite::ffi::SQLITE_OK {
                    continue;
                } else if rc == rusqlite::ffi::SQLITE_BUSY || rc == rusqlite::ffi::SQLITE_LOCKED {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    continue;
                } else {
                    break;
                }
            }
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

pub fn is_valid_backup_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with('.') {
            return false;
        }
    }
    // 必须是非空目录：空目录（例如备份失败残留）不能算有效备份
    if let Ok(mut entries) = fs::read_dir(path) {
        entries.next().is_some()
    } else {
        false
    }
}

pub fn is_program_backup_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let manifest_path = path.join("manifest.json");
    if !manifest_path.is_file() {
        return false;
    }
    if let Ok(content) = fs::read(&manifest_path) {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&content) {
            return val.get("version").is_some() && val.get("databases").is_some();
        }
    }
    false
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
/// 2. ~/.wechat-rpa/test.sqlite3 (if exists)
/// 3. ~/Library/Application Support/message-gateway/service.sqlite3 (if exists)
/// 4. ~/.wechat-rpa/test.sqlite3
pub fn resolve_gateway_db_path() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("MESSAGE_GATEWAY_DB") {
        let p = PathBuf::from(env_path);
        return Some(p);
    }

    if let Ok(home) = std::env::var("HOME") {
        let home_path = PathBuf::from(home);
        let test_db = home_path.join(".wechat-rpa").join("test.sqlite3");
        if test_db.exists() {
            return Some(test_db);
        }
        let service_db = home_path
            .join("Library/Application Support/message-gateway")
            .join("service.sqlite3");
        if service_db.exists() {
            return Some(service_db);
        }
        return Some(test_db);
    }

    None
}

/// Runs a backup for both history.sqlite3 and test/service.sqlite3 according to config.
/// Uses a temporary directory and atomic rename so that interrupted/failed backups never
/// leave incomplete directories behind or get treated as successful backups.
/// Destination:
/// - If `custom_out` is provided, backs up directly into that directory.
/// - Otherwise backs up into `<config.state_dir>/backups/YYYYMMDD-HHMMSS/` and prunes old backups to 7.
pub fn run_backup(config: &BotConfig, custom_out: Option<&Path>) -> Result<PathBuf> {
    let (out_dir, is_custom) = match custom_out {
        Some(dir) => (dir.to_path_buf(), true),
        None => {
            let timestamp = format_timestamp(SystemTime::now());
            (config.state_dir.join("backups").join(timestamp), false)
        }
    };

    if out_dir.exists() {
        let mut entries = fs::read_dir(&out_dir)
            .with_context(|| format!("failed to read directory {}", out_dir.display()))?;
        if entries.next().is_some() {
            if !is_program_backup_dir(&out_dir) {
                bail!("Output directory exists, is not empty, and is not a valid backup directory: {}", out_dir.display());
            }
        }
    }

    let parent_dir = out_dir.parent().context("destination has no parent dir")?.to_path_buf();
    if !parent_dir.exists() {
        fs::create_dir_all(&parent_dir)
            .with_context(|| format!("failed to create parent dir {}", parent_dir.display()))?;
        fs::set_permissions(&parent_dir, fs::Permissions::from_mode(0o700))?;
    }

    let temp_name = format!(".tmp-backup-{}", uuid::Uuid::new_v4().simple());
    let temp_dir = parent_dir.join(temp_name);
    fs::create_dir_all(&temp_dir)
        .with_context(|| format!("failed to create temp backup dir {}", temp_dir.display()))?;
    fs::set_permissions(&temp_dir, fs::Permissions::from_mode(0o700))?;

    let run_backup_internal = || -> Result<()> {
        let history_db = config.state_dir.join("history").join("history.sqlite3");
        let gateway_db = resolve_gateway_db_path();

        let mut backed_up_count = 0;
        let mut database_entries = Vec::new();
        let mut db_total_bytes = 0u64;

        if history_db.exists() {
            let target = temp_dir.join("history.sqlite3");
            backup_sqlite_db(&history_db, &target)
                .with_context(|| format!("failed to backup history db from {}", history_db.display()))?;
            let entry = inspect_db(&target, "history.sqlite3")?;
            db_total_bytes += entry.size_bytes;
            database_entries.push(entry);
            backed_up_count += 1;
        }

        if let Some(gw_path) = &gateway_db {
            if gw_path.exists() {
                let gw_file_name = gw_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("test.sqlite3");
                let target = temp_dir.join(gw_file_name);
                backup_sqlite_db(gw_path, &target)
                    .with_context(|| format!("failed to backup gateway db from {}", gw_path.display()))?;
                let entry = inspect_db(&target, gw_file_name)?;
                db_total_bytes += entry.size_bytes;
                database_entries.push(entry);
                backed_up_count += 1;
            }
        }

        if backed_up_count == 0 {
            bail!("No source databases found to backup (checked history at {} and gateway database)", history_db.display());
        }

        let src_media_dir = config.state_dir.join("history").join("media");
        let (media_file_count, media_total_bytes) = if src_media_dir.exists() && src_media_dir.is_dir() {
            let dst_media_dir = temp_dir.join("history").join("media");
            copy_dir_all(&src_media_dir, &dst_media_dir)?
        } else {
            (0, 0)
        };

        let manifest = BackupManifest {
            version: 2,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            database_count: database_entries.len(),
            database_total_bytes: db_total_bytes,
            databases: database_entries,
            media: MediaManifestEntry {
                file_count: media_file_count,
                total_bytes: media_total_bytes,
            },
        };
        let manifest_path = temp_dir.join("manifest.json");
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        fs::set_permissions(&manifest_path, fs::Permissions::from_mode(0o600))?;

        let mut trash_dir_opt = None;
        if out_dir.exists() {
            let trash_name = format!(".trash-{}", uuid::Uuid::new_v4().simple());
            let trash_dir = parent_dir.join(trash_name);
            fs::rename(&out_dir, &trash_dir)
                .with_context(|| format!("failed to move old backup to trash {}", trash_dir.display()))?;
            trash_dir_opt = Some(trash_dir);
        }

        if let Err(e) = fs::rename(&temp_dir, &out_dir) {
            if let Some(trash_dir) = &trash_dir_opt {
                let _ = fs::rename(trash_dir, &out_dir);
            }
            return Err(e).with_context(|| format!("failed to rename temp backup to {}", out_dir.display()));
        }
        fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o700))?;

        if let Some(trash_dir) = trash_dir_opt {
            let _ = fs::remove_dir_all(&trash_dir);
        }

        if !is_custom {
            let backups_root = config.state_dir.join("backups");
            let _ = prune_old_backups(&backups_root, DEFAULT_BACKUP_RETENTION);
        }

        Ok(())
    };

    match run_backup_internal() {
        Ok(()) => Ok(out_dir),
        Err(err) => {
            let _ = fs::remove_dir_all(&temp_dir);
            Err(err)
        }
    }
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
            if is_valid_backup_dir(&path) {
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

// TODO: Replace with core::serve::core_is_running when Task B is merged.
pub fn core_is_running(state_dir: &Path) -> bool {
    let lock_path = state_dir.join("core.lock");
    if !lock_path.exists() {
        return false;
    }
    match OpenOptions::new().read(true).write(true).open(&lock_path) {
        Ok(file) => {
            use fs2::FileExt;
            match file.try_lock_exclusive() {
                Ok(()) => {
                    let _ = file.unlock();
                    false
                }
                Err(_) => true,
            }
        }
        Err(_) => false,
    }
}

pub fn run_restore(backup_dir: &Path, config: &BotConfig) -> Result<()> {
    if !backup_dir.exists() || !backup_dir.is_dir() {
        bail!("Backup directory does not exist or is not a directory: {}", backup_dir.display());
    }

    let manifest_path = backup_dir.join("manifest.json");
    if !manifest_path.exists() {
        bail!("Manifest not found in backup directory: {}", manifest_path.display());
    }

    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read manifest at {}", manifest_path.display()))?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| "failed to parse backup manifest JSON")?;

    if manifest.databases.is_empty() {
        bail!("No databases recorded in backup manifest");
    }

    // 1. 校验 manifest 和 sha256
    for db_entry in &manifest.databases {
        let bak_db = backup_dir.join(&db_entry.file);
        if !bak_db.exists() {
            bail!("Database file {} recorded in manifest does not exist in backup", db_entry.file);
        }
        let actual_sha = file_sha256(&bak_db)?;
        if actual_sha != db_entry.sha256 {
            bail!(
                "SHA256 digest mismatch for {}: manifest sha256={}, actual sha256={}",
                db_entry.file,
                db_entry.sha256,
                actual_sha
            );
        }
    }

    // 2. 确认核心未运行
    if core_is_running(&config.state_dir) {
        bail!("Cannot restore while core is running. Please stop the service first.");
    }

    if !config.state_dir.exists() {
        fs::create_dir_all(&config.state_dir)?;
        fs::set_permissions(&config.state_dir, fs::Permissions::from_mode(0o700))?;
    }

    // 3. 当前库和 media 移到 .restore-old-<ts>
    let ts_str = format_timestamp(SystemTime::now());
    let old_backup_name = format!(".restore-old-{}-{}", ts_str, uuid::Uuid::new_v4().simple());
    let old_backup_dir = config.state_dir.join(old_backup_name);
    fs::create_dir_all(&old_backup_dir)?;
    fs::set_permissions(&old_backup_dir, fs::Permissions::from_mode(0o700))?;

    let current_history_dir = config.state_dir.join("history");
    let has_old_history = current_history_dir.exists();
    if has_old_history {
        fs::rename(&current_history_dir, old_backup_dir.join("history"))
            .with_context(|| "failed to move existing history to old backup dir")?;
    }

    let gw_dest = resolve_gateway_db_path();
    let mut old_gw_moved = false;
    if let Some(gw_path) = &gw_dest {
        if gw_path.exists() {
            if let Some(gw_name) = gw_path.file_name() {
                let _ = fs::rename(gw_path, old_backup_dir.join(gw_name));
                old_gw_moved = true;
            }
        }
    }

    let rollback = || {
        if current_history_dir.exists() {
            let _ = fs::remove_dir_all(&current_history_dir);
        }
        if has_old_history {
            let _ = fs::rename(old_backup_dir.join("history"), &current_history_dir);
        }
        if old_gw_moved {
            if let Some(gw_path) = &gw_dest {
                if let Some(gw_name) = gw_path.file_name() {
                    let _ = fs::rename(old_backup_dir.join(gw_name), gw_path);
                }
            }
        }
    };

    let do_restore = || -> Result<()> {
        fs::create_dir_all(&current_history_dir)?;
        fs::set_permissions(&current_history_dir, fs::Permissions::from_mode(0o700))?;

        for db_entry in &manifest.databases {
            let src_db = backup_dir.join(&db_entry.file);
            let target_db = if db_entry.file == "history.sqlite3" {
                current_history_dir.join("history.sqlite3")
            } else if let Some(gw_path) = &gw_dest {
                gw_path.clone()
            } else {
                current_history_dir.join(&db_entry.file)
            };

            if let Some(parent) = target_db.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
                }
            }

            fs::copy(&src_db, &target_db)
                .with_context(|| format!("failed to copy restored db to {}", target_db.display()))?;
            fs::set_permissions(&target_db, fs::Permissions::from_mode(0o600))?;
        }

        let bak_media_dir = backup_dir.join("history").join("media");
        if bak_media_dir.exists() && bak_media_dir.is_dir() {
            let dst_media_dir = current_history_dir.join("media");
            copy_dir_all(&bak_media_dir, &dst_media_dir)?;
        }

        for db_entry in &manifest.databases {
            let target_db = if db_entry.file == "history.sqlite3" {
                current_history_dir.join("history.sqlite3")
            } else if let Some(gw_path) = &gw_dest {
                gw_path.clone()
            } else {
                current_history_dir.join(&db_entry.file)
            };

            let conn = Connection::open_with_flags(
                &target_db,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            ).with_context(|| format!("failed to open restored database at {}", target_db.display()))?;

            let integrity: String = conn
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .with_context(|| format!("failed to run integrity_check on {}", target_db.display()))?;
            if integrity != "ok" {
                bail!("Restored database integrity check failed on {}: {}", target_db.display(), integrity);
            }

            let mut fk_stmt = conn.prepare("PRAGMA foreign_key_check")?;
            let fk_rows = fk_stmt.query_map([], |row| {
                let table: String = row.get(0)?;
                let rowid: i64 = row.get(1)?;
                let target: String = row.get(2)?;
                let fkid: i64 = row.get(3)?;
                Ok(format!("{}:{} -> {}:{}", table, rowid, target, fkid))
            })?.collect::<Result<Vec<_>, _>>()?;

            if !fk_rows.is_empty() {
                bail!("Restored database foreign_key_check failed on {}: {:?}", target_db.display(), fk_rows);
            }

            for (tbl, expected_count) in &db_entry.table_rows {
                let actual_count: i64 = conn
                    .query_row(&format!("SELECT count(*) FROM \"{}\"", tbl.replace('"', "\"\"")), [], |r| r.get(0))
                    .with_context(|| format!("failed to count rows in table {}", tbl))?;
                if actual_count != *expected_count {
                    bail!(
                        "Row count mismatch in table '{}' for database {}: expected {}, actual {}",
                        tbl,
                        db_entry.file,
                        expected_count,
                        actual_count
                    );
                }
            }
        }

        Ok(())
    };

    if let Err(e) = do_restore() {
        rollback();
        return Err(e);
    }

    Ok(())
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
        let _env = crate::core::env_lock();
        let orig_gw = std::env::var("MESSAGE_GATEWAY_DB").ok();
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

        if let Some(g) = orig_gw {
            unsafe { std::env::set_var("MESSAGE_GATEWAY_DB", g) };
        } else {
            unsafe { std::env::remove_var("MESSAGE_GATEWAY_DB") };
        }
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
    fn test_backup_failure_does_not_leave_empty_dir_as_latest_backup() {
        let temp = TempDir::new("failure_empty_dir");
        let state_dir = temp.0.join("state");
        let backups_dir = state_dir.join("backups");
        fs::create_dir_all(&backups_dir).unwrap();

        // 模拟一个失败遗留的空备份目录
        let fake_failed_dir = backups_dir.join("20261108-100000");
        fs::create_dir_all(&fake_failed_dir).unwrap();

        // latest_backup_timestamp 应当忽略空目录或无效备份目录，返回 0.0
        let ts = latest_backup_timestamp(&backups_dir);
        assert_eq!(ts, 0.0, "Empty or corrupted backup directory must not count as latest backup");

        // 同样，如果 run_backup 失败，不能留下未完成的空目标目录
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();
        let history_db = history_dir.join("history.sqlite3");
        // 写入一个非 SQLite 的损坏文件，导致备份或 integrity_check 失败
        fs::write(&history_db, b"not a sqlite db").unwrap();

        let config = sample_config(&state_dir);
        let res = run_backup(&config, None);
        assert!(res.is_err(), "Corrupted database backup must fail");

        // 验证失败后不会留下新的空目录
        let ts_after = latest_backup_timestamp(&backups_dir);
        assert_eq!(ts_after, 0.0, "Failed backup attempt must not update latest_backup_timestamp");
    }

    #[test]
    fn test_resolve_gateway_db_path_fallback_service_sqlite3() {
        let _env = crate::core::env_lock();
        let temp = TempDir::new("service_sqlite3_test");
        let orig_home = std::env::var("HOME").ok();
        let orig_gw = std::env::var("MESSAGE_GATEWAY_DB").ok();

        unsafe {
            std::env::remove_var("MESSAGE_GATEWAY_DB");
            std::env::set_var("HOME", temp.0.to_str().unwrap());
        }

        let app_support = temp.0.join("Library/Application Support/message-gateway");
        fs::create_dir_all(&app_support).unwrap();
        let service_db = app_support.join("service.sqlite3");
        fs::write(&service_db, b"dummy").unwrap();

        let resolved = resolve_gateway_db_path();

        if let Some(h) = orig_home {
            unsafe { std::env::set_var("HOME", h) };
        }
        if let Some(g) = orig_gw {
            unsafe { std::env::set_var("MESSAGE_GATEWAY_DB", g) };
        }

        assert_eq!(resolved, Some(service_db), "Should fallback to Library/Application Support/message-gateway/service.sqlite3 when test.sqlite3 is absent");
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

    #[test]
    fn backup_survives_concurrent_writer() {
        let temp = TempDir::new("backup_concurrent_writer");
        let src = temp.0.join("source.sqlite3");
        let dst = temp.0.join("backup.sqlite3");

        // 初始化 WAL 模式数据库
        {
            let conn = Connection::open(&src).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE counter (id INTEGER PRIMARY KEY, val INTEGER);
                 INSERT INTO counter VALUES (1, 0);",
            ).unwrap();
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let src_clone = src.clone();

        // 并发高频写入事务
        let writer = thread::spawn(move || {
            let mut conn = Connection::open(&src_clone).unwrap();
            conn.busy_timeout(Duration::from_millis(5000)).unwrap();
            let mut i = 0;
            while !stop_clone.load(Ordering::SeqCst) {
                let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate);
                if let Ok(tx) = tx {
                    let _ = tx.execute("UPDATE counter SET val = val + 1 WHERE id = 1", []);
                    let _ = tx.commit();
                }
                i += 1;
                if i % 10 == 0 {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        });

        // 执行备份
        let res = backup_sqlite_db(&src, &dst);
        stop.store(true, Ordering::SeqCst);
        writer.join().unwrap();

        assert!(res.is_ok(), "backup_sqlite_db should succeed during concurrent writes: {:?}", res.err());
        let dst_conn = Connection::open(&dst).unwrap();
        let integrity: String = dst_conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(integrity, "ok");
    }

    #[test]
    fn backup_includes_media_and_digests() {
        let _env = crate::core::env_lock();
        let temp = TempDir::new("media_and_digests");
        let state_dir = temp.0.join("state");
        let history_dir = state_dir.join("history");
        let media_dir = history_dir.join("media").join("ab");
        fs::create_dir_all(&media_dir).unwrap();

        // 写入 history.sqlite3
        let history_db = history_dir.join("history.sqlite3");
        {
            let conn = Connection::open(&history_db).unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 42;
                 CREATE TABLE messages (id INTEGER PRIMARY KEY, content TEXT);
                 INSERT INTO messages VALUES (1, 'hello'), (2, 'world');",
            ).unwrap();
        }

        // 写入测试媒体文件
        let media_file = media_dir.join("abcdef123456.jpg");
        fs::write(&media_file, b"fake-jpg-content-123456789").unwrap();

        let config = sample_config(&state_dir);
        let out = run_backup(&config, None).unwrap();

        // 验证 history/media/ 相对路径被复制
        let backed_media = out.join("history").join("media").join("ab").join("abcdef123456.jpg");
        assert!(backed_media.exists(), "Media file must be preserved in backup relative path");
        assert_eq!(fs::read(&backed_media).unwrap(), b"fake-jpg-content-123456789");

        // 验证 manifest 结构
        let manifest_bytes = fs::read(out.join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();

        assert!(manifest.get("databases").is_some());
        let dbs = manifest["databases"].as_array().expect("databases must be an array");
        let h_entry = dbs.iter().find(|d| d["file"] == "history.sqlite3").expect("history.sqlite3 in manifest");
        assert!(h_entry["sha256"].as_str().is_some());
        assert_eq!(h_entry["user_version"], 42);
        assert!(h_entry["size_bytes"].as_u64().unwrap() > 0);

        let media_meta = manifest.get("media").expect("manifest must contain media metadata");
        assert_eq!(media_meta["file_count"], 1);
        assert_eq!(media_meta["total_bytes"], b"fake-jpg-content-123456789".len() as u64);
    }

    #[test]
    fn backup_refuses_foreign_directory() {
        let temp = TempDir::new("refuses_foreign");
        let state_dir = temp.0.join("state");
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();
        let history_db = history_dir.join("history.sqlite3");
        {
            let conn = Connection::open(&history_db).unwrap();
            conn.execute_batch("CREATE TABLE t (id INT);").unwrap();
        }

        let foreign_dir = temp.0.join("my_foreign_dir");
        fs::create_dir_all(&foreign_dir).unwrap();
        let foreign_file = foreign_dir.join("important_user_doc.txt");
        fs::write(&foreign_file, b"do not delete me!").unwrap();

        let config = sample_config(&state_dir);
        let res = run_backup(&config, Some(&foreign_dir));
        assert!(res.is_err(), "run_backup must refuse to overwrite foreign non-backup directory");
        assert!(foreign_file.exists(), "foreign file must remain intact");
        assert_eq!(fs::read(&foreign_file).unwrap(), b"do not delete me!");
    }

    #[test]
    fn restore_refuses_while_core_running() {
        let _env = crate::core::env_lock();
        let temp = TempDir::new("refuses_running");
        let state_dir = temp.0.join("state");
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();
        let history_db = history_dir.join("history.sqlite3");
        {
            let conn = Connection::open(&history_db).unwrap();
            conn.execute_batch("CREATE TABLE t (id INT);").unwrap();
        }

        let config = sample_config(&state_dir);
        let backup_dir = temp.0.join("my_backup");
        let _ = run_backup(&config, Some(&backup_dir)).unwrap();

        // 模拟 core.lock 被持有
        fs::create_dir_all(&state_dir).unwrap();
        let lock_path = state_dir.join("core.lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        use fs2::FileExt;
        lock_file.lock_exclusive().unwrap();

        let res = run_restore(&backup_dir, &config);
        assert!(res.is_err(), "run_restore must fail while core is running");
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("core is running") || err_msg.contains("stop"), "Error msg: {}", err_msg);
    }

    #[test]
    fn restore_rolls_back_on_digest_mismatch() {
        let _env = crate::core::env_lock();
        let temp = TempDir::new("restore_rollback");
        let state_dir = temp.0.join("state");
        let history_dir = state_dir.join("history");
        fs::create_dir_all(&history_dir).unwrap();
        let history_db = history_dir.join("history.sqlite3");
        {
            let conn = Connection::open(&history_db).unwrap();
            conn.execute_batch(
                "CREATE TABLE messages (id INT PRIMARY KEY, content TEXT);
                 INSERT INTO messages VALUES (1, 'initial_content');",
            ).unwrap();
        }

        let config = sample_config(&state_dir);
        let backup_dir = temp.0.join("my_backup");
        let _ = run_backup(&config, Some(&backup_dir)).unwrap();

        // 修改当前 state 中的数据库为 "modified_content"
        {
            let conn = Connection::open(&history_db).unwrap();
            conn.execute("UPDATE messages SET content = 'original_unbroken' WHERE id = 1", []).unwrap();
        }

        // 篡改备份中的 history.sqlite3 导致 sha256 不匹配
        let bak_db = backup_dir.join("history.sqlite3");
        fs::write(&bak_db, b"tampered corrupt sqlite data").unwrap();

        let res = run_restore(&backup_dir, &config);
        assert!(res.is_err(), "run_restore must fail on digest mismatch");
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("SHA256") || err_msg.contains("mismatch"), "Error msg: {}", err_msg);

        // 验证原数据自动回滚并保持完好
        assert!(history_db.exists(), "original database must be preserved");
        let conn = Connection::open(&history_db).unwrap();
        let content: String = conn.query_row("SELECT content FROM messages WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(content, "original_unbroken");
    }

    #[test]
    fn restore_roundtrip() {
        let _env = crate::core::env_lock();
        let temp = TempDir::new("restore_roundtrip");
        let state_dir_a = temp.0.join("state_a");
        let history_dir_a = state_dir_a.join("history");
        let media_dir_a = history_dir_a.join("media").join("cd");
        fs::create_dir_all(&media_dir_a).unwrap();

        let db_a = history_dir_a.join("history.sqlite3");
        {
            let conn = Connection::open(&db_a).unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 7;
                 CREATE TABLE users (id INT PRIMARY KEY, name TEXT);
                 INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob');",
            ).unwrap();
        }
        let media_file_a = media_dir_a.join("photo.png");
        fs::write(&media_file_a, b"png-binary-data-test").unwrap();

        let config_a = sample_config(&state_dir_a);
        let backup_dir = temp.0.join("roundtrip_backup");
        let _ = run_backup(&config_a, Some(&backup_dir)).unwrap();

        // 准备目标 state_dir_b
        let state_dir_b = temp.0.join("state_b");
        let history_dir_b = state_dir_b.join("history");
        fs::create_dir_all(&history_dir_b).unwrap();
        let db_b = history_dir_b.join("history.sqlite3");
        {
            let conn = Connection::open(&db_b).unwrap();
            conn.execute_batch(
                "CREATE TABLE users (id INT PRIMARY KEY, name TEXT);
                 INSERT INTO users VALUES (99, 'OldGuy');",
            ).unwrap();
        }

        let config_b = sample_config(&state_dir_b);
        let res = run_restore(&backup_dir, &config_b);
        assert!(res.is_ok(), "run_restore should succeed: {:?}", res.err());

        // 验证数据库内容已恢复
        let conn_b = Connection::open(&db_b).unwrap();
        let integrity: String = conn_b.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(integrity, "ok");

        let count: i64 = conn_b.query_row("SELECT count(*) FROM users", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
        let name: String = conn_b.query_row("SELECT name FROM users WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "Alice");

        // 验证 media 已恢复
        let media_file_b = history_dir_b.join("media").join("cd").join("photo.png");
        assert!(media_file_b.exists());
        assert_eq!(fs::read(&media_file_b).unwrap(), b"png-binary-data-test");
    }
}
