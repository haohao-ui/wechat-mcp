use anyhow::{Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStage {
    Staged,
    PublisherVerified,
    LocalSigned,
    Draining,
    BackupVerified,
    Switching,
    CandidateTesting,
    Committed,
    RollbackStarted,
    RolledBack,
    RecoveryRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateJournal {
    pub transaction_id: String,
    pub stage: TransactionStage,
    pub source_release_id: String,
    pub target_release_id: String,
    pub target_sequence: u64,
    pub timestamp: u64,
    pub backup_path: Option<String>,
    pub error: Option<String>,
    pub maintenance_marker_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    Idle,
    DiscardCandidate,
    ResumeOldRelease,
    TestCandidate,
    RollbackToPrevious,
    CommitSuccess,
    ManualInterventionRequired(String),
}

/// 跨进程排他更新锁守卫
#[derive(Debug)]
pub struct UpdateLockGuard {
    _file: File,
    pub lock_path: PathBuf,
}

impl UpdateJournal {
    pub fn new(
        transaction_id: impl Into<String>,
        source_release_id: impl Into<String>,
        target_release_id: impl Into<String>,
        target_sequence: u64,
        timestamp: u64,
    ) -> Self {
        Self {
            transaction_id: transaction_id.into(),
            stage: TransactionStage::Staged,
            source_release_id: source_release_id.into(),
            target_release_id: target_release_id.into(),
            target_sequence,
            timestamp,
            backup_path: None,
            error: None,
            maintenance_marker_confirmed: false,
        }
    }

    /// 原子持久化写入 journal 文件（临时文件写入 -> fsync -> rename -> 目录 fsync）
    pub fn persist_atomic(&self, state_dir: &Path) -> Result<()> {
        if !state_dir.exists() {
            fs::create_dir_all(state_dir)?;
            fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700))?;
        }

        let journal_path = state_dir.join("update-journal.json");
        let tmp_path = state_dir.join(format!("update-journal.tmp.{}", uuid::Uuid::new_v4()));

        let json_bytes = serde_json::to_vec_pretty(self)?;

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp_path)?;

        file.write_all(&json_bytes)?;
        file.sync_all()?;
        drop(file);

        fs::rename(&tmp_path, &journal_path)?;

        // 目录 fsync
        sync_dir(state_dir)?;

        Ok(())
    }

    /// 从 state_dir 读取 journal。损坏或坏 JSON 严禁静默吞掉，必须严格抛错
    pub fn load_from_state(state_dir: &Path) -> Result<Option<Self>> {
        let journal_path = state_dir.join("update-journal.json");
        if !journal_path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&journal_path)?;
        let journal: Self = serde_json::from_slice(&bytes)?;
        Ok(Some(journal))
    }

    /// 评估断电/崩溃后的恢复动作（采用严格规范化路径精确匹配，绝不使用 contains 模糊匹配，绝不把未知 current 当旧版安全）
    pub fn evaluate_recovery(&self, _releases_dir: &Path, current_link: &Path) -> RecoveryAction {
        match self.stage {
            TransactionStage::Staged
            | TransactionStage::PublisherVerified
            | TransactionStage::LocalSigned => RecoveryAction::DiscardCandidate,
            TransactionStage::Draining | TransactionStage::BackupVerified => {
                // 尚未切换指针，安全恢复旧版
                RecoveryAction::ResumeOldRelease
            }
            TransactionStage::Switching => {
                // 检查实际 current 指向（必须精确匹配，绝不能模糊 contains）
                match fs::read_link(current_link) {
                    Ok(target) => {
                        let target_str = target.to_string_lossy().to_string();
                        let target_leaf = target
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();

                        if target_leaf == self.target_release_id
                            || target_str.ends_with(&format!("/{}", self.target_release_id))
                        {
                            RecoveryAction::TestCandidate
                        } else if target_leaf == self.source_release_id
                            || target_str.ends_with(&format!("/{}", self.source_release_id))
                        {
                            RecoveryAction::ResumeOldRelease
                        } else {
                            // 未知 current 目标：严格 fail closed，绝不静默当作旧版
                            RecoveryAction::ManualInterventionRequired(format!(
                                "unknown_current_target: current symlink points to '{}', which matches neither target '{}' nor source '{}'",
                                target_str, self.target_release_id, self.source_release_id
                            ))
                        }
                    }
                    Err(e) => {
                        // current 软链损坏或不存在：严格要求人工介入
                        RecoveryAction::ManualInterventionRequired(format!(
                            "unreadable_current_symlink: {}",
                            e
                        ))
                    }
                }
            }
            TransactionStage::CandidateTesting => {
                // 候选自检阶段中断，需重新自检；若自检失败则切回 previous
                RecoveryAction::TestCandidate
            }
            TransactionStage::RollbackStarted => RecoveryAction::RollbackToPrevious,
            TransactionStage::Committed => RecoveryAction::CommitSuccess,
            TransactionStage::RolledBack => RecoveryAction::ResumeOldRelease,
            TransactionStage::RecoveryRequired => RecoveryAction::ManualInterventionRequired(
                self.error
                    .clone()
                    .unwrap_or_else(|| "manual intervention required".into()),
            ),
        }
    }
}

/// 获取跨进程排他更新锁，全事务持有
pub fn acquire_update_lock(state_dir: &Path) -> Result<UpdateLockGuard> {
    if !state_dir.exists() {
        fs::create_dir_all(state_dir)?;
        fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700))?;
    }
    let lock_path = state_dir.join("update.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)?;

    file.try_lock_exclusive().map_err(|_| {
        anyhow::anyhow!(
            "concurrent_update_in_progress: another update transaction holds update.lock"
        )
    })?;

    Ok(UpdateLockGuard {
        _file: file,
        lock_path,
    })
}

/// 已安装 release 元数据闭合结构
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CurrentReleaseMetadata {
    pub release_id: String,
    pub sequence: u64,
    pub channel: String,
    pub components_consistent: bool,
    pub installed_at: u64,
}

pub fn save_current_release_metadata(
    state_dir: &Path,
    meta: &CurrentReleaseMetadata,
) -> Result<()> {
    let meta_path = state_dir.join("current_release_metadata.json");
    let tmp_path = state_dir.join(format!(
        "current_release_metadata.tmp.{}",
        uuid::Uuid::new_v4()
    ));
    let json_bytes = serde_json::to_vec_pretty(meta)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp_path)?;
    file.write_all(&json_bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp_path, &meta_path)?;
    sync_dir(state_dir)?;
    Ok(())
}

pub fn load_current_release_metadata(state_dir: &Path) -> Result<Option<CurrentReleaseMetadata>> {
    let meta_path = state_dir.join("current_release_metadata.json");
    if !meta_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&meta_path)?;
    let meta: CurrentReleaseMetadata = serde_json::from_slice(&bytes)?;
    Ok(Some(meta))
}

pub fn set_maintenance_active(state_dir: &Path, reason: &str) -> Result<()> {
    let m_path = state_dir.join("core.maintenance");
    let tmp_path = state_dir.join(format!("core.maintenance.tmp.{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, reason.as_bytes())?;
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp_path, &m_path)?;
    sync_dir(state_dir)?;
    Ok(())
}

pub fn clear_maintenance_if_safe(state_dir: &Path) -> Result<()> {
    let m_path = state_dir.join("core.maintenance");
    if m_path.exists() {
        fs::remove_file(&m_path)?;
        sync_dir(state_dir)?;
    }
    Ok(())
}

/// 同步目录元数据
pub fn sync_dir(dir: &Path) -> Result<()> {
    let f = File::open(dir)?;
    use std::os::unix::io::AsRawFd;
    let fd = f.as_raw_fd();
    unsafe {
        if libc::fsync(fd) != 0 {
            let err = std::io::Error::last_os_error();
            return Err(err.into());
        }
    }
    Ok(())
}

/// 写入或核对持久 highest_accepted_sequence。文件损坏时严格返回 Err，禁止静默重置为 0。
pub fn load_highest_accepted_sequence(state_dir: &Path) -> Result<u64> {
    let seq_path = state_dir.join("highest_accepted_sequence");
    if !seq_path.exists() {
        return Ok(0);
    }
    let content = fs::read_to_string(&seq_path)?;
    match content.trim().parse::<u64>() {
        Ok(val) => Ok(val),
        Err(e) => bail!(
            "corrupt_highest_accepted_sequence_file: '{}' ({})",
            seq_path.display(),
            e
        ),
    }
}

pub fn save_highest_accepted_sequence(state_dir: &Path, seq: u64) -> Result<()> {
    let seq_path = state_dir.join("highest_accepted_sequence");
    let tmp_path = state_dir.join(format!(
        "highest_accepted_sequence.tmp.{}",
        uuid::Uuid::new_v4()
    ));
    fs::write(&tmp_path, seq.to_string())?;
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp_path, seq_path)?;
    sync_dir(state_dir)?;
    Ok(())
}
