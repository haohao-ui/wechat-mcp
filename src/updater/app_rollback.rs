//! Offline whole-bundle rollback for the existing `deploy_host.sh` installation.
//! Business data and the versioned updater's release records are never changed.
use super::journal::{TransactionStage, UpdateJournal, acquire_update_lock, sync_dir};
use super::manual_rollback::RollbackStatus;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const NAME: &str = "微信客服助手.app";
const JOURNAL: &str = "app-rollback-journal.json";

pub struct AppRollback {
    app: PathBuf,
    backups: PathBuf,
    enabled: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Intent {
    journal: UpdateJournal,
    original_inode: u64,
    original_device: u64,
    was_enabled: bool,
    #[serde(default)]
    cleanup_complete: bool,
}

fn write_json(state: &Path, name: &str, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = state.join(format!(".{name}.{}", uuid::Uuid::new_v4()));
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

fn plist(app: &Path) -> Result<serde_json::Value> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()?;
    ensure!(
        output.status.success(),
        "备份的版本信息无法读取，请重新安装应用。"
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn version(app: &Path) -> Result<String> {
    let p = plist(app)?;
    let v = p["CFBundleVersion"]
        .as_str()
        .filter(|v| !v.trim().is_empty() && v.len() <= 128 && !v.chars().any(char::is_control));
    v.map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("备份没有有效版本号，无法回滚。"))
}

fn validate(app: &Path) -> Result<String> {
    ensure!(
        fs::symlink_metadata(app)?.is_dir(),
        "备份不完整，无法回滚。"
    );
    let p = plist(app)?;
    ensure!(
        p["CFBundleIdentifier"] == "local.wechat-mcp.host"
            && p["CFBundleExecutable"] == "wechat-cs-host",
        "备份不是有效的微信客服助手应用。"
    );
    for name in [
        "MacOS/wechat-cs-host",
        "Resources/brain-default-role.json",
        "Resources/gateway.test.json",
        "Resources/AppIcon.icns",
    ] {
        let path = app.join("Contents").join(name);
        ensure!(
            fs::metadata(&path)?.len() > 0
                && path.is_file()
                && path.canonicalize()?.starts_with(app.canonicalize()?),
            "备份不完整，无法回滚。"
        );
    }
    ensure!(
        fs::metadata(app.join("Contents/MacOS/wechat-cs-host"))?
            .permissions()
            .mode()
            & 0o111
            != 0,
        "备份的应用程序不可运行。"
    );
    let signed = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    ensure!(signed.success(), "备份已损坏或签名校验未通过，无法回滚。");
    version(app)
}

// Flush copied program files before the atomic rename can expose them after a crash.
fn sync_bundle(root: &Path, path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        ensure!(
            path.canonicalize()?.starts_with(root),
            "external_bundle_link"
        );
    } else if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            sync_bundle(root, &entry?.path())?;
        }
        sync_dir(path)?;
    } else {
        ensure!(meta.is_file(), "invalid_bundle_file");
        fs::File::open(path)?.sync_all()?;
    }
    Ok(())
}

fn publish_marker(state: &Path, tx: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temp = state.join(format!(".app-maintenance-{tx}"));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    f.write_all(tx.as_bytes())?;
    f.sync_all()?;
    // link publishes the complete UUID without replacing another owner's marker.
    fs::hard_link(&temp, state.join("core.maintenance"))?;
    fs::remove_file(temp)?;
    sync_dir(state)
}

// macOS RENAME_SWAP exchanges two directories atomically on the same volume.
fn exchange(a: &Path, b: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    #[cfg(target_os = "macos")]
    let rc = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    #[cfg(not(target_os = "macos"))]
    let rc = -1;
    ensure!(rc == 0, "应用替换失败，当前版本已保留。");
    Ok(())
}

impl AppRollback {
    #[must_use]
    pub fn for_home(home: &Path) -> Self {
        Self {
            app: home.join("Applications").join(NAME),
            backups: home.join(".wechat-rpa/backups"),
            enabled: home.join(".wechat-rpa/core.enabled"),
        }
    }

    fn candidate(&self) -> Result<(PathBuf, u64)> {
        ensure!(self.backups.exists(), "没有找到上一版本的应用备份。");
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&self.backups)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(ts) = name
                .strip_prefix(&format!("{NAME}.before-"))
                .and_then(|n| n.strip_suffix(".bak"))
                .and_then(|n| n.parse::<u64>().ok())
            {
                candidates.push((entry.path(), ts));
            }
        }
        candidates
            .into_iter()
            .max_by_key(|(_, ts)| *ts)
            .ok_or_else(|| anyhow::anyhow!("没有找到上一版本的应用备份。"))
    }

    fn load(state: &Path) -> Result<Option<Intent>> {
        let path = state.join(JOURNAL);
        if !path.try_exists()? {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
    }

    fn check_idle(state: &Path) -> Result<()> {
        ensure!(
            !state.join("core.maintenance").try_exists()?
                && !state.join("core.maintenance.json").try_exists()?,
            "上次维护尚未完成，请先恢复当前版本。"
        );
        if let Some(i) = Self::load(state)? {
            ensure!(
                matches!(
                    i.journal.stage,
                    TransactionStage::Committed | TransactionStage::RolledBack
                ) && i.cleanup_complete,
                "上次回滚尚未完成，请先恢复当前版本。"
            );
        }
        if let Some(j) = UpdateJournal::load_from_state(state)? {
            ensure!(
                matches!(j.stage, TransactionStage::Committed),
                "应用更新尚未完成，请先完成更新恢复。"
            );
        }
        Ok(())
    }

    /// Read-only availability query. It does not create locks or stop services.
    #[must_use]
    pub fn status(&self, state: &Path) -> RollbackStatus {
        let mut out = RollbackStatus {
            version: None,
            installed_at: None,
            can_rollback: false,
            reason: None,
        };
        let check = (|| -> Result<()> {
            Self::check_idle(state)?;
            let (backup, _) = self.candidate()?;
            // Backup timestamps identify order, not the old application's install time.
            // Existing deployment backups contain no trustworthy installed_at receipt.
            out.version = Some(version(&backup)?);
            validate(&backup)?;
            ensure!(
                fs::symlink_metadata(&self.app)?.is_dir(),
                "invalid_current_app"
            );
            ensure!(
                version(&self.app)? != out.version.as_deref().unwrap_or_default(),
                "当前已是备份版本，没有可回滚的上一版本。"
            );
            Ok(())
        })();
        match check {
            Ok(()) => out.can_rollback = true,
            Err(e) => {
                let msg = e.to_string();
                // Only application-authored Chinese messages may reach the host.
                out.reason = Some(
                    if [
                        "没有找到上一版本的应用备份。",
                        "备份的版本信息无法读取，请重新安装应用。",
                        "备份没有有效版本号，无法回滚。",
                        "备份不完整，无法回滚。",
                        "备份不是有效的微信客服助手应用。",
                        "备份的应用程序不可运行。",
                        "备份已损坏或签名校验未通过，无法回滚。",
                        "当前已是备份版本，没有可回滚的上一版本。",
                        "上次维护尚未完成，请先恢复当前版本。",
                        "上次回滚尚未完成，请先恢复当前版本。",
                        "应用更新尚未完成，请先完成更新恢复。",
                    ]
                    .contains(&msg.as_str())
                    {
                        msg
                    } else {
                        "无法验证应用备份或回滚记录，请检查备份是否完整。".into()
                    },
                );
            }
        }
        out
    }

    fn saved(&self, tx: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(tx)?;
        Ok(self
            .app
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid_app"))?
            .join(format!(".rollback-current-{tx}.app")))
    }

    fn stop(&self, state: &Path) -> Result<fs::File> {
        if self.enabled.try_exists()? {
            fs::remove_file(&self.enabled)?;
            sync_dir(self.enabled.parent().unwrap())?;
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while crate::core::serve::core_is_running(state) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        crate::core::serve::acquire_core_lock_for_maintenance(state)
    }

    fn finish(&self, state: &Path, i: &mut Intent) -> Result<()> {
        if i.cleanup_complete {
            return Ok(());
        }
        // Restore the pre-operation enable switch; host/core must be restarted by the caller.
        if i.was_enabled {
            fs::write(&self.enabled, b"")?;
            sync_dir(self.enabled.parent().unwrap())?;
        }
        let marker = state.join("core.maintenance");
        if marker.try_exists()? {
            ensure!(
                fs::read_to_string(&marker)? == i.journal.transaction_id,
                "maintenance_owner_changed"
            );
            fs::remove_file(marker)?;
            sync_dir(state)?;
        }
        i.cleanup_complete = true;
        write_json(state, JOURNAL, i)
    }

    /// Stops the core, stages and verifies the bundle, then atomically exchanges it.
    /// # Errors
    /// Fails closed on invalid backup, stale confirmation, busy core or IO failure.
    pub fn execute(&self, state: &Path, expected: &str, now: u64) -> Result<UpdateJournal> {
        self.execute_inner(state, expected, now, false)
    }

    fn execute_inner(
        &self,
        state: &Path,
        expected: &str,
        now: u64,
        fail_after_switch: bool,
    ) -> Result<UpdateJournal> {
        let _update = acquire_update_lock(state)?;
        Self::check_idle(state)?;
        let (backup, _) = self.candidate()?;
        ensure!(validate(&backup)? == expected, "previous_version_changed");
        let source = validate(&self.app)?;
        ensure!(source != expected, "already_installed");
        let meta = fs::symlink_metadata(&self.app)?;
        ensure!(meta.is_dir(), "invalid_current_app");
        let tx = uuid::Uuid::new_v4().to_string();
        let saved = self.saved(&tx)?;
        let mut i = Intent {
            journal: UpdateJournal::new(&tx, source, expected, 0, now),
            original_inode: meta.ino(),
            original_device: meta.dev(),
            was_enabled: self.enabled.try_exists()?,
            cleanup_complete: false,
        };
        i.journal.stage = TransactionStage::RollbackStarted;
        write_json(state, JOURNAL, &i)?;
        // Keep the core lock through both replacement and any failure restoration.
        let mut core_guard = None;
        let op = (|| -> Result<()> {
            core_guard = Some(self.stop(state)?);
            publish_marker(state, &tx)?;
            i.journal.stage = TransactionStage::Draining;
            write_json(state, JOURNAL, &i)?;
            ensure!(
                Command::new("/usr/bin/ditto")
                    .arg(&backup)
                    .arg(&saved)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()?
                    .success(),
                "copy_failed"
            );
            ensure!(validate(&saved)? == expected, "staged_backup_invalid");
            sync_bundle(&saved.canonicalize()?, &saved)?;
            sync_dir(self.app.parent().unwrap())?;
            i.journal.stage = TransactionStage::BackupVerified;
            write_json(state, JOURNAL, &i)?;
            i.journal.stage = TransactionStage::Switching;
            write_json(state, JOURNAL, &i)?;
            exchange(&self.app, &saved)?;
            sync_dir(self.app.parent().unwrap())?;
            if fail_after_switch {
                bail!("injected_failure");
            }
            ensure!(validate(&self.app)? == expected, "installed_bundle_invalid");
            i.journal.stage = TransactionStage::Committed;
            write_json(state, JOURNAL, &i)?;
            self.finish(state, &mut i)?;
            Ok(())
        })();
        if op.is_err() {
            // Record that we must restore the source, even if commit was already written.
            // A later recovery must not mistake a failed cleanup/undo for a successful commit.
            i.journal.stage = TransactionStage::RecoveryRequired;
            i.journal.error = Some("rollback_recovery_required".into());
            i.cleanup_complete = false;
            // Still attempt the physical restore if the journal volume itself has failed.
            let _ = write_json(state, JOURNAL, &i);
            self.restore(state, &mut i)?;
            bail!("rollback_failed_current_restored");
        }
        Ok(i.journal)
    }

    fn restore(&self, state: &Path, i: &mut Intent) -> Result<()> {
        let saved = self.saved(&i.journal.transaction_id)?;
        let current = fs::symlink_metadata(&self.app)?;
        if current.ino() != i.original_inode || current.dev() != i.original_device {
            let original = fs::symlink_metadata(&saved)?;
            ensure!(
                original.ino() == i.original_inode && original.dev() == i.original_device,
                "invalid_recovery_copy"
            );
            exchange(&self.app, &saved)?;
            sync_dir(self.app.parent().unwrap())?;
        }
        i.cleanup_complete = false;
        i.journal.stage = TransactionStage::RolledBack;
        i.journal.error = Some("rollback_failed_current_restored".into());
        write_json(state, JOURNAL, i)?;
        self.finish(state, i)
    }

    /// Restores the original bundle after an interrupted operation; safe to retry.
    /// # Errors
    /// Refuses invalid recovery evidence or an unavailable core lock.
    pub fn recover(&self, state: &Path) -> Result<UpdateJournal> {
        let _update = acquire_update_lock(state)?;
        let mut i = Self::load(state)?.ok_or_else(|| anyhow::anyhow!("no_rollback_journal"))?;
        if matches!(
            i.journal.stage,
            TransactionStage::Committed | TransactionStage::RolledBack
        ) {
            let expected = if i.journal.stage == TransactionStage::Committed {
                &i.journal.target_release_id
            } else {
                &i.journal.source_release_id
            };
            ensure!(version(&self.app)? == *expected, "terminal_bundle_changed");
            self.finish(state, &mut i)?;
            return Ok(i.journal);
        }
        let _core = self.stop(state)?;
        self.restore(state, &mut i)?;
        Ok(i.journal)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    struct Fixture {
        home: PathBuf,
        state: PathBuf,
        rollback: AppRollback,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }
    fn bundle(path: &Path, v: &str) {
        fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(path.join("Contents/Resources")).unwrap();
        fs::copy("/usr/bin/true", path.join("Contents/MacOS/wechat-cs-host")).unwrap();
        fs::copy(
            "native/bootstrap/Info.plist",
            path.join("Contents/Info.plist"),
        )
        .unwrap();
        assert!(
            Command::new("/usr/bin/plutil")
                .args(["-replace", "CFBundleVersion", "-string", v])
                .arg(path.join("Contents/Info.plist"))
                .status()
                .unwrap()
                .success()
        );
        for name in [
            "brain-default-role.json",
            "gateway.test.json",
            "AppIcon.icns",
        ] {
            fs::copy(
                Path::new("native/bootstrap").join(name),
                path.join("Contents/Resources").join(name),
            )
            .unwrap();
        }
        assert!(
            Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-", "--timestamp=none"])
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    fn fixture() -> Fixture {
        let home = std::env::temp_dir().join(format!("app-rollback-test-{}", uuid::Uuid::new_v4()));
        let rollback = AppRollback::for_home(&home);
        let state = home.join(".wechat-rpa/state");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&rollback.backups).unwrap();
        bundle(&rollback.app, "2.0");
        bundle(
            &rollback.backups.join(format!("{NAME}.before-100.bak")),
            "1.0",
        );
        fs::write(&rollback.enabled, b"").unwrap();
        Fixture {
            home,
            state,
            rollback,
        }
    }
    #[test]
    fn available_bundle_is_swapped_and_current_preserved() {
        let f = fixture();
        let q = f.rollback.status(&f.state);
        assert!(q.can_rollback, "{:?}", q.reason);
        assert_eq!(q.version.as_deref(), Some("1.0"));
        assert_eq!(q.installed_at, None);
        fs::write(f.state.join("business.db"), b"untouched").unwrap();
        let j = f.rollback.execute(&f.state, "1.0", 200).unwrap();
        assert_eq!(version(&f.rollback.app).unwrap(), "1.0");
        assert_eq!(
            validate(&f.rollback.saved(&j.transaction_id).unwrap()).unwrap(),
            "2.0"
        );
        assert_eq!(fs::read(f.state.join("business.db")).unwrap(), b"untouched");
        assert!(f.rollback.enabled.exists());
        assert!(!f.state.join("core.maintenance").exists());
        assert!(!f.state.join("update-journal.json").exists());
        assert!(!f.rollback.status(&f.state).can_rollback);
    }
    #[test]
    fn missing_backup_query_does_not_write() {
        let f = fixture();
        fs::remove_dir_all(&f.rollback.backups).unwrap();
        let q = f.rollback.status(&f.state);
        assert!(!q.can_rollback);
        assert_eq!(q.reason.as_deref(), Some("没有找到上一版本的应用备份。"));
        assert_eq!(fs::read_dir(&f.state).unwrap().count(), 0);
    }
    #[test]
    fn corrupt_and_unreadable_version_are_disabled() {
        let f = fixture();
        let backup = f.rollback.candidate().unwrap().0;
        fs::write(
            backup.join("Contents/Resources/brain-default-role.json"),
            "damaged",
        )
        .unwrap();
        let q = f.rollback.status(&f.state);
        assert!(!q.can_rollback);
        assert!(q.reason.unwrap().contains("签名"));
        assert!(f.rollback.execute(&f.state, "1.0", 200).is_err());
        assert_eq!(version(&f.rollback.app).unwrap(), "2.0");
        fs::write(
            backup.join("Contents/Info.plist"),
            "secret-provider-raw-error",
        )
        .unwrap();
        let q = f.rollback.status(&f.state);
        assert!(!q.can_rollback);
        let json = serde_json::to_string(&q).unwrap();
        assert!(!json.contains("secret-provider"));
        assert!(!json.contains(&f.home.to_string_lossy().to_string()));
        assert!(q.version.is_none());
    }
    #[test]
    fn newest_backup_is_selected_without_skipping_corruption() {
        let f = fixture();
        let newest = f.rollback.backups.join(format!("{NAME}.before-999.bak"));
        fs::create_dir(&newest).unwrap();
        assert_eq!(f.rollback.candidate().unwrap().0, newest);
        assert!(!f.rollback.status(&f.state).can_rollback);
        assert!(f.rollback.execute(&f.state, "1.0", 200).is_err());
    }
    #[test]
    fn failure_after_exchange_restores_current_and_records_failure() {
        let f = fixture();
        let original = fs::metadata(&f.rollback.app).unwrap().ino();
        assert!(
            f.rollback
                .execute_inner(&f.state, "1.0", 200, true)
                .is_err()
        );
        assert_eq!(fs::metadata(&f.rollback.app).unwrap().ino(), original);
        assert_eq!(validate(&f.rollback.app).unwrap(), "2.0");
        assert_eq!(
            AppRollback::load(&f.state).unwrap().unwrap().journal.stage,
            TransactionStage::RolledBack
        );
        assert!(f.rollback.enabled.exists());
        assert!(!f.state.join("core.maintenance").exists());
        f.rollback.recover(&f.state).unwrap();
        assert_eq!(version(&f.rollback.app).unwrap(), "2.0");
    }
    #[test]
    fn recovery_handles_crash_on_either_side_of_atomic_exchange() {
        for swapped in [false, true] {
            let f = fixture();
            let tx = uuid::Uuid::new_v4().to_string();
            let meta = fs::metadata(&f.rollback.app).unwrap();
            let mut j = UpdateJournal::new(&tx, "2.0", "1.0", 0, 200);
            j.stage = TransactionStage::Switching;
            let i = Intent {
                journal: j,
                original_inode: meta.ino(),
                original_device: meta.dev(),
                was_enabled: true,
                cleanup_complete: false,
            };
            write_json(&f.state, JOURNAL, &i).unwrap();
            fs::write(f.state.join("core.maintenance"), tx.as_bytes()).unwrap();
            fs::remove_file(&f.rollback.enabled).unwrap();
            let saved = f.rollback.saved(&tx).unwrap();
            bundle(&saved, "1.0");
            if swapped {
                exchange(&f.rollback.app, &saved).unwrap();
            }
            assert!(!f.rollback.status(&f.state).can_rollback);
            f.rollback.recover(&f.state).unwrap();
            assert_eq!(validate(&f.rollback.app).unwrap(), "2.0");
            assert!(f.rollback.enabled.exists());
            assert!(!f.state.join("core.maintenance").exists());
        }
    }
    #[test]
    fn stale_confirmation_and_busy_updater_preserve_current() {
        let f = fixture();
        assert!(f.rollback.execute(&f.state, "0.9", 200).is_err());
        let lock = acquire_update_lock(&f.state).unwrap();
        assert!(f.rollback.execute(&f.state, "1.0", 200).is_err());
        drop(lock);
        assert_eq!(validate(&f.rollback.app).unwrap(), "2.0");
    }
    #[test]
    fn stops_running_core_before_exchange() {
        let f = fixture();
        let core = crate::core::serve::acquire_core_lock(&f.state).unwrap();
        assert!(f.rollback.status(&f.state).can_rollback);
        let enabled = f.rollback.enabled.clone();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while enabled.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(!enabled.exists(), "rollback must request graceful shutdown");
                drop(core);
            });
            f.rollback.execute(&f.state, "1.0", 200).unwrap();
        });
        assert_eq!(validate(&f.rollback.app).unwrap(), "1.0");
    }

    #[test]
    fn committed_recovery_cleans_marker_without_reverting_bundle() {
        let f = fixture();
        let j = f.rollback.execute(&f.state, "1.0", 200).unwrap();
        fs::write(f.state.join("core.maintenance"), &j.transaction_id).unwrap();
        let mut i = AppRollback::load(&f.state).unwrap().unwrap();
        i.cleanup_complete = false;
        write_json(&f.state, JOURNAL, &i).unwrap();
        let recovered = f.rollback.recover(&f.state).unwrap();
        assert_eq!(recovered.stage, TransactionStage::Committed);
        assert_eq!(validate(&f.rollback.app).unwrap(), "1.0");
        assert!(!f.state.join("core.maintenance").exists());
        fs::remove_file(&f.rollback.enabled).unwrap();
        f.rollback.recover(&f.state).unwrap();
        assert_eq!(validate(&f.rollback.app).unwrap(), "1.0");
        assert!(
            !f.rollback.enabled.exists(),
            "repeat recovery must preserve a later user stop"
        );
    }

    #[test]
    fn cli_contract_and_redaction() {
        let f = fixture();
        let config = f.home.join("bot.json");
        fs::write(&config, serde_json::to_vec(&serde_json::json!({"schema_version":1,"host":"local","account_id":"test-account","allowlist":[{"recipient":"test","kind":"private"}],"state_dir":f.state})).unwrap()).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let exe = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("wechat-updater");
        let run = |args: &[&str]| {
            Command::new(&exe)
                .env("HOME", &f.home)
                .args(args)
                .arg("--config")
                .arg(&config)
                .output()
                .unwrap()
        };
        let query = run(&["rollback-status"]);
        assert!(
            query.status.success(),
            "{}",
            String::from_utf8_lossy(&query.stdout)
        );
        let q: serde_json::Value = serde_json::from_slice(&query.stdout).unwrap();
        assert_eq!(q["can_rollback"], true);
        assert_eq!(q.as_object().unwrap().len(), 4);
        let execute = run(&["rollback", "--expected-version", "1.0"]);
        assert!(
            execute.status.success(),
            "{}",
            String::from_utf8_lossy(&execute.stdout)
        );
        let j: serde_json::Value = serde_json::from_slice(&execute.stdout).unwrap();
        assert_eq!(j["status"], "rolled_back");
        assert_eq!(j["version"], "1.0");
        assert_eq!(j["restart_required"], true);
        assert!(j["transaction_id"].is_string());
        fs::write(&config, "secret-marker-invalid-json").unwrap();
        let bad = run(&["rollback-status"]);
        assert_eq!(bad.status.code(), Some(2));
        assert!(!String::from_utf8_lossy(&bad.stdout).contains("secret-marker"));
        assert_eq!(bad.stderr, Vec::<u8>::new());
    }
}
