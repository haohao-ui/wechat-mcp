//! Production download cache. This stage never pauses or modifies the running
//! installation. A cache receipt is informational; apply must reverify its bytes.
use super::executor::{VerifiedManifestEnvelope, calculate_manifest_digest};
use super::journal::load_highest_accepted_sequence;
use super::manifest::load_trusted_keys;
use super::private_fs::PrivateDirectory;
use super::source::Transport;
use crate::core::config::BotConfig;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadReceipt {
    pub status: String,
    pub candidate_id: String,
    pub release_id: String,
    pub sequence: u64,
    pub manifest_sha256: String,
}

struct UncommittedDownload<'a> {
    parent: &'a PrivateDirectory,
    directory: &'a PrivateDirectory,
    name: std::ffi::CString,
    committed: bool,
}
impl Drop for UncommittedDownload<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Only the three fixed files created by this operation may be removed.
        // Never recursively delete a caller-provided path or installation root.
        for name in [c"envelope.json", c"artifact.tar.gz", c"receipt.json"] {
            unsafe {
                libc::unlinkat(self.directory.0.as_raw_fd(), name.as_ptr(), 0);
            }
        }
        unsafe {
            libc::unlinkat(
                self.parent.0.as_raw_fd(),
                self.name.as_ptr(),
                libc::AT_REMOVEDIR,
            );
        }
        let _ = self.parent.0.sync_all();
    }
}

pub fn download_configured(config: &BotConfig) -> bool {
    config
        .updater
        .as_ref()
        .is_some_and(|settings| load_trusted_keys(&settings.trusted_keys).is_ok())
        && PrivateDirectory::open(&config.state_dir).is_ok()
}

const MAX_CACHE_ENTRIES: usize = 8;
const MAX_CACHE_BYTES: u64 = 1024 * 1024 * 1024;
fn check_cache_quota(directory: &PrivateDirectory, path: &Path, incoming: u64) -> Result<()> {
    let expected = directory.0.metadata()?;
    let actual = std::fs::symlink_metadata(path)?;
    if !actual.is_dir() || actual.dev() != expected.dev() || actual.ino() != expected.ino() {
        bail!("download_cache_binding_changed");
    }
    let mut count = 0;
    let mut size = incoming;
    for entry in std::fs::read_dir(path)? {
        count += 1;
        if count >= MAX_CACHE_ENTRIES {
            bail!("download_cache_quota_exceeded");
        }
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("download_cache_unrecognized_entry"))?;
        if uuid::Uuid::parse_str(&name)
            .map(|id| id.to_string())
            .ok()
            .as_deref()
            != Some(name.as_str())
        {
            bail!("download_cache_unrecognized_entry");
        }
        let child = directory.open_child(&name)?;
        for leaf in std::fs::read_dir(entry.path())? {
            let leaf = leaf?;
            let name = leaf
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("download_cache_unrecognized_entry"))?;
            if !["envelope.json", "artifact.tar.gz", "receipt.json"].contains(&name.as_str()) {
                bail!("download_cache_unrecognized_entry");
            }
            let file = child.read_file(&name, super::source::ARTIFACT_LIMIT)?;
            size = size
                .checked_add(file.metadata()?.len())
                .context("download_cache_size_overflow")?;
            if size > MAX_CACHE_BYTES {
                bail!("download_cache_quota_exceeded");
            }
        }
    }
    if size > MAX_CACHE_BYTES {
        bail!("download_cache_quota_exceeded");
    }
    Ok(())
}

/// Downloads one immutable candidate into a private UUID namespace. The receipt
/// is written last, only after publisher, compatibility, size and hash checks.
/// Rejected downloads retain no valid receipt and can never be selected by apply.
pub async fn download_candidate(
    config: &BotConfig,
    host_os_version: &str,
) -> Result<DownloadReceipt> {
    let settings = config.updater.as_ref().context("updater_unconfigured")?;
    let store = load_trusted_keys(&settings.trusted_keys)?;
    let highest = load_highest_accepted_sequence(&config.state_dir)?;
    let transport = Transport::open(&settings.source)?;
    let bytes = transport.envelope().await?;
    let verified = VerifiedManifestEnvelope::verify_and_construct(
        &bytes,
        &store,
        std::env::consts::ARCH,
        host_os_version,
        highest,
    )?;
    let manifest = verified.manifest();
    let state = PrivateDirectory::open(&config.state_dir)?;
    let _download_lock = state.exclusive_lock("update-download.lock")?;
    let downloads = state.child("update-downloads", false)?;
    check_cache_quota(
        &downloads,
        &config.state_dir.join("update-downloads"),
        manifest
            .artifact
            .size_bytes
            .saturating_add(bytes.len() as u64)
            .saturating_add(4096),
    )?;
    let candidate_id = uuid::Uuid::new_v4().to_string();
    let candidate = downloads.child(&candidate_id, true)?;
    let mut cleanup = UncommittedDownload {
        parent: &downloads,
        directory: &candidate,
        name: std::ffi::CString::new(candidate_id.as_str())?,
        committed: false,
    };
    candidate.write("envelope.json", &bytes)?;
    let mut archive = candidate.create_file("artifact.tar.gz")?;
    transport
        .artifact(
            &manifest.artifact.filename,
            manifest.artifact.size_bytes,
            &mut archive,
        )
        .await?;
    archive.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = archive.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if hex_digest(digest.finalize().as_slice()) != manifest.artifact.sha256 {
        bail!("artifact_sha256_mismatch");
    }
    let receipt = DownloadReceipt {
        status: "downloaded".into(),
        candidate_id,
        release_id: manifest.release_id.clone(),
        sequence: manifest.sequence,
        manifest_sha256: calculate_manifest_digest(manifest)?,
    };
    candidate.write("receipt.json", &serde_json::to_vec_pretty(&receipt)?)?;
    cleanup.committed = true;
    Ok(receipt)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::updater::manifest::{
        ReleaseManifest, SignedEnvelope, sign_manifest_with_ed25519_key,
    };
    use ring::signature::KeyPair;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    pub(crate) struct Fixture {
        pub(crate) root: std::path::PathBuf,
        pub(crate) config: BotConfig,
        pub(crate) envelope: SignedEnvelope,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        pub(crate) fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("production-download-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            for leaf in ["state", "feed"] {
                std::fs::create_dir(root.join(leaf)).unwrap();
                std::fs::set_permissions(root.join(leaf), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
            }
            let rng = ring::rand::SystemRandom::new();
            let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
            let key = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
            let public = key
                .public_key()
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let trust = serde_json::json!({"schema":1,"keys":[{"key_id":"ephemeral-test-only","public_key_hex":public}]});
            std::fs::write(root.join("trusted.json"), trust.to_string()).unwrap();
            std::fs::set_permissions(
                root.join("trusted.json"),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
            let payload = b"release bytes";
            std::fs::write(root.join("feed/payload.tar.gz"), payload).unwrap();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let manifest: ReleaseManifest = serde_json::from_value(serde_json::json!({
                "manifest_schema":1,"release_id":"1.0.0-downloadtest","sequence":1,"channel":"stable",
                "published_at":now - 60,"expires_at":now + 3600,
                "target":{"os":"macos","arch":std::env::consts::ARCH,"min_os_version":"12.0","max_os_version":null},
                "artifact":{"filename":"payload.tar.gz","sha256":hex_digest(Sha256::digest(payload).as_slice()),"size_bytes":payload.len(),"max_unpacked_bytes":1024,"max_file_count":10},
                "components":[{"path":"bin/wechat-mcp","kind":"executable","file_mode":493,"sha256":hex_digest(Sha256::digest(payload).as_slice()),"size_bytes":payload.len()}],
                "compatibility":{"config_schema_min":1,"config_schema_max":2,"profile_schema_min":1,"profile_schema_max":2,"history_schema_min":1,"history_schema_max":7,"min_updater_version":1,"in_place_rollback_supported":true},
                "release_notes":"transport fixture"
            })).unwrap();
            let envelope = SignedEnvelope {
                envelope_schema: 1,
                key_id: "ephemeral-test-only".into(),
                signature_algorithm: "ed25519".into(),
                signature: sign_manifest_with_ed25519_key(&key, &manifest).unwrap(),
                manifest,
            };
            std::fs::write(
                root.join("feed/envelope.json"),
                serde_json::to_vec(&envelope).unwrap(),
            )
            .unwrap();
            let config_value = serde_json::json!({"schema_version":1,"host":"local","account_id":"test-only","allowlist":[{"recipient":"fixture-only","kind":"private"}],"state_dir":root.join("state"),"install_root":root.join("install"),
                "updater":{"source":{"kind":"local","directory":root.join("feed")},"trusted_keys":root.join("trusted.json"),"signing_identity":"fixture-unused","keychain":root.join("unused.keychain"),"keychain_password_file":root.join("unused.pass")}});
            std::fs::write(root.join("config.json"), config_value.to_string()).unwrap();
            let config = BotConfig::load(root.join("config.json")).unwrap();
            Self {
                root,
                config,
                envelope,
            }
        }
        pub(crate) fn resign(&mut self) {
            use ring::signature::KeyPair;
            let pkcs8 =
                ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                    .unwrap();
            let key = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
            let public: String = key
                .public_key()
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            std::fs::write(self.root.join("trusted.json"),serde_json::json!({"schema":1,"keys":[{"key_id":"ephemeral-test-only","public_key_hex":public}]}).to_string()).unwrap();
            self.envelope.signature =
                sign_manifest_with_ed25519_key(&key, &self.envelope.manifest).unwrap();
            std::fs::write(
                self.root.join("feed/envelope.json"),
                serde_json::to_vec(&self.envelope).unwrap(),
            )
            .unwrap();
        }
        fn receipt_count(&self) -> usize {
            std::fs::read_dir(self.root.join("state/update-downloads"))
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().join("receipt.json").is_file())
                .count()
        }
    }

    #[tokio::test]
    async fn publishes_receipt_only_for_complete_publisher_verified_artifact() {
        let fixture = Fixture::new();
        let receipt = download_candidate(&fixture.config, "12.7.6").await.unwrap();
        assert_eq!(receipt.release_id, fixture.envelope.manifest.release_id);
        assert_eq!(fixture.receipt_count(), 1);
        let path = fixture
            .root
            .join("state/update-downloads")
            .join(receipt.candidate_id);
        assert_eq!(
            std::fs::read(path.join("artifact.tar.gz")).unwrap(),
            b"release bytes"
        );
        assert_eq!(
            std::fs::metadata(path.join("receipt.json")).unwrap().mode() & 0o777,
            0o600
        );
        assert!(!fixture.root.join("state/current-release.json").exists());
        assert!(!fixture.root.join("state/core.maintenance").exists());
    }

    #[tokio::test]
    async fn bad_signature_truncation_and_tampered_payload_never_publish_candidate() {
        let mut fixture = Fixture::new();
        std::fs::write(fixture.root.join("feed/payload.tar.gz"), b"short").unwrap();
        assert!(download_candidate(&fixture.config, "12.7.6").await.is_err());
        assert_eq!(fixture.receipt_count(), 0);
        std::fs::write(fixture.root.join("feed/payload.tar.gz"), b"RELEASE BYTES").unwrap();
        let error = download_candidate(&fixture.config, "12.7.6")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("sha256"), "{error}");
        assert_eq!(fixture.receipt_count(), 0);
        fixture.envelope.signature = "00".repeat(64);
        std::fs::write(
            fixture.root.join("feed/envelope.json"),
            serde_json::to_vec(&fixture.envelope).unwrap(),
        )
        .unwrap();
        assert!(download_candidate(&fixture.config, "12.7.6").await.is_err());
        assert_eq!(fixture.receipt_count(), 0);
    }

    #[tokio::test]
    async fn cache_symlink_is_rejected_without_touching_destination() {
        let fixture = Fixture::new();
        let outside = fixture.root.join("untouched");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, fixture.root.join("state/update-downloads")).unwrap();
        assert!(download_candidate(&fixture.config, "12.7.6").await.is_err());
        assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn cache_lock_and_entry_quota_reject_without_publishing() {
        let fixture = Fixture::new();
        let state = PrivateDirectory::open(&fixture.config.state_dir).unwrap();
        let lock = state.exclusive_lock("update-download.lock").unwrap();
        assert!(
            download_candidate(&fixture.config, "12.7.6")
                .await
                .unwrap_err()
                .to_string()
                .contains("concurrent_download")
        );
        drop(lock);
        let cache = state.child("update-downloads", false).unwrap();
        for _ in 0..MAX_CACHE_ENTRIES {
            cache
                .child(&uuid::Uuid::new_v4().to_string(), true)
                .unwrap();
        }
        assert!(
            download_candidate(&fixture.config, "12.7.6")
                .await
                .unwrap_err()
                .to_string()
                .contains("quota_exceeded")
        );
        assert_eq!(fixture.receipt_count(), 0);
    }

    #[test]
    fn aggregate_cache_byte_quota_uses_metadata_without_reading_sparse_files() {
        let fixture = Fixture::new();
        let state = PrivateDirectory::open(&fixture.config.state_dir).unwrap();
        let cache = state.child("update-downloads", false).unwrap();
        for _ in 0..2 {
            let child = cache
                .child(&uuid::Uuid::new_v4().to_string(), true)
                .unwrap();
            child
                .create_file("artifact.tar.gz")
                .unwrap()
                .set_len(super::super::source::ARTIFACT_LIMIT)
                .unwrap();
        }
        assert!(
            check_cache_quota(
                &cache,
                &fixture.config.state_dir.join("update-downloads"),
                1
            )
            .unwrap_err()
            .to_string()
            .contains("quota_exceeded")
        );
    }
}
