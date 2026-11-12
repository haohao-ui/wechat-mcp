use anyhow::{Context, Result, bail};
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
    pub journal_schema: u32,
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
            journal_schema: 1,
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

/// 只读受限安全读取私有文件：
/// 防 TOCTOU 与特殊文件挂死：使用 O_NOFOLLOW | O_NONBLOCK 打开，fstat 核验属主 UID、0600 权限与普通文件，严格限长 max_bytes 读取。
/// 只有在显式返回 NotFound (ENOENT) 时才返回 Ok(None)，其他系统错误一律向上传播为 Err。
pub fn read_limited_private_file(path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;

    let file_res = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);

    let file = match file_res {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            bail!("file_must_not_be_symlink: '{}'", path.display());
        }
        Err(e) => bail!("cannot_open_file_safely: '{}' ({})", path.display(), e),
    };

    let meta = file
        .metadata()
        .with_context(|| format!("cannot_stat_opened_file: '{}'", path.display()))?;

    if !meta.file_type().is_file() {
        bail!("file_must_be_regular_file: '{}'", path.display());
    }

    let current_uid = unsafe { libc::geteuid() };
    if meta.uid() != current_uid {
        bail!(
            "file_uid_mismatch: '{}' (owner: {}, current: {})",
            path.display(),
            meta.uid(),
            current_uid
        );
    }

    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        bail!(
            "file_permissions_must_be_0600: '{}' (got 0o{:o})",
            path.display(),
            mode
        );
    }

    if meta.len() > max_bytes as u64 {
        bail!(
            "file_size_exceeds_limit: '{}' (size: {}, limit: {})",
            path.display(),
            meta.len(),
            max_bytes
        );
    }

    // 流式硬限制：take(max_bytes + 1)，防止 fstat 后文件并发增长导致无界内存消耗
    let mut limited = file.take((max_bytes + 1) as u64);
    let mut buf = Vec::with_capacity(std::cmp::min(meta.len() as usize, max_bytes));
    limited
        .read_to_end(&mut buf)
        .with_context(|| format!("failed_to_read_file: '{}'", path.display()))?;

    if buf.len() > max_bytes {
        bail!(
            "file_size_exceeded_limit_during_read: '{}' (bytes read exceeded {})",
            path.display(),
            max_bytes
        );
    }

    Ok(Some(buf))
}

/// 验证 release_id 命名安全：非空且仅含字母、数字、点、减号、下划线，绝不含 / 或 ..
pub fn validate_release_id_safe(release_id: &str) -> Result<()> {
    if release_id.is_empty() {
        bail!("release_id_cannot_be_empty");
    }
    if release_id == "." || release_id == ".." {
        bail!("release_id_cannot_be_dot: '{}'", release_id);
    }
    if !release_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        bail!("invalid_release_id_characters: '{}'", release_id);
    }
    Ok(())
}

/// 精确核验 current 软链接的目标必须是受控 releases_dir 下指定的物理目录，拒绝任何外部逃逸或悬空
struct SafeFd(libc::c_int);
impl Drop for SafeFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            unsafe { libc::close(self.0) };
        }
    }
}

pub fn verify_current_link_target(
    current_link: &Path,
    releases_dir: &Path,
    expected_release_id: &str,
) -> Result<()> {
    verify_current_link_target_with_root(current_link, releases_dir, expected_release_id, None)
}

/// 精确核验 current 软链接的目标必须是受控 releases_dir 下指定的物理目录，拒绝任何外部逃逸或悬空
pub fn verify_current_link_target_with_root(
    current_link: &Path,
    releases_dir: &Path,
    expected_release_id: &str,
    install_root: Option<&Path>,
) -> Result<()> {
    // a. 校验 release_id 命名格式安全
    validate_release_id_safe(expected_release_id)?;

    // b. 核验 current 软链接自身是否存在且为 symlink
    let link_meta = match fs::symlink_metadata(current_link) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("current_link_does_not_exist: '{}'", current_link.display());
        }
        Err(e) => bail!(
            "cannot_stat_current_link: '{}' ({})",
            current_link.display(),
            e
        ),
    };

    if !link_meta.file_type().is_symlink() {
        bail!("current_link_must_be_symlink: '{}'", current_link.display());
    }

    let current_uid = unsafe { libc::geteuid() };

    // c. 锚定受控 releases_dir：若提供 install_root，先校验 install_root 自身非 symlink 且归属当前 UID，
    //    然后通过 openat 打开 releases，防止父目录符号链接逃逸！
    let (root_fd, _ir_guard) = if let Some(ir) = install_root {
        if !ir.is_absolute() {
            bail!("install_root_must_be_absolute: '{}'", ir.display());
        }
        let c_ir = std::ffi::CString::new(
            ir.to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid_install_root_path"))?,
        )?;
        let ir_fd = unsafe {
            libc::open(
                c_ir.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if ir_fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ELOOP) {
                bail!("install_root_must_not_be_symlink: '{}'", ir.display());
            }
            bail!("cannot_open_install_root: '{}' ({})", ir.display(), err);
        }
        let ir_guard = SafeFd(ir_fd);

        let mut ir_stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(ir_fd, &mut ir_stat) } != 0 {
            bail!("cannot_fstat_install_root: '{}'", ir.display());
        }
        if (ir_stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
            bail!("install_root_must_be_directory: '{}'", ir.display());
        }
        if ir_stat.st_uid != current_uid {
            bail!(
                "install_root_uid_mismatch: '{}' (owner: {}, current: {})",
                ir.display(),
                ir_stat.st_uid,
                current_uid
            );
        }

        let c_rel_name = std::ffi::CString::new("releases")?;
        let r_fd = unsafe {
            libc::openat(
                ir_fd,
                c_rel_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if r_fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ELOOP) {
                bail!(
                    "releases_dir_must_not_be_symlink: '{}'",
                    releases_dir.display()
                );
            }
            bail!(
                "cannot_open_releases_dir: '{}' ({})",
                releases_dir.display(),
                err
            );
        }
        (r_fd, Some(ir_guard))
    } else {
        let c_releases = std::ffi::CString::new(
            releases_dir
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid_releases_dir_path"))?,
        )?;
        let r_fd = unsafe {
            libc::open(
                c_releases.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if r_fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::NotFound {
                bail!("releases_dir_does_not_exist: '{}'", releases_dir.display());
            }
            if err.raw_os_error() == Some(libc::ELOOP) {
                bail!(
                    "releases_dir_must_not_be_symlink: '{}'",
                    releases_dir.display()
                );
            }
            bail!(
                "cannot_open_releases_dir: '{}' ({})",
                releases_dir.display(),
                err
            );
        }
        (r_fd, None)
    };
    let _root_guard = SafeFd(root_fd);

    let mut root_stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(root_fd, &mut root_stat) } != 0 {
        bail!("cannot_fstat_releases_dir: '{}'", releases_dir.display());
    }
    if (root_stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        bail!(
            "releases_dir_must_be_directory: '{}'",
            releases_dir.display()
        );
    }
    let current_uid = unsafe { libc::geteuid() };
    if root_stat.st_uid != current_uid {
        bail!(
            "releases_dir_uid_mismatch: '{}' (owner: {}, current: {})",
            releases_dir.display(),
            root_stat.st_uid,
            current_uid
        );
    }

    // d. 在受控 root_fd 相对使用 openat 逐级打开 expected_release_id 物理目录（O_DIRECTORY | O_NOFOLLOW 拒绝中间及自身符号链接）
    let c_expected = std::ffi::CString::new(expected_release_id)?;
    let release_fd = unsafe {
        libc::openat(
            root_fd,
            c_expected.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if release_fd < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::NotFound {
            bail!(
                "expected_release_dir_does_not_exist: '{}'",
                expected_release_id
            );
        }
        if err.raw_os_error() == Some(libc::ELOOP) {
            bail!(
                "expected_release_dir_must_not_be_symlink: '{}'",
                expected_release_id
            );
        }
        if err.raw_os_error() == Some(libc::ENOTDIR) {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    root_fd,
                    c_expected.as_ptr(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } == 0
                && (st.st_mode & libc::S_IFMT) == libc::S_IFLNK
            {
                bail!(
                    "expected_release_dir_must_not_be_symlink: '{}'",
                    expected_release_id
                );
            }
            bail!(
                "expected_release_path_must_be_directory: '{}'",
                expected_release_id
            );
        }
        bail!(
            "cannot_open_expected_release_dir: '{}' ({})",
            expected_release_id,
            err
        );
    }
    let _release_guard = SafeFd(release_fd);

    let mut release_stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(release_fd, &mut release_stat) } != 0 {
        bail!(
            "cannot_fstat_expected_release_dir: '{}'",
            expected_release_id
        );
    }
    if (release_stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        bail!(
            "expected_release_path_must_be_directory: '{}'",
            expected_release_id
        );
    }
    if release_stat.st_uid != current_uid {
        bail!(
            "expected_release_dir_uid_mismatch: '{}' (owner: {}, current: {})",
            expected_release_id,
            release_stat.st_uid,
            current_uid
        );
    }

    // e. 读取 current 软链接物理目标，打开并比对内核 (st_dev, st_ino) 身份凭证
    let raw_target = fs::read_link(current_link)
        .with_context(|| format!("cannot_read_link: '{}'", current_link.display()))?;

    let target_path = if raw_target.is_relative() {
        let parent = current_link
            .parent()
            .ok_or_else(|| anyhow::anyhow!("current_link_has_no_parent"))?;
        parent.join(&raw_target)
    } else {
        raw_target.clone()
    };

    let c_target = std::ffi::CString::new(
        target_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid_target_path"))?,
    )?;
    let target_fd = unsafe {
        libc::open(
            c_target.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if target_fd < 0 {
        let err = std::io::Error::last_os_error();
        bail!(
            "current_target_cannot_be_opened: '{}' ({})",
            target_path.display(),
            err
        );
    }
    let _target_guard = SafeFd(target_fd);

    let mut target_stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(target_fd, &mut target_stat) } != 0 {
        bail!("cannot_fstat_current_target: '{}'", target_path.display());
    }

    // f. 终极内核凭据核验：(st_dev, st_ino) 必须与被锁定的 release_fd 严格一致！
    if target_stat.st_dev != release_stat.st_dev || target_stat.st_ino != release_stat.st_ino {
        bail!(
            "current_target_escape_detected: target (dev:{}, ino:{}) does not match anchored release (dev:{}, ino:{})",
            target_stat.st_dev,
            target_stat.st_ino,
            release_stat.st_dev,
            release_stat.st_ino
        );
    }

    Ok(())
}

pub fn clear_maintenance_for_tx(state_dir: &Path, current_tx_id: &str) -> Result<()> {
    let pure_uuid = extract_pure_uuid(current_tx_id)?;

    let marker_json_path = state_dir.join("core.maintenance.json");
    let raw_flag = state_dir.join("core.maintenance");

    // 1. 全量严格预检：任一失败绝不删除任何文件
    let has_json = match read_limited_private_file(&marker_json_path, 64 * 1024)? {
        Some(bytes) => {
            let marker: MaintenanceMarker = serde_json::from_slice(&bytes)
                .map_err(|e| anyhow::anyhow!("corrupt_maintenance_marker: {}", e))?;
            if marker.tx_id != current_tx_id {
                bail!(
                    "maintenance_marker_tx_mismatch: existing marker belongs to tx '{}', cannot clear by tx '{}'",
                    marker.tx_id,
                    current_tx_id
                );
            }
            true
        }
        None => false,
    };

    let has_raw = match read_limited_private_file(&raw_flag, 128)? {
        Some(bytes) => {
            // 严格 36 字节 ASCII，不接受多余字节或换行
            if bytes.len() != 36 {
                bail!(
                    "maintenance_marker_raw_size_invalid: expected 36 bytes, got {}",
                    bytes.len()
                );
            }
            let raw_str = std::str::from_utf8(&bytes)
                .map_err(|e| anyhow::anyhow!("maintenance_marker_raw_not_utf8: {}", e))?;
            if raw_str != pure_uuid {
                bail!(
                    "maintenance_marker_uuid_mismatch: existing marker has UUID '{}', cannot clear by tx '{}' (uuid '{}')",
                    raw_str,
                    current_tx_id,
                    pure_uuid
                );
            }
            true
        }
        None => false,
    };

    // 2. 只有两项预检全部通过后才执行安全删除
    if has_json {
        fs::remove_file(&marker_json_path)?;
    }
    if has_raw {
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

/// 只读探测旧 legacy 状态根目录是否存在未完成维护标记或损坏状态
/// 严格只读，遇未完成维护或文件损坏返回 true（指示需要抑制或恢复），绝不删除或修改旧文件
pub fn detect_legacy_maintenance_marker(legacy_dir: &Path) -> bool {
    let maint_path = legacy_dir.join("core.maintenance");
    match read_limited_private_file(&maint_path, 128) {
        Ok(Some(_)) => return true,
        Err(_) => return true, // 权限/软链/无法安全打开均保守判定为需维护阻断
        Ok(None) => {}
    }

    let json_path = legacy_dir.join("core.maintenance.json");
    match read_limited_private_file(&json_path, 4096) {
        Ok(Some(_)) => return true,
        Err(_) => return true, // 权限/软链/无法安全打开同样必须判定为需阻断，绝不能吞掉
        Ok(None) => {}
    }

    let journal_path = legacy_dir.join("update-journal.json");
    match read_limited_private_file(&journal_path, 64 * 1024) {
        // The legacy root has no trusted installation mapping. Even Committed
        // cannot establish record/current consistency without explicit recovery.
        Ok(Some(_)) => return true,
        Err(_) => return true,
        Ok(None) => {}
    }

    false
}

/// 纯只读启动双检查 Guard（零写入、零删除、零副作用、防 TOCTOU）
pub fn check_startup_guard_readonly(
    state_dir: &Path,
    releases_dir: Option<&Path>,
    current_link: Option<&Path>,
) -> StartupDualCheckDecision {
    check_startup_guard_readonly_with_root(state_dir, releases_dir, current_link, None, None)
}

/// 纯只读启动双检查 Guard（支持受控 install_root 与 legacy 状态根探测）
pub fn check_startup_guard_readonly_with_root(
    state_dir: &Path,
    releases_dir: Option<&Path>,
    current_link: Option<&Path>,
    install_root: Option<&Path>,
    legacy_dir: Option<&Path>,
) -> StartupDualCheckDecision {
    // 0. 探测旧 legacy 状态根（若已配置且与当前 state_dir 不同，发现未完成维护标记或损坏严格 Fail-Closed，绝不删除）
    if let Some(leg) = legacy_dir {
        if leg != state_dir && detect_legacy_maintenance_marker(leg) {
            return StartupDualCheckDecision::RecoveryRequired {
                reason: "legacy_maintenance_marker_detected".into(),
            };
        }
    }

    // 1. 检查 core.maintenance（安全打开 + fstat 校验）
    let maint_path = state_dir.join("core.maintenance");
    let mut maint_uuid: Option<String> = None;
    match read_limited_private_file(&maint_path, 128) {
        Ok(Some(bytes)) => {
            if bytes.len() != 36 {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!(
                        "core.maintenance_size_must_be_36_bytes, got {}",
                        bytes.len()
                    ),
                };
            }
            let raw_str = match std::str::from_utf8(&bytes) {
                Ok(s) => s,
                Err(e) => {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!("core.maintenance_not_utf8: {}", e),
                    };
                }
            };
            let parsed_uuid = match uuid::Uuid::parse_str(raw_str) {
                Ok(u) => u.to_string(),
                Err(e) => {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!("core.maintenance_content_not_valid_uuid: {}", e),
                    };
                }
            };
            maint_uuid = Some(parsed_uuid);
        }
        Ok(None) => {}
        Err(e) => {
            // EACCES / ELOOP / Symlink 等非 ENOENT 错误严格 Fail-Closed
            return StartupDualCheckDecision::RecoveryRequired {
                reason: format!("cannot_read_core_maintenance_safely: {}", e),
            };
        }
    }

    // 2. 检查 update-journal.json（安全打开 + fstat 校验）
    let journal_path = state_dir.join("update-journal.json");
    match read_limited_private_file(&journal_path, 64 * 1024) {
        Ok(Some(bytes)) => {
            let journal: UpdateJournal = match serde_json::from_slice(&bytes) {
                Ok(j) => j,
                Err(e) => {
                    return StartupDualCheckDecision::RecoveryRequired {
                        reason: format!("update-journal.json_corrupt_or_invalid: {}", e),
                    };
                }
            };

            // 严格校验 journal_schema
            if journal.journal_schema != 1 {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("unsupported_journal_schema: {}", journal.journal_schema),
                };
            }

            // 校验 transaction_id
            if extract_pure_uuid(&journal.transaction_id).is_err() {
                return StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("invalid_journal_tx_id: '{}'", journal.transaction_id),
                };
            }

            match journal.stage {
                TransactionStage::RecoveryRequired => StartupDualCheckDecision::RecoveryRequired {
                    reason: format!("journal_in_recovery_required_stage: {:?}", journal.error),
                },
                TransactionStage::Staged
                | TransactionStage::PublisherVerified
                | TransactionStage::LocalSigned
                | TransactionStage::Draining
                | TransactionStage::BackupVerified
                | TransactionStage::Switching
                | TransactionStage::CandidateTesting
                | TransactionStage::CandidateTestingPassed
                | TransactionStage::RollbackStarted => {
                    StartupDualCheckDecision::MaintenanceSuppressed {
                        tx_id: journal.transaction_id,
                        stage: Some(journal.stage),
                    }
                }
                TransactionStage::RolledBack => {
                    // 本阶段尚无真实 rollback 恢复协议，处于 RolledBack 的日志一律判为 RecoveryRequired，严禁凭猜测放行
                    StartupDualCheckDecision::RecoveryRequired {
                        reason: "rolled_back_stage_requires_recovery_protocol".into(),
                    }
                }
                TransactionStage::Committed => {
                    // 终态全要素二次只读核验：严禁仅凭 Committed 标签放行！
                    let (Some(releases_dir), Some(current_link)) = (releases_dir, current_link)
                    else {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: "unconfigured_production_install_root: formal releases_dir and current_link are required".into(),
                        };
                    };

                    // a. 读取并校验 current_release.json
                    let record_path = state_dir.join("current_release.json");
                    let rec_bytes = match read_limited_private_file(&record_path, 64 * 1024) {
                        Ok(Some(b)) => b,
                        Ok(None) => {
                            return StartupDualCheckDecision::RecoveryRequired {
                                reason: "committed_journal_missing_current_release_record".into(),
                            };
                        }
                        Err(e) => {
                            return StartupDualCheckDecision::RecoveryRequired {
                                reason: format!("cannot_read_current_release_record: {}", e),
                            };
                        }
                    };

                    let commit_record: CurrentReleaseCommitRecord =
                        match serde_json::from_slice(&rec_bytes) {
                            Ok(r) => r,
                            Err(e) => {
                                return StartupDualCheckDecision::RecoveryRequired {
                                    reason: format!("current_release_record_corrupt: {}", e),
                                };
                            }
                        };

                    if commit_record.record_schema != 1 {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: format!(
                                "unsupported_record_schema: {}",
                                commit_record.record_schema
                            ),
                        };
                    }

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

                    // 必需 digest 格式与全量匹配校验（严禁跳过）
                    let Some(ref j_digest) = journal.manifest_digest else {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: "committed_journal_missing_manifest_digest".into(),
                        };
                    };
                    if j_digest.len() != 64 || !j_digest.chars().all(|c| c.is_ascii_hexdigit()) {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: format!("invalid_manifest_digest_format: '{}'", j_digest),
                        };
                    }
                    if commit_record.manifest_digest != *j_digest {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: "commit_record_digest_mismatch".into(),
                        };
                    }

                    // b. 核查 current 软链接物理目标（拒绝外部目录与悬空）
                    if let Err(e) = verify_current_link_target_with_root(
                        current_link,
                        releases_dir,
                        &journal.target_release_id,
                        install_root,
                    ) {
                        return StartupDualCheckDecision::RecoveryRequired {
                            reason: format!("current_link_verification_failed: {}", e),
                        };
                    }

                    if let Some(uid) = maint_uuid {
                        StartupDualCheckDecision::MaintenanceSuppressed {
                            tx_id: uid,
                            stage: Some(TransactionStage::Committed),
                        }
                    } else {
                        StartupDualCheckDecision::AllowStartup
                    }
                }
            }
        }
        Ok(None) => {
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
        Err(e) => {
            // EACCES / ELOOP 等非 ENOENT 错误严格 Fail-Closed
            StartupDualCheckDecision::RecoveryRequired {
                reason: format!("cannot_read_update_journal_safely: {}", e),
            }
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
