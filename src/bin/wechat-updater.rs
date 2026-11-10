use std::fs;
use std::path::{Path, PathBuf};
use wechat_mcp::core::backup;
use wechat_mcp::core::config;
use wechat_mcp::updater::journal::{self, RecoveryAction, TransactionStage, UpdateJournal};
use wechat_mcp::updater::manifest::{self, SignedEnvelope};
use wechat_mcp::updater::rollback::{
    has_new_data_since_backup, rollback_release_atomic, switch_release_atomic,
};
use wechat_mcp::updater::signing::LocalSigner;
use wechat_mcp::updater::unpack::safe_unpack_and_verify;

fn print_usage() {
    eprintln!(
        "Usage: wechat-updater <status|check|apply|rollback|recover> [options]\n\
         \n\
         Commands:\n\
           status    --config <path>\n\
           check     --config <path> --manifest <path>\n\
           apply     --config <path> --manifest <path> --archive <path> [--identity <name>] [--dry-run]\n\
           rollback  --app-root <path>\n\
           recover   --config <path>\n"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(2);
    }

    match args[1].as_str() {
        "--version" | "-v" => {
            println!("wechat-updater 0.1.0");
            std::process::exit(0);
        }
        "status" => {
            let mut config_path: Option<PathBuf> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let Some(cfg_p) = config_path else {
                eprintln!("error: status requires --config <path>");
                std::process::exit(2);
            };
            let cfg = match config::BotConfig::load(&cfg_p) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error loading config: {e}");
                    std::process::exit(2);
                }
            };
            let journal = UpdateJournal::load_from_state(&cfg.state_dir)
                .ok()
                .flatten();
            let highest_seq = journal::load_highest_accepted_sequence(&cfg.state_dir);
            let maintenance = cfg.state_dir.join("core.maintenance").exists();
            let out = serde_json::json!({
                "highest_accepted_sequence": highest_seq,
                "maintenance_active": maintenance,
                "journal": journal,
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            std::process::exit(0);
        }
        "check" => {
            let mut config_path: Option<PathBuf> = None;
            let mut manifest_path: Option<PathBuf> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--manifest" && i + 1 < args.len() {
                    manifest_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let (Some(cfg_p), Some(m_p)) = (config_path, manifest_path) else {
                eprintln!("error: check requires --config <path> and --manifest <path>");
                std::process::exit(2);
            };
            let cfg = match config::BotConfig::load(&cfg_p) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error loading config: {e}");
                    std::process::exit(2);
                }
            };
            let manifest_bytes = match fs::read(&m_p) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error reading manifest: {e}");
                    std::process::exit(1);
                }
            };
            let envelope: SignedEnvelope = match serde_json::from_slice(&manifest_bytes) {
                Ok(env) => env,
                Err(e) => {
                    eprintln!("error parsing signed envelope: {e}");
                    std::process::exit(1);
                }
            };
            let highest_seq = journal::load_highest_accepted_sequence(&cfg.state_dir);
            match manifest::validate_manifest_compatibility(
                &envelope.manifest,
                std::env::consts::ARCH,
                "12.0",
                highest_seq,
            ) {
                Ok(()) => {
                    println!("manifest is valid and compatible");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("compatibility check failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "apply" => {
            let mut config_path: Option<PathBuf> = None;
            let mut manifest_path: Option<PathBuf> = None;
            let mut archive_path: Option<PathBuf> = None;
            let mut identity: Option<String> = None;
            let mut dry_run = false;

            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--manifest" && i + 1 < args.len() {
                    manifest_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--archive" && i + 1 < args.len() {
                    archive_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--identity" && i + 1 < args.len() {
                    identity = Some(args[i + 1].clone());
                    i += 2;
                } else if args[i] == "--dry-run" {
                    dry_run = true;
                    i += 1;
                } else {
                    i += 1;
                }
            }

            let (Some(cfg_p), Some(m_p), Some(a_p)) = (config_path, manifest_path, archive_path)
            else {
                eprintln!("error: apply requires --config, --manifest, and --archive");
                std::process::exit(2);
            };

            let cfg = match config::BotConfig::load(&cfg_p) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error loading config: {e}");
                    std::process::exit(2);
                }
            };

            let app_root = cfg
                .state_dir
                .parent()
                .unwrap_or(&cfg.state_dir)
                .to_path_buf();
            let code =
                run_apply_pipeline(&cfg, &app_root, &m_p, &a_p, identity.as_deref(), dry_run);
            std::process::exit(code);
        }
        "rollback" => {
            let mut app_root: Option<PathBuf> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--app-root" && i + 1 < args.len() {
                    app_root = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let Some(root) = app_root else {
                eprintln!("error: rollback requires --app-root <path>");
                std::process::exit(2);
            };
            match rollback_release_atomic(&root) {
                Ok(prev) => {
                    println!("Successfully rolled back to {}", prev.display());
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Rollback failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "recover" => {
            let mut config_path: Option<PathBuf> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let Some(cfg_p) = config_path else {
                eprintln!("error: recover requires --config <path>");
                std::process::exit(2);
            };
            let cfg = match config::BotConfig::load(&cfg_p) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error loading config: {e}");
                    std::process::exit(2);
                }
            };
            let app_root = cfg
                .state_dir
                .parent()
                .unwrap_or(&cfg.state_dir)
                .to_path_buf();
            let journal = match UpdateJournal::load_from_state(&cfg.state_dir) {
                Ok(Some(j)) => j,
                Ok(None) => {
                    println!("No update journal found; system is clean");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Corrupted journal: {e}; fail closed: retaining maintenance");
                    let _ = fs::write(cfg.state_dir.join("core.maintenance"), "corrupt-recovery");
                    std::process::exit(1);
                }
            };
            let action =
                journal.evaluate_recovery(&app_root.join("releases"), &app_root.join("current"));
            println!("Recovery action: {:?}", action);
            match action {
                RecoveryAction::DiscardCandidate | RecoveryAction::ResumeOldRelease => {
                    let _ = fs::remove_file(cfg.state_dir.join("update-journal.json"));
                    let _ = fs::remove_file(cfg.state_dir.join("core.maintenance"));
                    println!("Recovered cleanly to old release");
                    std::process::exit(0);
                }
                RecoveryAction::RollbackToPrevious => {
                    let _ = rollback_release_atomic(&app_root);
                    let _ = fs::remove_file(cfg.state_dir.join("update-journal.json"));
                    let _ = fs::remove_file(cfg.state_dir.join("core.maintenance"));
                    println!("Rolled back and cleared journal");
                    std::process::exit(0);
                }
                RecoveryAction::ManualInterventionRequired(msg) => {
                    eprintln!("Manual intervention required: {msg}");
                    std::process::exit(1);
                }
                _ => {
                    std::process::exit(0);
                }
            }
        }
        cmd => {
            eprintln!("unknown command: {cmd}");
            print_usage();
            std::process::exit(2);
        }
    }
}

fn run_apply_pipeline(
    cfg: &config::BotConfig,
    app_root: &Path,
    manifest_path: &Path,
    archive_path: &Path,
    signing_identity: Option<&str>,
    dry_run: bool,
) -> i32 {
    let manifest_bytes = match fs::read(manifest_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read manifest: {e}");
            return 1;
        }
    };
    let envelope: SignedEnvelope = match serde_json::from_slice(&manifest_bytes) {
        Ok(env) => env,
        Err(e) => {
            eprintln!("failed to parse envelope: {e}");
            return 1;
        }
    };

    let highest_seq = journal::load_highest_accepted_sequence(&cfg.state_dir);
    if let Err(e) = manifest::validate_manifest_compatibility(
        &envelope.manifest,
        std::env::consts::ARCH,
        "12.0",
        highest_seq,
    ) {
        eprintln!("manifest incompatible: {e}");
        return 1;
    }

    if dry_run {
        println!(
            "Dry run: package is compatible. Sequence: {}",
            envelope.manifest.sequence
        );
        return 0;
    }

    let tx_id = format!("tx-{}", uuid::Uuid::new_v4());
    let mut journal = UpdateJournal::new(
        &tx_id,
        "0.1.0",
        &envelope.manifest.release_id,
        envelope.manifest.sequence,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );

    // 1. 持久化 Staged
    if let Err(e) = journal.persist_atomic(&cfg.state_dir) {
        eprintln!("failed to persist journal: {e}");
        return 1;
    }

    // 2. 解包到 releases/<release_id>
    let releases_dir = app_root.join("releases");
    let target_release_dir = releases_dir.join(&envelope.manifest.release_id);
    if let Err(e) = safe_unpack_and_verify(
        archive_path,
        &target_release_dir,
        &envelope.manifest.artifact,
        &envelope.manifest.components,
    ) {
        eprintln!("unpack and verification failed: {e}");
        let _ = fs::remove_file(cfg.state_dir.join("update-journal.json"));
        return 1;
    }

    journal.stage = TransactionStage::PublisherVerified;
    let _ = journal.persist_atomic(&cfg.state_dir);

    // 3. 本地代码签名（如需要）
    if let Some(id) = signing_identity {
        let signer = LocalSigner::new(id);
        for comp in &envelope.manifest.components {
            if comp.kind == "executable" {
                let bin_p = target_release_dir.join(&comp.path);
                if let Err(e) = signer.sign_and_verify(&bin_p) {
                    eprintln!("local code signing failed on {}: {e}", bin_p.display());
                    journal.stage = TransactionStage::RecoveryRequired;
                    journal.error = Some(format!("local signing failed: {e}"));
                    let _ = journal.persist_atomic(&cfg.state_dir);
                    let _ = fs::remove_dir_all(&target_release_dir);
                    return 1;
                }
            }
        }
        journal.stage = TransactionStage::LocalSigned;
        let _ = journal.persist_atomic(&cfg.state_dir);
    }

    // 4. 持久化 I4 维护标志抑制 Supervisor
    let maintenance_file = cfg.state_dir.join("core.maintenance");
    if let Err(e) = fs::write(&maintenance_file, &tx_id) {
        eprintln!("failed to write maintenance marker: {e}");
        return 1;
    }
    journal.stage = TransactionStage::Draining;
    journal.maintenance_marker_confirmed = true;
    let _ = journal.persist_atomic(&cfg.state_dir);

    // 5. 数据完整备份
    let backup_res = backup::run_backup(cfg, None);
    let backup_dir = match backup_res {
        Ok(b) => b,
        Err(e) => {
            eprintln!("backup failed: {e}");
            let _ = fs::remove_file(&maintenance_file);
            let _ = fs::remove_file(cfg.state_dir.join("update-journal.json"));
            return 1;
        }
    };
    journal.backup_path = Some(backup_dir.to_string_lossy().to_string());
    journal.stage = TransactionStage::BackupVerified;
    let _ = journal.persist_atomic(&cfg.state_dir);

    // 6. 原子切换指针
    journal.stage = TransactionStage::Switching;
    let _ = journal.persist_atomic(&cfg.state_dir);
    if let Err(e) = switch_release_atomic(app_root, &envelope.manifest.release_id) {
        eprintln!("atomic switch failed: {e}");
        journal.stage = TransactionStage::RollbackStarted;
        let _ = rollback_release_atomic(app_root);
        let _ = fs::remove_file(&maintenance_file);
        return 1;
    }

    // 7. 自检
    journal.stage = TransactionStage::CandidateTesting;
    let _ = journal.persist_atomic(&cfg.state_dir);

    let self_check_ok = envelope
        .manifest
        .components
        .iter()
        .all(|c| target_release_dir.join(&c.path).exists());

    if !self_check_ok {
        eprintln!("candidate self-test failed; rolling back");
        // 检查是否有新数据
        let has_new =
            has_new_data_since_backup(&cfg.state_dir.join("history.sqlite3"), journal.timestamp)
                .unwrap_or(false);
        if has_new {
            eprintln!(
                "CRITICAL: new business data created since update; retaining maintenance to avoid data loss"
            );
            journal.stage = TransactionStage::RecoveryRequired;
            journal.error = Some("new business data created before rollback".into());
            let _ = journal.persist_atomic(&cfg.state_dir);
            return 1;
        }
        let _ = rollback_release_atomic(app_root);
        journal.stage = TransactionStage::RolledBack;
        let _ = journal.persist_atomic(&cfg.state_dir);
        let _ = fs::remove_file(&maintenance_file);
        return 1;
    }

    // 8. Commit
    let _ = journal::save_highest_accepted_sequence(&cfg.state_dir, envelope.manifest.sequence);
    journal.stage = TransactionStage::Committed;
    let _ = journal.persist_atomic(&cfg.state_dir);

    // 解除维护标志，让 supervisor 恢复拉起
    let _ = fs::remove_file(&maintenance_file);
    let _ = fs::remove_file(cfg.state_dir.join("update-journal.json"));

    println!(
        "Update applied successfully to release {}",
        envelope.manifest.release_id
    );
    0
}
