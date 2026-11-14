use std::path::PathBuf;
use wechat_mcp::core::config;
use wechat_mcp::updater::executor;
use wechat_mcp::updater::journal::{self, TransactionStage, UpdateJournal};
use wechat_mcp::updater::manifest;

fn print_usage() {
    eprintln!(
        "Usage: wechat-updater <status|check|download|apply|rollback|recover> [--config <path>] [--manifest <path>] [--trusted-keys <path>]"
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
        "download" => {
            if args.len() != 4 || args[2] != "--config" {
                eprintln!("error: download requires --config <path>");
                std::process::exit(2);
            }
            let result = (|| -> anyhow::Result<_> {
                let config = config::BotConfig::load(&args[3])?;
                let output = std::process::Command::new("sw_vers")
                    .arg("-productVersion")
                    .output()?;
                if !output.status.success() {
                    anyhow::bail!("host_os_version_unavailable");
                }
                let host_os = String::from_utf8(output.stdout)?.trim().to_string();
                tokio::runtime::Runtime::new()?.block_on(
                    wechat_mcp::updater::download::download_candidate(&config, &host_os),
                )
            })();
            match result {
                Ok(receipt) => println!("{}", serde_json::to_string_pretty(&receipt).unwrap()),
                Err(error) => {
                    eprintln!("error: {error:#}");
                    std::process::exit(1);
                }
            }
        }
        "status" => {
            let mut config_path: Option<PathBuf> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    eprintln!("error: unrecognized argument '{}'", args[i]);
                    std::process::exit(2);
                }
            }
            let Some(cfg_p) = config_path else {
                eprintln!("error: status requires --config <path>");
                std::process::exit(2);
            };
            let cfg = match config::BotConfig::load(&cfg_p) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "error: invalid_config: failed to load config from '{}': {e}",
                        cfg_p.display()
                    );
                    std::process::exit(2);
                }
            };
            let state_dir = cfg.state_dir.clone();

            let journal_res = UpdateJournal::load_from_state(&state_dir);
            let (journal, journal_err) = match journal_res {
                Ok(j) => (j, None),
                Err(e) => (None, Some(format!("corrupt_or_unreadable_journal: {e}"))),
            };

            let seq_res = journal::load_highest_accepted_sequence(&state_dir);
            let (highest_seq, seq_err) = match seq_res {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(format!("corrupt_sequence_file: {e}"))),
            };

            let maintenance = state_dir.join("core.maintenance.json").exists()
                || state_dir.join("core.maintenance").exists();

            let status_str = if journal_err.is_some() || seq_err.is_some() {
                "recovery_required"
            } else if let Some(ref j) = journal {
                match j.stage {
                    TransactionStage::Committed => "committed",
                    TransactionStage::RolledBack => "rolled_back",
                    TransactionStage::RecoveryRequired => "recovery_required",
                    _ => "in_progress",
                }
            } else {
                "idle"
            };

            let current_meta = journal::load_current_release_record(&state_dir)
                .ok()
                .flatten();

            let root = cfg.install_root.as_ref();
            let releases = root.map(|path| path.join("releases"));
            let current = root.map(|path| path.join("current"));
            let guard_ready = journal::check_startup_guard_readonly_with_root(
                &state_dir,
                releases.as_deref(),
                current.as_deref(),
                root.map(|p| p.as_path()),
                None,
            ) == journal::StartupDualCheckDecision::AllowStartup;
            let download_ready = guard_ready
                && !maintenance
                && matches!(status_str, "idle" | "committed")
                && wechat_mcp::updater::download::download_configured(&cfg);
            let out = serde_json::json!({
                "status": status_str,
                "highest_accepted_sequence": highest_seq,
                "current_release": current_meta,
                "maintenance_active": maintenance || journal_err.is_some() || seq_err.is_some(),
                "journal": journal,
                "error": journal_err.or(seq_err),
                "capabilities": {"schema":1,"download":download_ready,"prepare":download_ready,"install":false,"rollback":false,"recover":false},
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            std::process::exit(0);
        }
        "check"
            if !args
                .iter()
                .any(|arg| arg == "--manifest" || arg == "--trusted-keys") =>
        {
            if args.len() != 4 || args[2] != "--config" {
                eprintln!("error: source check requires --config <path>");
                std::process::exit(2);
            }
            let config = match config::BotConfig::load(&args[3]) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!("error: invalid_config: {error:#}");
                    std::process::exit(2);
                }
            };
            if config.updater.is_none() {
                eprintln!("error: check requires --manifest unless updater.source is configured");
                std::process::exit(2);
            }
            let result = wechat_mcp::updater::check::check_release_blocking(config, None);
            match result {
                Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
                Err(error) => {
                    eprintln!("error: {error:#}");
                    std::process::exit(1);
                }
            }
        }
        "check" => {
            let mut config_path: Option<PathBuf> = None;
            let mut manifest_path: Option<PathBuf> = None;
            let mut trusted_keys_path: Option<PathBuf> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--manifest" && i + 1 < args.len() {
                    manifest_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--trusted-keys" && i + 1 < args.len() {
                    trusted_keys_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    eprintln!("error: unrecognized argument '{}'", args[i]);
                    std::process::exit(2);
                }
            }
            let (Some(cfg_p), Some(m_p)) = (config_path, manifest_path) else {
                eprintln!("error: check requires --config <path> and --manifest <path>");
                std::process::exit(2);
            };
            let cfg = match config::BotConfig::load(&cfg_p) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "error: invalid_config: failed to load config from '{}': {e}",
                        cfg_p.display()
                    );
                    std::process::exit(2);
                }
            };
            let manifest_bytes = match std::fs::read(&m_p) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error reading manifest: {e}");
                    std::process::exit(1);
                }
            };
            let _envelope = match manifest::parse_signed_envelope(&manifest_bytes) {
                Ok(env) => env,
                Err(e) => {
                    eprintln!("error parsing signed envelope: {e}");
                    std::process::exit(1);
                }
            };

            // The app still has no built-in root; an explicit offline key file is
            // the only way to verify here. Without it, remain fail-closed.
            let Some(trusted_keys) = trusted_keys_path else {
                eprintln!(
                    "error: trust_root_unconfigured: production public key root is not configured; provide --trusted-keys for offline verification"
                );
                std::process::exit(1);
            };
            let store = match manifest::load_trusted_keys(&trusted_keys) {
                Ok(store) => store,
                Err(e) => {
                    eprintln!("error: trusted_keys_invalid: {e}");
                    std::process::exit(1);
                }
            };
            let highest_sequence = match journal::load_highest_accepted_sequence(&cfg.state_dir) {
                Ok(sequence) => sequence,
                Err(e) => {
                    eprintln!("error: corrupt_sequence_file: {e}");
                    std::process::exit(1);
                }
            };
            let host_os = match std::process::Command::new("sw_vers")
                .arg("-productVersion")
                .output()
            {
                Ok(output) if output.status.success() => {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                }
                _ => {
                    eprintln!("error: host_os_version_unavailable");
                    std::process::exit(1);
                }
            };
            let verified = match executor::VerifiedManifestEnvelope::verify_and_construct(
                &manifest_bytes,
                &store,
                std::env::consts::ARCH,
                &host_os,
                highest_sequence,
            ) {
                Ok(verified) => verified,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            let digest = match executor::calculate_manifest_digest(verified.manifest()) {
                Ok(digest) => digest,
                Err(e) => {
                    eprintln!("error: manifest_digest_failed: {e}");
                    std::process::exit(1);
                }
            };
            let out = serde_json::json!({
                "status": "verified",
                "release_id": verified.manifest().release_id,
                "sequence": verified.manifest().sequence,
                "manifest_sha256": digest,
                "host_arch": std::env::consts::ARCH,
                "host_os_version": host_os,
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            std::process::exit(0);
        }
        "apply" if args.iter().any(|arg| arg == "--candidate") => {
            let result = (|| -> anyhow::Result<_> {
                let mut config_path = None;
                let mut candidate_id = None;
                let mut dry_run = false;
                let mut index = 2;
                while index < args.len() {
                    match args[index].as_str() {
                        "--config" if config_path.is_none() && index + 1 < args.len() => {
                            config_path = Some(args[index + 1].clone());
                            index += 2;
                        }
                        "--candidate" if candidate_id.is_none() && index + 1 < args.len() => {
                            candidate_id = Some(args[index + 1].clone());
                            index += 2;
                        }
                        "--dry-run" if !dry_run => {
                            dry_run = true;
                            index += 1;
                        }
                        _ => anyhow::bail!("invalid_candidate_apply_argument"),
                    }
                }
                let config_path = config_path
                    .ok_or_else(|| anyhow::anyhow!("candidate_apply_requires_config"))?;
                let candidate_id = candidate_id
                    .ok_or_else(|| anyhow::anyhow!("candidate_apply_requires_candidate"))?;
                if !dry_run {
                    anyhow::bail!(
                        "production_install_pending_transaction_implementation: use --dry-run to prepare without installation"
                    );
                }
                let config = config::BotConfig::load(config_path)?;
                let output = std::process::Command::new("sw_vers")
                    .arg("-productVersion")
                    .output()?;
                if !output.status.success() {
                    anyhow::bail!("host_os_version_unavailable");
                }
                let host_os = String::from_utf8(output.stdout)?.trim().to_string();
                tokio::runtime::Runtime::new()?.block_on(
                    wechat_mcp::updater::prepare::prepare_candidate(
                        &config,
                        &candidate_id,
                        &host_os,
                    ),
                )
            })();
            match result {
                Ok(prepared) => println!("{}", serde_json::to_string_pretty(&prepared).unwrap()),
                Err(error) => {
                    eprintln!("error: {error:#}");
                    std::process::exit(1);
                }
            }
        }
        "apply" | "rollback" | "recover" => {
            // 检查是否有未知参数，若有未知参数报错退出码 2
            let action = &args[1];
            let mut i = 2;
            while i < args.len() {
                if (args[i] == "--config" || args[i] == "--manifest" || args[i] == "--releases-dir")
                    && i + 1 < args.len()
                {
                    i += 2;
                } else if args[i] == "--dry-run" {
                    i += 1;
                } else {
                    eprintln!("error: unrecognized argument '{}'", args[i]);
                    std::process::exit(2);
                }
            }

            // 协调者限定批准：全部真实写操作入口必须无副作用返回 unsupported，不得写真实 state / 钥匙串 / 切换版本。
            eprintln!(
                "error: unsupported: real update {action} is disabled in this phase (fixture adapters only)"
            );
            std::process::exit(1);
        }
        cmd => {
            eprintln!("unknown command: {cmd}");
            print_usage();
            std::process::exit(2);
        }
    }
}
