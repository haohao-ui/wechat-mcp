//! File sending: which local files may be sent, and whether the chat input shows the pasted file.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Upper bound for one sent file (same limit as the bot's media handling).
pub const MAX_SEND_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileToSend {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: u64,
}

impl FileToSend {
    /// What the outbox stores as the "message": the same key with a different or changed file is refused.
    pub fn outbox_text(&self) -> String {
        format!("[文件] {} ({} 字节, 修改于 {})", self.path.display(), self.size, self.modified)
    }
}

/// Folders a file may be sent from: the host user's Desktop, Documents and Downloads.
pub fn default_send_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    ["Desktop", "Documents", "Downloads"].iter().map(|dir| home.join(dir)).collect()
}

/// Checks a requested path before anything touches the desktop.
pub fn prepare_send_file(raw: &str, roots: &[PathBuf]) -> Result<FileToSend> {
    let requested = Path::new(raw);
    if !requested.is_absolute() {
        bail!("path 必须是绝对路径");
    }
    // Resolves symlinks and `..`, so a link cannot point outside the allowed folders.
    let path = requested
        .canonicalize()
        .with_context(|| format!("文件不存在或无法访问: {raw}"))?;
    let allowed = roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| path.starts_with(&root));
    if !allowed {
        bail!("只允许发送桌面、文稿或下载文件夹中的文件");
    }
    let metadata = std::fs::metadata(&path).context("读取文件信息失败")?;
    if !metadata.is_file() {
        bail!("path 不是普通文件");
    }
    let size = metadata.len();
    if size == 0 || size > MAX_SEND_FILE_BYTES {
        bail!("文件大小必须在 1 字节到 10MB 之间");
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let name = path
        .file_name()
        .context("文件名无效")?
        .to_string_lossy()
        .into_owned();
    Ok(FileToSend { path, name, size, modified })
}

/// True when OCR of the chat input shows the pasted file card for `name`.
/// WeChat may shorten long names with an ellipsis, so a 6+ character prefix before it also counts.
pub fn input_shows_file(lines: &[String], name: &str) -> bool {
    let compact = |text: &str| text.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let name = compact(name);
    lines.iter().map(|line| compact(line)).any(|line| {
        if line.contains(&name) {
            return true;
        }
        let shown = line.trim_end_matches(['…', '.']).to_string();
        line.len() > shown.len() && shown.chars().count() >= 6 && name.starts_with(&shown)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("send-file-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_a_file_inside_an_allowed_folder() {
        let root = scratch();
        let file = root.join("岗位信息更新2.13.pdf");
        std::fs::write(&file, b"%PDF-1.4 test").unwrap();
        let prepared = prepare_send_file(file.to_str().unwrap(), &[root.clone()]).unwrap();
        assert_eq!(prepared.name, "岗位信息更新2.13.pdf");
        assert_eq!(prepared.size, 13);
        assert!(prepared.outbox_text().contains("岗位信息更新2.13.pdf"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_relative_outside_symlinked_empty_and_oversized_files() {
        let root = scratch();
        let outside = scratch();
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, b"secret").unwrap();
        assert!(prepare_send_file("Desktop/a.pdf", &[root.clone()]).is_err());
        assert!(prepare_send_file(secret.to_str().unwrap(), &[root.clone()]).is_err());
        let link = root.join("link.txt");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(prepare_send_file(link.to_str().unwrap(), &[root.clone()]).is_err());
        let empty = root.join("empty.pdf");
        std::fs::write(&empty, b"").unwrap();
        assert!(prepare_send_file(empty.to_str().unwrap(), &[root.clone()]).is_err());
        let big = root.join("big.pdf");
        std::fs::File::create(&big).unwrap().set_len(MAX_SEND_FILE_BYTES + 1).unwrap();
        assert!(prepare_send_file(big.to_str().unwrap(), &[root.clone()]).is_err());
        assert!(prepare_send_file(root.to_str().unwrap(), &[root.clone()]).is_err());
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn input_check_matches_file_card_text_measured_on_wechat_4() {
        // OCR of the input after pasting, measured 2026-11-05 on the remote Mac.
        let lines: Vec<String> = ["岗位信息更新2.13.pdf", "220.3K", "R", "® 0白兴、0", "发送"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(input_shows_file(&lines, "岗位信息更新2.13.pdf"));
        assert!(input_shows_file(&["岗位信息更新…".to_string()], "岗位信息更新2.13.pdf"));
        assert!(!input_shows_file(&["发送".to_string()], "岗位信息更新2.13.pdf"));
        assert!(!input_shows_file(&["报价…".to_string()], "岗位信息更新2.13.pdf"));
    }
}
