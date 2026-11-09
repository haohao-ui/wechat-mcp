//! Durable terminal outcomes; active turns and their channels remain in memory.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_RECORDS: usize = 1000;
const MAX_AGE: u64 = 7 * 86400;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalRecord {
    pub turn_id: String,
    pub outcome: String,
    pub action: Option<String>,
    pub text: Option<String>,
    pub text_sha256: Option<String>,
    pub at: u64,
}

#[derive(Serialize, Deserialize)]
struct LedgerFile {
    version: u32,
    terminal: Vec<TerminalRecord>,
}

pub struct TurnLedger {
    path: PathBuf,
    terminal: Vec<TerminalRecord>,
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn text_sha256(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    Sha256::digest(text.as_bytes())
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}

impl TurnLedger {
    pub fn open(state_dir: &Path) -> Result<Self> {
        let dir = state_dir.join("open-api");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let path = dir.join("turns.json");
        let terminal = match fs::read(&path) {
            Ok(bytes) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
                match Self::decode(&bytes) {
                    Ok(records) => records,
                    Err(error) => {
                        let backup = dir.join(format!(
                            "turns.json.corrupt-{}-{}",
                            now(),
                            uuid::Uuid::new_v4()
                        ));
                        fs::rename(&path, &backup).context("preserve corrupt turn ledger")?;
                        tracing::warn!(%error, path = %backup.display(), "corrupt turn ledger; starting empty");
                        Vec::new()
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let mut ledger = Self { path, terminal };
        Self::prune(&mut ledger.terminal);
        Ok(ledger)
    }

    fn decode(bytes: &[u8]) -> Result<Vec<TerminalRecord>> {
        let file: LedgerFile = serde_json::from_slice(bytes)?;
        if file.version != 1 {
            bail!("unsupported ledger version");
        }
        let mut seen = std::collections::HashSet::new();
        for rec in &file.terminal {
            if !seen.insert(&rec.turn_id) {
                bail!("duplicate terminal turn");
            }
            match rec.outcome.as_str() {
                "accepted" => {
                    let action = rec.action.as_deref().context("missing accepted action")?;
                    if !["send", "skip", "handoff"].contains(&action) {
                        bail!("invalid accepted action");
                    }
                    if action == "send" {
                        let text = rec.text.as_deref().context("missing accepted text")?;
                        if rec.text_sha256.as_deref() != Some(text_sha256(text).as_str()) {
                            bail!("invalid text hash");
                        }
                    }
                }
                "expired" | "revoked" => {}
                _ => bail!("invalid terminal outcome"),
            }
        }
        Ok(file.terminal)
    }

    pub fn get(&self, turn_id: &str) -> Option<&TerminalRecord> {
        self.terminal.iter().find(|rec| rec.turn_id == turn_id)
    }

    fn prune(records: &mut Vec<TerminalRecord>) {
        let cutoff = now().saturating_sub(MAX_AGE);
        records.retain(|rec| rec.at >= cutoff);
        records.sort_by_key(|rec| rec.at);
        if records.len() > MAX_RECORDS {
            records.drain(..records.len() - MAX_RECORDS);
        }
    }

    pub fn record(&mut self, rec: TerminalRecord) -> Result<()> {
        // A failed write must leave both the old file and the in-memory view intact.
        if self.get(&rec.turn_id).is_some() {
            bail!("turn already terminal");
        }
        let mut terminal = self.terminal.clone();
        terminal.push(rec);
        Self::prune(&mut terminal);
        let bytes = serde_json::to_vec(&LedgerFile {
            version: 1,
            terminal: terminal.clone(),
        })?;
        let temp = self
            .path
            .with_file_name(format!(".turns-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        self.terminal = terminal;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("turn-ledger-{}", uuid::Uuid::new_v4()))
    }
    fn record(id: &str, at: u64) -> TerminalRecord {
        TerminalRecord {
            turn_id: id.into(),
            outcome: "accepted".into(),
            action: Some("send".into()),
            text: Some("你好".into()),
            text_sha256: Some(text_sha256("你好")),
            at,
        }
    }
    #[test]
    fn ledger_roundtrip_atomic() {
        let dir = directory();
        let mut ledger = TurnLedger::open(&dir).unwrap();
        let rec = record("one", now());
        ledger.record(rec.clone()).unwrap();
        let reopened = TurnLedger::open(&dir).unwrap();
        assert_eq!(reopened.get("one"), Some(&rec));
        assert_eq!(
            std::fs::metadata(dir.join("open-api"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(dir.join("open-api/turns.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_dir(dir.join("open-api")).unwrap().count(), 1);
        let before = std::fs::read(dir.join("open-api/turns.json")).unwrap();
        std::fs::rename(dir.join("open-api"), dir.join("saved")).unwrap();
        std::fs::write(dir.join("open-api"), "not a directory").unwrap();
        assert!(ledger.record(record("failed", now())).is_err());
        assert!(ledger.get("failed").is_none());
        assert_eq!(std::fs::read(dir.join("saved/turns.json")).unwrap(), before);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ledger_prunes_by_count_and_age() {
        let dir = directory();
        let mut ledger = TurnLedger::open(&dir).unwrap();
        let at = now();
        ledger.record(record("old", at - 7 * 86400 - 1)).unwrap();
        assert!(ledger.get("old").is_none());
        for i in 0..1001 {
            ledger.record(record(&i.to_string(), at)).unwrap();
        }
        assert!(ledger.get("0").is_none());
        assert!(ledger.get("1000").is_some());
        let reopened = TurnLedger::open(&dir).unwrap();
        assert_eq!(reopened.terminal.len(), 1000);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ledger_corrupt_file_starts_empty_and_preserves_backup() {
        let dir = directory();
        std::fs::create_dir_all(dir.join("open-api")).unwrap();
        std::fs::write(dir.join("open-api/turns.json"), "broken").unwrap();
        let ledger = TurnLedger::open(&dir).unwrap();
        assert!(ledger.terminal.is_empty());
        let backup = std::fs::read_dir(dir.join("open-api"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("turns.json.corrupt-")
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "broken");
        assert_eq!(
            std::fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
