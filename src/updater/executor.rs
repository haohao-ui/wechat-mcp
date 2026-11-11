use anyhow::{Result, bail};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::journal::{
    CurrentReleaseCommitRecord, TransactionStage, UpdateJournal, acquire_update_lock,
    atomic_commit_release_record, clear_maintenance_for_tx, load_highest_accepted_sequence,
    set_structured_maintenance_marker, sync_dir,
};
use super::manifest::{
    ReleaseManifest, SignedEnvelope, TrustedKeyStore, validate_manifest_compatibility,
};
use super::rollback::{rollback_release_atomic, switch_release_atomic};
use super::signing::LocalSigner;
use super::unpack::safe_unpack_and_verify;

pub const SANDBOX_MARKER_FILE: &str = ".sandbox_update_marker";

/// 仅能通过公钥验签与兼容性核验生成的强类型发布封套，内部字段私有不可篡改
#[derive(Debug, Clone)]
pub struct VerifiedManifestEnvelope {
    envelope: SignedEnvelope,
}

impl VerifiedManifestEnvelope {
    pub fn verify_and_construct(
        envelope_bytes: &[u8],
        store: &TrustedKeyStore,
        host_arch: &str,
        host_os_ver: &str,
        highest_accepted_seq: u64,
    ) -> Result<Self> {
        let envelope = super::manifest::parse_signed_envelope(envelope_bytes)?;
        store.verify_envelope(&envelope)?;
        validate_manifest_compatibility(
            &envelope.manifest,
            host_arch,
            host_os_ver,
            highest_accepted_seq,
        )?;
        Ok(Self { envelope })
    }

    pub fn manifest(&self) -> &ReleaseManifest {
        &self.envelope.manifest
    }

    pub fn envelope(&self) -> &SignedEnvelope {
        &self.envelope
    }
}

/// 隔离沙箱更新根目录：私有字段，禁止接受任意 Path 构造或生产调用
#[derive(Debug)]
pub struct SandboxUpdateRoot {
    root_dir: PathBuf,
    root_fd: std::os::unix::io::RawFd,
}

impl Drop for SandboxUpdateRoot {
    fn drop(&mut self) {
        if self.root_fd >= 0 {
            unsafe {
                libc::close(self.root_fd);
            }
        }
        let _ = fs::remove_dir_all(&self.root_dir);
    }
}

impl SandboxUpdateRoot {
    #[cfg(test)]
    pub fn create_test_isolated_sandbox() -> Result<Self> {
        use std::ffi::CString;
        use std::os::unix::fs::MetadataExt;
        let base_temp = std::env::temp_dir();
        let unique_dir = base_temp.join(format!(
            "wechat_mcp_sandbox_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));

        // 原子创建 0700 专用测试根
        fs::create_dir_all(&unique_dir)?;
        fs::set_permissions(&unique_dir, fs::Permissions::from_mode(0o700))?;

        // 校验 ownership 与无 symlink
        let meta = fs::symlink_metadata(&unique_dir)?;
        if meta.file_type().is_symlink() {
            bail!("sandbox_root_must_not_be_symlink");
        }
        let current_uid = unsafe { libc::getuid() };
        if meta.uid() != current_uid {
            bail!("sandbox_root_uid_mismatch");
        }

        // 打开该目录持有持续根 FD (O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        let c_path = CString::new(unique_dir.to_str().unwrap())?;
        let fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let err = std::io::Error::last_os_error();
            bail!("failed_to_open_sandbox_root_fd: {}", err);
        }

        // 初始化必要子目录
        let releases_dir = unique_dir.join("releases");
        let state_dir = unique_dir.join("state");
        fs::create_dir_all(&releases_dir)?;
        fs::create_dir_all(&state_dir)?;
        fs::set_permissions(&releases_dir, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700))?;

        Ok(Self {
            root_dir: unique_dir,
            root_fd: fd,
        })
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn root_fd(&self) -> std::os::unix::io::RawFd {
        self.root_fd
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root_dir.join("state")
    }

    pub fn releases_dir(&self) -> PathBuf {
        self.root_dir.join("releases")
    }

    pub fn current_link(&self) -> PathBuf {
        self.root_dir.join("current")
    }

    pub fn previous_link(&self) -> PathBuf {
        self.root_dir.join("previous")
    }

    pub fn db_path(&self) -> PathBuf {
        self.root_dir.join("state").join("wechat.db")
    }
}

/// 计算规范化 Manifest 的 SHA-256 摘要（使用 RFC 8785 规范化 JSON，绝不能错标为 artifact.sha256）
pub fn calculate_manifest_digest(manifest: &ReleaseManifest) -> Result<String> {
    let manifest_val = serde_json::to_value(manifest)?;
    let canonical_bytes = serde_json_canonicalizer::to_vec(&manifest_val)?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&canonical_bytes);
    let hash_bytes = hasher.finalize();
    let mut hex_str = String::with_capacity(64);
    for b in hash_bytes {
        use std::fmt::Write;
        let _ = write!(hex_str, "{:02x}", b);
    }
    Ok(hex_str)
}

/// 沙箱排空适配器证明
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerifiedSandboxDrainProof {
    pub drained_at: u64,
    pub tx_id: String,
}

pub trait SandboxDrainAdapter: Send + Sync {
    fn execute_drain(&self, tx_id: &str, now: u64) -> Result<VerifiedSandboxDrainProof>;
}

/// 沙箱备份适配器证明
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VerifiedSandboxBackupProof {
    NoDatabaseVerified,
    DatabaseBackupVerified {
        backup_path: PathBuf,
        backup_sha256: String,
    },
}

pub trait SandboxBackupAdapter: Send + Sync {
    fn execute_backup(&self, tx_id: &str, state_dir: &Path) -> Result<VerifiedSandboxBackupProof>;
}

/// 沙箱候选自检适配器证明
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerifiedSandboxCandidateProof {
    pub tested_at: u64,
    pub exit_code: i32,
}

pub trait SandboxCandidateAdapter: Send + Sync {
    fn execute_candidate_test(&self, candidate_dir: &Path)
    -> Result<VerifiedSandboxCandidateProof>;
}

/// 沙箱排空测试 Fixture 适配器
pub struct SandboxFixtureDrainAdapter;
impl SandboxDrainAdapter for SandboxFixtureDrainAdapter {
    fn execute_drain(&self, tx_id: &str, now: u64) -> Result<VerifiedSandboxDrainProof> {
        Ok(VerifiedSandboxDrainProof {
            drained_at: now,
            tx_id: tx_id.to_string(),
        })
    }
}

/// 沙箱备份测试 Fixture 适配器（支持显式 NoDatabase 声明校验与真实 DB 在线备份+restore 恢复核验）
pub struct SandboxFixtureBackupAdapter {
    pub expected_no_database: bool,
    pub db_path: Option<PathBuf>,
}

impl SandboxFixtureBackupAdapter {
    pub fn new_no_database() -> Self {
        Self {
            expected_no_database: true,
            db_path: None,
        }
    }

    pub fn new_with_db(db_path: impl Into<PathBuf>) -> Self {
        Self {
            expected_no_database: false,
            db_path: Some(db_path.into()),
        }
    }
}

impl SandboxBackupAdapter for SandboxFixtureBackupAdapter {
    fn execute_backup(&self, tx_id: &str, state_dir: &Path) -> Result<VerifiedSandboxBackupProof> {
        if self.expected_no_database {
            // 校验 state 目录中确实无任何数据库文件或 WAL 遗留
            if state_dir.exists() {
                for entry in fs::read_dir(state_dir)? {
                    let p = entry?.path();
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    if name.ends_with(".db")
                        || name.ends_with(".db-wal")
                        || name.ends_with(".db-shm")
                        || name == "wechat.db"
                    {
                        bail!(
                            "database_file_detected_when_expected_no_database: {}",
                            p.display()
                        );
                    }
                }
            }
            return Ok(VerifiedSandboxBackupProof::NoDatabaseVerified);
        }

        let db_p = self
            .db_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("db_path_not_configured_for_backup"))?;
        if !db_p.exists() {
            bail!("source_db_not_found: {}", db_p.display());
        }

        let backup_path = state_dir.join(format!("backup_{}.db", tx_id));
        let sha256 = super::rollback::backup_and_verify_sqlite_db(db_p, &backup_path)?;

        // 真实 restore 核验：打开备份数据库执行 PRAGMA integrity_check 并确认元数据与业务表结构可读
        super::rollback::verify_backup_restorable(&backup_path)?;

        Ok(VerifiedSandboxBackupProof::DatabaseBackupVerified {
            backup_path,
            backup_sha256: sha256,
        })
    }
}

/// 受信沙箱候选自检测试 Fixture 适配器（硬编码固定绝对路径、固定参数、超时防挂死、输出洪泛防护、kill+wait 回收自有进程）
pub struct SandboxFixtureCandidateAdapter {
    pub executable_path: PathBuf,
    pub fixed_args: Vec<String>,
    pub timeout: std::time::Duration,
    pub max_output_bytes: usize,
}

impl SandboxFixtureCandidateAdapter {
    pub fn new(executable_path: impl Into<PathBuf>, fixed_args: Vec<String>) -> Self {
        Self {
            executable_path: executable_path.into(),
            fixed_args,
            timeout: std::time::Duration::from_secs(5),
            max_output_bytes: 64 * 1024,
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, max: usize) -> Self {
        self.max_output_bytes = max;
        self
    }
}

impl SandboxCandidateAdapter for SandboxFixtureCandidateAdapter {
    fn execute_candidate_test(
        &self,
        candidate_dir: &Path,
    ) -> Result<VerifiedSandboxCandidateProof> {
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::thread;
        use std::time::Instant;

        if !self.executable_path.is_absolute() {
            bail!(
                "candidate_adapter_executable_must_be_absolute: '{}'",
                self.executable_path.display()
            );
        }
        let exe_str = self.executable_path.to_string_lossy();
        if exe_str.contains("..") {
            bail!(
                "candidate_adapter_executable_contains_traversal: '{}'",
                exe_str
            );
        }
        if !self.executable_path.exists() {
            bail!(
                "candidate_adapter_executable_missing: '{}'",
                self.executable_path.display()
            );
        }

        let mut child = Command::new(&self.executable_path)
            .args(&self.fixed_args)
            .env("CANDIDATE_RELEASE_DIR", candidate_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed_to_spawn_candidate_test: {}", e))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let max_bytes = self.max_output_bytes;

        let stdout_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stream) = stdout {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = stream.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if buf.len() < max_bytes {
                        let to_take = n.min(max_bytes - buf.len());
                        buf.extend_from_slice(&chunk[..to_take]);
                    }
                    // 超过 max_bytes 后继续读空丢弃，防止对端写入触发 SIGPIPE
                }
            }
            buf
        });

        let stderr_handle = thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stream) = stderr {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = stream.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if buf.len() < max_bytes {
                        let to_take = n.min(max_bytes - buf.len());
                        buf.extend_from_slice(&chunk[..to_take]);
                    }
                }
            }
            buf
        });

        let start = Instant::now();
        let mut exit_status = None;
        while start.elapsed() < self.timeout {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit_status = Some(status);
                    break;
                }
                Ok(None) => {
                    thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("candidate_test_wait_error: {}", e);
                }
            }
        }

        let status = match exit_status {
            Some(st) => st,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                bail!("candidate_test_timeout_exceeded: limit {:?}", self.timeout);
            }
        };

        let _ = stdout_handle.join();
        let _ = stderr_handle.join();

        let code = status.code().unwrap_or(-1);
        if code != 0 {
            bail!("candidate_test_failed_exit_code: {}", code);
        }

        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(VerifiedSandboxCandidateProof {
            tested_at: now_ts,
            exit_code: 0,
        })
    }
}

/// 故障注入配置（用于验证状态机回滚与断电恢复逻辑）
#[derive(Debug, Clone, Default)]
pub struct FaultInjection {
    pub fail_after_stage: Option<TransactionStage>,
    pub fail_candidate_testing: bool,
    pub fail_backup: bool,
    pub fail_record_commit: bool,
    pub fail_after_record_commit: bool,
}

/// 隔离沙箱本地更新执行器
pub struct SandboxUpdateExecutor<'a> {
    pub sandbox: &'a SandboxUpdateRoot,
    pub fault: FaultInjection,
    pub drain_adapter: Option<std::sync::Arc<dyn SandboxDrainAdapter>>,
    pub backup_adapter: Option<std::sync::Arc<dyn SandboxBackupAdapter>>,
    pub candidate_adapter: Option<std::sync::Arc<dyn SandboxCandidateAdapter>>,
}

/// 严格发布标识校验：字母、数字、点、下划线、短横线，长度 1..=64，严禁穿越符或分隔符
pub fn is_valid_release_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

impl<'a> SandboxUpdateExecutor<'a> {
    pub fn new(sandbox: &'a SandboxUpdateRoot) -> Self {
        Self {
            sandbox,
            fault: FaultInjection::default(),
            drain_adapter: None,
            backup_adapter: None,
            candidate_adapter: None,
        }
    }

    pub fn with_fault(mut self, fault: FaultInjection) -> Self {
        self.fault = fault;
        self
    }

    pub fn with_drain_adapter(mut self, adapter: std::sync::Arc<dyn SandboxDrainAdapter>) -> Self {
        self.drain_adapter = Some(adapter);
        self
    }

    pub fn with_backup_adapter(
        mut self,
        adapter: std::sync::Arc<dyn SandboxBackupAdapter>,
    ) -> Self {
        self.backup_adapter = Some(adapter);
        self
    }

    pub fn with_candidate_adapter(
        mut self,
        adapter: std::sync::Arc<dyn SandboxCandidateAdapter>,
    ) -> Self {
        self.candidate_adapter = Some(adapter);
        self
    }

    /// 执行完整本地更新事务
    pub fn execute_transaction(
        &self,
        verified: &VerifiedManifestEnvelope,
        archive_path: &Path,
        signer: &LocalSigner,
        now: u64,
    ) -> Result<UpdateJournal> {
        let manifest = verified.manifest();
        let state_dir = self.sandbox.state_dir();
        let releases_dir = self.sandbox.releases_dir();
        let current_link = self.sandbox.current_link();

        // 0. 前置能力预检：三类 adapter 缺失任何必要能力必须在产生写副作用前返回 unsupported，严禁产生写副作用
        let drain_adapter = self.drain_adapter.as_ref().ok_or_else(|| {
            anyhow::anyhow!("unsupported_update_execution: missing required drain_adapter")
        })?;
        let backup_adapter = self.backup_adapter.as_ref().ok_or_else(|| {
            anyhow::anyhow!("unsupported_update_execution: missing required backup_adapter")
        })?;
        let candidate_adapter = self.candidate_adapter.as_ref().ok_or_else(|| {
            anyhow::anyhow!("unsupported_update_execution: missing required candidate_adapter")
        })?;

        // 校验 release_id 命名合法性
        if !is_valid_release_id(&manifest.release_id) {
            bail!("invalid_release_id: '{}'", manifest.release_id);
        }

        // 1. 全程持有排他更新锁
        let _lock = acquire_update_lock(&state_dir)?;

        // 2. 检查当前最高水位
        let highest_seq = load_highest_accepted_sequence(&state_dir)?;
        if manifest.sequence <= highest_seq {
            bail!(
                "downgrade_rejected: candidate sequence {} <= highest accepted {}",
                manifest.sequence,
                highest_seq
            );
        }

        // 识别 source_release_id（严格校验，绝不把未知 current 当旧版安全）
        let source_release_id =
            if current_link.exists() || fs::symlink_metadata(&current_link).is_ok() {
                let link_target = fs::read_link(&current_link)
                    .map_err(|e| anyhow::anyhow!("cannot_read_current_symlink: {}", e))?;
                let target_leaf = link_target
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "invalid_current_symlink_target: '{}'",
                            link_target.display()
                        )
                    })?;

                if !is_valid_release_id(target_leaf) {
                    bail!("current_symlink_target_name_invalid: '{}'", target_leaf);
                }
                let target_dir = releases_dir.join(target_leaf);
                if !target_dir.is_dir() {
                    bail!(
                        "current_symlink_target_not_found_or_not_dir: '{}'",
                        target_dir.display()
                    );
                }
                target_leaf.to_string()
            } else {
                "none".to_string()
            };

        // 检查 releases/<release_id> 是否已存在，严禁覆盖或删除已有版本目录
        let target_release_dir = releases_dir.join(&manifest.release_id);
        if target_release_dir.exists() {
            bail!(
                "release_directory_already_exists: '{}'",
                target_release_dir.display()
            );
        }

        let tx_id = format!("tx-{}", uuid::Uuid::new_v4());
        let mut journal = UpdateJournal::new(
            &tx_id,
            &source_release_id,
            &manifest.release_id,
            manifest.sequence,
            now,
        );

        // 使用隔离临时暂存目录 staging/<tx_id>，全部校验和重签名完成前不进入 releases/
        let staging_root = self.sandbox.root_dir.join("staging");
        let staging_dir = staging_root.join(&tx_id);
        fs::create_dir_all(&staging_dir)?;
        fs::set_permissions(&staging_dir, fs::Permissions::from_mode(0o700))?;

        // 阶段 1: Staged (解压与白名单/摘要逐文件校验至受限 staging 目录)
        journal.stage = TransactionStage::Staged;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::Staged) {
            bail!("injected_fault_after_staged");
        }

        safe_unpack_and_verify(
            archive_path,
            &staging_dir,
            &manifest.artifact,
            &manifest.components,
        )?;

        // 阶段 2: PublisherVerified
        journal.stage = TransactionStage::PublisherVerified;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::PublisherVerified) {
            bail!("injected_fault_after_publisher_verified");
        }

        // 阶段 3: LocalSigned (本机代码重签名在 staging 中完成)
        for comp in &manifest.components {
            if comp.kind == "executable" {
                let bin_path = staging_dir.join(&comp.path);
                signer.sign_and_verify(&bin_path)?;
            }
        }
        journal.stage = TransactionStage::LocalSigned;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::LocalSigned) {
            bail!("injected_fault_after_local_signed");
        }

        // 解压与校验签名均已在受限 staging 目录中完成，原子发布到 releases/<release_id>
        fs::rename(&staging_dir, &target_release_dir)?;
        sync_dir(&releases_dir)?;

        // 阶段 4: Draining (仅通过沙箱排空适配器产生证据后推进)
        set_structured_maintenance_marker(&state_dir, &tx_id, "updater draining", now)?;
        let drain_proof = drain_adapter.execute_drain(&tx_id, now)?;
        journal.stage = TransactionStage::Draining;
        journal.drain_proof = Some(serde_json::to_string(&drain_proof)?);
        journal.maintenance_marker_confirmed = true;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::Draining) {
            bail!("injected_fault_after_draining");
        }

        // 阶段 5: BackupVerified (仅通过沙箱备份适配器产生证据并核验 restore 后推进)
        if self.fault.fail_backup {
            bail!("injected_fault_in_backup");
        }
        let backup_proof = backup_adapter.execute_backup(&tx_id, &state_dir)?;
        match &backup_proof {
            VerifiedSandboxBackupProof::DatabaseBackupVerified { backup_path, .. } => {
                journal.backup_path = Some(backup_path.to_string_lossy().to_string());
            }
            VerifiedSandboxBackupProof::NoDatabaseVerified => {
                journal.backup_path = None;
            }
        }
        journal.stage = TransactionStage::BackupVerified;
        journal.backup_proof = Some(serde_json::to_string(&backup_proof)?);
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::BackupVerified) {
            bail!("injected_fault_after_backup_verified");
        }

        // 阶段 6: Switching (原子切换 current 指针)
        journal.stage = TransactionStage::Switching;
        journal.persist_atomic(&state_dir)?;
        switch_release_atomic(&self.sandbox.root_dir, &manifest.release_id)?;
        if self.fault.fail_after_stage == Some(TransactionStage::Switching) {
            bail!("injected_fault_after_switching");
        }

        // 阶段 7: CandidateTesting (执行受信适配器自检并收集 Proof)
        journal.stage = TransactionStage::CandidateTesting;
        journal.persist_atomic(&state_dir)?;

        let candidate_test_result = if self.fault.fail_candidate_testing {
            Err(anyhow::anyhow!("injected_fault_candidate_testing_failed"))
        } else {
            candidate_adapter.execute_candidate_test(&target_release_dir)
        };

        match candidate_test_result {
            Ok(candidate_proof) => {
                let manifest_digest = calculate_manifest_digest(manifest)?;

                // 阶段 7.1: CandidateTestingPassed (自检通过，已持有 proof，但尚未提交权威 record)
                journal.stage = TransactionStage::CandidateTestingPassed;
                journal.candidate_proof = Some(serde_json::to_string(&candidate_proof)?);
                journal.manifest_digest = Some(manifest_digest.clone());
                journal.persist_atomic(&state_dir)?;

                if self.fault.fail_after_stage == Some(TransactionStage::CandidateTestingPassed) {
                    bail!("injected_fault_after_candidate_testing_passed");
                }

                // 准备单一权威提交记录
                let commit_record = CurrentReleaseCommitRecord {
                    record_schema: 1,
                    tx_id: tx_id.clone(),
                    release_id: manifest.release_id.clone(),
                    sequence: manifest.sequence,
                    channel: manifest.channel.clone(),
                    manifest_digest,
                    installed_at: now,
                };

                // 模拟 record 写/fsync 失败故障
                if self.fault.fail_record_commit {
                    bail!("injected_fault_record_commit_failed");
                }

                // 执行 record 原子落盘 (写 tmp -> fsync -> rename -> sync_dir)
                atomic_commit_release_record(&state_dir, &commit_record)?;

                // 模拟 record 成功但写入 journal Committed 前崩溃的故障
                if self.fault.fail_after_record_commit {
                    bail!("injected_fault_after_record_commit_before_journal_committed");
                }

                // 阶段 8: Committed (权威记录成功原子生效后，journal 才能推进到 Committed)
                journal.stage = TransactionStage::Committed;
                journal.persist_atomic(&state_dir)?;

                // 清除本事务的维护态标志
                clear_maintenance_for_tx(&state_dir, &tx_id)?;

                Ok(journal)
            }
            Err(test_err) => {
                // 自检失败 -> 触发回滚
                journal.stage = TransactionStage::RollbackStarted;
                journal.error = Some(format!("candidate_test_failed: {}", test_err));
                journal.persist_atomic(&state_dir)?;

                // 业务数据安全准则：当受控存在数据库时，首版默认 UnknownDataState 保持维护，严禁自动回滚覆盖数据库
                match backup_proof {
                    VerifiedSandboxBackupProof::DatabaseBackupVerified { .. } => {
                        journal.stage = TransactionStage::RecoveryRequired;
                        journal.error = Some(
                            "database_state_unknown_cannot_auto_rollback_requires_manual_recovery"
                                .into(),
                        );
                        journal.persist_atomic(&state_dir)?;
                        set_structured_maintenance_marker(
                            &state_dir,
                            &tx_id,
                            "manual recovery required: database state unknown",
                            now,
                        )?;
                        Ok(journal)
                    }
                    VerifiedSandboxBackupProof::NoDatabaseVerified => {
                        // 确认无业务数据库：安全回退程序指针
                        rollback_release_atomic(&self.sandbox.root_dir)?;
                        journal.stage = TransactionStage::RolledBack;
                        journal.persist_atomic(&state_dir)?;
                        clear_maintenance_for_tx(&state_dir, &tx_id)?;
                        Ok(journal)
                    }
                }
            }
        }
    }
}
