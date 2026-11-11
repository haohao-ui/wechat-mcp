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
    CandidateTestingPassed,
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
    #[serde(default)]
    pub drain_proof: Option<String>,
    #[serde(default)]
    pub backup_proof: Option<String>,
    #[serde(default)]
    pub candidate_proof: Option<String>,
    #[serde(default)]
    pub manifest_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    Idle,
    DiscardCandidate,
    ResumeOldRelease,
    TestCandidate,
    RollbackToPrevious,
    CompletePendingCommit,
    CommitSuccess,
    ManualInterventionRequired(String),
}

/// 跨进程排他更新锁守卫
#[derive(Debug)]
pub struct UpdateLockGuard {
    _file: File,
    pub lock_path: PathBuf,
}

/// 在指定父目录 FD 下通过 openat(dir_fd, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW) 验证子目录真实存在且不是软链接
pub fn verify_subdir_secure_at(parent_fd: std::os::unix::io::RawFd, name: &str) -> Result<()> {
    use std::ffi::CString;
    let c_name = CString::new(name)?;
    let child_fd = unsafe {
        libc::openat(
            parent_fd,
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if child_fd < 0 {
        let err = std::io::Error::last_os_error();
        bail!("target_subdir_not_found_or_symlink: '{}' ({})", name, err);
    }
    unsafe {
        libc::close(child_fd);
    }
    Ok(())
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
            drain_proof: None,
            backup_proof: None,
            candidate_proof: None,
            manifest_digest: None,
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

    /// 校验 Proof 证据结构体全要素一致性（防伪造空串或非当前 tx 证明）
    pub fn verify_proofs_consistent(&self, expected_tx_id: &str) -> Result<(), String> {
        // 1. drain_proof 校验
        let drain_str = self
            .drain_proof
            .as_ref()
            .ok_or_else(|| "missing_drain_proof".to_string())?;
        #[derive(Deserialize)]
        struct DrainProofCheck {
            tx_id: String,
        }
        let drain: DrainProofCheck = serde_json::from_str(drain_str)
            .map_err(|e| format!("corrupt_drain_proof_json: {}", e))?;
        if drain.tx_id != expected_tx_id {
            return Err(format!(
                "drain_proof_tx_mismatch: proof tx '{}' != journal tx '{}'",
                drain.tx_id, expected_tx_id
            ));
        }

        // 2. candidate_proof 校验
        let cand_str = self
            .candidate_proof
            .as_ref()
            .ok_or_else(|| "missing_candidate_proof".to_string())?;
        #[derive(Deserialize)]
        struct CandidateProofCheck {
            #[serde(default)]
            tx_id: Option<String>,
            exit_code: i32,
        }
        let cand: CandidateProofCheck = serde_json::from_str(cand_str)
            .map_err(|e| format!("corrupt_candidate_proof_json: {}", e))?;
        if cand.exit_code != 0 {
            return Err(format!(
                "candidate_proof_exit_code_non_zero: {}",
                cand.exit_code
            ));
        }
        if let Some(ref tx) = cand.tx_id {
            if tx != expected_tx_id {
                return Err(format!(
                    "candidate_proof_tx_mismatch: proof tx '{}' != journal tx '{}'",
                    tx, expected_tx_id
                ));
            }
        }

        // 3. backup_proof 存在性校验
        if self.backup_proof.is_none() {
            return Err("missing_backup_proof".to_string());
        }

        Ok(())
    }

    /// 内核级安全核验 current 软链接确实指向 target 且在 releases 目录中绝非伪造 symlink
    pub fn verify_current_link_secure(
        &self,
        releases_dir: &Path,
        current_link: &Path,
    ) -> Result<(), String> {
        let target = fs::read_link(current_link)
            .map_err(|e| format!("unreadable_current_symlink: {}", e))?;

        let expected_target = releases_dir.join(&self.target_release_id);
        let is_matching_target = target == expected_target
            || target == Path::new("releases").join(&self.target_release_id)
            || target == Path::new(&self.target_release_id);
        if !is_matching_target {
            return Err(format!(
                "current_symlink_target_mismatch: current points to '{}', expected '{}'",
                target.display(),
                self.target_release_id
            ));
        }

        use std::ffi::CString;
        let c_releases = match CString::new(releases_dir.to_str().unwrap_or_default()) {
            Ok(c) => c,
            Err(_) => return Err("invalid_releases_path".into()),
        };
        let r_fd = unsafe {
            libc::open(
                c_releases.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if r_fd < 0 {
            return Err("unreadable_releases_dir_fd".into());
        }

        let verified = verify_subdir_secure_at(r_fd, &self.target_release_id);
        unsafe {
            libc::close(r_fd);
        }
        verified.map_err(|e| format!("target_subdir_in_releases_dir_invalid: {}", e))
    }

    /// 评估断电/崩溃后的恢复动作（采用严格规范化路径精确匹配，绝不使用 contains 模糊匹配，绝不把未知 current 当旧版安全）
    pub fn evaluate_recovery(&self, releases_dir: &Path, current_link: &Path) -> RecoveryAction {
        let state_dir = releases_dir.parent().map(|p| p.join("state"));
        self.evaluate_recovery_with_state(state_dir.as_deref(), releases_dir, current_link)
    }

    pub fn evaluate_recovery_with_state(
        &self,
        state_dir: Option<&Path>,
        releases_dir: &Path,
        current_link: &Path,
    ) -> RecoveryAction {
        match self.stage {
            TransactionStage::Staged
            | TransactionStage::PublisherVerified
            | TransactionStage::LocalSigned => RecoveryAction::DiscardCandidate,
            TransactionStage::Draining | TransactionStage::BackupVerified => {
                // 尚未切换指针，若存在旧版则安全恢复旧版，若首次安装则需人工介入
                if self.source_release_id == "none" {
                    RecoveryAction::ManualInterventionRequired(
                        "initial_install_interrupted_no_source_release".into(),
                    )
                } else {
                    RecoveryAction::ResumeOldRelease
                }
            }
            TransactionStage::Switching => {
                // 检查实际 current 指向（必须受限在 releases_dir 内，且精确匹配）
                match fs::read_link(current_link) {
                    Ok(target) => {
                        let expected_target = releases_dir.join(&self.target_release_id);
                        let expected_source = releases_dir.join(&self.source_release_id);

                        let is_matching_target = target == expected_target
                            || target == Path::new("releases").join(&self.target_release_id)
                            || target == Path::new(&self.target_release_id);
                        let is_matching_source = target == expected_source
                            || target == Path::new("releases").join(&self.source_release_id)
                            || target == Path::new(&self.source_release_id);

                        use std::ffi::CString;
                        let c_releases =
                            match CString::new(releases_dir.to_str().unwrap_or_default()) {
                                Ok(c) => c,
                                Err(_) => {
                                    return RecoveryAction::ManualInterventionRequired(
                                        "invalid_releases_path".into(),
                                    );
                                }
                            };
                        let r_fd = unsafe {
                            libc::open(
                                c_releases.as_ptr(),
                                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                            )
                        };
                        if r_fd < 0 {
                            return RecoveryAction::ManualInterventionRequired(
                                "unreadable_releases_dir_fd".into(),
                            );
                        }

                        if is_matching_target {
                            let verified_target =
                                verify_subdir_secure_at(r_fd, &self.target_release_id);
                            unsafe {
                                libc::close(r_fd);
                            }
                            if verified_target.is_ok() {
                                RecoveryAction::TestCandidate
                            } else {
                                RecoveryAction::ManualInterventionRequired(
                                    "target_subdir_in_releases_dir_invalid".into(),
                                )
                            }
                        } else if is_matching_source {
                            let verified_source =
                                verify_subdir_secure_at(r_fd, &self.source_release_id);
                            unsafe {
                                libc::close(r_fd);
                            }
                            if verified_source.is_ok() {
                                RecoveryAction::ResumeOldRelease
                            } else {
                                RecoveryAction::ManualInterventionRequired(
                                    "source_subdir_in_releases_dir_invalid".into(),
                                )
                            }
                        } else {
                            unsafe {
                                libc::close(r_fd);
                            }
                            RecoveryAction::ManualInterventionRequired(format!(
                                "foreign_or_unknown_current_target: current symlink points to '{}', which matches neither target nor source in releases_dir",
                                target.display()
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
            TransactionStage::CandidateTestingPassed => {
                // 候选自检已通过但尚未写入 Committed 的崩溃窗口：精确恢复核验
                let state_d = match state_dir {
                    Some(d) => d,
                    None => {
                        return RecoveryAction::ManualInterventionRequired(
                            "candidate_passed_recovery_missing_state_dir".into(),
                        );
                    }
                };
                match load_current_release_record(state_d) {
                    Ok(Some(record)) => {
                        // 1. record 全要素校验
                        if record.tx_id != self.transaction_id
                            || record.release_id != self.target_release_id
                            || record.sequence != self.target_sequence
                        {
                            return RecoveryAction::ManualInterventionRequired(
                                "candidate_passed_but_commit_record_mismatch".into(),
                            );
                        }
                        if let Some(ref digest) = self.manifest_digest {
                            if record.manifest_digest != *digest {
                                return RecoveryAction::ManualInterventionRequired(
                                    "candidate_passed_but_manifest_digest_mismatch".into(),
                                );
                            }
                        }
                        // 2. proof 全要素校验
                        if let Err(proof_err) = self.verify_proofs_consistent(&self.transaction_id)
                        {
                            return RecoveryAction::ManualInterventionRequired(format!(
                                "candidate_passed_proof_inconsistent: {}",
                                proof_err
                            ));
                        }
                        // 3. current 软链校验
                        if let Err(link_err) =
                            self.verify_current_link_secure(releases_dir, current_link)
                        {
                            return RecoveryAction::ManualInterventionRequired(format!(
                                "candidate_passed_current_link_insecure: {}",
                                link_err
                            ));
                        }
                        // 全要素一致：权威记录已生效，精确恢复为推进完成提交
                        RecoveryAction::CompletePendingCommit
                    }
                    Ok(None) => {
                        // 权威记录不存在（写 record 失败或断电）：绝不猜成功，严格保持维护态
                        RecoveryAction::ManualInterventionRequired(
                            "record_commit_not_found_after_candidate_testing_passed".into(),
                        )
                    }
                    Err(e) => RecoveryAction::ManualInterventionRequired(format!(
                        "corrupt_current_release_record: {}",
                        e
                    )),
                }
            }
            TransactionStage::RollbackStarted => RecoveryAction::RollbackToPrevious,
            TransactionStage::Committed => {
                // 全要素严格核验：proof、权威 record、受限 target 目录全要素匹配；若缺失证明严格保持人工介入
                if let Err(proof_err) = self.verify_proofs_consistent(&self.transaction_id) {
                    return RecoveryAction::ManualInterventionRequired(format!(
                        "committed_journal_proof_missing_or_inconsistent: {}",
                        proof_err
                    ));
                }

                if let Err(link_err) = self.verify_current_link_secure(releases_dir, current_link) {
                    return RecoveryAction::ManualInterventionRequired(format!(
                        "committed_current_link_insecure: {}",
                        link_err
                    ));
                }

                if let Some(state_d) = state_dir {
                    match load_current_release_record(state_d) {
                        Ok(Some(record)) => {
                            if record.tx_id != self.transaction_id
                                || record.release_id != self.target_release_id
                                || record.sequence != self.target_sequence
                            {
                                return RecoveryAction::ManualInterventionRequired(
                                    "committed_record_mismatch_with_journal".into(),
                                );
                            }
                            if let Some(ref digest) = self.manifest_digest {
                                if record.manifest_digest != *digest {
                                    return RecoveryAction::ManualInterventionRequired(
                                        "committed_record_manifest_digest_mismatch".into(),
                                    );
                                }
                            }
                        }
                        Ok(None) => {
                            return RecoveryAction::ManualInterventionRequired(
                                "committed_state_missing_authoritative_release_record".into(),
                            );
                        }
                        Err(e) => {
                            return RecoveryAction::ManualInterventionRequired(format!(
                                "committed_state_corrupt_authoritative_release_record: {}",
                                e
                            ));
                        }
                    }
                }

                RecoveryAction::CommitSuccess
            }
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

/// 单一权威提交记录：闭合版本化，原子绑定 tx_id、最高水位、manifest 摘要与元数据
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CurrentReleaseCommitRecord {
    pub record_schema: u32,
    pub tx_id: String,
    pub release_id: String,
    pub sequence: u64,
    pub channel: String,
    pub manifest_digest: String,
    pub installed_at: u64,
}

pub fn atomic_commit_release_record(
    state_dir: &Path,
    record: &CurrentReleaseCommitRecord,
) -> Result<()> {
    if !state_dir.exists() {
        fs::create_dir_all(state_dir)?;
        fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700))?;
    }

    let record_path = state_dir.join("current_release.json");
    let tmp_path = state_dir.join(format!("current_release.tmp.{}", uuid::Uuid::new_v4()));

    let json_bytes = serde_json::to_vec_pretty(record)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp_path)?;
    file.write_all(&json_bytes)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_path, &record_path)?;

    // 同时原子更新高水位文件，确保旧组件读取兼容
    save_highest_accepted_sequence(state_dir, record.sequence)?;

    sync_dir(state_dir)?;
    Ok(())
}

pub fn load_current_release_record(state_dir: &Path) -> Result<Option<CurrentReleaseCommitRecord>> {
    let record_path = state_dir.join("current_release.json");
    if !record_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&record_path)?;
    let record: CurrentReleaseCommitRecord = serde_json::from_slice(&bytes)?;
    Ok(Some(record))
}

/// 向后兼容接口：别名转发至 load_current_release_record
pub fn load_current_release_metadata(
    state_dir: &Path,
) -> Result<Option<CurrentReleaseCommitRecord>> {
    load_current_release_record(state_dir)
}

/// 向后兼容接口：设置维护标志（自动生成合法 36 字节 UUID）
pub fn set_maintenance_active(state_dir: &Path, reason: &str) -> Result<()> {
    let dummy_tx = format!("tx-{}", uuid::Uuid::new_v4());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    set_structured_maintenance_marker(state_dir, &dummy_tx, reason, now)
}

/// 从 tx_id 中精确提取 36 字节小写标准 UUID（如 "tx-<uuid>" 或 "<uuid>"）
pub fn extract_pure_uuid(tx_id: &str) -> Result<String> {
    let raw = if let Some(stripped) = tx_id.strip_prefix("tx-") {
        stripped
    } else {
        tx_id
    };
    let parsed = uuid::Uuid::parse_str(raw).map_err(|e| {
        anyhow::anyhow!(
            "invalid_tx_uuid: '{}' is not a valid 36-byte UUID: {}",
            tx_id,
            e
        )
    })?;
    let lower = parsed.to_string();
    if lower.len() != 36 {
        bail!(
            "extracted_uuid_length_mismatch: expected 36, got {}",
            lower.len()
        );
    }
    Ok(lower)
}

/// 沙箱内部结构化维护标志
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceMarker {
    pub marker_schema: u32,
    pub tx_id: String,
    pub reason: String,
    pub timestamp: u64,
}

pub fn set_structured_maintenance_marker(
    state_dir: &Path,
    tx_id: &str,
    reason: &str,
    now: u64,
) -> Result<()> {
    let pure_uuid = extract_pure_uuid(tx_id)?;
    let marker_path = state_dir.join("core.maintenance.json");
    let tmp_json_path = state_dir.join(format!(".maintenance.json.tmp.{}", uuid::Uuid::new_v4()));

    let marker = MaintenanceMarker {
        marker_schema: 1,
        tx_id: tx_id.to_string(),
        reason: reason.to_string(),
        timestamp: now,
    };

    let json_bytes = serde_json::to_vec_pretty(&marker)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp_json_path)?;
    file.write_all(&json_bytes)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_json_path, &marker_path)?;

    // 写入严格 36 字节纯 ASCII UUID 的 core.maintenance 标志，权限严格 0600
    let raw_flag = state_dir.join("core.maintenance");
    let tmp_raw_path = state_dir.join(format!(".maintenance.tmp.{}", uuid::Uuid::new_v4()));
    let mut raw_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp_raw_path)?;
    raw_file.write_all(pure_uuid.as_bytes())?;
    raw_file.sync_all()?;
    drop(raw_file);

    fs::rename(&tmp_raw_path, &raw_flag)?;

    sync_dir(state_dir)?;
    Ok(())
}

pub fn clear_maintenance_for_tx(state_dir: &Path, current_tx_id: &str) -> Result<()> {
    let pure_uuid = extract_pure_uuid(current_tx_id)?;

    let marker_path = state_dir.join("core.maintenance.json");
    if marker_path.exists() {
        let bytes = fs::read(&marker_path)?;
        let marker: MaintenanceMarker = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("corrupt_maintenance_marker: {}", e))?;

        if marker.tx_id != current_tx_id {
            bail!(
                "maintenance_marker_tx_mismatch: existing marker belongs to tx '{}', cannot clear by tx '{}'",
                marker.tx_id,
                current_tx_id
            );
        }
        fs::remove_file(&marker_path)?;
    }

    let raw_flag = state_dir.join("core.maintenance");
    if raw_flag.exists() {
        let bytes = fs::read(&raw_flag)?;
        let existing = String::from_utf8_lossy(&bytes).trim().to_string();
        if existing != pure_uuid {
            bail!(
                "maintenance_marker_uuid_mismatch: existing marker has UUID '{}', cannot clear by tx '{}' (uuid '{}')",
                existing,
                current_tx_id,
                pure_uuid
            );
        }
        fs::remove_file(&raw_flag)?;
    }

    sync_dir(state_dir)?;
    Ok(())
}

/// 启动前双检查决策
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupDualCheckDecision {
    /// 正常启动允许：无维护标志且无未完成/损坏的 journal
    AllowStartup,
    /// 维护抑制：存在合法维护标记或处于合法的非终态更新事务中
    MaintenanceSuppressed {
        tx_id: String,
        stage: Option<TransactionStage>,
    },
    /// 故障保护：文件损坏、反序列化失败、符号链接冒充、权限/UID异常、或 Committed 但实际 record/指针不一致，必须 Fail-Closed
    RecoveryRequired { reason: String },
}

/// 纯只读启动双检查 Guard（零写入、零删除、零副作用）
pub fn check_startup_guard_readonly(
    state_dir: &Path,
    releases_dir: &Path,
    current_link: &Path,
) -> StartupDualCheckDecision {
    use std::os::unix::fs::MetadataExt;
    let current_uid = unsafe { libc::geteuid() };

    // 1. 检查 core.maintenance
    let maint_path = state_dir.join("core.maintenance");
    let mut maint_uuid: Option<String> = None;
    if maint_path.exists() || fs::symlink_metadata(&maint_path).is_ok() {
        let meta = match fs::symlink_metadata(&maint_path) {
            Ok(m) => m,
            Err(e) => {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("cannot_stat_maintenance_marker: {}", e),
                };
            }
        };
        if meta.file_type().is_symlink() {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: "core.maintenance_must_not_be_symlink".into(),
            };
        }
        if meta.uid() != current_uid {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: "core.maintenance_uid_mismatch".into(),
            };
        }
        if (meta.permissions().mode() & 0o777) != 0o600 {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: "core.maintenance_permissions_must_be_0600".into(),
            };
        }
        if meta.len() != 36 {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: format!("core.maintenance_size_must_be_36_bytes, got {}", meta.len()),
            };
        }
        let content = match fs::read_to_string(&maint_path) {
            Ok(s) => s,
            Err(e) => {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("cannot_read_core_maintenance: {}", e),
                };
            }
        };
        let parsed_uuid = match uuid::Uuid::parse_str(content.trim()) {
            Ok(u) => u.to_string(),
            Err(e) => {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("core.maintenance_content_not_valid_uuid: {}", e),
                };
            }
        };
        maint_uuid = Some(parsed_uuid);
    }

    // 2. 检查 update-journal.json
    let journal_path = state_dir.join("update-journal.json");
    if journal_path.exists() || fs::symlink_metadata(&journal_path).is_ok() {
        let meta = match fs::symlink_metadata(&journal_path) {
            Ok(m) => m,
            Err(e) => {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("cannot_stat_update_journal: {}", e),
                };
            }
        };
        if meta.file_type().is_symlink() {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: "update-journal.json_must_not_be_symlink".into(),
            };
        }
        if meta.uid() != current_uid {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: "update-journal.json_uid_mismatch".into(),
            };
        }
        if (meta.permissions().mode() & 0o777) != 0o600 {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: "update-journal.json_permissions_must_be_0600".into(),
            };
        }
        let journal = match UpdateJournal::load_from_state(state_dir) {
            Ok(Some(j)) => j,
            Ok(None) => {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: "update-journal.json_file_missing_on_load".into(),
                };
            }
            Err(e) => {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("update-journal.json_corrupt_or_invalid: {}", e),
                };
            }
        };

        match journal.stage {
            TransactionStage::RecoveryRequired => {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("journal_in_recovery_required_stage: {:?}", journal.error),
                };
            }
            TransactionStage::Staged
            | TransactionStage::PublisherVerified
            | TransactionStage::LocalSigned
            | TransactionStage::Draining
            | TransactionStage::BackupVerified
            | TransactionStage::Switching
            | TransactionStage::CandidateTesting
            | TransactionStage::CandidateTestingPassed
            | TransactionStage::RollbackStarted => {
                return StartupDualCheckDecision::MaintenanceSuppressed {
                    tx_id: journal.transaction_id,
                    stage: Some(journal.stage),
                };
            }
            TransactionStage::RolledBack => {
                if let Some(uid) = maint_uuid {
                    return StartupDualCheckDecision::MaintenanceSuppressed {
                        tx_id: uid,
                        stage: Some(TransactionStage::RolledBack),
                    };
                }
                return StartupDualCheckDecision::AllowStartup;
            }
            TransactionStage::Committed => {
                // 终态全要素二次只读核验：严禁仅凭 Committed 标签放行！
                // a. 核查 current_release.json
                let record_path = state_dir.join("current_release.json");
                if !record_path.exists() {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: "committed_journal_missing_current_release_record".into(),
                    };
                }
                let rec_meta = match fs::symlink_metadata(&record_path) {
                    Ok(m) => m,
                    Err(e) => {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: format!("cannot_stat_current_release_record: {}", e),
                        };
                    }
                };
                if rec_meta.file_type().is_symlink() || rec_meta.uid() != current_uid {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: "current_release_record_invalid_symlink_or_uid".into(),
                    };
                }
                let commit_record = match load_current_release_record(state_dir) {
                    Ok(Some(r)) => r,
                    _ => {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: "failed_to_load_current_release_record".into(),
                        };
                    }
                };

                if commit_record.tx_id != journal.transaction_id {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!(
                            "commit_record_tx_mismatch: record '{}' != journal '{}'",
                            commit_record.tx_id, journal.transaction_id
                        ),
                    };
                }
                if commit_record.release_id != journal.target_release_id {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!(
                            "commit_record_release_id_mismatch: record '{}' != journal '{}'",
                            commit_record.release_id, journal.target_release_id
                        ),
                    };
                }
                if commit_record.sequence != journal.target_sequence {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!(
                            "commit_record_sequence_mismatch: record '{}' != journal '{}'",
                            commit_record.sequence, journal.target_sequence
                        ),
                    };
                }
                if let Some(ref j_digest) = journal.manifest_digest {
                    if commit_record.manifest_digest != *j_digest {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: "commit_record_digest_mismatch".into(),
                        };
                    }
                }

                // b. 核查 current 软链接与目标目录
                if !current_link.exists() && fs::symlink_metadata(current_link).is_err() {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: "current_link_does_not_exist".into(),
                    };
                }
                let link_meta = match fs::symlink_metadata(current_link) {
                    Ok(m) => m,
                    Err(e) => {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: format!("cannot_stat_current_link: {}", e),
                        };
                    }
                };
                if !link_meta.file_type().is_symlink() {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: "current_link_must_be_symlink".into(),
                    };
                }
                let link_target = match fs::read_link(current_link) {
                    Ok(t) => t,
                    Err(e) => {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: format!("cannot_read_current_link: {}", e),
                        };
                    }
                };
                let target_leaf = link_target
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if target_leaf != journal.target_release_id {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!(
                            "current_link_target_mismatch: link points to '{}', expected '{}'",
                            target_leaf, journal.target_release_id
                        ),
                    };
                }
                let target_dir = releases_dir.join(target_leaf);
                if !target_dir.is_dir() {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!(
                            "current_link_target_directory_missing: '{}'",
                            target_dir.display()
                        ),
                    };
                }

                if let Some(uid) = maint_uuid {
                    return StartupDualCheckDecision::MaintenanceSuppressed {
                        tx_id: uid,
                        stage: Some(TransactionStage::Committed),
                    };
                }
                return StartupDualCheckDecision::AllowStartup;
            }
        }
    }

    // journal 不存在
    if let Some(uid) = maint_uuid {
        StartupDualCheckDecision::MaintenanceSuppressed {
            tx_id: uid,
            stage: None,
        }
    } else {
        StartupDualCheckDecision::AllowStartup
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

/// 读取旧版原始 highest_accepted_sequence 文件
pub fn load_raw_highest_accepted_sequence(state_dir: &Path) -> Result<Option<u64>> {
    let seq_path = state_dir.join("highest_accepted_sequence");
    if !seq_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&seq_path)?;
    match content.trim().parse::<u64>() {
        Ok(val) => Ok(Some(val)),
        Err(e) => bail!(
            "corrupt_highest_accepted_sequence_file: '{}' ({})",
            seq_path.display(),
            e
        ),
    }
}

/// 写入或核对最高水位：统一核对单一权威 current_release.json 与旧版文件，冲突/损坏时严格 fail closed，绝不高水位回 0
pub fn load_highest_accepted_sequence(state_dir: &Path) -> Result<u64> {
    let record_opt = load_current_release_record(state_dir)?;
    let old_seq_opt = load_raw_highest_accepted_sequence(state_dir)?;

    match (record_opt, old_seq_opt) {
        (Some(record), Some(old_seq)) => {
            if record.sequence != old_seq {
                bail!(
                    "sequence_conflict_detected: current_release.json sequence {} != highest_accepted_sequence file {}",
                    record.sequence,
                    old_seq
                );
            }
            Ok(record.sequence)
        }
        (Some(record), None) => Ok(record.sequence),
        (None, Some(old_seq)) => Ok(old_seq),
        (None, None) => Ok(0),
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
