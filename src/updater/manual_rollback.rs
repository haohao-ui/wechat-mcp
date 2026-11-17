//! Offline program rollback. Never restores business data or contacts a publisher.
use super::{
    executor::is_valid_release_id,
    journal::{
        CurrentReleaseCommitRecord, StartupDualCheckDecision, TransactionStage, UpdateJournal,
        acquire_update_lock, check_startup_guard_readonly_with_root, load_current_release_record,
        load_highest_accepted_sequence, sync_dir, verify_current_link_target_with_root,
    },
    rollback::verify_backup_restorable,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviousRelease {
    pub record: CurrentReleaseCommitRecord,
    pub files: BTreeMap<String, String>,
    pub database_schemas: BTreeMap<String, String>,
    pub backup: super::executor::VerifiedSandboxBackupProof,
    pub update_transaction_id: String,
    pub in_place_rollback_supported: bool,
}

// Keep the original journal schema readable by the previous application binary.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RollbackCommit {
    highest_sequence: u64,
    journal: UpdateJournal,
}

pub(super) fn committed_high_watermark(
    state: &Path,
    record: &CurrentReleaseCommitRecord,
) -> Result<Option<u64>> {
    let path = state.join("last-rollback.json");
    if !path.exists() {
        return Ok(None);
    }
    let proof: RollbackCommit = serde_json::from_slice(&fs::read(path)?)?;
    let current = UpdateJournal::load_from_state(state)?;
    Ok((proof.journal.stage == TransactionStage::Committed
        && proof.journal.transaction_id == record.tx_id
        && proof.journal.target_release_id == record.release_id
        && proof.journal.target_sequence == record.sequence
        && current.as_ref() == Some(&proof.journal))
    .then_some(proof.highest_sequence))
}

#[derive(Debug, Serialize)]
pub struct RollbackStatus {
    pub version: Option<String>,
    pub installed_at: Option<u64>,
    pub can_rollback: bool,
    pub reason: Option<String>,
}

fn digest_file(path: &Path) -> Result<String> {
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for b in Sha256::digest(fs::read(path)?) {
        write!(&mut hex, "{b:02x}")?;
    }
    Ok(hex)
}

fn inventory(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        let m = fs::symlink_metadata(&p)?;
        if m.file_type().is_symlink() {
            bail!("release_symlink_rejected");
        }
        if m.is_dir() {
            inventory(&p, base, out)?;
        } else if m.is_file() {
            out.insert(
                p.strip_prefix(base)?.to_string_lossy().into_owned(),
                format!("{:o}:{}", m.permissions().mode() & 0o777, digest_file(&p)?),
            );
        } else {
            bail!("release_special_file_rejected");
        }
    }
    Ok(())
}

/// Called under the update lock, after backup and before replacing current.
///
/// # Errors
/// Returns an error if release files or rollback evidence cannot be read or persisted.
pub fn preserve_previous(
    root: &Path,
    state: &Path,
    tx: &str,
    backup: &super::executor::VerifiedSandboxBackupProof,
    in_place_rollback_supported: bool,
) -> Result<()> {
    let Some(record) = load_current_release_record(state)? else {
        return Ok(());
    };
    // Interrupted legacy updates may have switched current without committing a record.
    // They provide no trustworthy previous metadata; never mint a rollback receipt for them.
    let current_target = fs::read_link(root.join("current"))?;
    if current_target != root.join("releases").join(&record.release_id)
        && current_target != Path::new("releases").join(&record.release_id)
    {
        return Ok(());
    }
    verify_current_link_target_with_root(
        &root.join("current"),
        &root.join("releases"),
        &record.release_id,
        Some(root),
    )?;
    let mut files = BTreeMap::new();
    inventory(
        &root.join("releases").join(&record.release_id),
        &root.join("releases").join(&record.release_id),
        &mut files,
    )?;
    if files.is_empty() {
        bail!("empty_previous_release");
    }
    write_json(
        state,
        "previous-release.json",
        &PreviousRelease {
            record,
            database_schemas: database_schemas(state, backup)?,
            files,
            backup: backup.clone(),
            update_transaction_id: tx.into(),
            in_place_rollback_supported,
        },
    )
}

fn write_json(state: &Path, name: &str, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = state.join(format!("{name}.tmp.{}", uuid::Uuid::new_v4()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&serde_json::to_vec(value)?)?;
    f.sync_all()?;
    fs::rename(tmp, state.join(name))?;
    sync_dir(state)
}

type SchemaRows = Vec<(String, String, String, Option<String>)>;
fn schema(path: &Path) -> Result<SchemaRows> {
    if fs::metadata(format!("{}-wal", path.display())).is_ok_and(|m| m.len() > 0) {
        bail!("database_checkpoint_required");
    }
    let c =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if ok != "ok" {
        bail!("database_corrupt");
    }
    let user_version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let mut s = c.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")?;
    let mut rows = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.push((
        "pragma".into(),
        "user_version".into(),
        user_version.to_string(),
        None,
    ));
    Ok(rows)
}

fn database_schemas(
    state: &Path,
    backup: &super::executor::VerifiedSandboxBackupProof,
) -> Result<BTreeMap<String, String>> {
    fn walk(
        dir: &Path,
        base: &Path,
        excluded: Option<&Path>,
        out: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let p = entry?.path();
            if Some(p.as_path()) == excluded {
                continue;
            }
            let m = fs::symlink_metadata(&p)?;
            if m.file_type().is_symlink() {
                bail!("state_symlink_rejected");
            }
            if m.is_dir() {
                walk(&p, base, excluded, out)?;
            } else if matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("db" | "sqlite" | "sqlite3")
            ) {
                out.insert(
                    p.strip_prefix(base)?.to_string_lossy().into_owned(),
                    serde_json::to_string(&schema(&p)?)?,
                );
            }
        }
        Ok(())
    }
    let excluded = match backup {
        super::executor::VerifiedSandboxBackupProof::DatabaseBackupVerified {
            backup_path, ..
        } => Some(backup_path.as_path()),
        super::executor::VerifiedSandboxBackupProof::NoDatabaseVerified => None,
    };
    let mut out = BTreeMap::new();
    walk(state, state, excluded, &mut out)?;
    Ok(out)
}

fn validate(root: &Path, state: &Path, previous: &PreviousRelease) -> Result<()> {
    if !previous.in_place_rollback_supported {
        bail!("rollback_not_supported_by_release");
    }
    if !is_valid_release_id(&previous.record.release_id) {
        bail!("invalid_previous_release");
    }
    let current = load_current_release_record(state)?
        .ok_or_else(|| anyhow::anyhow!("missing_current_record"))?;
    if previous.record.record_schema != 1
        || previous.record.sequence >= current.sequence
        || previous.record.manifest_digest.len() != 64
        || !previous
            .record
            .manifest_digest
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    {
        bail!("invalid_previous_record");
    }
    let j =
        UpdateJournal::load_from_state(state)?.ok_or_else(|| anyhow::anyhow!("missing_journal"))?;
    if j.stage != TransactionStage::Committed
        || j.transaction_id != previous.update_transaction_id
        || current.tx_id != j.transaction_id
        || current.release_id != j.target_release_id
        || j.source_release_id != previous.record.release_id
    {
        bail!("no_matching_completed_update");
    }
    if !matches!(
        check_startup_guard_readonly_with_root(
            state,
            Some(&root.join("releases")),
            Some(&root.join("current")),
            Some(root),
            None
        ),
        StartupDualCheckDecision::AllowStartup
    ) {
        bail!("recovery_required");
    }
    verify_current_link_target_with_root(
        &root.join("previous"),
        &root.join("releases"),
        &previous.record.release_id,
        Some(root),
    )?;
    let dir = root.join("releases").join(&previous.record.release_id);
    let mut files = BTreeMap::new();
    inventory(&dir, &dir, &mut files)?;
    if files.is_empty() || files != previous.files {
        bail!("previous_release_corrupt");
    }
    match &previous.backup {
        super::executor::VerifiedSandboxBackupProof::DatabaseBackupVerified {
            backup_path,
            backup_sha256,
        } => {
            if digest_file(backup_path)? != *backup_sha256 {
                bail!("backup_corrupt");
            }
            verify_backup_restorable(backup_path)?;
        }
        super::executor::VerifiedSandboxBackupProof::NoDatabaseVerified => {}
    }
    if database_schemas(state, &previous.backup)? != previous.database_schemas {
        bail!("database_schema_changed");
    }
    load_highest_accepted_sequence(state)?;
    Ok(())
}

/// Pure read-only query: no locks are created and no state is rewritten.
#[must_use]
pub fn status(root: &Path, state: &Path) -> RollbackStatus {
    let mut result = RollbackStatus {
        version: None,
        installed_at: None,
        can_rollback: false,
        reason: None,
    };
    let check = (|| -> Result<()> {
        let p = state.join("previous-release.json");
        if !p.exists() {
            bail!("no_previous_version");
        }
        let previous: PreviousRelease = serde_json::from_slice(&fs::read(p)?)?;
        if !is_valid_release_id(&previous.record.release_id) {
            bail!("invalid_previous_release");
        }
        result.version = Some(previous.record.release_id.clone());
        result.installed_at = Some(previous.record.installed_at);
        if crate::core::serve::core_is_running(state) {
            bail!("stop_core_first");
        }
        validate(root, state, &previous)?;
        Ok(())
    })();
    match check {
        Ok(()) => result.can_rollback = true,
        Err(e) => {
            // Only fixed categories; never expose paths, config contents, or raw errors.
            let msg = e.to_string();
            result.reason = Some(match msg.as_str() {
                "no_previous_version"
                | "no_matching_completed_update"
                | "previous_release_corrupt"
                | "backup_corrupt"
                | "rollback_not_supported_by_release"
                | "database_schema_changed"
                | "database_checkpoint_required"
                | "database_state_changed"
                | "stop_core_first"
                | "recovery_required" => msg,
                _ => "rollback_state_invalid".into(),
            });
        }
    }
    result
}

/// Switches only program files with both updater and core locks held.
///
/// # Errors
/// Fails on stale confirmation, invalid evidence, busy core, or failed durable writes.
pub fn execute(root: &Path, state: &Path, expected: &str, now: u64) -> Result<UpdateJournal> {
    execute_inner(root, state, expected, now, false)
}

fn execute_inner(
    root: &Path,
    state: &Path,
    expected: &str,
    now: u64,
    fail_after_switch: bool,
) -> Result<UpdateJournal> {
    let _update = acquire_update_lock(state)?;
    let _core = crate::core::serve::acquire_core_lock_for_maintenance(state)?;
    let previous: PreviousRelease =
        serde_json::from_slice(&fs::read(state.join("previous-release.json"))?)?;
    validate(root, state, &previous)?;
    if previous.record.release_id != expected {
        bail!("previous_version_changed");
    }
    let old_j =
        UpdateJournal::load_from_state(state)?.ok_or_else(|| anyhow::anyhow!("missing_journal"))?;
    let old_record = load_current_release_record(state)?
        .ok_or_else(|| anyhow::anyhow!("missing_current_record"))?;
    let old_link = fs::read_link(root.join("current"))?;
    let highest = load_highest_accepted_sequence(state)?;
    let mut j = UpdateJournal::new(
        format!("tx-{}", uuid::Uuid::new_v4()),
        &old_record.release_id,
        &previous.record.release_id,
        previous.record.sequence,
        now,
    );
    j.stage = TransactionStage::RollbackStarted;
    j.manifest_digest = Some(previous.record.manifest_digest.clone());
    j.backup_proof = Some(serde_json::to_string(&previous.backup)?);
    // Persist undo evidence before any pointer/record mutation (also used after power loss).
    write_json(
        state,
        "rollback-undo.json",
        &(old_j.clone(), old_record.clone(), old_link.clone()),
    )?;
    write_json(
        state,
        "rollback-intent.json",
        &RollbackCommit {
            highest_sequence: highest,
            journal: j.clone(),
        },
    )?;
    j.persist_atomic(state)?;
    let operation = (|| -> Result<()> {
        replace_link(
            root,
            &root.join("releases").join(&previous.record.release_id),
        )?;
        if fail_after_switch {
            bail!("injected_rollback_failure");
        }
        let mut record = previous.record.clone();
        record.tx_id.clone_from(&j.transaction_id);
        // Keep the publisher anti-replay high watermark unchanged.
        write_json(state, "current_release.json", &record)?;
        j.stage = TransactionStage::Committed;
        write_json(
            state,
            "last-rollback.json",
            &RollbackCommit {
                highest_sequence: highest,
                journal: j.clone(),
            },
        )?;
        j.persist_atomic(state)?;
        Ok(())
    })();
    if operation.is_err() {
        replace_link(root, &old_link)?;
        write_json(state, "current_release.json", &old_record)?;
        old_j.persist_atomic(state)?;
        j.stage = TransactionStage::RolledBack;
        j.error = Some("rollback_failed_current_restored".into());
        write_json(
            state,
            "last-rollback.json",
            &RollbackCommit {
                highest_sequence: highest,
                journal: j.clone(),
            },
        )?;
        bail!("rollback_failed_current_restored");
    }
    Ok(j)
}

fn replace_link(root: &Path, target: &Path) -> Result<()> {
    let tmp: PathBuf = root.join(format!("current.tmp.{}", uuid::Uuid::new_v4()));
    symlink(target, &tmp)?;
    fs::rename(tmp, root.join("current"))?;
    sync_dir(root)
}

/// Interrupted rollback always resumes the previously running release, preserving data.
///
/// # Errors
/// Rejects missing or inconsistent undo evidence and fails if restoration cannot be persisted.
pub fn recover(root: &Path, state: &Path) -> Result<()> {
    let _update = acquire_update_lock(state)?;
    let _core = crate::core::serve::acquire_core_lock_for_maintenance(state)?;
    let active =
        UpdateJournal::load_from_state(state)?.ok_or_else(|| anyhow::anyhow!("missing_journal"))?;
    let intent: RollbackCommit =
        serde_json::from_slice(&fs::read(state.join("rollback-intent.json"))?)?;
    if active.stage != TransactionStage::RollbackStarted || intent.journal != active {
        bail!("no_interrupted_rollback");
    }
    let (old_j, old_record, old_link): (UpdateJournal, CurrentReleaseCommitRecord, PathBuf) =
        serde_json::from_slice(&fs::read(state.join("rollback-undo.json"))?)?;
    if old_j.stage != TransactionStage::Committed
        || old_j.transaction_id != old_record.tx_id
        || old_record.release_id != active.source_release_id
        || !is_valid_release_id(&old_record.release_id)
    {
        bail!("invalid_undo_record");
    }
    // Validate the undo pointer using the same anchored path checks before writing it.
    let expected = root.join("releases").join(&old_record.release_id);
    if old_link != expected && old_link != Path::new("releases").join(&old_record.release_id) {
        bail!("invalid_undo_pointer");
    }
    // The original current directory must still be a physical owned release.
    let meta = fs::symlink_metadata(&expected)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("invalid_undo_release");
    }
    replace_link(root, &old_link)?;
    write_json(state, "current_release.json", &old_record)?;
    old_j.persist_atomic(state)?;
    let mut failed = active;
    failed.stage = TransactionStage::RolledBack;
    failed.error = Some("interrupted_rollback_current_restored".into());
    write_json(
        state,
        "last-rollback.json",
        &RollbackCommit {
            highest_sequence: intent.highest_sequence,
            journal: failed,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::super::executor::{SandboxUpdateRoot, VerifiedSandboxBackupProof};
    use super::super::journal::atomic_commit_release_record;
    use super::super::rollback::switch_release_atomic;
    use super::*;
    fn fixture() -> SandboxUpdateRoot {
        let sandbox = SandboxUpdateRoot::create_test_isolated_sandbox().unwrap();
        let root = sandbox.root_dir();
        let state = sandbox.state_dir();
        for id in ["1.0", "2.0"] {
            fs::create_dir(root.join("releases").join(id)).unwrap();
            fs::write(root.join("releases").join(id).join("app"), id).unwrap();
        }
        symlink(root.join("releases/1.0"), root.join("current")).unwrap();
        let old = CurrentReleaseCommitRecord {
            record_schema: 1,
            tx_id: format!("tx-{}", uuid::Uuid::new_v4()),
            release_id: "1.0".into(),
            sequence: 1,
            channel: "stable".into(),
            manifest_digest: "a".repeat(64),
            installed_at: 100,
        };
        atomic_commit_release_record(&state, &old).unwrap();
        let tx = format!("tx-{}", uuid::Uuid::new_v4());
        preserve_previous(
            root,
            &state,
            &tx,
            &VerifiedSandboxBackupProof::NoDatabaseVerified,
            true,
        )
        .unwrap();
        switch_release_atomic(root, "2.0").unwrap();
        let mut current = old;
        current.release_id = "2.0".into();
        current.sequence = 2;
        current.tx_id = tx.clone();
        atomic_commit_release_record(&state, &current).unwrap();
        let mut j = UpdateJournal::new(tx, "1.0", "2.0", 2, 200);
        j.stage = TransactionStage::Committed;
        j.manifest_digest = Some("a".repeat(64));
        j.persist_atomic(&state).unwrap();
        sandbox
    }
    fn allowed(s: &SandboxUpdateRoot) -> bool {
        matches!(
            check_startup_guard_readonly_with_root(
                &s.state_dir(),
                Some(&s.releases_dir()),
                Some(&s.current_link()),
                Some(s.root_dir()),
                None
            ),
            StartupDualCheckDecision::AllowStartup
        )
    }
    #[test]
    fn rollback_success_preserves_high_watermark_and_allows_startup() {
        let s = fixture();
        assert!(status(s.root_dir(), &s.state_dir()).can_rollback);
        execute(s.root_dir(), &s.state_dir(), "1.0", 300).unwrap();
        assert_eq!(
            fs::read_link(s.current_link()).unwrap(),
            s.releases_dir().join("1.0")
        );
        assert_eq!(load_highest_accepted_sequence(&s.state_dir()).unwrap(), 2);
        assert!(allowed(&s));
        assert!(!status(s.root_dir(), &s.state_dir()).can_rollback);
    }
    #[test]
    fn no_previous_query_is_readonly() {
        let s = fixture();
        fs::remove_file(s.state_dir().join("previous-release.json")).unwrap();
        let before = fs::read(s.state_dir().join("update-journal.json")).unwrap();
        assert_eq!(
            status(s.root_dir(), &s.state_dir()).reason.as_deref(),
            Some("no_previous_version")
        );
        assert_eq!(
            before,
            fs::read(s.state_dir().join("update-journal.json")).unwrap()
        );
        assert!(allowed(&s));
    }
    #[test]
    fn corrupted_backup_rejects_before_switch() {
        let s = fixture();
        let p = s.state_dir().join("previous-release.json");
        let mut receipt: PreviousRelease = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        let backup = s.state_dir().join("backup.db");
        fs::write(&backup, "corrupt").unwrap();
        receipt.backup = VerifiedSandboxBackupProof::DatabaseBackupVerified {
            backup_path: backup,
            backup_sha256: "0".repeat(64),
        };
        write_json(&s.state_dir(), "previous-release.json", &receipt).unwrap();
        assert_eq!(
            status(s.root_dir(), &s.state_dir()).reason.as_deref(),
            Some("backup_corrupt")
        );
        assert!(execute(s.root_dir(), &s.state_dir(), "1.0", 300).is_err());
        assert!(allowed(&s));
    }
    #[test]
    fn failure_after_switch_restores_current_and_journal() {
        let s = fixture();
        assert!(execute_inner(s.root_dir(), &s.state_dir(), "1.0", 300, true).is_err());
        assert_eq!(
            fs::read_link(s.current_link()).unwrap(),
            s.releases_dir().join("2.0")
        );
        assert!(allowed(&s));
        assert!(s.state_dir().join("last-rollback.json").exists());
    }
    #[test]
    fn busy_and_stale_confirmation_are_rejected() {
        let s = fixture();
        let lock = crate::core::serve::acquire_core_lock(&s.state_dir()).unwrap();
        assert!(!status(s.root_dir(), &s.state_dir()).can_rollback);
        assert!(execute(s.root_dir(), &s.state_dir(), "1.0", 300).is_err());
        drop(lock);
        assert!(execute(s.root_dir(), &s.state_dir(), "0.9", 300).is_err());
        assert!(allowed(&s));
    }
    #[test]
    fn tampered_program_is_rejected() {
        let s = fixture();
        fs::write(s.releases_dir().join("1.0/app"), "changed").unwrap();
        assert!(!status(s.root_dir(), &s.state_dir()).can_rollback);
        assert!(allowed(&s));
    }

    #[test]
    fn database_rows_survive_and_schema_changes_block() {
        let s = fixture();
        let db = s.state_dir().join("history.sqlite3");
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE messages(body TEXT); INSERT INTO messages VALUES ('before');",
        )
        .unwrap();
        let p = s.state_dir().join("previous-release.json");
        let mut receipt: PreviousRelease = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        receipt.database_schemas = database_schemas(&s.state_dir(), &receipt.backup).unwrap();
        write_json(&s.state_dir(), "previous-release.json", &receipt).unwrap();
        c.execute("INSERT INTO messages VALUES ('after')", [])
            .unwrap();
        assert!(status(s.root_dir(), &s.state_dir()).can_rollback);
        execute(s.root_dir(), &s.state_dir(), "1.0", 300).unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        let s = fixture();
        let c = rusqlite::Connection::open(s.state_dir().join("new.sqlite3")).unwrap();
        c.execute_batch("CREATE TABLE new_data(id INTEGER)")
            .unwrap();
        assert_eq!(
            status(s.root_dir(), &s.state_dir()).reason.as_deref(),
            Some("database_schema_changed")
        );
    }
    #[test]
    fn interrupted_rollback_recovery_resumes_original_current() {
        let s = fixture();
        let old_j = UpdateJournal::load_from_state(&s.state_dir())
            .unwrap()
            .unwrap();
        let old_record = load_current_release_record(&s.state_dir())
            .unwrap()
            .unwrap();
        let old_link = fs::read_link(s.current_link()).unwrap();
        write_json(
            &s.state_dir(),
            "rollback-undo.json",
            &(old_j, old_record, old_link),
        )
        .unwrap();
        let mut j =
            UpdateJournal::new(format!("tx-{}", uuid::Uuid::new_v4()), "2.0", "1.0", 1, 300);
        j.stage = TransactionStage::RollbackStarted;
        write_json(
            &s.state_dir(),
            "rollback-intent.json",
            &RollbackCommit {
                highest_sequence: 2,
                journal: j.clone(),
            },
        )
        .unwrap();
        j.persist_atomic(&s.state_dir()).unwrap();
        replace_link(s.root_dir(), &s.releases_dir().join("1.0")).unwrap();
        assert!(!allowed(&s));
        recover(s.root_dir(), &s.state_dir()).unwrap();
        assert!(allowed(&s));
        assert_eq!(
            fs::read_link(s.current_link()).unwrap(),
            s.releases_dir().join("2.0")
        );
    }

    #[test]
    fn cli_query_execute_and_redacted_errors() {
        use std::os::unix::fs::PermissionsExt;
        let s = fixture();
        let config = s.root_dir().join("bot.json");
        fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({
                "schema_version":1,"host":"local","account_id":"test-account",
                "allowlist":[{"recipient":"test","kind":"private"}],
                "state_dir":s.state_dir(),"install_root":s.root_dir()
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let exe = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("wechat-updater");
        let query = std::process::Command::new(&exe)
            .args(["rollback-status", "--config"])
            .arg(&config)
            .output()
            .unwrap();
        assert!(
            query.status.success(),
            "{}",
            String::from_utf8_lossy(&query.stdout)
        );
        let value: serde_json::Value = serde_json::from_slice(&query.stdout).unwrap();
        assert_eq!(value["version"], "1.0");
        assert_eq!(value["can_rollback"], true);
        let run = std::process::Command::new(&exe)
            .args(["rollback", "--config"])
            .arg(&config)
            .args(["--expected-version", "1.0"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stdout)
        );
        assert!(allowed(&s));
        fs::write(&config, "secret-marker-not-json").unwrap();
        let bad = std::process::Command::new(&exe)
            .args(["rollback-status", "--config"])
            .arg(&config)
            .output()
            .unwrap();
        assert_eq!(bad.status.code(), Some(2));
        assert!(!String::from_utf8_lossy(&bad.stdout).contains("secret-marker"));
        assert!(bad.stderr.is_empty());
    }

    #[test]
    fn publisher_opt_out_and_damaged_commit_proof_fail_closed() {
        let s = fixture();
        let p = s.state_dir().join("previous-release.json");
        let mut receipt: PreviousRelease = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        receipt.in_place_rollback_supported = false;
        write_json(&s.state_dir(), "previous-release.json", &receipt).unwrap();
        assert_eq!(
            status(s.root_dir(), &s.state_dir()).reason.as_deref(),
            Some("rollback_not_supported_by_release")
        );
        receipt.in_place_rollback_supported = true;
        write_json(&s.state_dir(), "previous-release.json", &receipt).unwrap();
        execute(s.root_dir(), &s.state_dir(), "1.0", 300).unwrap();
        fs::remove_file(s.state_dir().join("highest_accepted_sequence")).unwrap();
        assert_eq!(load_highest_accepted_sequence(&s.state_dir()).unwrap(), 2);
        fs::write(s.state_dir().join("last-rollback.json"), "bad proof").unwrap();
        assert!(load_highest_accepted_sequence(&s.state_dir()).is_err());
    }
}
