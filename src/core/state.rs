use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Health {
    pub pid: u32,
    pub state: String,
    pub updated_at: f64,
    pub account_id: String,
    pub allowlist: Vec<String>,
    pub groups_enabled: bool,
    pub model_backend: String,
    pub platform_message_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<serde_json::Map<String, Value>>,
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

struct TempFileGuard<'a>(&'a Path);
impl<'a> Drop for TempFileGuard<'a> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
    }
}

pub fn write_private(path: impl AsRef<Path>, value: &impl Serialize) -> Result<()> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("invalid file name")?;

    let tmp_name = format!("{}.{}.tmp", file_name, uuid::Uuid::new_v4().simple());
    let tmp_path = parent.join(tmp_name);

    let guard = TempFileGuard(&tmp_path);

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp_path)
        .with_context(|| {
            format!(
                "failed to create temporary private file {}",
                tmp_path.display()
            )
        })?;

    let json_bytes = serde_json::to_vec(value)?;
    file.write_all(&json_bytes)?;
    file.flush()?;
    file.sync_all()?;

    fs::rename(&tmp_path, path)
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;

    std::mem::forget(guard);
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read file {}", path.display()))?;
    let val = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse JSON from {}", path.display()))?;
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("test-bot-state-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn test_write_private_mode_and_atomicity() {
        let td = temp_dir();
        let target = td.join("cursor.json");
        let payload = json!({"status": "active", "counter": 42});

        write_private(&target, &payload).unwrap();
        assert!(target.exists());

        // Check file permissions 0o600
        let meta = fs::metadata(&target).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        // Check no temp files left behind
        let files: Vec<_> = fs::read_dir(&td)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files, vec!["cursor.json"]);

        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_read_json_deserialization() {
        let td = temp_dir();
        let target = td.join("data.json");
        let payload = json!({"name": "test", "num": 123});

        write_private(&target, &payload).unwrap();
        let read_val: serde_json::Value = read_json(&target).unwrap();
        assert_eq!(read_val, payload);

        let _ = fs::remove_dir_all(td);
    }

    #[test]
    fn test_health_format_single_and_multi_session() {
        let td = temp_dir();
        let target = td.join("health.json");

        let mut details = Map::new();
        details.insert("reason".into(), json!("watching"));

        let mut sessions = Map::new();
        sessions.insert("示例联系人甲".into(), json!({"state": "watching"}));
        sessions.insert("示例群".into(), json!({"state": "idle"}));

        let health = Health {
            pid: 12345,
            state: "running".into(),
            updated_at: 1700000000.0,
            account_id: "test-acc".into(),
            allowlist: vec!["示例联系人甲".into(), "示例群".into()],
            groups_enabled: true,
            model_backend: "codex-exec".into(),
            platform_message_verified: false,
            sessions: Some(sessions),
            details,
        };

        write_private(&target, &health).unwrap();
        let loaded: Health = read_json(&target).unwrap();
        assert_eq!(loaded.pid, 12345);
        assert_eq!(loaded.state, "running");
        assert_eq!(loaded.allowlist, vec!["示例联系人甲", "示例群"]);
        assert!(loaded.groups_enabled);
        assert_eq!(loaded.details.get("reason"), Some(&json!("watching")));
        assert!(loaded.sessions.is_some());
        assert_eq!(
            loaded.sessions.as_ref().unwrap().get("示例联系人甲"),
            Some(&json!({"state": "watching"}))
        );

        let _ = fs::remove_dir_all(td);
    }
}
