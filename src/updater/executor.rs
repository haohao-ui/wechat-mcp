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
}

/// 故障注入配置（用于验证状态机回滚与断电恢复逻辑）
#[derive(Debug, Clone, Default)]
pub struct FaultInjection {
    pub fail_after_stage: Option<TransactionStage>,
    pub fail_candidate_testing: bool,
    pub simulate_new_business_data_on_rollback: bool,
}

/// 隔离沙箱本地更新执行器
pub struct SandboxUpdateExecutor<'a> {
    pub sandbox: &'a SandboxUpdateRoot,
    pub fault: FaultInjection,
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
        }
    }

    pub fn with_fault(mut self, fault: FaultInjection) -> Self {
        self.fault = fault;
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

        // 阶段 4: Draining (模拟排空，设置带 tx_id 绑定的结构化维护标记)
        set_structured_maintenance_marker(&state_dir, &tx_id, "updater draining", now)?;
        journal.stage = TransactionStage::Draining;
        journal.maintenance_marker_confirmed = true;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::Draining) {
            bail!("injected_fault_after_draining");
        }

        // 阶段 5: BackupVerified
        journal.stage = TransactionStage::BackupVerified;
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

        // 阶段 7: CandidateTesting
        journal.stage = TransactionStage::CandidateTesting;
        journal.persist_atomic(&state_dir)?;

        let testing_passed = !self.fault.fail_candidate_testing;
        if testing_passed {
            // 自检通过 -> Committed
            journal.stage = TransactionStage::Committed;
            journal.persist_atomic(&state_dir)?;

            // 单一权威提交记录：原子写入 current_release.json（含最高水位与 manifest 摘要）
            let commit_record = CurrentReleaseCommitRecord {
                record_schema: 1,
                tx_id: tx_id.clone(),
                release_id: manifest.release_id.clone(),
                sequence: manifest.sequence,
                channel: manifest.channel.clone(),
                manifest_digest: manifest.artifact.sha256.clone(),
                installed_at: now,
            };
            atomic_commit_release_record(&state_dir, &commit_record)?;

            // 清除本事务的维护态标志
            clear_maintenance_for_tx(&state_dir, &tx_id)?;

            Ok(journal)
        } else {
            // 自检失败 -> 触发回滚
            journal.stage = TransactionStage::RollbackStarted;
            journal.error = Some("candidate_self_test_failed".into());
            journal.persist_atomic(&state_dir)?;

            if self.fault.simulate_new_business_data_on_rollback {
                // 模拟已产生新业务数据，严禁静默回退，必须保留维护态并标记 RecoveryRequired
                journal.stage = TransactionStage::RecoveryRequired;
                journal.error = Some("new_business_data_detected_cannot_auto_rollback".into());
                journal.persist_atomic(&state_dir)?;
                set_structured_maintenance_marker(
                    &state_dir,
                    &tx_id,
                    "manual recovery required: new business data conflict",
                    now,
                )?;
                return Ok(journal);
            }

            rollback_release_atomic(&self.sandbox.root_dir)?;
            journal.stage = TransactionStage::RolledBack;
            journal.persist_atomic(&state_dir)?;

            // 回退成功，安全清除本事务的维护标志
            clear_maintenance_for_tx(&state_dir, &tx_id)?;
            Ok(journal)
        }
    }
}
