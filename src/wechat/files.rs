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

/// Where `upload_file` stores files sent from another machine (inside Documents, so send_file accepts them).
pub fn default_upload_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Documents/微信客服附件/待发送"))
}

/// Saves an uploaded file as `<root>/<random folder>/<name>` and returns its path.
/// The name is kept as is because WeChat shows it to the recipient; the fresh folder keeps
/// same-named uploads apart. Everything is checked before anything is written.
pub fn store_upload(name: &str, content_base64: &str, root: &Path) -> Result<PathBuf> {
    let valid_name = !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control());
    if !valid_name {
        bail!("name 必须是不含路径的文件名（不能以 . 开头、不含 / \\ 或控制字符，≤255 字节）");
    }
    let bytes = decode_base64(content_base64)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_SEND_FILE_BYTES {
        bail!("文件大小必须在 1 字节到 10MB 之间");
    }
    let folder = root.join(&uuid::Uuid::new_v4().simple().to_string()[..12]);
    std::fs::create_dir_all(&folder).context("创建上传目录失败")?;
    let path = folder.join(name);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, &bytes));
    if let Err(error) = written {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(error).context("写入上传文件失败");
    }
    Ok(path)
}

/// Strict standard base64 (whitespace ignored, padding only at the end), size-capped before decoding.
pub fn decode_base64(text: &str) -> Result<Vec<u8>> {
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() || !clean.len().is_multiple_of(4) {
        bail!("content_base64 不是有效的 base64");
    }
    if (clean.len() / 4 * 3) as u64 > MAX_SEND_FILE_BYTES + 2 {
        bail!("文件大小必须在 1 字节到 10MB 之间");
    }
    let padding = clean.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 || clean[..clean.len() - padding].contains(&b'=') {
        bail!("content_base64 不是有效的 base64");
    }
    let value = |b: u8| -> Result<u32> {
        Ok(match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 0,
            _ => bail!("content_base64 不是有效的 base64"),
        } as u32)
    };
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for &b in chunk {
            n = (n << 6) | value(b)?;
        }
        out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
    }
    out.truncate(out.len() - padding);
    Ok(out)
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
        if line.len() > shown.len() && shown.chars().count() >= 6 && name.starts_with(&shown) {
            return true;
        }
        middle_elided(&line, &name)
    })
}

/// WeChat shortens long names in the middle ("桃桃是在偷藏…_v2.mp4"); OCR may drop the ellipsis.
/// Accept when the shown text is a 4+ character prefix of the name, then only ellipsis-like filler,
/// then a suffix of the name that includes the whole extension.
fn middle_elided(shown: &str, name: &str) -> bool {
    let shown: Vec<char> = shown.chars().collect();
    let full: Vec<char> = name.chars().collect();
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.chars().count() + 1).unwrap_or(0);
    let prefix = shown.iter().zip(&full).take_while(|(a, b)| a == b).count();
    let suffix = shown.iter().rev().zip(full.iter().rev()).take_while(|(a, b)| a == b).count();
    if prefix < 4 || extension == 0 || suffix < extension + 1 || prefix + suffix > full.len() {
        return false;
    }
    let middle_end = shown.len().saturating_sub(suffix);
    shown[prefix.min(middle_end)..middle_end]
        .iter()
        .all(|c| matches!(c, '…' | '.' | '_' | '-' | '·'))
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
    fn upload_keeps_the_name_in_a_fresh_folder_inside_the_send_roots() {
        let root = scratch();
        let first = store_upload("报价单 v2.pdf", "JVBERi0xLjQgdGVzdA==", &root).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"%PDF-1.4 test");
        assert_eq!(first.file_name().unwrap(), "报价单 v2.pdf");
        assert!(first.starts_with(&root));
        // Same name again: a separate folder, the earlier upload is untouched.
        let second = store_upload("报价单 v2.pdf", "aGk=", &root).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"%PDF-1.4 test");
        assert_eq!(std::fs::read(&second).unwrap(), b"hi");
        // The stored file passes the send_file checks.
        assert_eq!(prepare_send_file(second.to_str().unwrap(), &[root.clone()]).unwrap().name, "报价单 v2.pdf");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn upload_refuses_bad_names_bad_base64_and_bad_sizes() {
        let root = scratch();
        for name in ["", ".", "..", "../x.pdf", "a/b.pdf", ".hidden", "a\\b.pdf", "x\0.pdf"] {
            assert!(store_upload(name, "aGk=", &root).is_err(), "name {name:?}");
        }
        for content in ["", "a", "aGk", "aG=k", "a*k=", "aGk==="] {
            assert!(store_upload("a.txt", content, &root).is_err(), "content {content:?}");
        }
        assert_eq!(decode_base64("aGk=\n").unwrap(), b"hi");
        assert_eq!(decode_base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(decode_base64("/+8=").unwrap(), vec![0xff, 0xef]);
        let too_big = "A".repeat(((MAX_SEND_FILE_BYTES as usize + 3) / 3 + 1) * 4);
        assert!(store_upload("big.bin", &too_big, &root).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "nothing left behind");
        std::fs::remove_dir_all(root).unwrap();
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
        // Long names are shortened in the middle; OCR read the card as below (2026-11-05, mp4).
        assert!(input_shows_file(&["桃桃是在偷藏__v2.mp4".to_string(), "4.1M".to_string()], "桃桃是在偷藏_轻字幕版_v2.mp4"));
        assert!(input_shows_file(&["桃桃是在偷藏…_v2.mp4".to_string()], "桃桃是在偷藏_轻字幕版_v2.mp4"));
        assert!(!input_shows_file(&["别的视频__v2.mp4".to_string()], "桃桃是在偷藏_轻字幕版_v2.mp4"));
        assert!(!input_shows_file(&["桃桃是在偷藏__v3.mp4".to_string()], "桃桃是在偷藏_轻字幕版_v2.mp4"));
    }
}
