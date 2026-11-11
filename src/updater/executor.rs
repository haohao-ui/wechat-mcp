use anyhow::{Result, bail};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::journal::{
    CurrentReleaseMetadata, TransactionStage, UpdateJournal, acquire_update_lock,
    clear_maintenance_if_safe, load_highest_accepted_sequence, save_current_release_metadata,
    save_highest_accepted_sequence, set_maintenance_active,
};
use super::manifest::{ReleaseManifest, SignedEnvelope, TrustedKeyStore, validate_manifest_compatibility};
use super::rollback::{rollback_release_atomic, switch_release_atomic};
use super::signing::LocalSigner;
use super::unpack::safe_unpack_and_verify;

pub const SANDBOX_MARKER_FILE: &str = ".sandbox_update_marker";

/// 仅能通过公钥验签与兼容性核验生成的强类型发布封套，防止未经审核的 Manifest 传入写执行器
#[derive(Debug, Clone)]
pub struct VerifiedManifestEnvelope {
    pub envelope: SignedEnvelope,
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
}

/// 隔离沙箱更新根目录，必须显式携带安全标记文件，禁止指向系统或生产目录
#[derive(Debug, Clone)]
pub struct SandboxUpdateRoot {
    pub root_dir: PathBuf,
}

impl SandboxUpdateRoot {
    pub fn new_isolated(root_dir: impl Into<PathBuf>) -> Result<Self> {
        let root = root_dir.into();
        Self::validate_not_production_or_system(&root)?;

        if !root.exists() {
            fs::create_dir_all(&root)?;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }

        let marker = root.join(SANDBOX_MARKER_FILE);
        if !marker.exists() {
            fs::write(&marker, b"wechat-mcp-isolated-test-sandbox")?;
            fs::set_permissions(&marker, fs::Permissions::from_mode(0o600))?;
        }

        // 初始化必要子目录
        let releases_dir = root.join("releases");
        let state_dir = root.join("state");
        fs::create_dir_all(&releases_dir)?;
        fs::create_dir_all(&state_dir)?;
        fs::set_permissions(&releases_dir, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&state_dir, fs::Permissions::from_mode(0o700))?;

        Ok(Self { root_dir: root })
    }

    fn validate_not_production_or_system(path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        if path_str == "/"
            || path_str.starts_with("/System")
            || path_str.starts_with("/Library")
            || path_str.starts_with("/Applications")
            || path_str.starts_with("/usr")
            || path_str.starts_with("/bin")
            || path_str.starts_with("/sbin")
            || path_str.contains(".wechat-rpa")
            || path_str.contains(".wechat-mcp")
        {
            bail!(
                "prohibited_sandbox_path: '{}' is a production or system directory",
                path.display()
            );
        }
        Ok(())
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

        // 识别 source_release_id
        let source_release_id = if current_link.exists() || fs::symlink_metadata(&current_link).is_ok() {
            fs::read_link(&current_link)
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                .unwrap_or_else(|| "0.0.0-unknown".into())
        } else {
            "none".to_string()
        };

        let tx_id = format!("tx-{}", uuid::Uuid::new_v4());
        let mut journal = UpdateJournal::new(
            &tx_id,
            &source_release_id,
            &manifest.release_id,
            manifest.sequence,
            now,
        );

        // 3. 准备解压目标目录 releases/<release_id>
        let target_release_dir = releases_dir.join(&manifest.release_id);
        if target_release_dir.exists() {
            // 已存在同版本目录，清理旧暂存
            fs::remove_dir_all(&target_release_dir)?;
        }
        fs::create_dir_all(&target_release_dir)?;
        fs::set_permissions(&target_release_dir, fs::Permissions::from_mode(0o700))?;

        // 阶段 1: Staged (解压与白名单/摘要逐文件校验)
        journal.stage = TransactionStage::Staged;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::Staged) {
            bail!("injected_fault_after_staged");
        }

        safe_unpack_and_verify(
            archive_path,
            &target_release_dir,
            &manifest.artifact,
            &manifest.components,
        )?;

        // 阶段 2: PublisherVerified
        journal.stage = TransactionStage::PublisherVerified;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::PublisherVerified) {
            bail!("injected_fault_after_publisher_verified");
        }

        // 阶段 3: LocalSigned (本机代码重签名)
        for comp in &manifest.components {
            if comp.kind == "executable" {
                let bin_path = target_release_dir.join(&comp.path);
                signer.sign_and_verify(&bin_path)?;
            }
        }
        journal.stage = TransactionStage::LocalSigned;
        journal.persist_atomic(&state_dir)?;
        if self.fault.fail_after_stage == Some(TransactionStage::LocalSigned) {
            bail!("injected_fault_after_local_signed");
        }

        // 阶段 4: Draining (模拟排空)
        set_maintenance_active(&state_dir, "updater draining")?;
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

            // 提升最高水位与持久化当前 Release 元数据
            save_highest_accepted_sequence(&state_dir, manifest.sequence)?;
            let meta = CurrentReleaseMetadata {
                release_id: manifest.release_id.clone(),
                sequence: manifest.sequence,
                channel: manifest.channel.clone(),
                components_consistent: true,
                installed_at: now,
            };
            save_current_release_metadata(&state_dir, &meta)?;

            // 清除维护态标志
            clear_maintenance_if_safe(&state_dir)?;

            Ok(journal)
        } else {
            // 自检失败 -> 触发回滚
            journal.stage = TransactionStage::RollbackStarted;
            journal.error = Some("candidate_self_test_failed".into());
            journal.persist_atomic(&state_dir)?;

            if self.fault.simulate_new_business_data_on_rollback {
                // 模拟已产生新业务数据，严禁静默回退，必须保留维护态
                journal.stage = TransactionStage::RecoveryRequired;
                journal.error = Some("new_business_data_detected_cannot_auto_rollback".into());
                journal.persist_atomic(&state_dir)?;
                set_maintenance_active(&state_dir, "manual recovery required: new business data conflict")?;
                return Ok(journal);
            }

            rollback_release_atomic(&self.sandbox.root_dir)?;
            journal.stage = TransactionStage::RolledBack;
            journal.persist_atomic(&state_dir)?;

            // 回退成功，安全清除维护标志
            clear_maintenance_if_safe(&state_dir)?;
            Ok(journal)
        }
    }
}
