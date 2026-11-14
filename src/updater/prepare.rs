//! Production candidate preparation. Nothing in this module stops business,
//! switches stable entry points, or writes an installed-release record.
use super::download::DownloadReceipt;
use super::executor::{VerifiedManifestEnvelope, calculate_manifest_digest};
use super::journal::{
    load_highest_accepted_sequence, read_limited_private_file, validate_release_id_safe,
};
use super::manifest::{ReleaseManifest, load_trusted_keys};
use super::private_fs::PrivateDirectory;
use super::process::{PathTool, run};
use super::source::{ARTIFACT_LIMIT, ENVELOPE_LIMIT};
use super::unpack::safe_unpack_and_verify;
use crate::core::config::BotConfig;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const APP: &str = "Apps/微信客服助手.app";
const HOST: &str = "Apps/微信客服助手.app/Contents/MacOS/wechat-cs-host";
const BINS: &[&str] = &[
    "wechat-mcp",
    "wechat-updater",
    "wechat_ocr",
    "wechat_click",
    "desktop_helper",
    "wechat_window",
    "wechat_bubbles",
    "wechat_runtime",
    "wechat_overlay",
    "wechat_bot_worker",
];

#[derive(Debug, Serialize)]
pub struct PreparedCandidate {
    pub status: String,
    pub preparation_id: String,
    pub candidate_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub local_components: Vec<LocalComponent>,
}
#[derive(Debug, Serialize)]
pub struct LocalComponent {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

pub(crate) fn validate_complete_payload(manifest: &ReleaseManifest) -> Result<()> {
    validate_release_id_safe(&manifest.release_id)?;
    let expected: std::collections::HashSet<String> = BINS
        .iter()
        .map(|name| format!("bin/{name}"))
        .chain(std::iter::once(HOST.into()))
        .collect();
    let mut found = std::collections::HashSet::new();
    for component in &manifest.components {
        if expected.contains(&component.path) {
            if component.kind != "executable"
                || component.file_mode != 0o755
                || component.size_bytes == 0
            {
                bail!("required_executable_metadata_invalid: {}", component.path);
            }
            found.insert(component.path.clone());
        } else if component.kind != "regular_file"
            || component.file_mode != 0o644
            || !(component.path.starts_with("resources/")
                || component
                    .path
                    .starts_with("Apps/微信客服助手.app/Contents/Resources/")
                || component.path == format!("{APP}/Contents/Info.plist"))
        {
            bail!(
                "production_payload_path_or_kind_not_allowed: {}",
                component.path
            );
        }
    }
    if found != expected {
        bail!("production_payload_missing_required_executable");
    }
    if !manifest
        .components
        .iter()
        .any(|c| c.path == format!("{APP}/Contents/Info.plist"))
    {
        bail!("production_payload_missing_app_info");
    }
    let compatibility = &manifest.compatibility;
    if compatibility.min_updater_version > 1 {
        bail!("updater_version_incompatible");
    }
    // These are the schemas understood by this build; actual on-disk schemas
    // must be checked again under the production transaction locks before apply.
    for (schema, min, max) in [
        (
            1,
            compatibility.config_schema_min,
            compatibility.config_schema_max,
        ),
        (
            2,
            compatibility.profile_schema_min,
            compatibility.profile_schema_max,
        ),
        (
            7,
            compatibility.history_schema_min,
            compatibility.history_schema_max,
        ),
    ] {
        if schema < min || schema > max {
            bail!("production_runtime_schema_incompatible");
        }
    }
    Ok(())
}

fn read_cached_candidate(
    config: &BotConfig,
    candidate_id: &str,
    host_os: &str,
) -> Result<(VerifiedManifestEnvelope, Vec<u8>, File)> {
    if uuid::Uuid::parse_str(candidate_id)
        .map(|id| id.to_string())
        .ok()
        .as_deref()
        != Some(candidate_id)
    {
        bail!("candidate_id_must_be_canonical_uuid");
    }
    let settings = config.updater.as_ref().context("updater_unconfigured")?;
    let state = PrivateDirectory::open(&config.state_dir)?;
    let candidate = state
        .open_child("update-downloads")?
        .open_child(candidate_id)?;
    let bytes = candidate.read("envelope.json", ENVELOPE_LIMIT)?;
    let store = load_trusted_keys(&settings.trusted_keys)?;
    let highest = load_highest_accepted_sequence(&config.state_dir)?;
    let verified = VerifiedManifestEnvelope::verify_and_construct(
        &bytes,
        &store,
        std::env::consts::ARCH,
        host_os,
        highest,
    )?;
    validate_complete_payload(verified.manifest())?;
    let receipt: DownloadReceipt =
        serde_json::from_slice(&candidate.read("receipt.json", ENVELOPE_LIMIT)?)?;
    let manifest = verified.manifest();
    if receipt.status != "downloaded"
        || receipt.candidate_id != candidate_id
        || receipt.release_id != manifest.release_id
        || receipt.sequence != manifest.sequence
        || receipt.manifest_sha256 != calculate_manifest_digest(manifest)?
    {
        bail!("cached_receipt_mismatch");
    }
    let archive = candidate.read_file("artifact.tar.gz", ARTIFACT_LIMIT)?;
    Ok((verified, bytes, archive))
}

fn same_directory(path: &Path, directory: &PrivateDirectory) -> Result<()> {
    let actual = fs::symlink_metadata(path)?;
    let expected = directory.0.metadata()?;
    if !actual.is_dir() || actual.dev() != expected.dev() || actual.ino() != expected.ino() {
        bail!("preparation_directory_binding_changed");
    }
    Ok(())
}

async fn tool(tool: PathTool, args: &[&OsStr]) -> Result<(Vec<u8>, Vec<u8>)> {
    run(&tool, args, Duration::from_secs(60)).await
}

fn requirement_argument(expression: &str) -> String {
    format!("={expression}")
}

fn certificate_requirement(designated: &str) -> Result<String> {
    let prefix = "certificate leaf = H\"";
    let start = designated
        .find(prefix)
        .context("installed_app_missing_explicit_leaf_certificate")?
        + prefix.len();
    let hash = designated[start..].split('"').next().unwrap_or_default();
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("installed_app_certificate_requirement_invalid");
    }
    Ok(format!("certificate leaf = H\"{hash}\""))
}

async fn verify_architecture(executable: &Path) -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    };
    tool(
        PathTool::Lipo,
        &[
            executable.as_os_str(),
            OsStr::new("-verify_arch"),
            OsStr::new(arch),
        ],
    )
    .await
    .with_context(|| {
        format!(
            "candidate_architecture_check_failed: {}",
            executable.file_name().unwrap_or_default().to_string_lossy()
        )
    })?;
    Ok(())
}

async fn sign_payload(
    config: &BotConfig,
    payload: &Path,
    manifest: &ReleaseManifest,
) -> Result<()> {
    let settings = config.updater.as_ref().context("updater_unconfigured")?;
    let home = std::env::var_os("HOME").context("home_unavailable")?;
    let installed = PathBuf::from(home).join("Applications/微信客服助手.app");
    let app_meta = fs::symlink_metadata(&installed)?;
    if !app_meta.is_dir() || app_meta.uid() != unsafe { libc::geteuid() } {
        bail!("installed_app_owner_or_type_invalid");
    }
    tool(
        PathTool::CodeSign,
        &[
            OsStr::new("--verify"),
            OsStr::new("--deep"),
            OsStr::new("--strict"),
            installed.as_os_str(),
        ],
    )
    .await?;
    let (mut requirement, diagnostics) = tool(
        PathTool::CodeSign,
        &[OsStr::new("-dr"), OsStr::new("-"), installed.as_os_str()],
    )
    .await?;
    requirement.extend_from_slice(&diagnostics);
    let requirement = String::from_utf8(requirement)?;
    if !requirement.contains("identifier \"local.wechat-mcp.host\"") {
        bail!("installed_app_identifier_invalid");
    }
    let certificate = certificate_requirement(&requirement)?;
    let certificate_test = requirement_argument(&certificate);
    let info = payload.join(APP).join("Contents/Info.plist");
    // Publisher-provided version and identity are validated, never rewritten
    // after publisher verification or silently derived from a VERSION text file.
    let sequence = manifest.sequence.to_string();
    for (key, value) in [
        ("CFBundleIdentifier", "local.wechat-mcp.host"),
        ("CFBundleExecutable", "wechat-cs-host"),
        ("CFBundleVersion", sequence.as_str()),
        ("CFBundleShortVersionString", manifest.release_id.as_str()),
    ] {
        let (actual, _) = tool(
            PathTool::Plutil,
            &[
                OsStr::new("-extract"),
                OsStr::new(key),
                OsStr::new("raw"),
                OsStr::new("-o"),
                OsStr::new("-"),
                info.as_os_str(),
            ],
        )
        .await?;
        if String::from_utf8(actual)?.trim() != value {
            bail!("candidate_app_metadata_mismatch: {key}");
        }
    }
    let keychain_meta = fs::symlink_metadata(&settings.keychain)?;
    if !keychain_meta.is_file()
        || keychain_meta.uid() != unsafe { libc::geteuid() }
        || keychain_meta.mode() & 0o022 != 0
    {
        bail!("keychain_owner_or_type_invalid");
    }
    let password_bytes = read_limited_private_file(&settings.keychain_password_file, 4096)?
        .context("keychain_password_missing")?;
    let password = std::str::from_utf8(&password_bytes)?.trim_end_matches(['\r', '\n']);
    if password.is_empty() || password.contains('\0') {
        bail!("keychain_password_invalid");
    }
    tool(
        PathTool::Security,
        &[
            OsStr::new("unlock-keychain"),
            OsStr::new("-p"),
            OsStr::new(password),
            settings.keychain.as_os_str(),
        ],
    )
    .await?;
    for executable in BINS
        .iter()
        .map(|name| payload.join("bin").join(name))
        .chain(std::iter::once(payload.join(HOST)))
    {
        verify_architecture(&executable).await?;
        tool(
            PathTool::CodeSign,
            &[
                OsStr::new("--force"),
                OsStr::new("--sign"),
                OsStr::new(&settings.signing_identity),
                OsStr::new("--keychain"),
                settings.keychain.as_os_str(),
                executable.as_os_str(),
            ],
        )
        .await?;
        tool(
            PathTool::CodeSign,
            &[
                OsStr::new("--verify"),
                OsStr::new("--strict"),
                OsStr::new("-R"),
                OsStr::new(&certificate_test),
                executable.as_os_str(),
            ],
        )
        .await?;
    }
    let app = payload.join(APP);
    tool(
        PathTool::CodeSign,
        &[
            OsStr::new("--force"),
            OsStr::new("--sign"),
            OsStr::new(&settings.signing_identity),
            OsStr::new("--keychain"),
            settings.keychain.as_os_str(),
            app.as_os_str(),
        ],
    )
    .await?;
    let expected_app = format!("identifier \"local.wechat-mcp.host\" and {certificate}");
    let expected_app = requirement_argument(&expected_app);
    tool(
        PathTool::CodeSign,
        &[
            OsStr::new("--verify"),
            OsStr::new("--deep"),
            OsStr::new("--strict"),
            OsStr::new("-R"),
            OsStr::new(&expected_app),
            app.as_os_str(),
        ],
    )
    .await?;
    Ok(())
}

fn unpack_cached_archive(
    archive: File,
    manifest: &ReleaseManifest,
    directory: &PrivateDirectory,
    path: &Path,
) -> Result<PathBuf> {
    let mut copy = directory.create_file("artifact.tar.gz")?;
    let count = std::io::copy(
        &mut archive.take(manifest.artifact.size_bytes.saturating_add(1)),
        &mut copy,
    )?;
    if count != manifest.artifact.size_bytes {
        bail!("cached_archive_size_mismatch");
    }
    copy.sync_all()?;
    let payload = directory.child("payload", true)?;
    let payload_path = path.join("payload");
    safe_unpack_and_verify(
        &path.join("artifact.tar.gz"),
        &payload_path,
        &manifest.artifact,
        &manifest.components,
    )?;
    same_directory(&payload_path, &payload)?;
    Ok(payload_path)
}

/// Reverify, unpack and sign a complete candidate while the current app runs.
/// The returned preparation is not an installed version or permission to switch.
pub async fn prepare_candidate(
    config: &BotConfig,
    candidate_id: &str,
    host_os: &str,
) -> Result<PreparedCandidate> {
    let (verified, bytes, archive) = read_cached_candidate(config, candidate_id, host_os)?;
    let manifest = verified.manifest();
    let install_root = config
        .install_root
        .as_ref()
        .context("install_root_unconfigured")?;
    let root = PrivateDirectory::open(install_root)?;
    let staging = root.child("staging", false)?;
    let preparation_id = uuid::Uuid::new_v4().to_string();
    let directory = staging.child(&preparation_id, true)?;
    let path = install_root.join("staging").join(&preparation_id);
    let result = async {
        same_directory(&path, &directory)?;
        directory.write("envelope.json", &bytes)?;
        let payload_path = unpack_cached_archive(archive, manifest, &directory, &path)?;
        sign_payload(config, &payload_path, manifest).await?;
        let mut local_components = Vec::new();
        let signature_resource = format!("{APP}/Contents/_CodeSignature/CodeResources");
        for component_path in manifest
            .components
            .iter()
            .map(|component| component.path.as_str())
            .chain(std::iter::once(signature_resource.as_str()))
        {
            let mut file = File::open(payload_path.join(component_path))?;
            let mut hasher = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            local_components.push(LocalComponent {
                path: component_path.to_string(),
                sha256: hasher
                    .finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                size_bytes: file.metadata()?.len(),
            });
        }
        let prepared = PreparedCandidate {
            status: "prepared".into(),
            preparation_id: preparation_id.clone(),
            candidate_id: candidate_id.into(),
            release_id: manifest.release_id.clone(),
            manifest_sha256: calculate_manifest_digest(manifest)?,
            local_components,
        };
        same_directory(&path, &directory)?;
        directory.write("prepared.json", &serde_json::to_vec_pretty(&prepared)?)?;
        Ok(prepared)
    }
    .await;
    if result.is_err() && same_directory(&path, &directory).is_ok() {
        let _ = fs::remove_dir_all(&path);
    }
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn certificate_anchor_rejects_adhoc_and_malformed_hashes() {
        assert!(certificate_requirement("identifier \"local.wechat-mcp.host\" and adhoc").is_err());
        assert!(certificate_requirement("certificate leaf = H\"bad\"").is_err());
        let requirement = format!(
            "identifier \"local.wechat-mcp.host\" and certificate leaf = H\"{}\"",
            "ab".repeat(20)
        );
        assert_eq!(
            certificate_requirement(&requirement).unwrap(),
            format!("certificate leaf = H\"{}\"", "ab".repeat(20))
        );
    }
    pub(crate) fn complete_fixture(
        extra_archive_file: bool,
    ) -> crate::updater::download::tests::Fixture {
        use ring::signature::KeyPair;
        let mut fixture = crate::updater::download::tests::Fixture::new();
        let paths = BINS
            .iter()
            .map(|name| format!("bin/{name}"))
            .chain(std::iter::once(HOST.into()))
            .chain(std::iter::once(format!("{APP}/Contents/Info.plist")));
        let mut components = Vec::new();
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(encoder);
        for path in paths {
            // Deliberately not a real executable: this fixture proves unpack
            // integrity only and is never a signing or startup acceptance proof.
            let bytes = b"non-executable integrity fixture";
            let executable = !path.ends_with("Info.plist");
            let mut header = tar::Header::new_ustar();
            header.set_size(bytes.len() as u64);
            header.set_mode(if executable { 0o755 } else { 0o644 });
            header.set_cksum();
            tar.append_data(&mut header, &path, &bytes[..]).unwrap();
            components.push(crate::updater::manifest::ComponentSpec {
                path,
                kind: if executable {
                    "executable".into()
                } else {
                    "regular_file".into()
                },
                file_mode: if executable { 0o755 } else { 0o644 },
                sha256: Sha256::digest(bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                size_bytes: bytes.len() as u64,
            });
        }
        if extra_archive_file {
            let mut header = tar::Header::new_ustar();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "resources/unlisted.txt", &b"bad"[..])
                .unwrap();
        }
        let bytes = tar.into_inner().unwrap().finish().unwrap();
        fs::write(fixture.root.join("feed/payload.tar.gz"), &bytes).unwrap();
        fixture.envelope.manifest.components = components;
        fixture.envelope.manifest.artifact.sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        fixture.envelope.manifest.artifact.size_bytes = bytes.len() as u64;
        fixture.envelope.manifest.artifact.max_unpacked_bytes = 1024 * 1024;
        fixture.envelope.manifest.artifact.max_file_count = 200;
        let key = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        let key = ring::signature::Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        let public: String = key
            .public_key()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        fs::write(fixture.root.join("trusted.json"),serde_json::json!({"schema":1,"keys":[{"key_id":"ephemeral-test-only","public_key_hex":public}]}).to_string()).unwrap();
        fixture.envelope.signature = crate::updater::manifest::sign_manifest_with_ed25519_key(
            &key,
            &fixture.envelope.manifest,
        )
        .unwrap();
        fs::write(
            fixture.root.join("feed/envelope.json"),
            serde_json::to_vec(&fixture.envelope).unwrap(),
        )
        .unwrap();
        fixture
    }

    #[tokio::test]
    async fn revalidates_download_and_extracts_complete_component_set() {
        let fixture = complete_fixture(false);
        let receipt = crate::updater::download::download_candidate(&fixture.config, "12.7.6")
            .await
            .unwrap();
        let (verified, _, archive) =
            read_cached_candidate(&fixture.config, &receipt.candidate_id, "12.7.6").unwrap();
        let state = PrivateDirectory::open(&fixture.root.join("state")).unwrap();
        let stage = state.child("test-stage", true).unwrap();
        let payload = unpack_cached_archive(
            archive,
            verified.manifest(),
            &stage,
            &fixture.root.join("state/test-stage"),
        )
        .unwrap();
        assert!(payload.join(HOST).is_file());
        assert_eq!(
            fs::read_dir(payload.join("bin")).unwrap().count(),
            BINS.len()
        );
        assert!(!fixture.root.join("state/current_release.json").exists());
        assert!(read_cached_candidate(&fixture.config, "../outside", "12.7.6").is_err());
        let cached = fixture
            .root
            .join("state/update-downloads")
            .join(&receipt.candidate_id);
        fs::write(cached.join("receipt.json"), b"{}").unwrap();
        assert!(read_cached_candidate(&fixture.config, &receipt.candidate_id, "12.7.6").is_err());
    }

    #[tokio::test]
    async fn rejects_post_download_tampering_and_unlisted_archive_content() {
        for extra in [false, true] {
            let fixture = complete_fixture(extra);
            let receipt = crate::updater::download::download_candidate(&fixture.config, "12.7.6")
                .await
                .unwrap();
            if !extra {
                let cached = fixture
                    .root
                    .join("state/update-downloads")
                    .join(&receipt.candidate_id);
                let archive = cached.join("artifact.tar.gz");
                let mut bytes = fs::read(&archive).unwrap();
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
                fs::write(archive, bytes).unwrap();
            }
            let (verified, _, archive) =
                read_cached_candidate(&fixture.config, &receipt.candidate_id, "12.7.6").unwrap();
            let state = PrivateDirectory::open(&fixture.root.join("state")).unwrap();
            let stage = state.child("test-stage", true).unwrap();
            assert!(
                unpack_cached_archive(
                    archive,
                    verified.manifest(),
                    &stage,
                    &fixture.root.join("state/test-stage")
                )
                .is_err()
            );
            assert!(!fixture.root.join("state/current_release.json").exists());
        }
    }

    #[test]
    fn partial_runtime_or_unexpected_executable_is_not_a_production_candidate() {
        let fixture = complete_fixture(false);
        let mut manifest = fixture.envelope.manifest.clone();
        validate_complete_payload(&manifest).unwrap();
        manifest.components.pop();
        assert!(validate_complete_payload(&manifest).is_err());
        let mut manifest = fixture.envelope.manifest.clone();
        manifest.components[0].path = "resources/hidden-executable".into();
        assert!(validate_complete_payload(&manifest).is_err());
        let mut manifest = fixture.envelope.manifest.clone();
        manifest.compatibility.history_schema_max = 3;
        assert!(validate_complete_payload(&manifest).is_err());
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn actual_lipo_accepts_current_macho_and_rejects_source_text() {
        verify_architecture(&std::env::current_exe().unwrap())
            .await
            .unwrap();
        assert!(
            verify_architecture(Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/updater/prepare.rs"
            )))
            .await
            .is_err()
        );
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn actual_codesign_interprets_requirement_as_text_instead_of_a_filename() {
        let executable = std::env::current_exe().unwrap();
        let literal = requirement_argument("always");
        tool(
            PathTool::CodeSign,
            &[
                OsStr::new("--verify"),
                OsStr::new("--strict"),
                OsStr::new("-R"),
                OsStr::new(&literal),
                executable.as_os_str(),
            ],
        )
        .await
        .unwrap();
    }
}
