use super::journal::*;
use super::manifest::*;
use super::rollback::*;
use super::signing::*;
use super::unpack::*;
use sha2::{Digest, Sha256};
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
    // 转换为 hex 字符串
    hex_encode(sig.as_ref())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
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
            history_schema_max: 3,
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

    // 正常通过
    assert!(validate_manifest_compatibility(&manifest, "arm64", "13.2.1", 1004).is_ok());

    // 架构不符拒绝
    let err_arch =
        validate_manifest_compatibility(&manifest, "x86_64", "13.2.1", 1004).unwrap_err();
    assert!(err_arch.to_string().contains("architecture_mismatch"));

    // 系统版本过低拒绝
    let err_os = validate_manifest_compatibility(&manifest, "arm64", "11.5", 1004).unwrap_err();
    assert!(err_os.to_string().contains("os_version_too_low"));

    // 防降级拒绝：candidate sequence (1005) <= highest accepted (1005)
    let err_down1 =
        validate_manifest_compatibility(&manifest, "arm64", "13.2.1", 1005).unwrap_err();
    assert!(err_down1.to_string().contains("downgrade_rejected"));

    // 防降级拒绝：candidate sequence (1005) <= highest accepted (1010)
    let err_down2 =
        validate_manifest_compatibility(&manifest, "arm64", "13.2.1", 1010).unwrap_err();
    assert!(err_down2.to_string().contains("downgrade_rejected"));
}

#[test]
fn test_safe_unpack_rejects_symlink_and_traversal() {
    let temp = TempTestDir::new("unpack_safety");
    let staging = temp.0.join("staging");

    // 创建一个包含软链接的 tar.gz
    let bad_src = temp.0.join("bad_src");
    fs::create_dir_all(&bad_src).unwrap();
    let target_file = bad_src.join("target.txt");
    fs::write(&target_file, b"content").unwrap();
    let bad_symlink = bad_src.join("link.txt");
    std::os::unix::fs::symlink(&target_file, &bad_symlink).unwrap();

    let tar_path = temp.0.join("bad.tar.gz");
    let st = std::process::Command::new("/usr/bin/tar")
        .arg("-czf")
        .arg(&tar_path)
        .arg("-C")
        .arg(&bad_src)
        .arg(".")
        .status()
        .unwrap();
    assert!(st.success());

    let tar_len = fs::metadata(&tar_path).unwrap().len();
    let tar_sha = {
        let bytes = fs::read(&tar_path).unwrap();
        let mut h = Sha256::new();
        h.update(&bytes);
        hex_encode(&h.finalize())
    };

    let artifact = ArtifactSpec {
        filename: "bad.tar.gz".into(),
        sha256: tar_sha,
        size_bytes: tar_len,
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };

    let comp = ComponentSpec {
        path: "target.txt".into(),
        kind: "regular_file".into(),
        file_mode: 0o644,
        sha256: "ed70c574b5e69799f14d61464a33d1515861bd7aa421371ec61fa62b789a14f2".into(),
        size_bytes: 7,
    };

    let res = safe_unpack_and_verify(&tar_path, &staging, &artifact, &[comp]);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("symlink_rejected") || err_msg.contains("unexpected_file"));
    // 确保解包失败后 staging 目录被全量清理
    assert!(!staging.exists());
}

#[test]
fn test_safe_unpack_valid_package_and_whitelist() {
    let temp = TempTestDir::new("unpack_valid");
    let staging = temp.0.join("staging");

    let good_src = temp.0.join("good_src");
    let bin_dir = good_src.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let bin_file = bin_dir.join("wechat-mcp");
    fs::write(&bin_file, b"executable-content").unwrap();
    fs::set_permissions(&bin_file, fs::Permissions::from_mode(0o755)).unwrap();

    let tar_path = temp.0.join("good.tar.gz");
    let st = std::process::Command::new("/usr/bin/tar")
        .arg("-czf")
        .arg(&tar_path)
        .arg("-C")
        .arg(&good_src)
        .arg("bin")
        .status()
        .unwrap();
    assert!(st.success());

    let tar_len = fs::metadata(&tar_path).unwrap().len();
    let tar_sha = {
        let bytes = fs::read(&tar_path).unwrap();
        let mut h = Sha256::new();
        h.update(&bytes);
        hex_encode(&h.finalize())
    };

    let artifact = ArtifactSpec {
        filename: "good.tar.gz".into(),
        sha256: tar_sha,
        size_bytes: tar_len,
        max_unpacked_bytes: 100000,
        max_file_count: 10,
    };

    let file_sha = {
        let bytes = fs::read(&bin_file).unwrap();
        let mut h = Sha256::new();
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

    // 成功解包并核对白名单
    let res = safe_unpack_and_verify(&tar_path, &staging, &artifact, &[comp.clone()]);
    assert!(res.is_ok());
    assert!(staging.join("bin/wechat-mcp").exists());

    // 若清单有多余未提供文件 -> 拒绝
    let missing_comp = ComponentSpec {
        path: "bin/desktop_helper".into(),
        kind: "executable".into(),
        file_mode: 0o755,
        sha256: "00".repeat(32),
        size_bytes: 10,
    };
    let res_missing = safe_unpack_and_verify(&tar_path, &staging, &artifact, &[comp, missing_comp]);
    assert!(res_missing.is_err());
    assert!(
        res_missing
            .unwrap_err()
            .to_string()
            .contains("missing_component")
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

    // Staged 阶段断电恢复评估 -> 丢弃候选包，旧版未受影响
    let current_link = temp.0.join("current");
    let releases_dir = temp.0.join("releases");
    let act = loaded.evaluate_recovery(&releases_dir, &current_link);
    assert_eq!(act, RecoveryAction::DiscardCandidate);

    // 推进阶段到 CandidateTesting
    journal.stage = TransactionStage::CandidateTesting;
    journal.persist_atomic(&state_dir).unwrap();
    let act2 = journal.evaluate_recovery(&releases_dir, &current_link);
    assert_eq!(act2, RecoveryAction::TestCandidate);

    // 推进阶段到 RecoveryRequired
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
}

#[test]
fn test_rollback_atomic_switch_and_previous() {
    let temp = TempTestDir::new("rollback_test");
    let app_root = temp.0.clone();
    let releases = app_root.join("releases");
    let r1 = releases.join("0.1.0");
    let r2 = releases.join("0.2.0");
    fs::create_dir_all(&r1).unwrap();
    fs::create_dir_all(&r2).unwrap();

    // 首次切换到 0.1.0
    switch_release_atomic(&app_root, "0.1.0").unwrap();
    assert_eq!(fs::read_link(app_root.join("current")).unwrap(), r1);

    // 再次切换到 0.2.0（previous 应当指向 0.1.0）
    switch_release_atomic(&app_root, "0.2.0").unwrap();
    assert_eq!(fs::read_link(app_root.join("current")).unwrap(), r2);
    assert_eq!(fs::read_link(app_root.join("previous")).unwrap(), r1);

    // 执行回退 -> current 切回 0.1.0
    let target = rollback_release_atomic(&app_root).unwrap();
    assert_eq!(target, r1);
    assert_eq!(fs::read_link(app_root.join("current")).unwrap(), r1);
}

#[test]
fn test_has_new_data_prevents_silent_overwrite() {
    let temp = TempTestDir::new("db_protection");
    let db_path = temp.0.join("history.sqlite3");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY, created_at INTEGER, body TEXT);
             INSERT INTO messages (id, created_at, body) VALUES (1, 1000, 'old message');",
        )
        .unwrap();
    }

    // backup_time 为 1000：此时没有比 1000 更新的数据
    assert!(!has_new_data_since_backup(&db_path, 1000).unwrap());

    // 模拟新版本启动后产生了新消息（created_at = 2000）
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("INSERT INTO messages (id, created_at, body) VALUES (2, 2000, 'new message after update');", []).unwrap();
    }

    // 此时检测必须报告存在新数据！
    assert!(has_new_data_since_backup(&db_path, 1000).unwrap());
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
