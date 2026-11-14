//! Read-only release discovery for the existing control/UI contract. Checking a
//! signed manifest never asserts that the archive or running program was tested.
use super::executor::calculate_manifest_digest;
use super::journal::{
    self, CurrentReleaseCommitRecord, StartupDualCheckDecision, TransactionStage, UpdateJournal,
};
use super::manifest::{load_trusted_keys, parse_signed_envelope, validate_manifest_environment};
use super::prepare::validate_complete_payload;
use super::private_fs::PrivateDirectory;
use super::process::{PathTool, run};
use super::source::{ENVELOPE_LIMIT, Transport};
use crate::core::config::BotConfig;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn read_manifest(path: &Path) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
        || metadata.len() > ENVELOPE_LIMIT
    {
        bail!("release_manifest_file_invalid");
    }
    let mut bytes = Vec::new();
    file.take(ENVELOPE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > ENVELOPE_LIMIT {
        bail!("release_manifest_file_too_large");
    }
    Ok(bytes)
}

fn unconfigured() -> Value {
    json!({"status":"unconfigured","reason":"distribution_source_not_configured","candidate":null})
}

/// Host version is explicit for deterministic tests. Production obtains it from
/// the actual OS tool rather than pretending all devices run macOS 12.0.
pub async fn check_release(
    config: &BotConfig,
    manifest_path: Option<&Path>,
    host_os: &str,
) -> Result<Value> {
    let bytes = match (manifest_path, &config.updater) {
        (Some(path), _) => read_manifest(path)?,
        (None, Some(settings)) => tokio::time::timeout(
            Duration::from_secs(10),
            Transport::open(&settings.source)?.envelope(),
        )
        .await
        .context("release_manifest_request_timeout")??,
        (None, None) => return Ok(unconfigured()),
    };
    let envelope = parse_signed_envelope(&bytes)?;
    let manifest = &envelope.manifest;
    let verified = if let Some(settings) = &config.updater {
        load_trusted_keys(&settings.trusted_keys)?.verify_envelope(&envelope)?;
        true
    } else {
        false
    };
    validate_manifest_environment(manifest, std::env::consts::ARCH, host_os)?;
    let candidate = json!({"release_id":manifest.release_id,"sequence":manifest.sequence,"published_at":manifest.published_at,"size_bytes":manifest.artifact.size_bytes,"release_notes":manifest.release_notes,"compatible":true,"requires_local_signing":true});
    if !verified {
        return Ok(
            json!({"status":"unverified_trust_root","candidate":candidate,"verification":{"publisher_verified":false,"key_id":envelope.key_id}}),
        );
    }
    validate_complete_payload(manifest)?;
    // Check state inputs with bounded, descriptor-anchored reads before using
    // the unchanged v1 journal helpers. Nothing here migrates or writes state.
    let state = PrivateDirectory::open(&config.state_dir)?;
    for (name, limit) in [
        ("current_release.json", 65536),
        ("highest_accepted_sequence", 64),
    ] {
        match state.read(name, limit) {
            Ok(_) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    if let Some(bytes) =
        journal::read_limited_private_file(&config.state_dir.join("current_release.json"), 65536)?
    {
        let record: CurrentReleaseCommitRecord = serde_json::from_slice(&bytes)?;
        if record.record_schema != 1 {
            bail!("unsupported_current_release_record_schema");
        }
    }
    let highest = journal::load_highest_accepted_sequence(&config.state_dir)?;
    let status = if manifest.sequence > highest {
        "available"
    } else if manifest.sequence < highest {
        bail!("downgrade_rejected: source sequence is below accepted high water mark");
    } else {
        let record_bytes = journal::read_limited_private_file(
            &config.state_dir.join("current_release.json"),
            65536,
        )?
        .context("sequence_accepted_without_matching_installation")?;
        let record: CurrentReleaseCommitRecord = serde_json::from_slice(&record_bytes)?;
        let journal_bytes = journal::read_limited_private_file(
            &config.state_dir.join("update-journal.json"),
            65536,
        )?
        .context("installed_release_journal_missing")?;
        let transaction: UpdateJournal = serde_json::from_slice(&journal_bytes)?;
        if record.record_schema != 1
            || record.release_id != manifest.release_id
            || record.sequence != manifest.sequence
            || record.manifest_digest != calculate_manifest_digest(manifest)?
            || transaction.stage != TransactionStage::Committed
            || transaction.transaction_id != record.tx_id
        {
            bail!("sequence_accepted_without_matching_installation");
        }
        let root = config
            .install_root
            .as_ref()
            .context("install_root_unconfigured")?;
        let releases = root.join("releases");
        let current = root.join("current");
        if journal::check_startup_guard_readonly_with_root(
            &config.state_dir,
            Some(&releases),
            Some(&current),
            Some(root),
            None,
        ) != StartupDualCheckDecision::AllowStartup
        {
            bail!("installed_release_guard_not_ready");
        }
        "already_installed"
    };
    Ok(
        json!({"status":status,"candidate":candidate,"manifest_sha256":calculate_manifest_digest(manifest)?,"verification":{"publisher_verified":true,"key_id":envelope.key_id}}),
    )
}

/// The control API is synchronous even when invoked from the core's Tokio
/// runtime. A dedicated thread avoids nesting a runtime on its calling thread.
pub fn check_release_blocking(config: BotConfig, manifest_path: Option<PathBuf>) -> Result<Value> {
    if config.updater.is_none() && manifest_path.is_none() {
        return Ok(unconfigured());
    }
    std::thread::spawn(move || -> Result<Value> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(12), async {
                    let (bytes, _) = run(
                        &PathTool::SwVers,
                        &[std::ffi::OsStr::new("-productVersion")],
                        Duration::from_secs(3),
                    )
                    .await?;
                    let host_os = String::from_utf8(bytes)?;
                    check_release(&config, manifest_path.as_deref(), host_os.trim()).await
                })
                .await
                .context("release_check_timeout")?
            })
    })
    .join()
    .map_err(|_| anyhow::anyhow!("release_check_worker_failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::prepare::tests::complete_fixture;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn authentic_source_check_does_not_download_or_change_state() {
        let fixture = complete_fixture(false);
        let response = check_release(&fixture.config, None, "12.7.6")
            .await
            .unwrap();
        assert_eq!(response["status"], "available");
        assert_eq!(response["verification"]["publisher_verified"], true);
        assert!(response["candidate"]["published_at"].is_u64());
        assert_eq!(fs::read_dir(fixture.root.join("state")).unwrap().count(), 0);
        assert!(!fixture.root.join("install").exists());
        assert!(!fixture.root.join("unused.keychain").exists());
    }

    #[tokio::test]
    async fn accepted_sequence_without_installed_record_is_never_reported_installed() {
        let mut fixture = complete_fixture(false);
        journal::save_highest_accepted_sequence(
            &fixture.config.state_dir,
            fixture.envelope.manifest.sequence,
        )
        .unwrap();
        let error = check_release(&fixture.config, None, "12.7.6")
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("without_matching_installation"),
            "{error}"
        );
        fixture.config.updater = None;
        let response = check_release(
            &fixture.config,
            Some(&fixture.root.join("feed/envelope.json")),
            "12.7.6",
        )
        .await
        .unwrap();
        assert_eq!(response["status"], "unverified_trust_root");
        assert_eq!(response["verification"]["publisher_verified"], false);
        assert_ne!(response["status"], "already_installed");
    }

    #[tokio::test]
    async fn installation_record_requires_matching_journal_and_current_pointer() {
        let fixture = complete_fixture(false);
        let manifest = &fixture.envelope.manifest;
        let tx = format!("tx-{}", uuid::Uuid::new_v4());
        let digest = calculate_manifest_digest(manifest).unwrap();
        let record = CurrentReleaseCommitRecord {
            record_schema: 1,
            tx_id: tx.clone(),
            release_id: manifest.release_id.clone(),
            sequence: manifest.sequence,
            channel: manifest.channel.clone(),
            manifest_digest: digest.clone(),
            installed_at: 1,
        };
        journal::atomic_commit_release_record(&fixture.config.state_dir, &record).unwrap();
        assert!(
            check_release(&fixture.config, None, "12.7.6")
                .await
                .unwrap_err()
                .to_string()
                .contains("journal_missing")
        );
        let root = fixture.config.install_root.as_ref().unwrap();
        fs::create_dir_all(root.join("releases").join(&manifest.release_id)).unwrap();
        for path in [
            root.clone(),
            root.join("releases"),
            root.join("releases").join(&manifest.release_id),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut transaction =
            UpdateJournal::new(&tx, "baseline", &manifest.release_id, manifest.sequence, 1);
        transaction.stage = TransactionStage::Committed;
        transaction.manifest_digest = Some(digest);
        transaction
            .persist_atomic(&fixture.config.state_dir)
            .unwrap();
        assert!(
            check_release(&fixture.config, None, "12.7.6")
                .await
                .is_err()
        );
        std::os::unix::fs::symlink(
            root.join("releases").join(&manifest.release_id),
            root.join("current"),
        )
        .unwrap();
        let response = check_release(&fixture.config, None, "12.7.6")
            .await
            .unwrap();
        assert_eq!(response["status"], "already_installed");
        transaction.stage = TransactionStage::RecoveryRequired;
        transaction
            .persist_atomic(&fixture.config.state_dir)
            .unwrap();
        assert!(
            check_release(&fixture.config, None, "12.7.6")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn authenticated_expired_future_or_unknown_schema_release_is_rejected() {
        for kind in [
            "expired",
            "future",
            "interval",
            "envelope_schema",
            "manifest_schema",
        ] {
            let mut fixture = complete_fixture(false);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            match kind {
                "expired" => {
                    fixture.envelope.manifest.published_at = now - 200;
                    fixture.envelope.manifest.expires_at = now - 1;
                }
                "future" => {
                    fixture.envelope.manifest.published_at = now + 600;
                    fixture.envelope.manifest.expires_at = now + 3600;
                }
                "interval" => {
                    fixture.envelope.manifest.expires_at = fixture.envelope.manifest.published_at
                }
                "envelope_schema" => fixture.envelope.envelope_schema = 2,
                "manifest_schema" => fixture.envelope.manifest.manifest_schema = 2,
                _ => unreachable!(),
            }
            fixture.resign();
            assert!(
                check_release(&fixture.config, None, "12.7.6")
                    .await
                    .is_err(),
                "{kind}"
            );
            assert!(
                crate::updater::download::download_candidate(&fixture.config, "12.7.6")
                    .await
                    .is_err(),
                "{kind}"
            );
            assert_eq!(fs::read_dir(&fixture.config.state_dir).unwrap().count(), 0);
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn existing_control_contract_works_inside_runtime_and_fails_closed() {
        let mut fixture = complete_fixture(false);
        let args = vec![
            "update-check".into(),
            "--config".into(),
            fixture
                .root
                .join("config.json")
                .to_string_lossy()
                .into_owned(),
        ];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code =
            crate::core::control::execute_control_full(&args, &mut stdout, &mut stderr, None, None);
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&stderr));
        let response: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(response["status"], "available");
        fixture.envelope.signature = "00".repeat(64);
        fs::write(
            fixture.root.join("feed/envelope.json"),
            serde_json::to_vec(&fixture.envelope).unwrap(),
        )
        .unwrap();
        stdout.clear();
        stderr.clear();
        assert_eq!(
            crate::core::control::execute_control_full(&args, &mut stdout, &mut stderr, None, None),
            0
        );
        let response: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(response["status"], "incompatible");
        assert!(response["candidate"].is_null());
        assert_eq!(response["verification"]["publisher_verified"], false);
        fs::write(fixture.root.join("config.json"), "{}").unwrap();
        stdout.clear();
        stderr.clear();
        assert_eq!(
            crate::core::control::execute_control_full(&args, &mut stdout, &mut stderr, None, None),
            2
        );
        assert!(stdout.is_empty());
    }
}
