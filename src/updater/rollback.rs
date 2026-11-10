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

/// 检查切换后是否已产生新业务数据（防止更新失败回退时静默覆盖新产生的数据）
/// 若发现数据库存在 backup_time 之后的新消息或轮次，拒绝自动覆盖恢复
pub fn has_new_data_since_backup(db_path: &Path, backup_time: u64) -> Result<bool> {
    if !db_path.exists() {
        return Ok(false);
    }

    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(e) => {
            // 打不开数据库或损坏
            bail!("cannot_inspect_db_for_new_data: {}", e);
        }
    };

    // 检查是否有消息表/轮次表带有新时间戳
    let mut stmt = match conn.prepare(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('messages', 'turns', 'history')"
    ) {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };

    let table_count: i64 = stmt.query_row([], |row| row.get(0)).unwrap_or(0);
    if table_count == 0 {
        return Ok(false);
    }

    // 依次检查可能的业务表
    for tbl in &["turns", "messages", "history"] {
        let check_sql = format!("SELECT COUNT(*) FROM {} WHERE created_at > ?", tbl);
        if let Ok(mut query) = conn.prepare(&check_sql) {
            let count: i64 = query
                .query_row([backup_time as i64], |row| row.get(0))
                .unwrap_or(0);
            if count > 0 {
                return Ok(true);
            }
        }
    }

    Ok(false)
}
