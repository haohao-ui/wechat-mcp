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
    assert!(res.is_ok(), "Expected valid unpack to succeed: {:?}", res.err());

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
    assert!(sym_res.unwrap_err().to_string().contains("symlink_rejected"));

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
    assert!(tr_res.unwrap_err().to_string().contains("invalid_relative_path"));

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
        builder.append_data(&mut h1, "bin/wechat-mcp", &content[..]).unwrap();

        let mut h2 = tar::Header::new_gnu();
        h2.set_size(content.len() as u64);
        h2.set_mode(0o755);
        h2.set_cksum();
        builder.append_data(&mut h2, "bin/WeChat-MCP", &content[..]).unwrap();

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
    let col_res = safe_unpack_and_verify(&collision_archive, &stage_col, &col_artifact, &components);
    assert!(col_res.is_err());
    let err_str = col_res.unwrap_err().to_string();
    assert!(err_str.contains("case_collision_detected"), "Actual error was: {}", err_str);

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
    assert!(bomb_res.unwrap_err().to_string().contains("unpacked_size_limit_exceeded"));

    // 6. 对抗包测试：条目数超限
    let count_bomb_artifact = ArtifactSpec {
        filename: "valid.tar.gz".into(),
        sha256: archive_sha.clone(),
        size_bytes: meta.len(),
        max_unpacked_bytes: 10000,
        max_file_count: 0, // 允许 0 个文件
    };
    let count_res = safe_unpack_and_verify(&valid_archive, &staging, &count_bomb_artifact, &components);
    assert!(count_res.is_err());
    assert!(count_res.unwrap_err().to_string().contains("file_count_limit_exceeded"));

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
        builder.append_data(&mut h1, "bin/unknown_backdoor", &content[..]).unwrap();

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
    assert!(unk_res.unwrap_err().to_string().contains("unexpected_file_in_archive"));

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
        builder.append_data(&mut h1, "bin/wechat-mcp", &content[..]).unwrap();

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
    assert!(su_res.unwrap_err().to_string().contains("forbidden_special_permissions"));
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
