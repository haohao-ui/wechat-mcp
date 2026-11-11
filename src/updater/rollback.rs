use anyhow::{Result, bail};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// 原子切换 current 版本指针并记录 previous 指针
pub fn switch_release_atomic(app_root: &Path, target_release_id: &str) -> Result<()> {
    let releases_dir = app_root.join("releases");
    let target_dir = releases_dir.join(target_release_id);
    if !target_dir.exists() {
        bail!("target_release_not_found: {}", target_dir.display());
    }

    let current_link = app_root.join("current");
    let previous_link = app_root.join("previous");

    // 记录旧的 current 指向到 previous
    if current_link.exists() || fs::symlink_metadata(&current_link).is_ok() {
        if let Ok(old_target) = fs::read_link(&current_link) {
            let tmp_prev = app_root.join(format!("previous.tmp.{}", uuid::Uuid::new_v4()));
            symlink(&old_target, &tmp_prev)?;
            fs::rename(&tmp_prev, &previous_link)?;
        }
    }

    // 原子切换 current 指向 target_dir
    let tmp_curr = app_root.join(format!("current.tmp.{}", uuid::Uuid::new_v4()));
    symlink(&target_dir, &tmp_curr)?;
    fs::rename(&tmp_curr, &current_link)?;

    super::journal::sync_dir(app_root)?;

    Ok(())
}

/// 原子切回 previous 版本指针
pub fn rollback_release_atomic(app_root: &Path) -> Result<PathBuf> {
    let current_link = app_root.join("current");
    let previous_link = app_root.join("previous");

    if !previous_link.exists() && fs::symlink_metadata(&previous_link).is_err() {
        bail!("no_previous_release_available: previous symlink does not exist");
    }

    let prev_target = fs::read_link(&previous_link)?;
    if !prev_target.exists() {
        bail!("previous_release_target_missing: {}", prev_target.display());
    }

    let tmp_curr = app_root.join(format!("current.tmp.{}", uuid::Uuid::new_v4()));
    symlink(&prev_target, &tmp_curr)?;
    fs::rename(&tmp_curr, &current_link)?;

    super::journal::sync_dir(app_root)?;

    Ok(prev_target)
}

/// 检查切换后是否已产生新业务数据（仅限合成 fixture 探测）。
/// 严格安全准则：对数据库不存在、权限异常、损坏、未知 schema 或查询 SQL 失败一律返回 Err。
/// 注意：该函数仅检查 messages/turns 时间戳，不能证明完整业务 schema（如 contacts、sessions 等）无数据变更；
/// 在无全量一致快照证明前，不得将 false 解释为可安全自动覆盖恢复旧数据库。
pub fn has_new_data_since_backup(db_path: &Path, backup_time: u64) -> Result<bool> {
    if !db_path.exists() {
        bail!("database_file_not_found: {}", db_path.display());
    }

    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| anyhow::anyhow!("cannot_open_db_for_inspection: {}", e))?;

    // 完整性快速校验（PRAGMA quick_check）
    let quick_check: String = conn
        .query_row("PRAGMA quick_check(1);", [], |row| row.get(0))
        .map_err(|e| anyhow::anyhow!("db_integrity_check_failed: {}", e))?;
    if quick_check != "ok" {
        bail!(
            "db_corrupted: PRAGMA quick_check returned '{}'",
            quick_check
        );
    }

    // 检查业务表 messages 是否存在
    let mut stmt = conn
        .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'messages';")?;
    let msg_table_exists: i64 = stmt.query_row([], |row| row.get(0))?;
    if msg_table_exists == 0 {
        bail!(
            "unknown_schema_missing_messages_table: schema does not match expected business tables"
        );
    }

    // 严格查询 messages 中是否存在大于 backup_time 的记录，错误绝不吞掉
    let mut msg_query = conn.prepare("SELECT COUNT(*) FROM messages WHERE created_at > ?;")?;
    let new_messages_count: i64 = msg_query.query_row([backup_time as i64], |row| row.get(0))?;
    if new_messages_count > 0 {
        return Ok(true);
    }

    // 检查 turns 表（若存在）
    let mut turns_stmt =
        conn.prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'turns';")?;
    let turns_table_exists: i64 = turns_stmt.query_row([], |row| row.get(0))?;
    if turns_table_exists > 0 {
        let mut turns_query = conn.prepare("SELECT COUNT(*) FROM turns WHERE created_at > ?;")?;
        let new_turns_count: i64 = turns_query.query_row([backup_time as i64], |row| row.get(0))?;
        if new_turns_count > 0 {
            return Ok(true);
        }
    }

    Ok(false)
}

/// 执行 SQLite 在线一致热备份并校验完整性与摘要
pub fn backup_and_verify_sqlite_db(src_db: &Path, backup_db: &Path) -> Result<String> {
    if !src_db.exists() {
        bail!("source_db_not_found: {}", src_db.display());
    }

    // 1. 打开源数据库只读连接
    let src_conn =
        rusqlite::Connection::open_with_flags(src_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| anyhow::anyhow!("cannot_open_src_db: {}", e))?;

    // 2. 若目标备份已存在先删除
    if backup_db.exists() {
        fs::remove_file(backup_db)?;
    }
    let mut dst_conn = rusqlite::Connection::open(backup_db)
        .map_err(|e| anyhow::anyhow!("cannot_open_dst_db: {}", e))?;

    // 3. 在线一致性备份 (rusqlite::backup::Backup)
    let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst_conn)
        .map_err(|e| anyhow::anyhow!("failed_to_initialize_backup: {}", e))?;
    backup
        .run_to_completion(10, std::time::Duration::from_millis(10), None)
        .map_err(|e| anyhow::anyhow!("backup_run_failed: {}", e))?;
    drop(backup);
    drop(src_conn);

    // 4. 对备份数据库运行完整性校验
    let integrity: String = dst_conn
        .query_row("PRAGMA integrity_check;", [], |row| row.get(0))
        .map_err(|e| anyhow::anyhow!("backup_integrity_query_failed: {}", e))?;
    if integrity != "ok" {
        bail!("backup_integrity_check_failed: returned '{}'", integrity);
    }
    drop(dst_conn);

    // 5. 计算并返回备份文件的 SHA-256
    let bytes = fs::read(backup_db)?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash_bytes = hasher.finalize();
    let mut hex_str = String::with_capacity(64);
    for b in hash_bytes {
        use std::fmt::Write;
        let _ = write!(hex_str, "{:02x}", b);
    }
    Ok(hex_str)
}

/// 数据库自动恢复守卫：
/// 在缺乏全量 WAL 快照与完整 schema 证明前，自动覆盖恢复旧 DB 必须明确返回 unsupported 并拒绝执行。
pub fn restore_database_backup_guarded(_backup_file: &Path, _target_db: &Path) -> Result<()> {
    bail!(
        "unsupported_db_rollback: automated database restoration without cryptographic write-ahead-log snapshot proof is unsupported to prevent silent data loss"
    );
}
