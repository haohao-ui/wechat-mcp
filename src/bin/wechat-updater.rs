use std::path::PathBuf;
use wechat_mcp::core::config;
use wechat_mcp::updater::journal::{self, TransactionStage, UpdateJournal};
use wechat_mcp::updater::manifest::{self, SignedEnvelope, TrustedKeyStore};

fn print_usage() {
    eprintln!(
        "Usage: wechat-updater <status|check|apply|rollback|recover> [options]\n\
         \n\
         Commands:\n\
           status    --config <path>\n\
           check     --config <path> --manifest <path> [--test-pubkey-hex <hex>]\n\
           apply     (disabled in this phase: unsupported)\n\
           rollback  (disabled in this phase: unsupported)\n\
           recover   (disabled in this phase: unsupported)\n"
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

            let journal_res = UpdateJournal::load_from_state(&cfg.state_dir);
            let (journal, journal_err) = match journal_res {
                Ok(j) => (j, None),
                Err(e) => (None, Some(format!("corrupt_or_unreadable_journal: {e}"))),
            };

            let seq_res = journal::load_highest_accepted_sequence(&cfg.state_dir);
            let (highest_seq, seq_err) = match seq_res {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(format!("corrupt_sequence_file: {e}"))),
            };

            let maintenance = cfg.state_dir.join("core.maintenance").exists();

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

            let out = serde_json::json!({
                "status": status_str,
                "highest_accepted_sequence": highest_seq,
                "maintenance_active": maintenance,
                "journal": journal,
                "error": journal_err.or(seq_err),
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            std::process::exit(0);
        }
        "check" => {
            let mut config_path: Option<PathBuf> = None;
            let mut manifest_path: Option<PathBuf> = None;
            let mut test_pubkey_hex: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--manifest" && i + 1 < args.len() {
                    manifest_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--test-pubkey-hex" && i + 1 < args.len() {
                    test_pubkey_hex = Some(args[i + 1].clone());
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
            let manifest_bytes = match std::fs::read(&m_p) {
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

            // 生产环境下无可信根公钥配置必须 fail closed；仅允许显式注入测试公钥进行 fixture 演练
            let Some(hex_key) = test_pubkey_hex else {
                eprintln!(
                    "error: trust_root_unconfigured: production public key root is not configured; updates cannot be verified without explicit trusted keys"
                );
                std::process::exit(1);
            };

            let pub_bytes = match hex_decode(&hex_key) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error: invalid test public key hex: {e}");
                    std::process::exit(1);
                }
            };

            let store = TrustedKeyStore::new().with_key(&envelope.key_id, &pub_bytes);
            if let Err(e) = store.verify_envelope(&envelope) {
                eprintln!("signature verification failed: {e}");
                std::process::exit(1);
            }

            let highest_seq = match journal::load_highest_accepted_sequence(&cfg.state_dir) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error loading sequence: {e}");
                    std::process::exit(1);
                }
            };

            match manifest::validate_manifest_compatibility(
                &envelope.manifest,
                std::env::consts::ARCH,
                "12.0",
                highest_seq,
            ) {
                Ok(()) => {
                    println!("manifest is valid and verified with injected test key");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("compatibility check failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "apply" | "rollback" | "recover" => {
            // 协调者限定批准：全部真实写操作入口必须无副作用返回 unsupported，不得写真实 state / 钥匙串 / 切换版本。
            let action = &args[1];
            eprintln!(
                "error: unsupported: real update {} is disabled in this phase (fixture adapters only)",
                action
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

fn hex_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        anyhow::bail!("odd length hex");
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        let b = u8::from_str_radix(&s[i..i + 2], 16)?;
        bytes.push(b);
    }
    Ok(bytes)
}
