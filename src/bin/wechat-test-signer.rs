use anyhow::{Context, Result, bail};
use ring::signature::KeyPair;
use std::fs;
use std::path::PathBuf;
use wechat_mcp::updater::manifest::{ReleaseManifest, SignedEnvelope, sign_manifest_with_ed25519_key};

fn print_usage() {
    eprintln!(
        "Usage: wechat-test-signer --manifest <path> --out <path> [--key-id <id>] [--generate-key-to <pub_file>]"
    );
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut manifest_path: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;
    let mut key_id = "test-root-v1".to_string();
    let mut pub_out: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" if i + 1 < args.len() => {
                manifest_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--out" if i + 1 < args.len() => {
                out_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--key-id" if i + 1 < args.len() => {
                key_id = args[i + 1].clone();
                i += 2;
            }
            "--generate-key-to" if i + 1 < args.len() => {
                pub_out = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => {
                bail!("unrecognized argument: '{}'", other);
            }
        }
    }

    let manifest_p = manifest_path.context("--manifest is required")?;
    let out_p = out_path.context("--out is required")?;

    let manifest_bytes = fs::read(&manifest_p)
        .with_context(|| format!("failed to read manifest at '{}'", manifest_p.display()))?;
    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| "failed to parse manifest as ReleaseManifest")?;

    let rng = ring::rand::SystemRandom::new();
    let pkcs8_doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|e| anyhow::anyhow!("failed to generate ed25519 pkcs8 key: {:?}", e))?;
    let key_pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8_doc.as_ref())
        .map_err(|e| anyhow::anyhow!("failed to construct keypair: {:?}", e))?;

    let pub_bytes = key_pair.public_key().as_ref();
    let mut pub_hex = String::with_capacity(pub_bytes.len() * 2);
    for b in pub_bytes {
        pub_hex.push_str(&format!("{:02x}", b));
    }

    if let Some(ref pub_path) = pub_out {
        fs::write(pub_path, &pub_hex)
            .with_context(|| format!("failed to write public key hex to '{}'", pub_path.display()))?;
    }

    let sig_hex = sign_manifest_with_ed25519_key(&key_pair, &manifest)?;

    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id,
        signature_algorithm: "ed25519".into(),
        signature: sig_hex,
        manifest,
    };

    let envelope_json = serde_json::to_string_pretty(&envelope)?;
    fs::write(&out_p, envelope_json)
        .with_context(|| format!("failed to write signed envelope to '{}'", out_p.display()))?;

    println!("Signed envelope successfully written to '{}'", out_p.display());
    if let Some(ref pub_path) = pub_out {
        println!("Public key hex written to '{}'", pub_path.display());
    }

    Ok(())
}
