//! Offline release-manifest signer.
//!
//! The private key is read from an owner-only (0600) PKCS#8 file; the output is
//! a signed envelope that `wechat-updater check` can verify against a trusted
//! public-key file. This binary never prints or copies the private key.

use anyhow::{Context, Result, bail};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use wechat_mcp::updater::manifest::{
    ReleaseManifest, SignedEnvelope, sign_manifest_with_ed25519_key,
};

fn print_usage() {
    eprintln!(
        "Usage: wechat-release-signer --manifest <unsigned.json> --key-file <pkcs8> --key-id <id> --out <envelope.json> [--public-key-out <hex>]"
    );
}

fn load_private_key(path: &Path) -> Result<Ed25519KeyPair> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("failed to stat signing key '{}'", path.display()))?;
    if meta.file_type().is_symlink() {
        bail!("signing_key_symlink_rejected");
    }
    if !meta.file_type().is_file() {
        bail!("signing_key_not_regular_file");
    }
    if meta.mode() & 0o077 != 0 {
        bail!("signing_key_permissions_too_open: expected 0600 or stricter");
    }
    if meta.uid() != unsafe { libc::getuid() } {
        bail!("signing_key_owner_mismatch");
    }
    if meta.len() > 16 * 1024 {
        bail!("signing_key_too_large");
    }
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read signing key '{}'", path.display()))?;
    Ed25519KeyPair::from_pkcs8(&bytes)
        .map_err(|error| anyhow::anyhow!("signing_key_invalid_pkcs8: {error:?}"))
}

fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("output path has no parent")?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("release-envelope"),
        std::process::id()
    ));
    fs::write(&temp, bytes).with_context(|| format!("failed to write '{}'", temp.display()))?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(mode))?;
    fs::rename(&temp, path).with_context(|| format!("failed to replace '{}'", path.display()))?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut manifest_path: Option<PathBuf> = None;
    let mut key_path: Option<PathBuf> = None;
    let mut key_id: Option<String> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut public_key_out: Option<PathBuf> = None;

    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--manifest" if index + 1 < args.len() => {
                manifest_path = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--key-file" if index + 1 < args.len() => {
                key_path = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--key-id" if index + 1 < args.len() => {
                key_id = Some(args[index + 1].clone());
                index += 2;
            }
            "--out" if index + 1 < args.len() => {
                out_path = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--public-key-out" if index + 1 < args.len() => {
                public_key_out = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => bail!("unrecognized argument: '{other}'"),
        }
    }

    let manifest_path = manifest_path.context("--manifest is required")?;
    let key_path = key_path.context("--key-file is required")?;
    let key_id = key_id.context("--key-id is required")?;
    let out_path = out_path.context("--out is required")?;
    if key_id.trim().is_empty() {
        bail!("--key-id must not be empty");
    }

    let key_pair = load_private_key(&key_path)?;
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read manifest '{}'", manifest_path.display()))?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(&manifest_bytes).context("failed to parse unsigned manifest")?;
    let signature = sign_manifest_with_ed25519_key(&key_pair, &manifest)?;
    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id,
        signature_algorithm: "ed25519".into(),
        signature,
        manifest,
    };
    let envelope_bytes = serde_json::to_vec_pretty(&envelope)?;
    write_atomic(&out_path, &envelope_bytes, 0o644)?;

    if let Some(public_key_out) = public_key_out {
        let mut public_hex = String::with_capacity(64);
        for byte in key_pair.public_key().as_ref() {
            let _ = write!(public_hex, "{byte:02x}");
        }
        fs::write(&public_key_out, public_hex.as_bytes())
            .with_context(|| format!("failed to write '{}'", public_key_out.display()))?;
        fs::set_permissions(&public_key_out, fs::Permissions::from_mode(0o644))?;
    }

    println!("Signed envelope written to '{}'", out_path.display());
    Ok(())
}
