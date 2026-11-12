use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const MANIFEST_DOMAIN_PREFIX: &[u8] = b"WECHAT_MCP_UPDATE_MANIFEST_V1\n";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedEnvelope {
    pub envelope_schema: u32,
    pub key_id: String,
    pub signature_algorithm: String,
    pub signature: String,
    pub manifest: ReleaseManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub manifest_schema: u32,
    pub release_id: String,
    pub sequence: u64,
    pub channel: String,
    pub published_at: u64,
    pub expires_at: u64,
    pub target: TargetSpec,
    pub artifact: ArtifactSpec,
    pub components: Vec<ComponentSpec>,
    pub compatibility: CompatibilitySpec,
    pub release_notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    pub os: String,
    pub arch: String,
    pub min_os_version: String,
    pub max_os_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSpec {
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub max_unpacked_bytes: u64,
    pub max_file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComponentSpec {
    pub path: String,
    pub kind: String,
    pub file_mode: u32,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompatibilitySpec {
    pub config_schema_min: u32,
    pub config_schema_max: u32,
    pub profile_schema_min: u32,
    pub profile_schema_max: u32,
    pub history_schema_min: u32,
    pub history_schema_max: u32,
    pub min_updater_version: u32,
    pub in_place_rollback_supported: bool,
}

#[derive(Debug, Clone)]
pub struct TrustedKeyStore {
    // key_id -> ed25519 public key bytes (32 bytes)
    pub keys: HashMap<String, Vec<u8>>,
}

impl Default for TrustedKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustedKeyStore {
    pub fn new() -> Self {
        Self {
            keys: HashMap::new(),
        }
    }

    pub fn with_key(mut self, key_id: impl Into<String>, public_key: &[u8]) -> Self {
        self.keys.insert(key_id.into(), public_key.to_vec());
        self
    }

    /// 验证签名封套：
    /// 1. 查找 key_id 是否在受信根公钥表中（拒绝未知 key_id，严禁信任包自带密钥）
    /// 2. 检查 signature_algorithm 是否为 ed25519
    /// 3. RFC 8785 规范化序列化 manifest
    /// 4. 添加域分离前缀 "WECHAT_MCP_UPDATE_MANIFEST_V1\n"
    /// 5. ring::signature::ED25519 验签
    pub fn verify_envelope(&self, envelope: &SignedEnvelope) -> Result<()> {
        let pubkey_bytes = match self.keys.get(&envelope.key_id) {
            Some(k) => k,
            None => bail!(
                "untrusted_key_id: key '{}' not found in trusted root keystore",
                envelope.key_id
            ),
        };

        if envelope.signature_algorithm != "ed25519" {
            bail!(
                "unsupported_signature_algorithm: '{}', only 'ed25519' is accepted",
                envelope.signature_algorithm
            );
        }

        let sig_bytes = match hex_or_base64_decode(&envelope.signature) {
            Ok(b) => b,
            Err(e) => bail!("invalid_signature_encoding: {}", e),
        };

        // RFC 8785 规范化
        let manifest_val = serde_json::to_value(&envelope.manifest)?;
        let canonical_bytes = serde_json_canonicalizer::to_vec(&manifest_val)?;

        // 拼接域分离前缀
        let mut message = Vec::with_capacity(MANIFEST_DOMAIN_PREFIX.len() + canonical_bytes.len());
        message.extend_from_slice(MANIFEST_DOMAIN_PREFIX);
        message.extend_from_slice(&canonical_bytes);

        let peer_public_key =
            ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pubkey_bytes);
        peer_public_key.verify(&message, &sig_bytes).map_err(|_| {
            anyhow::anyhow!("signature_verification_failed: cryptographic signature mismatch")
        })?;

        Ok(())
    }
}

/// Trusted release key file used by the CLI verification path.
///
/// The production app is expected to carry its own built-in root. This file
/// exists so release engineering can verify a signed envelope offline without
/// weakening the default fail-closed behavior.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedKeysFile {
    pub schema: u32,
    pub keys: Vec<TrustedKeyEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedKeyEntry {
    pub key_id: String,
    pub public_key_hex: String,
}

/// Loads a trusted-key file with the same narrow file checks used for secrets:
/// regular file, current owner, owner-only permissions, bounded size, no
/// duplicate JSON keys or key IDs, and 32-byte Ed25519 public keys.
pub fn load_trusted_keys(path: &Path) -> Result<TrustedKeyStore> {
    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to stat trusted keys '{}'", path.display()))?;
    if meta.file_type().is_symlink() {
        bail!("trusted_keys_symlink_rejected");
    }
    if !meta.file_type().is_file() {
        bail!("trusted_keys_not_regular_file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o077 != 0 {
            bail!("trusted_keys_permissions_too_open: expected 0600 or stricter");
        }
        if meta.uid() != unsafe { libc::getuid() } {
            bail!("trusted_keys_owner_mismatch");
        }
    }
    if meta.len() > 64 * 1024 {
        bail!("trusted_keys_file_too_large");
    }

    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read trusted keys '{}'", path.display()))?;
    let text = std::str::from_utf8(&bytes).context("trusted_keys_invalid_utf8")?;
    ensure_no_duplicate_json_keys(text)?;
    let file: TrustedKeysFile = serde_json::from_slice(&bytes)?;
    if file.schema != 1 {
        bail!("unsupported_trusted_keys_schema: {}", file.schema);
    }
    if file.keys.is_empty() {
        bail!("trusted_keys_empty");
    }

    let mut store = TrustedKeyStore::new();
    for entry in file.keys {
        if entry.key_id.trim().is_empty() {
            bail!("trusted_keys_empty_key_id");
        }
        let public_key = decode_ed25519_public_key_hex(&entry.public_key_hex)?;
        if store
            .keys
            .insert(entry.key_id.clone(), public_key)
            .is_some()
        {
            bail!("trusted_keys_duplicate_key_id: {}", entry.key_id);
        }
    }
    Ok(store)
}

fn decode_ed25519_public_key_hex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    if text.len() != 64 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("trusted_keys_invalid_public_key_hex: expected 64 hex characters");
    }
    let mut bytes = Vec::with_capacity(32);
    for index in (0..64).step_by(2) {
        let byte = u8::from_str_radix(&text[index..index + 2], 16)
            .context("trusted_keys_invalid_public_key_hex")?;
        bytes.push(byte);
    }
    Ok(bytes)
}

/// 为测试或受控构建提供规范的 Ed25519 签名生成（RFC 8785 规范化 + 域分离）
pub fn sign_manifest_with_ed25519_key(
    key_pair: &ring::signature::Ed25519KeyPair,
    manifest: &ReleaseManifest,
) -> Result<String> {
    let manifest_val = serde_json::to_value(manifest)?;
    let canonical = serde_json_canonicalizer::to_vec(&manifest_val)?;
    let mut message = Vec::with_capacity(MANIFEST_DOMAIN_PREFIX.len() + canonical.len());
    message.extend_from_slice(MANIFEST_DOMAIN_PREFIX);
    message.extend_from_slice(&canonical);

    let sig = key_pair.sign(&message);
    let mut s = String::with_capacity(sig.as_ref().len() * 2);
    for b in sig.as_ref() {
        s.push_str(&format!("{:02x}", b));
    }
    Ok(s)
}

fn hex_or_base64_decode(s: &str) -> Result<Vec<u8>> {
    let trimmed = s.trim();
    if trimmed.len() == 128 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut bytes = Vec::with_capacity(64);
        for i in (0..128).step_by(2) {
            let byte = u8::from_str_radix(&trimmed[i..i + 2], 16)?;
            bytes.push(byte);
        }
        return Ok(bytes);
    }
    // base64 decode
    // 纯手动/简洁 base64 解码，避免引入额外 crate
    base64_decode_standard(trimmed)
}

fn base64_decode_standard(input: &str) -> Result<Vec<u8>> {
    const B64_CHARS: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in B64_CHARS.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }

    let clean: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity((clean.len() * 3) / 4);

    let chunks = clean.chunks_exact(4);
    let remainder = chunks.remainder();

    for chunk in chunks {
        let (a, b, c, d) = (
            lookup[chunk[0] as usize],
            lookup[chunk[1] as usize],
            lookup[chunk[2] as usize],
            lookup[chunk[3] as usize],
        );
        if a == 255 || b == 255 || c == 255 || d == 255 {
            bail!("invalid base64 character");
        }
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
        out.push((c << 6) | d);
    }

    match remainder.len() {
        2 => {
            let (a, b) = (lookup[remainder[0] as usize], lookup[remainder[1] as usize]);
            if a == 255 || b == 255 {
                bail!("invalid base64 character");
            }
            out.push((a << 2) | (b >> 4));
        }
        3 => {
            let (a, b, c) = (
                lookup[remainder[0] as usize],
                lookup[remainder[1] as usize],
                lookup[remainder[2] as usize],
            );
            if a == 255 || b == 255 || c == 255 {
                bail!("invalid base64 character");
            }
            out.push((a << 2) | (b >> 4));
            out.push((b << 4) | (c >> 2));
        }
        1 => bail!("invalid base64 length"),
        _ => {}
    }

    Ok(out)
}

/// 架构、系统版本与防降级校验
pub fn validate_manifest_compatibility(
    manifest: &ReleaseManifest,
    host_arch: &str,
    host_os_ver: &str,
    highest_accepted_seq: u64,
) -> Result<()> {
    if manifest.target.os != "macos" {
        bail!(
            "os_incompatible: manifest target OS is '{}', host is macos",
            manifest.target.os
        );
    }

    if canonicalize_arch(&manifest.target.arch) != canonicalize_arch(host_arch) {
        bail!(
            "architecture_mismatch: manifest arch '{}' does not match host arch '{}'",
            manifest.target.arch,
            host_arch
        );
    }

    if compare_version_strings(host_os_ver, &manifest.target.min_os_version) < 0 {
        bail!(
            "os_version_too_low: host os {} < required min os {}",
            host_os_ver,
            manifest.target.min_os_version
        );
    }

    if let Some(ref max_os) = manifest.target.max_os_version {
        if compare_version_strings(host_os_ver, max_os) > 0 {
            bail!(
                "os_version_too_high: host os {} > verified max os {}",
                host_os_ver,
                max_os
            );
        }
    }

    if manifest.sequence == highest_accepted_seq {
        bail!(
            "sequence_already_installed: candidate sequence {} matches highest accepted sequence (re-installation rejected)",
            manifest.sequence
        );
    }
    if manifest.sequence < highest_accepted_seq {
        bail!(
            "downgrade_rejected: candidate sequence {} is lower than highest accepted sequence {}",
            manifest.sequence,
            highest_accepted_seq
        );
    }

    // 组件白名单与合理性初步检查
    if manifest.components.is_empty() {
        bail!("manifest_components_empty: at least one component required");
    }

    let mut seen_paths = std::collections::HashSet::new();
    for comp in &manifest.components {
        if comp.path.starts_with('/') || comp.path.contains("..") || comp.path.contains('\0') {
            bail!("invalid_component_path: '{}'", comp.path);
        }
        if !seen_paths.insert(&comp.path) {
            bail!("duplicate_component_path: '{}'", comp.path);
        }
        if comp.sha256.len() != 64 || !comp.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!(
                "invalid_component_sha256: '{}' for '{}'",
                comp.sha256,
                comp.path
            );
        }
    }

    Ok(())
}

fn canonicalize_arch(arch: &str) -> &str {
    match arch {
        "arm64" | "aarch64" => "arm64",
        "x86_64" | "amd64" => "x86_64",
        other => other,
    }
}

fn compare_version_strings(v1: &str, v2: &str) -> i32 {
    let p1: Vec<u64> = v1.split('.').filter_map(|s| s.parse().ok()).collect();
    let p2: Vec<u64> = v2.split('.').filter_map(|s| s.parse().ok()).collect();
    let max_len = p1.len().max(p2.len());
    for i in 0..max_len {
        let n1 = p1.get(i).copied().unwrap_or(0);
        let n2 = p2.get(i).copied().unwrap_or(0);
        if n1 < n2 {
            return -1;
        } else if n1 > n2 {
            return 1;
        }
    }
    0
}

/// 解析签名封套并严格校验：
/// 1. 拒绝包含重复 JSON 键的对象（包含嵌套对象）；
/// 2. 拒绝任何未定义字段（serde deny_unknown_fields）；
/// 3. 校验 UTF-8 编码。
pub fn parse_signed_envelope(bytes: &[u8]) -> Result<SignedEnvelope> {
    let json_str =
        std::str::from_utf8(bytes).map_err(|e| anyhow::anyhow!("invalid_utf8_in_envelope: {e}"))?;
    ensure_no_duplicate_json_keys(json_str)?;
    let envelope: SignedEnvelope = serde_json::from_slice(bytes)?;
    Ok(envelope)
}

/// 递归扫描 JSON 字符串中的所有对象，确保同一层级内不存在重复键
pub fn ensure_no_duplicate_json_keys(json_str: &str) -> Result<()> {
    let mut chars = json_str.chars().peekable();
    skip_whitespace(&mut chars);
    parse_json_value(&mut chars)?;
    skip_whitespace(&mut chars);
    if chars.next().is_some() {
        bail!("trailing_characters_in_json");
    }
    Ok(())
}

fn skip_whitespace(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

fn parse_json_string(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<String> {
    if chars.next() != Some('"') {
        bail!("expected_quote");
    }
    let mut s = String::new();
    while let Some(c) = chars.next() {
        if c == '"' {
            return Ok(s);
        }
        if c == '\\' {
            let esc = chars
                .next()
                .ok_or_else(|| anyhow::anyhow!("unterminated_escape"))?;
            s.push('\\');
            s.push(esc);
        } else {
            s.push(c);
        }
    }
    bail!("unterminated_string");
}

fn parse_json_value(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<()> {
    skip_whitespace(chars);
    match chars.peek() {
        Some('{') => parse_json_object(chars),
        Some('[') => parse_json_array(chars),
        Some('"') => {
            let _ = parse_json_string(chars)?;
            Ok(())
        }
        Some(_) => {
            while let Some(&c) = chars.peek() {
                if c == ',' || c == '}' || c == ']' || c.is_whitespace() {
                    break;
                }
                chars.next();
            }
            Ok(())
        }
        None => bail!("unexpected_eof"),
    }
}

fn parse_json_object(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<()> {
    if chars.next() != Some('{') {
        bail!("expected_open_brace");
    }
    let mut seen_keys = std::collections::HashSet::new();
    loop {
        skip_whitespace(chars);
        match chars.peek() {
            Some('}') => {
                chars.next();
                return Ok(());
            }
            Some('"') => {
                let key = parse_json_string(chars)?;
                if !seen_keys.insert(key.clone()) {
                    bail!(
                        "duplicate_json_key_rejected: key '{}' declared more than once",
                        key
                    );
                }
                skip_whitespace(chars);
                if chars.next() != Some(':') {
                    bail!("expected_colon_after_key");
                }
                parse_json_value(chars)?;
                skip_whitespace(chars);
                match chars.peek() {
                    Some(',') => {
                        chars.next();
                    }
                    Some('}') => {
                        chars.next();
                        return Ok(());
                    }
                    _ => bail!("expected_comma_or_close_brace"),
                }
            }
            _ => bail!("expected_key_or_close_brace"),
        }
    }
}

fn parse_json_array(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<()> {
    if chars.next() != Some('[') {
        bail!("expected_open_bracket");
    }
    loop {
        skip_whitespace(chars);
        match chars.peek() {
            Some(']') => {
                chars.next();
                return Ok(());
            }
            _ => {
                parse_json_value(chars)?;
                skip_whitespace(chars);
                match chars.peek() {
                    Some(',') => {
                        chars.next();
                    }
                    Some(']') => {
                        chars.next();
                        return Ok(());
                    }
                    _ => bail!("expected_comma_or_close_bracket"),
                }
            }
        }
    }
}
