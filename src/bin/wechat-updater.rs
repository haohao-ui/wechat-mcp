use std::path::PathBuf;
use wechat_mcp::core::config;
use wechat_mcp::updater::journal::{self, TransactionStage, UpdateJournal};
use wechat_mcp::updater::manifest;

fn print_usage() {
    eprintln!(
        "Usage: wechat-updater <status|check|apply|rollback|recover> [--config <path>] [--manifest <path>]"
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
            let state_dir = cfg.state_dir;

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

            let maintenance = state_dir.join("core.maintenance").exists();

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
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--config" && i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--manifest" && i + 1 < args.len() {
                    manifest_path = Some(PathBuf::from(&args[i + 1]));
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
            let _cfg = match config::BotConfig::load(&cfg_p) {
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

            // 生产环境下无可信根公钥配置必须 fail closed；测试公钥仅限 fixture 适配器注入，生产 CLI 严格拒绝无根或覆盖根
            eprintln!(
                "error: trust_root_unconfigured: production public key root is not configured; updates cannot be verified without trusted keys"
            );
            std::process::exit(1);
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
