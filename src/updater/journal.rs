use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

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

    /// 从 state_dir 读取 journal
    pub fn load_from_state(state_dir: &Path) -> Result<Option<Self>> {
        let journal_path = state_dir.join("update-journal.json");
        if !journal_path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&journal_path)?;
        let journal: Self = serde_json::from_slice(&bytes)?;
        Ok(Some(journal))
    }

    /// 评估断电/崩溃后的恢复动作
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
                // 检查实际 current 指向
                if let Ok(target) = fs::read_link(current_link) {
                    if target.to_string_lossy().contains(&self.target_release_id) {
                        RecoveryAction::TestCandidate
                    } else {
                        RecoveryAction::ResumeOldRelease
                    }
                } else {
                    RecoveryAction::ResumeOldRelease
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
