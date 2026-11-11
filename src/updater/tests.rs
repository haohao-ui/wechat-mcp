use super::executor::*;
use super::journal::*;
use super::manifest::*;
use super::rollback::*;
use super::signing::*;
use super::unpack::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

struct TempTestDir(PathBuf);
impl TempTestDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "wechat_updater_test_{}_{}",
            name,
            uuid::Uuid::new_v4()
        ));
        let _ = fs::create_dir_all(&p);
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        Self(p)
    }
}
impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn generate_test_ed25519_key() -> (ring::signature::Ed25519KeyPair, Vec<u8>) {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8_doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let key_pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8_doc.as_ref()).unwrap();
    use ring::signature::KeyPair;
    let pub_key_bytes = key_pair.public_key().as_ref().to_vec();
    (key_pair, pub_key_bytes)
}

fn sign_manifest(key_pair: &ring::signature::Ed25519KeyPair, manifest: &ReleaseManifest) -> String {
    let manifest_val = serde_json::to_value(manifest).unwrap();
    let canonical = serde_json_canonicalizer::to_vec(&manifest_val).unwrap();
    let mut message = Vec::with_capacity(MANIFEST_DOMAIN_PREFIX.len() + canonical.len());
    message.extend_from_slice(MANIFEST_DOMAIN_PREFIX);
    message.extend_from_slice(&canonical);

    let sig = key_pair.sign(&message);
    hex_encode(sig.as_ref())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn calculate_sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

fn sample_manifest(sequence: u64, arch: &str) -> ReleaseManifest {
    ReleaseManifest {
        manifest_schema: 1,
        release_id: format!("0.2.0-{}", sequence),
        sequence,
        channel: "stable".into(),
        published_at: 1794297600,
        expires_at: 1825833600,
        target: TargetSpec {
            os: "macos".into(),
            arch: arch.into(),
            min_os_version: "12.0".into(),
            max_os_version: None,
        },
        artifact: ArtifactSpec {
            filename: "wechat-mcp-test.tar.gz".into(),
            sha256: "0000000000000000000000000000000000000000000000000000000000000000".into(),
            size_bytes: 1024,
            max_unpacked_bytes: 100000,
            max_file_count: 50,
        },
        components: vec![ComponentSpec {
            path: "bin/wechat-mcp".into(),
            kind: "executable".into(),
            file_mode: 0o755,
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            size_bytes: 0,
        }],
        compatibility: CompatibilitySpec {
            config_schema_min: 1,
            config_schema_max: 2,
            profile_schema_min: 1,
            profile_schema_max: 2,
            history_schema_min: 1,
            history_schema_max: 5,
            min_updater_version: 1,
            in_place_rollback_supported: true,
        },
        release_notes: "Test release notes".into(),
    }
}

#[test]
fn test_manifest_signing_and_verification_domain_separated() {
    let (key_pair, pub_bytes) = generate_test_ed25519_key();
    let manifest = sample_manifest(1005, "arm64");
    let sig = sign_manifest(&key_pair, &manifest);

    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig,
        manifest: manifest.clone(),
    };

    let store = TrustedKeyStore::new().with_key("test-root-v1", &pub_bytes);
    assert!(store.verify_envelope(&envelope).is_ok());

    // 篡改测试 1：修改 manifest 中的 release_id
    let mut tampered_manifest = manifest.clone();
    tampered_manifest.release_id = "0.2.0-tampered".into();
    let mut tampered_envelope = envelope.clone();
    tampered_envelope.manifest = tampered_manifest;
    let err = store.verify_envelope(&tampered_envelope).unwrap_err();
    assert!(err.to_string().contains("signature_verification_failed"));

    // 篡改测试 2：修改 signature
    let mut tampered_sig_env = envelope.clone();
    tampered_sig_env.signature = "a".repeat(128);
    let err2 = store.verify_envelope(&tampered_sig_env).unwrap_err();
    assert!(err2.to_string().contains("signature_verification_failed"));

    // 未知 key_id 测试
    let unknown_key_env = SignedEnvelope {
        key_id: "unknown-hacker-key".into(),
        ..envelope.clone()
    };
    let err3 = store.verify_envelope(&unknown_key_env).unwrap_err();
    assert!(err3.to_string().contains("untrusted_key_id"));
}

#[test]
fn test_manifest_compatibility_and_downgrade_prevention() {
    let manifest = sample_manifest(1005, "arm64");

    // 正常通过（arm64 与 aarch64 等价兼容）
    assert!(validate_manifest_compatibility(&manifest, "aarch64", "13.2.1", 1004).is_ok());
    assert!(validate_manifest_compatibility(&manifest, "arm64", "13.2.1", 1004).is_ok());

    // 架构不符拒绝
    let err_arch =
        validate_manifest_compatibility(&manifest, "x86_64", "13.2.1", 1004).unwrap_err();
    assert!(err_arch.to_string().contains("architecture_mismatch"));

    // 系统版本过低拒绝
    let err_os = validate_manifest_compatibility(&manifest, "arm64", "11.5", 1004).unwrap_err();
    assert!(err_os.to_string().contains("os_version_too_low"));

    // 同 sequence 识别已安装（拒绝再次作为新升级执行）
    let err_same = validate_manifest_compatibility(&manifest, "arm64", "13.2.1", 1005).unwrap_err();
    assert!(err_same.to_string().contains("sequence_already_installed"));

    // 防降级拒绝：candidate sequence (1005) < highest accepted (1010)
    let err_down = validate_manifest_compatibility(&manifest, "arm64", "13.2.1", 1010).unwrap_err();
    assert!(err_down.to_string().contains("downgrade_rejected"));
}

#[test]
fn test_safe_unpack_and_verify_valid_and_adversarial() {
    let temp = TempTestDir::new("unpack_adversarial");
    let staging = temp.0.join("staging");
    fs::create_dir_all(&staging).unwrap();

    let valid_archive = temp.0.join("valid.tar.gz");
    let content = b"hello binary content";
    let content_sha = calculate_sha256_bytes(content);

    // 构造合法的 tar.gz 归档
    {
        let file = fs::File::create(&valid_archive).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "bin/wechat-mcp", &content[..])
            .unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }

    let meta = fs::metadata(&valid_archive).unwrap();
    let archive_sha = calculate_sha256_bytes(&fs::read(&valid_archive).unwrap());

    let artifact = ArtifactSpec {
        filename: "valid.tar.gz".into(),
        sha256: archive_sha.clone(),
        size_bytes: meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };

    let components = vec![ComponentSpec {
        path: "bin/wechat-mcp".into(),
        kind: "executable".into(),
        file_mode: 0o755,
        sha256: content_sha,
        size_bytes: content.len() as u64,
    }];

    // 1. 验证正常解包通过
    let stage_valid = temp.0.join("stage_valid");
    let res = safe_unpack_and_verify(&valid_archive, &stage_valid, &artifact, &components);
    assert!(
        res.is_ok(),
        "Expected valid unpack to succeed: {:?}",
        res.err()
    );

    // 2. 对抗包测试：软链接注入拒绝
    let symlink_archive = temp.0.join("symlink_evil.tar.gz");
    {
        let file = fs::File::create(&symlink_archive).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder
            .append_link(&mut header, "bin/evil_symlink", "/etc/passwd")
            .unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let sym_meta = fs::metadata(&symlink_archive).unwrap();
    let sym_sha = calculate_sha256_bytes(&fs::read(&symlink_archive).unwrap());
    let sym_artifact = ArtifactSpec {
        filename: "symlink_evil.tar.gz".into(),
        sha256: sym_sha,
        size_bytes: sym_meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };
    let sym_res = safe_unpack_and_verify(&symlink_archive, &staging, &sym_artifact, &components);
    assert!(sym_res.is_err());
    assert!(
        sym_res
            .unwrap_err()
            .to_string()
            .contains("symlink_rejected")
    );

    // 3. 对抗包测试：路径穿越逃逸拒绝 (../escaped)
    let traversal_archive = temp.0.join("traversal_evil.tar.gz");
    {
        let file = fs::File::create(&traversal_archive).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        let evil_name = b"../evil_escape\0";
        header.as_mut_bytes()[..evil_name.len()].copy_from_slice(evil_name);
        header.set_cksum();
        builder.append(&header, &content[..]).unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let tr_meta = fs::metadata(&traversal_archive).unwrap();
    let tr_sha = calculate_sha256_bytes(&fs::read(&traversal_archive).unwrap());
    let tr_artifact = ArtifactSpec {
        filename: "traversal_evil.tar.gz".into(),
        sha256: tr_sha,
        size_bytes: tr_meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };
    let tr_res = safe_unpack_and_verify(&traversal_archive, &staging, &tr_artifact, &components);
    assert!(tr_res.is_err());
    assert!(
        tr_res
            .unwrap_err()
            .to_string()
            .contains("invalid_relative_path")
    );

    // 4. 对抗包测试：大小写冲突攻击
    let collision_archive = temp.0.join("collision_evil.tar.gz");
    {
        let file = fs::File::create(&collision_archive).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut h1 = tar::Header::new_gnu();
        h1.set_size(content.len() as u64);
        h1.set_mode(0o755);
        h1.set_cksum();
        builder
            .append_data(&mut h1, "bin/wechat-mcp", &content[..])
            .unwrap();

        let mut h2 = tar::Header::new_gnu();
        h2.set_size(content.len() as u64);
        h2.set_mode(0o755);
        h2.set_cksum();
        builder
            .append_data(&mut h2, "bin/WeChat-MCP", &content[..])
            .unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let col_meta = fs::metadata(&collision_archive).unwrap();
    let col_sha = calculate_sha256_bytes(&fs::read(&collision_archive).unwrap());
    let col_artifact = ArtifactSpec {
        filename: "collision_evil.tar.gz".into(),
        sha256: col_sha,
        size_bytes: col_meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };
    let stage_col = temp.0.join("stage_col");
    let col_res =
        safe_unpack_and_verify(&collision_archive, &stage_col, &col_artifact, &components);
    assert!(col_res.is_err());
    let err_str = col_res.unwrap_err().to_string();
    assert!(
        err_str.contains("case_collision_detected"),
        "Actual error was: {}",
        err_str
    );

    // 5. 对抗包测试：解压大小炸弹攻击 (超出 max_unpacked_bytes)
    let bomb_artifact = ArtifactSpec {
        filename: "valid.tar.gz".into(),
        sha256: archive_sha.clone(),
        size_bytes: meta.len(),
        max_unpacked_bytes: 5, // 仅允许 5 字节，文件为 20 字节
        max_file_count: 10,
    };
    let bomb_res = safe_unpack_and_verify(&valid_archive, &staging, &bomb_artifact, &components);
    assert!(bomb_res.is_err());
    assert!(
        bomb_res
            .unwrap_err()
            .to_string()
            .contains("unpacked_size_limit_exceeded")
    );

    // 6. 对抗包测试：条目数超限
    let count_bomb_artifact = ArtifactSpec {
        filename: "valid.tar.gz".into(),
        sha256: archive_sha.clone(),
        size_bytes: meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 0, // 允许 0 个文件
    };
    let count_res =
        safe_unpack_and_verify(&valid_archive, &staging, &count_bomb_artifact, &components);
    assert!(count_res.is_err());
    assert!(
        count_res
            .unwrap_err()
            .to_string()
            .contains("file_count_limit_exceeded")
    );

    // 7. 对抗包测试：未在 components 清单声明的未知文件
    let unknown_archive = temp.0.join("unknown_evil.tar.gz");
    {
        let file = fs::File::create(&unknown_archive).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut h1 = tar::Header::new_gnu();
        h1.set_size(content.len() as u64);
        h1.set_mode(0o755);
        h1.set_cksum();
        builder
            .append_data(&mut h1, "bin/unknown_backdoor", &content[..])
            .unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let unk_meta = fs::metadata(&unknown_archive).unwrap();
    let unk_sha = calculate_sha256_bytes(&fs::read(&unknown_archive).unwrap());
    let unk_artifact = ArtifactSpec {
        filename: "unknown_evil.tar.gz".into(),
        sha256: unk_sha,
        size_bytes: unk_meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };
    let unk_res = safe_unpack_and_verify(&unknown_archive, &staging, &unk_artifact, &components);
    assert!(unk_res.is_err());
    assert!(
        unk_res
            .unwrap_err()
            .to_string()
            .contains("unexpected_file_in_archive")
    );

    // 8. 对抗包测试：非法权限位 (setuid)
    let setuid_archive = temp.0.join("setuid_evil.tar.gz");
    {
        let file = fs::File::create(&setuid_archive).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut h1 = tar::Header::new_gnu();
        h1.set_size(content.len() as u64);
        h1.set_mode(0o4755); // setuid
        h1.set_cksum();
        builder
            .append_data(&mut h1, "bin/wechat-mcp", &content[..])
            .unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let su_meta = fs::metadata(&setuid_archive).unwrap();
    let su_sha = calculate_sha256_bytes(&fs::read(&setuid_archive).unwrap());
    let su_artifact = ArtifactSpec {
        filename: "setuid_evil.tar.gz".into(),
        sha256: su_sha,
        size_bytes: su_meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };
    let su_res = safe_unpack_and_verify(&setuid_archive, &staging, &su_artifact, &components);
    assert!(su_res.is_err());
    assert!(
        su_res
            .unwrap_err()
            .to_string()
            .contains("forbidden_special_permissions")
    );

    // 9. 对抗包测试：尾部附加非零垃圾数据拒绝
    let trailing_archive = temp.0.join("trailing_garbage.tar.gz");
    {
        let mut gz_bytes = fs::read(&valid_archive).unwrap();
        // 尾部追加非零垃圾数据
        gz_bytes.extend_from_slice(b"NON_ZERO_TRAILING_GARBAGE_BYTES");
        fs::write(&trailing_archive, &gz_bytes).unwrap();
    }
    let tg_meta = fs::metadata(&trailing_archive).unwrap();
    let tg_sha = calculate_sha256_bytes(&fs::read(&trailing_archive).unwrap());
    let tg_artifact = ArtifactSpec {
        filename: "trailing_garbage.tar.gz".into(),
        sha256: tg_sha,
        size_bytes: tg_meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };
    let stage_tg = temp.0.join("stage_tg");
    let tg_res = safe_unpack_and_verify(&trailing_archive, &stage_tg, &tg_artifact, &components);
    assert!(tg_res.is_err());
    let tg_err = tg_res.unwrap_err().to_string();
    assert!(
        tg_err.contains("trailing_data_in_archive") || tg_err.contains("gzip_tail_read_error"),
        "Unexpected error: {}",
        tg_err
    );

    // 10. 对抗包测试：非 UTF-8 路径拒绝 (严禁使用 to_string_lossy)
    let non_utf8_archive = temp.0.join("non_utf8_evil.tar.gz");
    {
        let file = fs::File::create(&non_utf8_archive).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        // 填入包含非 UTF-8 序列的原始字节 (0xFF, 0xFE)
        let evil_raw_name = b"bin/bad_\xFF\xFE_name\0";
        header.as_mut_bytes()[..evil_raw_name.len()].copy_from_slice(evil_raw_name);
        header.set_cksum();
        builder.append(&header, &content[..]).unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let nu_meta = fs::metadata(&non_utf8_archive).unwrap();
    let nu_sha = calculate_sha256_bytes(&fs::read(&non_utf8_archive).unwrap());
    let nu_artifact = ArtifactSpec {
        filename: "non_utf8_evil.tar.gz".into(),
        sha256: nu_sha,
        size_bytes: nu_meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 10,
    };
    let stage_nu = temp.0.join("stage_nu");
    let nu_res = safe_unpack_and_verify(&non_utf8_archive, &stage_nu, &nu_artifact, &components);
    assert!(nu_res.is_err());
    assert!(
        nu_res
            .unwrap_err()
            .to_string()
            .contains("invalid_utf8_in_entry_path")
    );

    // 11. 对抗测试：解压目标祖先目录被替换为 symlink 拒绝
    let stage_sym_ancestor = temp.0.join("stage_sym_anc");
    fs::create_dir_all(&stage_sym_ancestor).unwrap();
    let real_other = temp.0.join("other_dir");
    fs::create_dir_all(&real_other).unwrap();
    // 将 bin 目录软链接指向外部目录
    std::os::unix::fs::symlink(&real_other, stage_sym_ancestor.join("bin")).unwrap();
    let anc_res =
        safe_unpack_and_verify(&valid_archive, &stage_sym_ancestor, &artifact, &components);
    assert!(anc_res.is_err());
    assert!(
        anc_res
            .unwrap_err()
            .to_string()
            .contains("symlink_in_directory_path")
    );
}

#[test]
fn test_safe_verify_staged_dir_rejects_symlink_and_hardlink() {
    let temp = TempTestDir::new("staged_links");
    let staging = temp.0.join("staging");
    fs::create_dir_all(staging.join("bin")).unwrap();

    let target_file = staging.join("bin/real");
    fs::write(&target_file, b"content").unwrap();

    // 1. 软链接拒绝
    let sym = staging.join("bin/sym");
    std::os::unix::fs::symlink(&target_file, &sym).unwrap();

    let artifact = ArtifactSpec {
        filename: "test.tar.gz".into(),
        sha256: "00".repeat(32),
        size_bytes: 100,
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };

    let res_sym = safe_verify_staged_dir(&staging, &artifact, &[]);
    assert!(res_sym.is_err());
    assert!(
        res_sym
            .unwrap_err()
            .to_string()
            .contains("symlink_rejected")
    );

    let _ = fs::remove_file(&sym);

    // 2. 硬链接拒绝（nlink > 1）
    let hard = staging.join("bin/hard");
    fs::hard_link(&target_file, &hard).unwrap();
    let res_hard = safe_verify_staged_dir(&staging, &artifact, &[]);
    assert!(res_hard.is_err());
    assert!(
        res_hard
            .unwrap_err()
            .to_string()
            .contains("hardlink_rejected")
    );
}

#[test]
fn test_safe_verify_staged_dir_rejects_case_and_client_hard_limits() {
    let temp = TempTestDir::new("case_and_limits");
    let staging = temp.0.join("staging");
    fs::create_dir_all(staging.join("bin")).unwrap();

    let f1 = staging.join("bin/file1");
    fs::write(&f1, b"hello payload").unwrap();

    // 1. 超过文件大小硬上限
    let artifact_size_limit = ArtifactSpec {
        filename: "test.tar.gz".into(),
        sha256: "00".repeat(32),
        size_bytes: 100,
        max_unpacked_bytes: 5, // 仅允许 5 字节，payload 是 13 字节
        max_file_count: 10,
    };
    let res_size = safe_verify_staged_dir(&staging, &artifact_size_limit, &[]);
    assert!(res_size.is_err());
    assert!(
        res_size
            .unwrap_err()
            .to_string()
            .contains("unpacked_size_limit_exceeded")
    );

    // 2. 超过文件数量硬上限
    let f2 = staging.join("bin/file2");
    fs::write(&f2, b"extra").unwrap();
    let artifact_count_limit = ArtifactSpec {
        filename: "test.tar.gz".into(),
        sha256: "00".repeat(32),
        size_bytes: 100,
        max_unpacked_bytes: 100000,
        max_file_count: 1, // 仅允许 1 个文件，staging 现有 2 个文件
    };
    let res_count = safe_verify_staged_dir(&staging, &artifact_count_limit, &[]);
    assert!(res_count.is_err());
    assert!(
        res_count
            .unwrap_err()
            .to_string()
            .contains("file_count_limit_exceeded")
    );

    // 3. 组件白名单 case 冲突
    let artifact_ok = ArtifactSpec {
        filename: "test.tar.gz".into(),
        sha256: "00".repeat(32),
        size_bytes: 100,
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };
    let case_conflict_comps = vec![
        ComponentSpec {
            path: "bin/file1".into(),
            kind: "file".into(),
            sha256: "00".repeat(32),
            size_bytes: 13,
            file_mode: 0o755,
        },
        ComponentSpec {
            path: "bin/FILE1".into(),
            kind: "file".into(),
            sha256: "00".repeat(32),
            size_bytes: 13,
            file_mode: 0o755,
        },
    ];
    let res_case = safe_verify_staged_dir(&staging, &artifact_ok, &case_conflict_comps);
    assert!(res_case.is_err());
    assert!(
        res_case
            .unwrap_err()
            .to_string()
            .contains("case_collision_detected")
    );

    // 4. 组件白名单非 NFC 规范化检查
    let non_nfc_path = "bin/caf\u{0065}\u{0301}"; // e + combining acute (NFD)
    let non_nfc_comps = vec![ComponentSpec {
        path: non_nfc_path.into(),
        kind: "file".into(),
        sha256: "00".repeat(32),
        size_bytes: 10,
        file_mode: 0o644,
    }];
    let res_nfc = safe_verify_staged_dir(&staging, &artifact_ok, &non_nfc_comps);
    assert!(res_nfc.is_err());
    assert!(
        res_nfc
            .unwrap_err()
            .to_string()
            .contains("unicode_normalization_rejected")
    );
}

#[test]
fn test_safe_verify_staged_dir_valid_fixture_and_whitelist() {
    let temp = TempTestDir::new("verify_valid");
    let staging = temp.0.join("staging");
    let bin_dir = staging.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();

    let bin_file = bin_dir.join("wechat-mcp");
    fs::write(&bin_file, b"executable-payload").unwrap();
    fs::set_permissions(&bin_file, fs::Permissions::from_mode(0o755)).unwrap();

    let file_sha = {
        let bytes = fs::read(&bin_file).unwrap();
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(&bytes);
        hex_encode(&h.finalize())
    };

    let comp = ComponentSpec {
        path: "bin/wechat-mcp".into(),
        kind: "executable".into(),
        file_mode: 0o755,
        sha256: file_sha,
        size_bytes: 18,
    };

    let artifact = ArtifactSpec {
        filename: "test.tar.gz".into(),
        sha256: "00".repeat(32),
        size_bytes: 100,
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };

    let res = safe_verify_staged_dir(&staging, &artifact, &[comp.clone()]);
    assert!(res.is_ok());

    // 多余未列文件测试
    let extra = bin_dir.join("extra_unlisted");
    fs::write(&extra, b"extra").unwrap();
    let res_extra = safe_verify_staged_dir(&staging, &artifact, &[comp]);
    assert!(res_extra.is_err());
    assert!(
        res_extra
            .unwrap_err()
            .to_string()
            .contains("unexpected_file")
    );
}

#[test]
fn test_journal_atomic_persistence_and_recovery_evaluation() {
    let temp = TempTestDir::new("journal_test");
    let state_dir = temp.0.join("state");

    let mut journal = UpdateJournal::new("tx-1234", "0.1.0", "0.2.0", 1002, 1700000000);
    assert_eq!(journal.stage, TransactionStage::Staged);

    // 持久化写
    journal.persist_atomic(&state_dir).unwrap();
    let loaded = UpdateJournal::load_from_state(&state_dir).unwrap().unwrap();
    assert_eq!(loaded.transaction_id, "tx-1234");
    assert_eq!(loaded.stage, TransactionStage::Staged);

    let current_link = temp.0.join("current");
    let releases_dir = temp.0.join("releases");
    let act = loaded.evaluate_recovery(&releases_dir, &current_link);
    assert_eq!(act, RecoveryAction::DiscardCandidate);

    // Switching 阶段恢复测试：精确匹配 vs 未知 current fail-closed
    journal.stage = TransactionStage::Switching;
    journal.persist_atomic(&state_dir).unwrap();

    // 1) current 指向 target (0.2.0) -> TestCandidate
    let rel_target = releases_dir.join("0.2.0");
    fs::create_dir_all(&rel_target).unwrap();
    let _ = fs::remove_file(&current_link);
    std::os::unix::fs::symlink(&rel_target, &current_link).unwrap();
    let act_switch_target = journal.evaluate_recovery(&releases_dir, &current_link);
    assert_eq!(act_switch_target, RecoveryAction::TestCandidate);

    // 2) current 指向 source (0.1.0) -> ResumeOldRelease
    let rel_source = releases_dir.join("0.1.0");
    fs::create_dir_all(&rel_source).unwrap();
    let _ = fs::remove_file(&current_link);
    std::os::unix::fs::symlink(&rel_source, &current_link).unwrap();
    let act_switch_source = journal.evaluate_recovery(&releases_dir, &current_link);
    assert_eq!(act_switch_source, RecoveryAction::ResumeOldRelease);

    // 3) current 指向未知目录 (0.9.9) -> ManualInterventionRequired (严禁当旧版默认安全)
    let rel_unknown = releases_dir.join("0.9.9");
    fs::create_dir_all(&rel_unknown).unwrap();
    let _ = fs::remove_file(&current_link);
    std::os::unix::fs::symlink(&rel_unknown, &current_link).unwrap();
    let act_switch_unknown = journal.evaluate_recovery(&releases_dir, &current_link);
    match act_switch_unknown {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(msg.contains("unknown_current_target"));
        }
        other => panic!("expected ManualInterventionRequired, got {:?}", other),
    }

    journal.stage = TransactionStage::CandidateTesting;
    journal.persist_atomic(&state_dir).unwrap();
    let act2 = journal.evaluate_recovery(&releases_dir, &current_link);
    assert_eq!(act2, RecoveryAction::TestCandidate);

    journal.stage = TransactionStage::RecoveryRequired;
    journal.error = Some("new business data conflict".into());
    journal.persist_atomic(&state_dir).unwrap();
    let act3 = journal.evaluate_recovery(&releases_dir, &current_link);
    match act3 {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(msg.contains("new business data conflict"));
        }
        _ => panic!("expected manual intervention"),
    }

    // 验证损坏的 journal 能够返回 Err（不伪装空闲）
    fs::write(state_dir.join("update-journal.json"), b"corrupted-non-json").unwrap();
    assert!(UpdateJournal::load_from_state(&state_dir).is_err());
}

#[test]
fn test_update_lock_exclusive_and_release_metadata() {
    let temp = TempTestDir::new("lock_and_meta");
    let state_dir = temp.0.clone();

    // 1. 排他锁测试
    let guard1 = acquire_update_lock(&state_dir).unwrap();
    let err_lock2 = acquire_update_lock(&state_dir).unwrap_err();
    assert!(
        err_lock2
            .to_string()
            .contains("concurrent_update_in_progress")
    );
    drop(guard1);

    // 释放后可再次获取
    let _guard3 = acquire_update_lock(&state_dir).unwrap();

    // 2. 单一权威 Release 提交记录原子保存与恢复测试
    let record = CurrentReleaseCommitRecord {
        record_schema: 1,
        tx_id: "tx-test-12345".into(),
        release_id: "0.2.0-test".into(),
        sequence: 1002,
        channel: "test".into(),
        manifest_digest: "sha256-test-digest".into(),
        installed_at: 1794368000,
    };
    atomic_commit_release_record(&state_dir, &record).unwrap();
    let loaded_record = load_current_release_record(&state_dir).unwrap().unwrap();
    assert_eq!(loaded_record, record);

    // 且水位同时原子同步更新
    assert_eq!(load_highest_accepted_sequence(&state_dir).unwrap(), 1002);

    // 3. 结构化维护态标记设置与精确 tx 清除测试
    let test_tx_uuid = uuid::Uuid::new_v4();
    let valid_tx_id = format!("tx-{}", test_tx_uuid);
    set_structured_maintenance_marker(
        &state_dir,
        &valid_tx_id,
        "test maintenance reason",
        1794368000,
    )
    .unwrap();
    assert!(state_dir.join("core.maintenance.json").exists());
    assert!(state_dir.join("core.maintenance").exists());

    // 尝试用错误的 tx_id 清除维护态，必须被拒绝！
    let wrong_tx_id = format!("tx-{}", uuid::Uuid::new_v4());
    let err_clear = clear_maintenance_for_tx(&state_dir, &wrong_tx_id).unwrap_err();
    assert!(
        err_clear
            .to_string()
            .contains("maintenance_marker_tx_mismatch")
            || err_clear
                .to_string()
                .contains("maintenance_marker_uuid_mismatch")
    );
    assert!(state_dir.join("core.maintenance.json").exists());

    // 用匹配的 tx_id 清除维护态，成功清除
    clear_maintenance_for_tx(&state_dir, &valid_tx_id).unwrap();
    assert!(!state_dir.join("core.maintenance.json").exists());
    assert!(!state_dir.join("core.maintenance").exists());
}

#[test]
fn test_sequence_corrupt_file_fails_closed() {
    let temp = TempTestDir::new("seq_corrupt");
    let state_dir = temp.0.clone();

    // 默认不存在时返回 0
    assert_eq!(load_highest_accepted_sequence(&state_dir).unwrap(), 0);

    // 正常保存与读取
    save_highest_accepted_sequence(&state_dir, 1005).unwrap();
    assert_eq!(load_highest_accepted_sequence(&state_dir).unwrap(), 1005);

    // 文件损坏（非整数）必须返回 Err，严禁重置为 0
    fs::write(state_dir.join("highest_accepted_sequence"), b"bad-sequence").unwrap();
    let err = load_highest_accepted_sequence(&state_dir).unwrap_err();
    assert!(
        err.to_string()
            .contains("corrupt_highest_accepted_sequence_file")
    );
}

#[test]
fn test_has_new_data_strict_errors_on_missing_corrupt_or_unknown_schema() {
    let temp = TempTestDir::new("db_protection_strict");

    // 1. 数据库文件不存在必须返回 Err
    let missing_db = temp.0.join("non_existent.sqlite3");
    let err_missing = has_new_data_since_backup(&missing_db, 1000).unwrap_err();
    assert!(err_missing.to_string().contains("database_file_not_found"));

    // 2. 数据库文件损坏（非 sqlite 格式）必须返回 Err 并拒绝回退
    let corrupt_db = temp.0.join("corrupt.sqlite3");
    fs::write(&corrupt_db, b"not a sqlite database garbage bytes").unwrap();
    let err_corrupt = has_new_data_since_backup(&corrupt_db, 1000).unwrap_err();
    assert!(
        err_corrupt
            .to_string()
            .contains("db_integrity_check_failed")
            || err_corrupt.to_string().contains("cannot_open_db")
    );

    // 3. 数据库存在但缺少 messages 表（未知 schema）必须返回 Err 并拒绝回退
    let unknown_schema_db = temp.0.join("unknown.sqlite3");
    {
        let conn = rusqlite::Connection::open(&unknown_schema_db).unwrap();
        conn.execute_batch("CREATE TABLE other_table (id INT);")
            .unwrap();
    }
    let err_schema = has_new_data_since_backup(&unknown_schema_db, 1000).unwrap_err();
    assert!(
        err_schema
            .to_string()
            .contains("unknown_schema_missing_messages_table")
    );

    // 4. 合法 schema 且存在真实 messages 表
    let valid_db = temp.0.join("valid.sqlite3");
    {
        let conn = rusqlite::Connection::open(&valid_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY, created_at INTEGER, body TEXT);
             INSERT INTO messages (id, created_at, body) VALUES (1, 1000, 'old message');",
        )
        .unwrap();
    }
    // backup_time 为 1000：无新数据，返回 Ok(false)
    assert!(!has_new_data_since_backup(&valid_db, 1000).unwrap());

    // 产生新数据（created_at = 2000）
    {
        let conn = rusqlite::Connection::open(&valid_db).unwrap();
        conn.execute(
            "INSERT INTO messages (id, created_at, body) VALUES (2, 2000, 'new message');",
            [],
        )
        .unwrap();
    }
    // 检测到新数据，返回 Ok(true)
    assert!(has_new_data_since_backup(&valid_db, 1000).unwrap());
}

#[test]
fn test_local_signer_failure_simulation() {
    let temp = TempTestDir::new("signer_test");
    let bin = temp.0.join("sample_bin");
    fs::write(&bin, b"bin").unwrap();

    let signer = LocalSigner::new("WeChat CS Local Signing").with_simulation("keychain_locked");

    let res = signer.sign_and_verify(&bin);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("keychain_locked"));
}

#[test]
fn test_duplicate_json_keys_rejected() {
    // 1. 顶层重复键
    let dup_top = r#"{"envelope_schema": 1, "envelope_schema": 2, "key_id": "k1", "signature_algorithm": "ed25519", "signature": "00", "manifest": {}}"#;
    let err_top = parse_signed_envelope(dup_top.as_bytes()).unwrap_err();
    assert!(
        err_top.to_string().contains("duplicate_json_key_rejected"),
        "expected duplicate key rejection, got: {err_top}"
    );

    // 2. 嵌套 manifest 重复键
    let dup_nested = r#"{
        "envelope_schema": 1,
        "key_id": "k1",
        "signature_algorithm": "ed25519",
        "signature": "00",
        "manifest": {
            "sequence": 1,
            "sequence": 2
        }
    }"#;
    let err_nested = parse_signed_envelope(dup_nested.as_bytes()).unwrap_err();
    assert!(
        err_nested
            .to_string()
            .contains("duplicate_json_key_rejected"),
        "expected duplicate key rejection in nested object, got: {err_nested}"
    );
}

#[test]
fn test_database_auto_rollback_is_unsupported() {
    let temp = TempTestDir::new("db_rollback_guard");
    let backup_file = temp.0.join("backup.sqlite3");
    let target_db = temp.0.join("target.sqlite3");
    let res = restore_database_backup_guarded(&backup_file, &target_db);
    assert!(res.is_err());
    assert!(
        res.unwrap_err()
            .to_string()
            .contains("unsupported_db_rollback")
    );
}

fn write_valid_bot_config(cfg_path: &PathBuf, state_dir: &PathBuf) {
    let val = serde_json::json!({
        "schema_version": 1,
        "host": "local",
        "account_id": "test-account",
        "allowlist": [{"recipient": "午夜咖啡", "kind": "private"}],
        "state_dir": state_dir.to_str().unwrap()
    });
    fs::write(cfg_path, serde_json::to_string_pretty(&val).unwrap()).unwrap();
    fs::set_permissions(cfg_path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn test_cli_rejects_test_pubkey_and_mutations_without_side_effects() {
    let temp = TempTestDir::new("cli_side_effects");
    let state_dir = temp.0.join("state");
    fs::create_dir_all(&state_dir).unwrap();
    let cfg_file = temp.0.join("config.json");
    write_valid_bot_config(&cfg_file, &state_dir);

    let manifest_file = temp.0.join("manifest.json");
    let (key_pair, pub_key_bytes) = generate_test_ed25519_key();
    let manifest = sample_manifest(1001, "arm64");
    let sig = sign_manifest(&key_pair, &manifest);
    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig,
        manifest,
    };
    fs::write(&manifest_file, serde_json::to_vec(&envelope).unwrap()).unwrap();

    let exe = std::env::var("CARGO_BIN_EXE_wechat-updater")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/wechat-updater")
        });

    // 1. 验证拒绝 --test-pubkey-hex 选项（退出码 2，unrecognized argument）
    let output_flag = std::process::Command::new(&exe)
        .args([
            "check",
            "--config",
            cfg_file.to_str().unwrap(),
            "--manifest",
            manifest_file.to_str().unwrap(),
            "--test-pubkey-hex",
            &hex_encode(&pub_key_bytes),
        ])
        .output()
        .unwrap();
    assert_eq!(output_flag.status.code(), Some(2));
    let stderr_flag = String::from_utf8_lossy(&output_flag.stderr);
    assert!(stderr_flag.contains("unrecognized argument"));

    // 2. 验证生产 check 无可信根时 fail closed（退出码 1，报 trust_root_unconfigured）
    let output_check = std::process::Command::new(&exe)
        .args([
            "check",
            "--config",
            cfg_file.to_str().unwrap(),
            "--manifest",
            manifest_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output_check.status.code(), Some(1));
    let stderr_check = String::from_utf8_lossy(&output_check.stderr);
    assert!(stderr_check.contains("trust_root_unconfigured"));

    // 3. 验证 apply / rollback / recover 返回 unsupported 且不产生副作用
    for action in &["apply", "rollback", "recover"] {
        let entries_before: Vec<_> = fs::read_dir(&temp.0).unwrap().collect();
        let output_act = std::process::Command::new(&exe)
            .args([
                action,
                "--config",
                cfg_file.to_str().unwrap(),
                "--manifest",
                manifest_file.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(output_act.status.code(), Some(1));
        let stderr_act = String::from_utf8_lossy(&output_act.stderr);
        assert!(stderr_act.contains("unsupported"));

        // 验证文件系统无新文件或变更
        let entries_after: Vec<_> = fs::read_dir(&temp.0).unwrap().collect();
        assert_eq!(entries_before.len(), entries_after.len());
        assert!(!state_dir.join("core.maintenance").exists());
        assert!(!temp.0.join("current").exists());
    }
}

#[test]
fn test_cli_rejects_invalid_or_missing_config_without_side_effects() {
    let temp = TempTestDir::new("cli_invalid_config");
    let exe = std::env::var("CARGO_BIN_EXE_wechat-updater")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/wechat-updater")
        });

    let entries_before: Vec<_> = fs::read_dir(&temp.0).unwrap().collect();

    // 1. 配置文件不存在必须退出码 2，报 invalid_config，绝不猜测目录或显示 idle
    let missing_cfg = temp.0.join("non_existent_config.json");
    let out_missing = std::process::Command::new(&exe)
        .args(["status", "--config", missing_cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out_missing.status.code(), Some(2));
    let stderr_missing = String::from_utf8_lossy(&out_missing.stderr);
    assert!(stderr_missing.contains("invalid_config"));

    // 2. 坏 JSON 格式配置必须退出码 2，报 invalid_config
    let corrupt_cfg = temp.0.join("corrupt_config.json");
    fs::write(&corrupt_cfg, b"{not-a-valid-json").unwrap();
    fs::set_permissions(&corrupt_cfg, fs::Permissions::from_mode(0o600)).unwrap();
    let out_corrupt = std::process::Command::new(&exe)
        .args(["status", "--config", corrupt_cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out_corrupt.status.code(), Some(2));
    let stderr_corrupt = String::from_utf8_lossy(&out_corrupt.stderr);
    assert!(stderr_corrupt.contains("invalid_config"));

    // 3. 缺少必填字段（如缺少 allowlist）必须退出码 2，绝不绕过为 idle
    let incomplete_cfg = temp.0.join("incomplete_config.json");
    fs::write(
        &incomplete_cfg,
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "host": "local",
            "account_id": "acc-1",
            "state_dir": temp.0.join("state").to_str().unwrap()
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&incomplete_cfg, fs::Permissions::from_mode(0o600)).unwrap();
    let out_incomplete = std::process::Command::new(&exe)
        .args(["status", "--config", incomplete_cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out_incomplete.status.code(), Some(2));
    let stderr_incomplete = String::from_utf8_lossy(&out_incomplete.stderr);
    assert!(stderr_incomplete.contains("invalid_config"));

    // 验证整个过程中没有产生任何多余文件或状态写入
    let entries_after: Vec<_> = fs::read_dir(&temp.0).unwrap().collect();
    // 仅包含新写入的 corrupt_config.json 和 incomplete_config.json 两个输入文件
    assert_eq!(entries_before.len() + 2, entries_after.len());
    assert!(!temp.0.join("state").exists());
}

#[test]
fn test_sandbox_executor_end_to_end_and_fault_injections() {
    let temp = TempTestDir::new("sandbox_e2e");
    let sandbox = SandboxUpdateRoot::create_test_isolated_sandbox().unwrap();

    // 0. 验证沙箱根持有有效 FD，且不可变借用
    assert!(sandbox.root_fd() >= 0);
    assert!(sandbox.root_dir().exists());

    // 初始化真实 SQLite 数据库与业务消息表
    let db_p = sandbox.db_path();
    {
        let conn = rusqlite::Connection::open(&db_p).unwrap();
        conn.execute(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY, content TEXT, created_at INTEGER);",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, content, created_at) VALUES (1, 'initial baseline message', 1794360000);",
            [],
        )
        .unwrap();
    }

    // 生成真实测试密钥对与受信任 Keystore
    let (key_pair, pub_bytes) = generate_test_ed25519_key();
    let key_store = TrustedKeyStore::new().with_key("test-root-v1", &pub_bytes);

    // 1. 构造一个包含真实组件的发布包归档
    let archive_path = temp.0.join("pkg-0.2.0.tar.gz");
    let mcp_bin_content = b"wechat-mcp-bin-v0.2.0";
    let mcp_sha = calculate_sha256_bytes(mcp_bin_content);
    {
        let file = fs::File::create(&archive_path).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut h = tar::Header::new_gnu();
        h.set_size(mcp_bin_content.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        builder
            .append_data(&mut h, "bin/wechat-mcp", &mcp_bin_content[..])
            .unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let meta = fs::metadata(&archive_path).unwrap();
    let archive_sha = calculate_sha256_bytes(&fs::read(&archive_path).unwrap());

    let mut manifest = sample_manifest(1005, "arm64");
    manifest.release_id = "0.2.0-1005".into();
    manifest.artifact = ArtifactSpec {
        filename: "pkg-0.2.0.tar.gz".into(),
        sha256: archive_sha,
        size_bytes: meta.len(),
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };
    manifest.components = vec![ComponentSpec {
        path: "bin/wechat-mcp".into(),
        kind: "executable".into(),
        file_mode: 0o755,
        sha256: mcp_sha,
        size_bytes: mcp_bin_content.len() as u64,
    }];

    // 签名封套
    let sig_hex = sign_manifest(&key_pair, &manifest);
    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex,
        manifest,
    };
    let env_bytes = serde_json::to_vec(&envelope).unwrap();

    // 强类型验证通过构造 VerifiedManifestEnvelope
    let verified = VerifiedManifestEnvelope::verify_and_construct(
        &env_bytes, &key_store, "arm64", "13.0", 1000, // highest accepted
    )
    .unwrap();

    let signer = LocalSigner::for_test_mock();

    let drain_adapter = std::sync::Arc::new(SandboxFixtureDrainAdapter);
    let backup_adapter = std::sync::Arc::new(SandboxFixtureBackupAdapter::new_with_db(&db_p));
    let cand_adapter = std::sync::Arc::new(SandboxFixtureCandidateAdapter::from_fixture(
        SandboxCandidateFixtureKind::Pass,
    ));

    // 执行更新全流程 -> Committed
    let executor = SandboxUpdateExecutor::new(&sandbox)
        .with_drain_adapter(drain_adapter.clone())
        .with_backup_adapter(backup_adapter.clone())
        .with_candidate_adapter(cand_adapter.clone());
    let res = executor.execute_transaction(&verified, &archive_path, &signer, 1794368000);
    assert!(res.is_ok(), "Update execution failed: {:?}", res.err());
    let journal = res.unwrap();
    assert_eq!(journal.stage, TransactionStage::Committed);

    // 证明记录有效
    assert!(journal.drain_proof.is_some());
    assert!(journal.candidate_proof.is_some());

    // 真实 SQLite 在线备份已成功生成与记录
    assert!(journal.backup_path.is_some());
    let backup_file = PathBuf::from(journal.backup_path.as_ref().unwrap());
    assert!(backup_file.exists());

    // 核验更新后状态：current 指向新版 releases/0.2.0-1005
    let current_target = fs::read_link(sandbox.current_link()).unwrap();
    assert!(current_target.to_string_lossy().contains("0.2.0-1005"));

    // 最高水位已提升
    let highest = load_highest_accepted_sequence(&sandbox.state_dir()).unwrap();
    assert_eq!(highest, 1005);

    // CurrentReleaseCommitRecord 已写入
    let current_meta = load_current_release_record(&sandbox.state_dir())
        .unwrap()
        .unwrap();
    assert_eq!(current_meta.release_id, "0.2.0-1005");
    assert_eq!(current_meta.sequence, 1005);
    assert_eq!(
        current_meta.manifest_digest,
        calculate_manifest_digest(verified.manifest()).unwrap()
    );

    // 维护态已安全清除
    assert!(!sandbox.state_dir().join("core.maintenance.json").exists());
    assert!(!sandbox.state_dir().join("core.maintenance").exists());

    // 2. 候选自检失败注入测试：存在业务数据库时，自检失败严格进入 RecoveryRequired 并保持维护态，绝不自动覆盖 DB
    // 构造更高版本 1006
    let mut manifest2 = verified.manifest().clone();
    manifest2.sequence = 1006;
    manifest2.release_id = "0.2.0-1006".into();
    let sig_hex2 = sign_manifest(&key_pair, &manifest2);
    let envelope2 = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex2,
        manifest: manifest2,
    };
    let env_bytes2 = serde_json::to_vec(&envelope2).unwrap();
    let verified2 = VerifiedManifestEnvelope::verify_and_construct(
        &env_bytes2,
        &key_store,
        "arm64",
        "13.0",
        1005,
    )
    .unwrap();

    let fail_test_executor = SandboxUpdateExecutor::new(&sandbox)
        .with_drain_adapter(drain_adapter.clone())
        .with_backup_adapter(std::sync::Arc::new(
            SandboxFixtureBackupAdapter::new_with_db(&db_p),
        ))
        .with_candidate_adapter(cand_adapter.clone())
        .with_fault(FaultInjection {
            fail_candidate_testing: true,
            ..Default::default()
        });
    let res_fail =
        fail_test_executor.execute_transaction(&verified2, &archive_path, &signer, 1794369000);
    assert!(res_fail.is_ok(), "res_fail failed: {:?}", res_fail.err());
    let journal_fail = res_fail.unwrap();
    assert_eq!(journal_fail.stage, TransactionStage::RecoveryRequired);
    // 维护标志保持激活，绝不自动清除
    assert!(sandbox.state_dir().join("core.maintenance.json").exists());
    assert!(sandbox.state_dir().join("core.maintenance").exists());

    // 3. 业务数据保护测试：当受控存在业务数据库时，自检失败必须保留维护态并标记 RecoveryRequired，严禁自动回滚覆盖
    let mut manifest3 = verified.manifest().clone();
    manifest3.sequence = 1007;
    manifest3.release_id = "0.2.0-1007".into();
    let sig_hex3 = sign_manifest(&key_pair, &manifest3);
    let envelope3 = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex3,
        manifest: manifest3,
    };
    let env_bytes3 = serde_json::to_vec(&envelope3).unwrap();
    let verified3 = VerifiedManifestEnvelope::verify_and_construct(
        &env_bytes3,
        &key_store,
        "arm64",
        "13.0",
        1005,
    )
    .unwrap();

    let conflict_executor = SandboxUpdateExecutor::new(&sandbox)
        .with_drain_adapter(drain_adapter.clone())
        .with_backup_adapter(std::sync::Arc::new(
            SandboxFixtureBackupAdapter::new_with_db(&db_p),
        ))
        .with_candidate_adapter(cand_adapter.clone())
        .with_fault(FaultInjection {
            fail_candidate_testing: true,
            ..Default::default()
        });
    let res_conflict =
        conflict_executor.execute_transaction(&verified3, &archive_path, &signer, 1794370000);
    assert!(res_conflict.is_ok());
    let journal_conflict = res_conflict.unwrap();
    assert_eq!(journal_conflict.stage, TransactionStage::RecoveryRequired);
    // 维护标志保持激活状态，绝不自动清除
    assert!(sandbox.state_dir().join("core.maintenance.json").exists());
    assert!(sandbox.state_dir().join("core.maintenance").exists());
}

#[test]
fn test_release_id_validation_and_existing_directory_protection() {
    // 1. 字符集与路径穿越校验
    assert!(is_valid_release_id("0.2.0-alpha.1"));
    assert!(is_valid_release_id("v1.0.0-rc_2"));
    assert!(!is_valid_release_id(""));
    assert!(!is_valid_release_id("."));
    assert!(!is_valid_release_id(".."));
    assert!(!is_valid_release_id("../escape"));
    assert!(!is_valid_release_id("release/nested"));
    assert!(!is_valid_release_id("has space"));
    assert!(!is_valid_release_id("null\0byte"));

    // 2. 已存在 Release 目录保护：严禁覆盖或删除已有版本
    let sandbox = SandboxUpdateRoot::create_test_isolated_sandbox().unwrap();
    let protected_dir = sandbox.releases_dir().join("0.2.0-existing");
    fs::create_dir_all(&protected_dir).unwrap();
    let canary_file = protected_dir.join("canary.txt");
    fs::write(&canary_file, b"precious_canary_data").unwrap();

    let (key_pair, pub_bytes) = generate_test_ed25519_key();
    let key_store = TrustedKeyStore::new().with_key("test-root-v1", &pub_bytes);

    let archive_path = sandbox.root_dir().join("dummy.tar.gz");
    fs::write(&archive_path, b"dummy").unwrap();

    let mut manifest = sample_manifest(2001, "arm64");
    manifest.release_id = "0.2.0-existing".into();
    let sig_hex = sign_manifest(&key_pair, &manifest);
    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex,
        manifest: manifest.clone(),
    };
    let env_bytes = serde_json::to_vec(&envelope).unwrap();
    let verified =
        VerifiedManifestEnvelope::verify_and_construct(&env_bytes, &key_store, "arm64", "13.0", 0)
            .unwrap();

    let signer = LocalSigner::for_test_mock();
    let executor = SandboxUpdateExecutor::new(&sandbox)
        .with_drain_adapter(std::sync::Arc::new(SandboxFixtureDrainAdapter))
        .with_backup_adapter(std::sync::Arc::new(
            SandboxFixtureBackupAdapter::new_no_database(),
        ))
        .with_candidate_adapter(std::sync::Arc::new(
            SandboxFixtureCandidateAdapter::from_fixture(SandboxCandidateFixtureKind::Pass),
        ));
    let err = executor
        .execute_transaction(&verified, &archive_path, &signer, 1794360100)
        .unwrap_err();

    assert!(
        err.to_string().contains("release_directory_already_exists"),
        "Unexpected error: {:?}",
        err
    );

    // 验证金丝雀文件依然完好无损，目录未被删除或清空！
    assert_eq!(fs::read(&canary_file).unwrap(), b"precious_canary_data");
}

#[test]
fn test_recovery_evaluation_rejects_unknown_external_or_invalid_targets() {
    let temp = TempTestDir::new("rec_eval");
    let releases_dir = temp.0.join("releases");
    let external_dir = temp.0.join("external");
    fs::create_dir_all(&releases_dir).unwrap();
    fs::create_dir_all(&external_dir).unwrap();

    let current_link = temp.0.join("current");

    let journal = UpdateJournal {
        journal_schema: 1,
        transaction_id: "tx-test-eval".into(),
        stage: TransactionStage::Switching,
        source_release_id: "0.1.0".into(),
        target_release_id: "0.2.0".into(),
        target_sequence: 1002,
        timestamp: 1794360000,
        backup_path: None,
        error: None,
        maintenance_marker_confirmed: true,
        drain_proof: None,
        backup_proof: None,
        candidate_proof: None,
        manifest_digest: None,
    };

    // 1. current 指向外部目录（不在 releases_dir 内）
    std::os::unix::fs::symlink(&external_dir, &current_link).unwrap();
    let rec1 = journal.evaluate_recovery(&releases_dir, &current_link);
    match rec1 {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(
                msg.contains("foreign_or_unknown_current_target")
                    || msg.contains("current_target_not_a_directory")
                    || msg.contains("unknown_current_target")
            );
        }
        other => panic!("Expected ManualInterventionRequired, got {:?}", other),
    }

    // 2. current 指向 releases_dir 内不存在的非目录/软链断裂
    let _ = fs::remove_file(&current_link);
    std::os::unix::fs::symlink(releases_dir.join("nonexistent_release"), &current_link).unwrap();
    let rec2 = journal.evaluate_recovery(&releases_dir, &current_link);
    match rec2 {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(
                msg.contains("foreign_or_unknown_current_target")
                    || msg.contains("target_subdir_in_releases_dir_invalid")
                    || msg.contains("current_target_not_a_directory")
            );
        }
        other => panic!("Expected ManualInterventionRequired, got {:?}", other),
    }
}

#[test]
fn test_whitelist_candidate_testing_and_exit_code_enforcement() {
    let temp = TempTestDir::new("cand_test");
    let sandbox = SandboxUpdateRoot::create_test_isolated_sandbox().unwrap();

    let (key_pair, pub_bytes) = generate_test_ed25519_key();
    let key_store = TrustedKeyStore::new().with_key("test-root-v1", &pub_bytes);

    // 构造包含自检可执行脚本的归档
    let archive_path = temp.0.join("cand-pkg.tar.gz");
    let script_content = b"#!/bin/sh\nif [ \"$1\" = \"--fail\" ]; then exit 42; fi\nexit 0\n";
    let script_sha = calculate_sha256_bytes(script_content);

    {
        let file = fs::File::create(&archive_path).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut h = tar::Header::new_gnu();
        h.set_size(script_content.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        builder
            .append_data(&mut h, "bin/candidate_check.sh", &script_content[..])
            .unwrap();

        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let meta = fs::metadata(&archive_path).unwrap();
    let archive_sha = calculate_sha256_bytes(&fs::read(&archive_path).unwrap());

    let mut manifest = sample_manifest(3001, "arm64");
    manifest.release_id = "0.3.0-test".into();
    manifest.artifact = ArtifactSpec {
        filename: "cand-pkg.tar.gz".into(),
        sha256: archive_sha,
        size_bytes: meta.len(),
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };
    manifest.components = vec![ComponentSpec {
        path: "bin/candidate_check.sh".into(),
        kind: "executable".into(),
        file_mode: 0o755,
        sha256: script_sha,
        size_bytes: script_content.len() as u64,
    }];

    let sig_hex = sign_manifest(&key_pair, &manifest);
    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex,
        manifest,
    };
    let env_bytes = serde_json::to_vec(&envelope).unwrap();
    let verified =
        VerifiedManifestEnvelope::verify_and_construct(&env_bytes, &key_store, "arm64", "13.0", 0)
            .unwrap();

    let signer = LocalSigner::for_test_mock();

    let drain_adapter = std::sync::Arc::new(SandboxFixtureDrainAdapter);
    let backup_adapter = std::sync::Arc::new(SandboxFixtureBackupAdapter::new_no_database());

    // 1. 真实自检命令返回 0 -> 成功提交
    let pass_cand_adapter = std::sync::Arc::new(SandboxFixtureCandidateAdapter::from_fixture(
        SandboxCandidateFixtureKind::Pass,
    ));
    let pass_executor = SandboxUpdateExecutor::new(&sandbox)
        .with_drain_adapter(drain_adapter.clone())
        .with_backup_adapter(backup_adapter.clone())
        .with_candidate_adapter(pass_cand_adapter);
    let res_pass = pass_executor
        .execute_transaction(&verified, &archive_path, &signer, 1794380000)
        .unwrap();
    assert_eq!(res_pass.stage, TransactionStage::Committed);

    // 2. 真实自检命令返回非 0 (exit code 42) -> 触发回滚
    let mut manifest_fail = sample_manifest(3002, "arm64");
    manifest_fail.release_id = "0.3.1-fail".into();
    manifest_fail.artifact = verified.manifest().artifact.clone();
    manifest_fail.components = verified.manifest().components.clone();

    let sig_hex_f = sign_manifest(&key_pair, &manifest_fail);
    let envelope_f = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex_f,
        manifest: manifest_fail,
    };
    let env_bytes_f = serde_json::to_vec(&envelope_f).unwrap();
    let verified_f = VerifiedManifestEnvelope::verify_and_construct(
        &env_bytes_f,
        &key_store,
        "arm64",
        "13.0",
        3001,
    )
    .unwrap();

    let fail_cand_adapter = std::sync::Arc::new(SandboxFixtureCandidateAdapter::from_fixture(
        SandboxCandidateFixtureKind::FailExitCode(42),
    ));
    let fail_executor = SandboxUpdateExecutor::new(&sandbox)
        .with_drain_adapter(drain_adapter.clone())
        .with_backup_adapter(backup_adapter.clone())
        .with_candidate_adapter(fail_cand_adapter);
    let res_fail = fail_executor
        .execute_transaction(&verified_f, &archive_path, &signer, 1794381000)
        .unwrap();
    assert_eq!(res_fail.stage, TransactionStage::RolledBack);
}

#[test]
fn test_executor_fails_closed_before_write_if_any_adapter_missing() {
    let sandbox = SandboxUpdateRoot::create_test_isolated_sandbox().unwrap();
    let (key_pair, pub_bytes) = generate_test_ed25519_key();
    let key_store = TrustedKeyStore::new().with_key("test-root-v1", &pub_bytes);

    let archive_path = sandbox.root_dir().join("dummy.tar.gz");
    fs::write(&archive_path, b"dummy").unwrap();

    let manifest = sample_manifest(4001, "arm64");
    let sig_hex = sign_manifest(&key_pair, &manifest);
    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex,
        manifest,
    };
    let env_bytes = serde_json::to_vec(&envelope).unwrap();
    let verified =
        VerifiedManifestEnvelope::verify_and_construct(&env_bytes, &key_store, "arm64", "13.0", 0)
            .unwrap();

    let signer = LocalSigner::for_test_mock();

    // 1. 完全未注入适配器 -> 立即返回 unsupported_update_execution
    let raw_executor = SandboxUpdateExecutor::new(&sandbox);
    let err = raw_executor
        .execute_transaction(&verified, &archive_path, &signer, 1794380000)
        .unwrap_err();
    assert!(err.to_string().contains("unsupported_update_execution"));

    // 证明零写副作用：无 staging 目录、无 release 目录、无 journal 写入！
    assert!(!sandbox.root_dir().join("staging").exists());
    assert!(!sandbox.state_dir().join("journal.json").exists());
    assert!(!sandbox.state_dir().join("current_release.json").exists());
}

#[test]
fn test_candidate_adapter_timeout_output_flood_and_process_cleanup() {
    let temp = TempTestDir::new("cand_limits");

    // 1. 超时测试：命令 sleep，适配器 timeout 200ms -> 拦截超时并杀死全进程组，毫秒级返回
    let start_hang = std::time::Instant::now();
    let sleep_adapter =
        SandboxFixtureCandidateAdapter::from_fixture(SandboxCandidateFixtureKind::TimeoutHang)
            .with_timeout(std::time::Duration::from_millis(200));

    let err = sleep_adapter
        .execute_candidate_test("tx-timeout", &temp.0)
        .unwrap_err();
    assert!(err.to_string().contains("candidate_test_timeout_exceeded"));
    assert!(
        start_hang.elapsed() < std::time::Duration::from_secs(2),
        "Timeout exceeded deadline: {:?}",
        start_hang.elapsed()
    );

    // 2. 进程组超时 fork 场景：子进程派生后台孙进程并在后台持续运行 -> 超时收割全组，join 不被绕过
    let start_fork = std::time::Instant::now();
    let fork_hang_adapter = SandboxFixtureCandidateAdapter::from_fixture(
        SandboxCandidateFixtureKind::TimeoutForkProcessGroup,
    )
    .with_timeout(std::time::Duration::from_millis(200));

    let err_fork = fork_hang_adapter
        .execute_candidate_test("tx-fork-hang", &temp.0)
        .unwrap_err();
    assert!(
        err_fork
            .to_string()
            .contains("candidate_test_timeout_exceeded")
    );
    assert!(
        start_fork.elapsed() < std::time::Duration::from_secs(2),
        "Fork hang exceeded deadline: {:?}",
        start_fork.elapsed()
    );

    // 3. 孙进程孤儿持有管道场景：父进程 exit 0 退出，孙进程后台 hold stdout sleep 30 -> 立即收割，join 绝不无界阻塞
    let start_orphan = std::time::Instant::now();
    let orphan_adapter = SandboxFixtureCandidateAdapter::from_fixture(
        SandboxCandidateFixtureKind::OrphanForkHoldsPipe,
    );
    let proof_orphan = orphan_adapter
        .execute_candidate_test("tx-orphan", &temp.0)
        .expect("Orphan pipe test should be reaped and succeed cleanly");
    assert_eq!(proof_orphan.exit_code, 0);
    assert_eq!(proof_orphan.tx_id, "tx-orphan");
    assert!(
        start_orphan.elapsed() < std::time::Duration::from_secs(2),
        "Orphan join hang detected: took {:?}",
        start_orphan.elapsed()
    );

    // 4. 输出洪泛硬上限与持续消耗防护：超限立即杀组拦截，绝不无限读空消耗资源
    let flood_limit_adapter =
        SandboxFixtureCandidateAdapter::from_fixture(SandboxCandidateFixtureKind::OutputFlood)
            .with_max_output_bytes(512)
            .with_max_total_drain_bytes(4096);

    let flood_err = flood_limit_adapter
        .execute_candidate_test("tx-flood", &temp.0)
        .unwrap_err();
    assert!(
        flood_err
            .to_string()
            .contains("candidate_test_output_flood_exceeded_limit"),
        "Unexpected error: {:?}",
        flood_err
    );

    // 5. 相对路径试图逃逸被严格静态拒绝
    let rel_adapter = SandboxFixtureCandidateAdapter::for_test_raw_command("../escape_bin", vec![]);
    let err_rel = rel_adapter
        .execute_candidate_test("tx-rel", &temp.0)
        .unwrap_err();
    assert!(
        err_rel.to_string().contains("must_be_absolute")
            || err_rel.to_string().contains("contains_traversal")
    );
}

#[test]
fn test_backup_adapter_no_database_and_restore_verification() {
    let temp = TempTestDir::new("backup_adapter_test");
    let state_dir = temp.0.clone();

    // 1. 声明 NoDatabase 但 state_dir 实际存在 wechat.db -> 严格抛错拒绝
    let dummy_db = state_dir.join("wechat.db");
    fs::write(&dummy_db, b"fake_db").unwrap();

    let no_db_adapter = SandboxFixtureBackupAdapter::new_no_database();
    let err = no_db_adapter
        .execute_backup("tx-no-db", &state_dir)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("database_file_detected_when_expected_no_database")
    );

    // 2. 真实 SQLite 数据库备份与 restore 核验
    fs::remove_file(&dummy_db).unwrap();
    {
        let conn = rusqlite::Connection::open(&dummy_db).unwrap();
        conn.execute(
            "CREATE TABLE test_data (id INTEGER PRIMARY KEY, name TEXT);",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO test_data (id, name) VALUES (1, 'val1');", [])
            .unwrap();
    }

    let with_db_adapter = SandboxFixtureBackupAdapter::new_with_db(&dummy_db);
    let proof = with_db_adapter
        .execute_backup("tx-db-test", &state_dir)
        .unwrap();
    match proof {
        VerifiedSandboxBackupProof::DatabaseBackupVerified {
            backup_path,
            backup_sha256,
        } => {
            assert!(backup_path.exists());
            assert_eq!(backup_sha256.len(), 64);
            // 验证真实 restore 校验函数对该备份有效
            assert!(super::rollback::verify_backup_restorable(&backup_path).is_ok());
        }
        other => panic!("Expected DatabaseBackupVerified, got {:?}", other),
    }
}

#[test]
fn test_highest_accepted_sequence_conflict_and_recovery_evaluation() {
    let temp = TempTestDir::new("seq_conflict_and_rec");
    let state_dir = temp.0.join("state");
    let releases_dir = temp.0.join("releases");
    let current_link = temp.0.join("current");
    fs::create_dir_all(&state_dir).unwrap();
    fs::create_dir_all(&releases_dir).unwrap();

    // 1. 水位冲突防倒退：current_release.json 为 2000，旧文件为 1000 -> 报错拒绝，绝不回 0
    let rec = CurrentReleaseCommitRecord {
        record_schema: 1,
        tx_id: "tx-seq-test".into(),
        release_id: "0.2.0-2000".into(),
        sequence: 2000,
        channel: "test".into(),
        manifest_digest: "sha256-test-digest".into(),
        installed_at: 1794360000,
    };
    atomic_commit_release_record(&state_dir, &rec).unwrap();
    // 强制把旧文件修改为不一致的值
    fs::write(state_dir.join("highest_accepted_sequence"), b"1000").unwrap();

    let conflict_err = load_highest_accepted_sequence(&state_dir).unwrap_err();
    assert!(
        conflict_err
            .to_string()
            .contains("sequence_conflict_detected")
    );

    // 2. Crash 恢复核验：阶段为 Committed 但缺少 drain_proof 或 candidate_proof -> 严格保持维护态
    let journal = UpdateJournal {
        journal_schema: 1,
        transaction_id: "tx-crash-test".into(),
        stage: TransactionStage::Committed,
        source_release_id: "0.1.0".into(),
        target_release_id: "0.2.0".into(),
        target_sequence: 2000,
        timestamp: 1794360000,
        backup_path: None,
        error: None,
        maintenance_marker_confirmed: true,
        drain_proof: None, // 缺失 proof
        backup_proof: None,
        candidate_proof: None,
        manifest_digest: None,
    };
    let rec_act = journal.evaluate_recovery(&releases_dir, &current_link);
    match rec_act {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(
                msg.contains("missing_drain_proof")
                    || msg.contains("missing_drain_or_candidate_proof")
                    || msg.contains("proof_missing_or_inconsistent")
            );
        }
        other => panic!("Expected ManualInterventionRequired, got {:?}", other),
    }
}

#[test]
fn test_atomic_commit_protocol_failure_and_crash_window_recovery() {
    let temp = TempTestDir::new("atomic_commit_rec");
    let sandbox = SandboxUpdateRoot::create_test_isolated_sandbox().unwrap();

    // 构造测试发布包归档
    let archive_path = temp.0.join("pkg-0.2.0.tar.gz");
    let mcp_bin_content = b"wechat-mcp-bin-v0.2.0";
    let mcp_sha = calculate_sha256_bytes(mcp_bin_content);
    {
        let file = fs::File::create(&archive_path).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        let mut h = tar::Header::new_gnu();
        h.set_size(mcp_bin_content.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        builder
            .append_data(&mut h, "bin/wechat-mcp", &mcp_bin_content[..])
            .unwrap();
        let enc = builder.into_inner().unwrap();
        enc.finish().unwrap();
    }
    let meta = fs::metadata(&archive_path).unwrap();
    let archive_sha = calculate_sha256_bytes(&fs::read(&archive_path).unwrap());

    let (key_pair, pub_bytes) = generate_test_ed25519_key();
    let key_store = TrustedKeyStore::new().with_key("test-root-v1", &pub_bytes);

    let mut manifest = sample_manifest(1010, "arm64");
    manifest.release_id = "0.2.0-1010".into();
    manifest.artifact = ArtifactSpec {
        filename: "pkg-0.2.0.tar.gz".into(),
        sha256: archive_sha,
        size_bytes: meta.len(),
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };
    manifest.components = vec![ComponentSpec {
        path: "bin/wechat-mcp".into(),
        kind: "executable".into(),
        file_mode: 0o755,
        sha256: mcp_sha,
        size_bytes: mcp_bin_content.len() as u64,
    }];
    let sig_hex = sign_manifest(&key_pair, &manifest);
    let envelope = SignedEnvelope {
        envelope_schema: 1,
        key_id: "test-root-v1".into(),
        signature_algorithm: "ed25519".into(),
        signature: sig_hex,
        manifest,
    };
    let env_bytes = serde_json::to_vec(&envelope).unwrap();
    let verified = VerifiedManifestEnvelope::verify_and_construct(
        &env_bytes, &key_store, "arm64", "13.0", 1000,
    )
    .unwrap();
    let signer = LocalSigner::for_test_mock();

    // 1. 模拟 record 写/fsync 失败 (fail_record_commit)
    let executor_fail_rec = SandboxUpdateExecutor::new(&sandbox)
        .with_drain_adapter(std::sync::Arc::new(SandboxFixtureDrainAdapter))
        .with_backup_adapter(std::sync::Arc::new(
            SandboxFixtureBackupAdapter::new_no_database(),
        ))
        .with_candidate_adapter(std::sync::Arc::new(
            SandboxFixtureCandidateAdapter::from_fixture(SandboxCandidateFixtureKind::Pass),
        ))
        .with_fault(FaultInjection {
            fail_record_commit: true,
            ..Default::default()
        });

    let res_fail =
        executor_fail_rec.execute_transaction(&verified, &archive_path, &signer, 1794380000);
    assert!(res_fail.is_err());
    let err_msg = res_fail.unwrap_err().to_string();
    assert!(err_msg.contains("injected_fault_record_commit_failed"));

    // 核心断言：
    // a. journal 处于 CandidateTestingPassed，绝不是 Committed！
    let journal = UpdateJournal::load_from_state(&sandbox.state_dir())
        .unwrap()
        .expect("journal should exist");
    assert_eq!(journal.stage, TransactionStage::CandidateTestingPassed);
    assert_ne!(journal.stage, TransactionStage::Committed);

    // b. 维护标记绝对未被清除
    assert!(sandbox.state_dir().join("core.maintenance.json").exists());

    // c. 权威提交记录 current_release.json 不存在
    assert!(
        load_current_release_record(&sandbox.state_dir())
            .unwrap()
            .is_none()
    );

    // d. 恢复评估绝不盲猜成功，严格返回人工干预
    let rec_action = journal.evaluate_recovery(&sandbox.releases_dir(), &sandbox.current_link());
    match rec_action {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(msg.contains("record_commit_not_found_after_candidate_testing_passed"));
        }
        other => panic!("Expected ManualInterventionRequired, got {:?}", other),
    }

    // 2. 模拟 record 成功写入但在 journal 写 Committed 前崩溃 (fail_after_record_commit)
    let sandbox_crash = SandboxUpdateRoot::create_test_isolated_sandbox().unwrap();
    let executor_crash_window = SandboxUpdateExecutor::new(&sandbox_crash)
        .with_drain_adapter(std::sync::Arc::new(SandboxFixtureDrainAdapter))
        .with_backup_adapter(std::sync::Arc::new(
            SandboxFixtureBackupAdapter::new_no_database(),
        ))
        .with_candidate_adapter(std::sync::Arc::new(
            SandboxFixtureCandidateAdapter::from_fixture(SandboxCandidateFixtureKind::Pass),
        ))
        .with_fault(FaultInjection {
            fail_after_record_commit: true,
            ..Default::default()
        });

    let res_crash =
        executor_crash_window.execute_transaction(&verified, &archive_path, &signer, 1794381000);
    assert!(res_crash.is_err());
    assert!(
        res_crash
            .unwrap_err()
            .to_string()
            .contains("injected_fault_after_record_commit_before_journal_committed")
    );

    // 此时权威记录已存在
    let current_rec = load_current_release_record(&sandbox_crash.state_dir())
        .unwrap()
        .expect("current_release.json should exist");
    assert_eq!(current_rec.sequence, 1010);
    assert_eq!(current_rec.release_id, "0.2.0-1010");

    let journal_crash = UpdateJournal::load_from_state(&sandbox_crash.state_dir())
        .unwrap()
        .expect("journal should exist");
    assert_eq!(
        journal_crash.stage,
        TransactionStage::CandidateTestingPassed
    );

    // 核心断言：崩溃恢复能精确识别 record 成功且要素完全匹配，返回 CompletePendingCommit
    let rec_exact = journal_crash
        .evaluate_recovery(&sandbox_crash.releases_dir(), &sandbox_crash.current_link());
    assert_eq!(rec_exact, RecoveryAction::CompletePendingCommit);

    // 3. 篡改与伪造对抗测试：若 record 与 journal 要素不匹配，绝不猜成功
    // 3a. 篡改 sequence
    let mut tampered_rec = current_rec.clone();
    tampered_rec.sequence = 9999;
    atomic_commit_release_record(&sandbox_crash.state_dir(), &tampered_rec).unwrap();
    let rec_tampered_seq = journal_crash
        .evaluate_recovery(&sandbox_crash.releases_dir(), &sandbox_crash.current_link());
    match rec_tampered_seq {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(msg.contains("candidate_passed_but_commit_record_mismatch"));
        }
        other => panic!("Expected ManualInterventionRequired, got {:?}", other),
    }

    // 3b. 恢复 sequence 但篡改 manifest_digest
    tampered_rec.sequence = 1010;
    tampered_rec.manifest_digest = "sha256-forged-digest-value".into();
    atomic_commit_release_record(&sandbox_crash.state_dir(), &tampered_rec).unwrap();
    let rec_tampered_digest = journal_crash
        .evaluate_recovery(&sandbox_crash.releases_dir(), &sandbox_crash.current_link());
    match rec_tampered_digest {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(msg.contains("manifest_digest_mismatch"));
        }
        other => panic!("Expected ManualInterventionRequired, got {:?}", other),
    }

    // 4. Committed 状态下的全要素防伪造校验
    let mut committed_journal = journal_crash.clone();
    committed_journal.stage = TransactionStage::Committed;
    // 恢复正确 record
    atomic_commit_release_record(&sandbox_crash.state_dir(), &current_rec).unwrap();

    // 4a. 伪造 drain_proof（非当前 tx_id）
    committed_journal.drain_proof = Some(
        serde_json::to_string(&VerifiedSandboxDrainProof {
            drained_at: 1794380000,
            tx_id: "forged-foreign-tx".into(),
        })
        .unwrap(),
    );
    let rec_forged_proof = committed_journal
        .evaluate_recovery(&sandbox_crash.releases_dir(), &sandbox_crash.current_link());
    match rec_forged_proof {
        RecoveryAction::ManualInterventionRequired(msg) => {
            assert!(msg.contains("drain_proof_tx_mismatch"));
        }
        other => panic!("Expected ManualInterventionRequired, got {:?}", other),
    }
}

#[test]
fn test_maintenance_marker_strict_36_bytes_uuid_and_startup_dual_check() {
    let dir = TempTestDir::new("maint_uuid_test");
    let state_dir = &dir.0;
    let releases_dir = dir.0.join("releases");
    let current_link = dir.0.join("current");
    fs::create_dir_all(&releases_dir).unwrap();

    let tx_uuid = uuid::Uuid::new_v4();
    let tx_id = format!("tx-{}", tx_uuid);

    // 0. 初始空状态测试：纯只读双检查返回 AllowStartup
    let init_decision = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    assert_eq!(init_decision, StartupDualCheckDecision::AllowStartup);

    // 1. 写入结构化维护标志
    set_structured_maintenance_marker(state_dir, &tx_id, "updater draining", 1794380000).unwrap();

    let raw_maint = state_dir.join("core.maintenance");
    assert!(raw_maint.exists(), "core.maintenance should exist");

    // 检查权限为 0600
    let meta = fs::metadata(&raw_maint).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "core.maintenance permissions must be 0600");

    // 检查大小严格为 36 字节
    assert_eq!(
        meta.len(),
        36,
        "core.maintenance size must be exactly 36 bytes"
    );

    // 检查内容必须为纯 36 字节标准小写 UUID，绝不是 reason 文本
    let content = fs::read_to_string(&raw_maint).unwrap();
    assert_eq!(
        content,
        tx_uuid.to_string(),
        "core.maintenance must contain exact 36-byte UUID"
    );

    // 此时双检查必须返回 MaintenanceSuppressed
    let sup_decision = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    assert_eq!(
        sup_decision,
        StartupDualCheckDecision::MaintenanceSuppressed {
            tx_id: tx_uuid.to_string(),
            stage: None,
        }
    );

    // 2. 尝试用错误 tx_id 清除：必须拒绝
    let wrong_tx = format!("tx-{}", uuid::Uuid::new_v4());
    let err_clear = clear_maintenance_for_tx(state_dir, &wrong_tx);
    assert!(err_clear.is_err());
    assert!(
        raw_maint.exists(),
        "core.maintenance must still exist when tx mismatch"
    );

    // 用正确 tx_id 清除
    clear_maintenance_for_tx(state_dir, &tx_id).unwrap();
    assert!(
        !raw_maint.exists(),
        "core.maintenance should be removed after clean"
    );
    let after_clean = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    assert_eq!(after_clean, StartupDualCheckDecision::AllowStartup);

    // 3. 损坏对抗场景测试
    // 3a. core.maintenance 大小不是 36 字节
    fs::write(&raw_maint, b"too_short").unwrap();
    fs::set_permissions(&raw_maint, fs::Permissions::from_mode(0o600)).unwrap();
    let dec_short = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_short {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("size_must_be_36_bytes"));
        }
        other => panic!("Expected RecoveryRequired, got {:?}", other),
    }

    // 3b. core.maintenance 包含非合法 UUID（恰好 36 字节但字符非法）
    fs::write(&raw_maint, b"12345678-1234-1234-1234-123456789zz!").unwrap();
    fs::set_permissions(&raw_maint, fs::Permissions::from_mode(0o600)).unwrap();
    let dec_bad_uuid = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_bad_uuid {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("content_not_valid_uuid"));
        }
        other => panic!("Expected RecoveryRequired, got {:?}", other),
    }
    fs::remove_file(&raw_maint).unwrap();

    // 3c. core.maintenance 为符号链接
    let dummy_target = state_dir.join("dummy_target");
    fs::write(&dummy_target, b"test").unwrap();
    std::os::unix::fs::symlink(&dummy_target, &raw_maint).unwrap();
    let dec_sym = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_sym {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(
                reason.contains("file_must_not_be_symlink")
                    || reason.contains("cannot_read_core_maintenance_safely")
            );
        }
        other => panic!("Expected RecoveryRequired, got {:?}", other),
    }
    fs::remove_file(&raw_maint).unwrap();
    fs::remove_file(&dummy_target).unwrap();

    // 4. UpdateJournal 阶段校验与 Committed 全要素二次只读核验
    let mut journal = UpdateJournal::new(&tx_id, "0.1.0", "0.2.0-test", 1005, 1794380000);
    // 4a. 中间状态 (Draining) -> MaintenanceSuppressed
    journal.stage = TransactionStage::Draining;
    journal.persist_atomic(state_dir).unwrap();
    let dec_draining = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    assert_eq!(
        dec_draining,
        StartupDualCheckDecision::MaintenanceSuppressed {
            tx_id: tx_id.clone(),
            stage: Some(TransactionStage::Draining),
        }
    );

    // 4b. 阶段为 Committed，但缺失 current_release.json -> RecoveryRequired (绝不盲目放行)
    journal.stage = TransactionStage::Committed;
    let valid_sha256 = "a1b2c3d4e5f67890a1b2c3d4e5f67890a1b2c3d4e5f67890a1b2c3d4e5f67890";
    journal.manifest_digest = Some(valid_sha256.into());
    journal.persist_atomic(state_dir).unwrap();
    let dec_missing_rec = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_missing_rec {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("missing_current_release_record"));
        }
        other => panic!("Expected RecoveryRequired, got {:?}", other),
    }

    // 4c. 提供 current_release.json 但要素不一致 (sequence 篡改)
    let commit_rec = CurrentReleaseCommitRecord {
        record_schema: 1,
        tx_id: tx_id.clone(),
        release_id: "0.2.0-test".into(),
        sequence: 9999, // 篡改
        channel: "stable".into(),
        manifest_digest: valid_sha256.into(),
        installed_at: 1794380000,
    };
    atomic_commit_release_record(state_dir, &commit_rec).unwrap();
    let dec_seq_mismatch = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_seq_mismatch {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("commit_record_sequence_mismatch"));
        }
        other => panic!("Expected RecoveryRequired, got {:?}", other),
    }

    // 4d. 修正 sequence，但 current 软链接缺失 -> RecoveryRequired
    let correct_rec = CurrentReleaseCommitRecord {
        sequence: 1005,
        ..commit_rec
    };
    atomic_commit_release_record(state_dir, &correct_rec).unwrap();
    let dec_missing_link = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_missing_link {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("current_link_does_not_exist"));
        }
        other => panic!("Expected RecoveryRequired, got {:?}", other),
    }

    // 4e. 建立正确的 current 软链并创建目标目录
    let target_dir = releases_dir.join("0.2.0-test");
    fs::create_dir_all(&target_dir).unwrap();
    std::os::unix::fs::symlink(&target_dir, &current_link).unwrap();

    // 全要素核对通过！
    let dec_all_match = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    assert_eq!(dec_all_match, StartupDualCheckDecision::AllowStartup);

    // 4f. 防 /tmp 外部目录逃逸：若 current 指向 /tmp 下同名目录，必须 Fail-Closed
    let escape_target = dir.0.join("external_tmp").join("0.2.0-test");
    fs::create_dir_all(&escape_target).unwrap();
    fs::remove_file(&current_link).unwrap();
    std::os::unix::fs::symlink(&escape_target, &current_link).unwrap();
    let dec_escape = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_escape {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(
                reason.contains("current_target_escape_detected")
                    || reason.contains("current_link_escapes_releases_dir")
            );
        }
        other => panic!("Expected RecoveryRequired for path escape, got {:?}", other),
    }
    // 恢复正常链接
    fs::remove_file(&current_link).unwrap();
    std::os::unix::fs::symlink(&target_dir, &current_link).unwrap();

    // 4f2. releases/id 自身为 symlink 指向外部，且 current 也指向该外部目录：必须拦截！
    fs::remove_file(&current_link).unwrap();
    fs::remove_dir(&target_dir).unwrap();
    std::os::unix::fs::symlink(&escape_target, &target_dir).unwrap();
    std::os::unix::fs::symlink(&escape_target, &current_link).unwrap();
    let dec_symlink_release = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_symlink_release {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("expected_release_dir_must_not_be_symlink"));
        }
        other => panic!(
            "Expected RecoveryRequired when releases/id is symlink, got {:?}",
            other
        ),
    }
    // 恢复原状
    fs::remove_file(&current_link).unwrap();
    fs::remove_file(&target_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    std::os::unix::fs::symlink(&target_dir, &current_link).unwrap();

    // 4g. digest 格式非法（非 64 字符 hex）必须 failclosed
    journal.manifest_digest = Some("not_a_valid_hex_sha256".into());
    journal.persist_atomic(state_dir).unwrap();
    let dec_bad_digest = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_bad_digest {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("invalid_manifest_digest_format"));
        }
        other => panic!("Expected RecoveryRequired for bad digest, got {:?}", other),
    }
    journal.manifest_digest = Some(valid_sha256.into());
    journal.persist_atomic(state_dir).unwrap();

    // 4h. RolledBack 终态核查：无法证明 source_release_id 真实生效时 failclosed
    journal.stage = TransactionStage::RolledBack;
    journal.source_release_id = "0.1.0".into();
    journal.persist_atomic(state_dir).unwrap();
    // 此时 current 仍指向 0.2.0-test，与 source_release_id 0.1.0 不符 -> 必须拦截
    let dec_rollback_mismatch =
        check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_rollback_mismatch {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("rollback_target_verification_failed"));
        }
        other => panic!(
            "Expected RecoveryRequired for rollback mismatch, got {:?}",
            other
        ),
    }
    // 建立 0.1.0 目录并切换 current 指向 0.1.0
    let source_dir = releases_dir.join("0.1.0");
    fs::create_dir_all(&source_dir).unwrap();
    fs::remove_file(&current_link).unwrap();
    std::os::unix::fs::symlink(&source_dir, &current_link).unwrap();

    // 4h2. 软链接已切回 0.1.0，但 current_release.json 仍为 0.2.0-test -> 必须拦截！
    let dec_rollback_record_mismatch =
        check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_rollback_record_mismatch {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("rollback_release_record_mismatch"));
        }
        other => panic!(
            "Expected RecoveryRequired for rollback record mismatch, got {:?}",
            other
        ),
    }

    // 4h3. 正确更新 current_release.json 为 0.1.0 后，全要素一致 -> AllowStartup
    let rollback_rec = CurrentReleaseCommitRecord {
        release_id: "0.1.0".into(),
        sequence: 1000,
        ..correct_rec
    };
    atomic_commit_release_record(state_dir, &rollback_rec).unwrap();
    let dec_rollback_ok = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    assert_eq!(dec_rollback_ok, StartupDualCheckDecision::AllowStartup);

    // 4i. 负向测试：update-journal.json 缺失 journal_schema 必须 Fail-Closed
    let journal_raw_missing_schema = serde_json::json!({
        "transaction_id": tx_id.clone(),
        "stage": "Committed",
        "source_release_id": "0.1.0",
        "target_release_id": "0.2.0-test",
        "target_sequence": 1005,
        "timestamp": 1794380000,
        "maintenance_marker_confirmed": true
    });
    let j_path = state_dir.join("update-journal.json");
    fs::write(
        &j_path,
        serde_json::to_vec_pretty(&journal_raw_missing_schema).unwrap(),
    )
    .unwrap();
    let dec_missing_schema = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    match dec_missing_schema {
        StartupDualCheckDecision::RecoveryRequired { reason } => {
            assert!(reason.contains("update-journal.json_corrupt_or_invalid"));
        }
        other => panic!(
            "Expected RecoveryRequired when journal_schema missing, got {:?}",
            other
        ),
    }
    // 恢复合法 journal
    journal.persist_atomic(state_dir).unwrap();

    // 4j. 负向测试：read_limited_private_file 流式超限截断与拦截
    let large_file = state_dir.join("large_test_file");
    fs::write(&large_file, vec![b'a'; 256]).unwrap();
    let read_res = read_limited_private_file(&large_file, 64);
    assert!(
        read_res.is_err(),
        "reading file larger than limit must error"
    );
    fs::remove_file(&large_file).unwrap();

    // 5. 纯只读性验证：检查前后 state 目录文件与元数据零副作用
    let files_before = fs::read_dir(state_dir).unwrap().count();
    let _ = check_startup_guard_readonly(state_dir, &releases_dir, &current_link);
    let files_after = fs::read_dir(state_dir).unwrap().count();
    assert_eq!(
        files_before, files_after,
        "startup guard must have zero write side effects"
    );
}
