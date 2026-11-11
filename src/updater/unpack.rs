use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

use super::manifest::{ArtifactSpec, ComponentSpec};

/// 客户端内置硬上限（防止清单自身配置被篡改或放大攻击）
pub const CLIENT_HARD_MAX_UNPACKED_BYTES: u64 = 500 * 1024 * 1024; // 500 MB
pub const CLIENT_HARD_MAX_FILE_COUNT: usize = 2000;

/// 限制解压总输出字节数的 Reader（含 trailing/多 member/padding），防止解压放大攻击
struct LimitedReader<R> {
    inner: R,
    remaining: u64,
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unpacked_size_limit_exceeded: uncompressed stream exceeds client limit",
            ));
        }
        let max_to_read = buf.len().min(self.remaining as usize);
        let n = self.inner.read(&mut buf[..max_to_read])?;
        self.remaining = self.remaining.saturating_sub(n as u64);
        Ok(n)
    }
}

/// 验证归档元数据（基于路径）
pub fn verify_artifact_archive(archive_path: &Path, expected: &ArtifactSpec) -> Result<()> {
    if !archive_path.exists() {
        bail!("archive_not_found: {}", archive_path.display());
    }

    let meta = fs::metadata(archive_path)?;
    if meta.len() != expected.size_bytes {
        bail!(
            "archive_size_mismatch: got {} bytes, expected {}",
            meta.len(),
            expected.size_bytes
        );
    }

    let mut file = File::open(archive_path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual_sha = to_hex(&hasher.finalize());

    if !actual_sha.eq_ignore_ascii_case(&expected.sha256) {
        bail!(
            "archive_sha256_mismatch: got {}, expected {}",
            actual_sha,
            expected.sha256
        );
    }

    Ok(())
}

/// 受控安全流式解包与校验（禁用外部 tar，防 TOCTOU、路径穿越、链接与大小写冲突）
pub fn safe_unpack_and_verify(
    archive_path: &Path,
    staging_dir: &Path,
    expected_artifact: &ArtifactSpec,
    expected_components: &[ComponentSpec],
) -> Result<()> {
    if !archive_path.exists() {
        bail!("archive_not_found: {}", archive_path.display());
    }
    if !staging_dir.exists() {
        fs::create_dir_all(staging_dir)?;
        fs::set_permissions(staging_dir, fs::Permissions::from_mode(0o700))?;
    }

    // 1. 防 TOCTOU：打开单一文件描述符 (FD)
    let mut file = File::open(archive_path)?;
    let meta = file.metadata()?;
    if meta.len() != expected_artifact.size_bytes {
        bail!(
            "archive_size_mismatch: got {} bytes, expected {}",
            meta.len(),
            expected_artifact.size_bytes
        );
    }

    // 计算哈希
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual_sha = to_hex(&hasher.finalize());
    if !actual_sha.eq_ignore_ascii_case(&expected_artifact.sha256) {
        bail!(
            "archive_sha256_mismatch: got {}, expected {}",
            actual_sha,
            expected_artifact.sha256
        );
    }

    // 倒带同一个 FD，绝不通过路径重新 open
    file.seek(std::io::SeekFrom::Start(0))?;

    let hard_max_bytes = expected_artifact
        .max_unpacked_bytes
        .min(CLIENT_HARD_MAX_UNPACKED_BYTES);
    let hard_max_count = expected_artifact
        .max_file_count
        .min(CLIENT_HARD_MAX_FILE_COUNT);

    // 构建有限字节流的解包器（解包流受限，不能只累计落盘文件）
    let gz_decoder = flate2::read::GzDecoder::new(file);
    let limited_reader = LimitedReader {
        inner: gz_decoder,
        remaining: hard_max_bytes,
    };
    let mut archive = tar::Archive::new(limited_reader);

    let mut seen_paths = HashSet::new();
    let mut seen_lower_paths = HashSet::new();
    let mut seen_dirs = HashSet::new();
    let mut unpacked_components = HashMap::new();
    let mut total_entries: usize = 0;
    let mut total_unpacked_bytes: u64 = 0;

    let mut comp_map: HashMap<String, &ComponentSpec> = HashMap::new();
    for comp in expected_components {
        comp_map.insert(comp.path.clone(), comp);
    }

    let entries = archive.entries()?;
    for entry_res in entries {
        let mut entry = match entry_res {
            Ok(e) => e,
            Err(e) => {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!("corrupt_tar_entry: {}", e);
            }
        };

        total_entries += 1;
        if total_entries > hard_max_count {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "file_count_limit_exceeded: archive entries exceed limit of {}",
                hard_max_count
            );
        }

        // 严格类型审查：严禁软链、硬链、特殊设备节点、稀疏/PAX/GNU 扩展
        let entry_type = entry.header().entry_type();
        match entry_type {
            tar::EntryType::Regular
            | tar::EntryType::Continuous
            | tar::EntryType::Directory => {}
            tar::EntryType::Symlink => {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!("symlink_rejected: symbolic links forbidden in archive");
            }
            tar::EntryType::Link => {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!("hardlink_rejected: hard links forbidden in archive");
            }
            other => {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!("special_entry_rejected: unsupported entry type {:?}", other);
            }
        }

        let raw_path = match entry.path() {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!("invalid_entry_path: {}", e);
            }
        };

        let norm_path = raw_path.trim_end_matches('/').to_string();
        if norm_path.is_empty() || norm_path == "." {
            continue;
        }

        // 路径长度与深度限制
        if norm_path.len() > 255 {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "path_too_long: path length {} exceeds 255 on '{}'",
                norm_path.len(),
                norm_path
            );
        }
        let depth = norm_path.split('/').count();
        if depth > 16 {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "path_depth_exceeded: path depth {} exceeds 16 on '{}'",
                depth,
                norm_path
            );
        }

        // 拒绝绝对路径、穿越段与特殊字符
        if norm_path.starts_with('/')
            || norm_path.contains("..")
            || norm_path.contains('\\')
            || norm_path.contains('\0')
        {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!("invalid_relative_path: '{}'", norm_path);
        }

        // Unicode NFC 规范化
        let nfc: String = norm_path.nfc().collect();
        if nfc != norm_path {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "unicode_normalization_rejected: path '{}' is not in Unicode NFC format",
                norm_path
            );
        }

        // 大小写冲突
        let lower = norm_path.to_lowercase();
        if !seen_lower_paths.insert(lower) {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "case_collision_detected: path '{}' conflicts in case-insensitive comparison",
                norm_path
            );
        }

        // 重复路径
        if !seen_paths.insert(norm_path.clone()) {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "duplicate_path_detected: duplicate entry for '{}'",
                norm_path
            );
        }

        if entry_type == tar::EntryType::Directory {
            if unpacked_components.contains_key(&norm_path) {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!(
                    "parent_child_collision: directory '{}' conflicts with existing file",
                    norm_path
                );
            }
            seen_dirs.insert(norm_path.clone());
            let target_dir = staging_dir.join(&norm_path);
            if let Err(e) = create_dir_all_secure(staging_dir, &target_dir) {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                return Err(e);
            }
            continue;
        }

        // 常规文件处理
        if seen_dirs.contains(&norm_path) {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "parent_child_collision: file '{}' conflicts with existing directory",
                norm_path
            );
        }

        let comp = match comp_map.get(&norm_path) {
            Some(c) => *c,
            None => {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!(
                    "unexpected_file_in_archive: '{}' is not declared in components whitelist",
                    norm_path
                );
            }
        };

        let entry_mode = entry.header().mode().unwrap_or(0);
        // 拒绝 setuid / setgid / sticky
        if (entry_mode & 0o7000) != 0 {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "forbidden_special_permissions: mode {:o} on '{}'",
                entry_mode,
                norm_path
            );
        }
        // 拒绝 group/other 可写位
        if (entry_mode & 0o022) != 0 {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "forbidden_world_writable_permissions: mode {:o} on '{}'",
                entry_mode,
                norm_path
            );
        }

        let actual_mode = entry_mode & 0o777;
        let expected_mode = comp.file_mode & 0o777;
        if actual_mode != expected_mode {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "component_mode_mismatch: '{}' mode {:o} != expected {:o}",
                norm_path,
                actual_mode,
                expected_mode
            );
        }

        let dest_path = staging_dir.join(&norm_path);
        if let Some(parent) = dest_path.parent() {
            if let Err(e) = create_dir_all_secure(staging_dir, parent) {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                return Err(e);
            }
        }

        // 使用 O_CREAT | O_EXCL 创建新文件，防跟随软链与文件覆盖
        let mut out_file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(expected_mode)
            .open(&dest_path)
        {
            Ok(f) => f,
            Err(e) => {
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!("failed_to_create_file '{}': {}", dest_path.display(), e);
            }
        };

        let mut file_hasher = Sha256::new();
        let mut written_bytes: u64 = 0;
        let mut write_buf = [0u8; 16384];

        loop {
            let n = match entry.read(&mut write_buf) {
                Ok(n) => n,
                Err(e) => {
                    drop(out_file);
                    cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                    bail!("read_entry_failed on '{}': {}", norm_path, e);
                }
            };
            if n == 0 {
                break;
            }
            written_bytes += n as u64;
            total_unpacked_bytes += n as u64;

            if written_bytes > comp.size_bytes {
                drop(out_file);
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!(
                    "component_size_exceeded: '{}' wrote {} > expected {}",
                    norm_path,
                    written_bytes,
                    comp.size_bytes
                );
            }
            if total_unpacked_bytes > hard_max_bytes {
                drop(out_file);
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!(
                    "unpacked_size_limit_exceeded: total unpacked bytes exceeds limit {}",
                    hard_max_bytes
                );
            }

            file_hasher.update(&write_buf[..n]);
            if let Err(e) = out_file.write_all(&write_buf[..n]) {
                drop(out_file);
                cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
                bail!("write_file_failed on '{}': {}", norm_path, e);
            }
        }

        if let Err(e) = out_file.sync_all() {
            drop(out_file);
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!("sync_file_failed on '{}': {}", norm_path, e);
        }
        drop(out_file);

        if written_bytes != comp.size_bytes {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "component_size_mismatch: '{}' wrote {} != expected {}",
                norm_path,
                written_bytes,
                comp.size_bytes
            );
        }

        let actual_file_sha = to_hex(&file_hasher.finalize());
        if !actual_file_sha.eq_ignore_ascii_case(&comp.sha256) {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "component_sha256_mismatch: '{}' sha256 {} != expected {}",
                norm_path,
                actual_file_sha,
                comp.sha256
            );
        }

        unpacked_components.insert(norm_path, (*comp).clone());
    }

    // 白名单闭合校验：解出的组件必须与 expected_components 1:1 闭合匹配
    for comp in expected_components {
        if !unpacked_components.contains_key(&comp.path) {
            cleanup_staged_content(staging_dir, &unpacked_components, &seen_dirs);
            bail!(
                "missing_component_in_archive: expected component '{}' not found in archive",
                comp.path
            );
        }
    }

    Ok(())
}

fn create_dir_all_secure(staging_root: &Path, target: &Path) -> Result<()> {
    if !target.starts_with(staging_root) {
        bail!("directory_escapes_staging_root: {}", target.display());
    }

    let rel = target.strip_prefix(staging_root)?;
    let mut current = staging_root.to_path_buf();
    for segment in rel.components() {
        current.push(segment);
        if current.exists() {
            let meta = fs::symlink_metadata(&current)?;
            if meta.file_type().is_symlink() {
                bail!("symlink_in_directory_path: {}", current.display());
            }
            if !meta.is_dir() {
                bail!("non_directory_in_path: {}", current.display());
            }
        } else {
            fs::create_dir(&current)?;
            fs::set_permissions(&current, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// 受控暂存区局部安全清理（仅清理刚刚解出的组件与目录，不可任意 remove_dir_all）
fn cleanup_staged_content(
    staging_dir: &Path,
    unpacked_files: &HashMap<String, ComponentSpec>,
    seen_dirs: &HashSet<String>,
) {
    for rel in unpacked_files.keys() {
        let p = staging_dir.join(rel);
        let _ = fs::remove_file(p);
    }
    let mut dirs: Vec<_> = seen_dirs.iter().cloned().collect();
    dirs.sort_by(|a, b| b.len().cmp(&a.len())); // 由深至浅清理
    for d in dirs {
        let p = staging_dir.join(d);
        let _ = fs::remove_dir(p);
    }
}

/// 针对已存在或合成的暂存目录进行全量类型、路径安全、客户端硬上限及白名单校验
pub fn safe_verify_staged_dir(
    staging_dir: &Path,
    expected_artifact: &ArtifactSpec,
    expected_components: &[ComponentSpec],
) -> Result<()> {
    if !staging_dir.exists() {
        bail!("staging_directory_not_found: {}", staging_dir.display());
    }

    let staging_canon = staging_dir.canonicalize()?;
    let mut found_files = HashMap::new();
    let mut seen_lower_paths = HashSet::new();
    let mut seen_dirs = HashSet::new();
    let mut total_bytes: u64 = 0;
    let mut total_count: usize = 0;

    let hard_max_bytes = expected_artifact
        .max_unpacked_bytes
        .min(CLIENT_HARD_MAX_UNPACKED_BYTES);
    let hard_max_count = expected_artifact
        .max_file_count
        .min(CLIENT_HARD_MAX_FILE_COUNT);

    collect_and_audit_dir(
        staging_dir,
        staging_dir,
        &staging_canon,
        &mut found_files,
        &mut seen_lower_paths,
        &mut seen_dirs,
        &mut total_bytes,
        &mut total_count,
        hard_max_bytes,
        hard_max_count,
    )?;

    if total_count > hard_max_count {
        bail!(
            "file_count_limit_exceeded: found {} files, client hard limit is {}",
            total_count,
            hard_max_count
        );
    }
    if total_bytes > hard_max_bytes {
        bail!(
            "unpacked_size_limit_exceeded: unpacked {} bytes, client hard limit is {}",
            total_bytes,
            hard_max_bytes
        );
    }

    // 白名单闭合校验与组件路径 NFC / 大小写冲突检查
    let mut expected_map = HashMap::new();
    let mut seen_comp_lower = HashSet::new();
    for comp in expected_components {
        let nfc: String = comp.path.nfc().collect();
        if nfc != comp.path {
            bail!(
                "unicode_normalization_rejected: component path '{}' is not in Unicode NFC format",
                comp.path
            );
        }
        let lower = comp.path.to_lowercase();
        if !seen_comp_lower.insert(lower) {
            bail!(
                "case_collision_detected: component '{}' conflicts in case-insensitive comparison",
                comp.path
            );
        }
        if expected_map.insert(comp.path.clone(), comp).is_some() {
            bail!(
                "duplicate_component_path: component '{}' declared more than once",
                comp.path
            );
        }
    }

    for found_path in found_files.keys() {
        if !expected_map.contains_key(found_path) {
            bail!(
                "unexpected_file_in_staged_dir: '{}' is not declared in components whitelist",
                found_path
            );
        }
    }

    for (rel_path, comp) in &expected_map {
        let file_info = match found_files.get(rel_path) {
            Some(info) => info,
            None => bail!(
                "missing_component_in_staged_dir: declared component '{}' not found",
                rel_path
            ),
        };

        if file_info.size_bytes != comp.size_bytes {
            bail!(
                "component_size_mismatch: '{}' size is {} bytes, expected {}",
                rel_path,
                file_info.size_bytes,
                comp.size_bytes
            );
        }

        if !file_info.sha256.eq_ignore_ascii_case(&comp.sha256) {
            bail!(
                "component_sha256_mismatch: '{}' sha256 is {}, expected {}",
                rel_path,
                file_info.sha256,
                comp.sha256
            );
        }

        let actual_mode = file_info.mode & 0o777;
        let expected_mode = comp.file_mode & 0o777;
        if actual_mode != expected_mode {
            bail!(
                "component_mode_mismatch: '{}' mode is {:o}, expected {:o}",
                rel_path,
                actual_mode,
                expected_mode
            );
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct UnpackedFileInfo {
    pub size_bytes: u64,
    pub sha256: String,
    pub mode: u32,
}

fn collect_and_audit_dir(
    root: &Path,
    current: &Path,
    root_canon: &Path,
    found: &mut HashMap<String, UnpackedFileInfo>,
    seen_lower_paths: &mut HashSet<String>,
    seen_dirs: &mut HashSet<String>,
    total_bytes: &mut u64,
    total_count: &mut usize,
    max_bytes: u64,
    max_count: usize,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let symlink_meta = fs::symlink_metadata(&path)?;

        // 1. 严格拒绝软链接
        if symlink_meta.file_type().is_symlink() {
            bail!(
                "symlink_rejected: symbolic link '{}' is forbidden",
                path.display()
            );
        }

        // 2. 严格拒绝硬链接（单文件链接数必须为 1）
        if symlink_meta.nlink() > 1 && symlink_meta.is_file() {
            bail!(
                "hardlink_rejected: file '{}' has nlink={}",
                path.display(),
                symlink_meta.nlink()
            );
        }

        let ft = symlink_meta.file_type();
        if !ft.is_dir() && !ft.is_file() {
            bail!(
                "special_file_rejected: '{}' is neither regular file nor directory",
                path.display()
            );
        }

        // 3. 路径穿越与绝对路径逃逸检查
        let canon = path.canonicalize()?;
        if !canon.starts_with(root_canon) {
            bail!(
                "path_traversal_detected: '{}' escapes root '{}'",
                canon.display(),
                root_canon.display()
            );
        }

        let rel = path.strip_prefix(root)?.to_string_lossy().to_string();
        if rel.contains("..") || rel.contains('\0') || rel.starts_with('/') {
            bail!("invalid_relative_path: '{}'", rel);
        }

        // 4. Unicode NFC 规范化检查与大小写冲突检查
        let nfc_normalized: String = rel.nfc().collect();
        if nfc_normalized != rel {
            bail!(
                "unicode_normalization_rejected: path '{}' is not in Unicode NFC format",
                rel
            );
        }

        let lower = rel.to_lowercase();
        if !seen_lower_paths.insert(lower.clone()) {
            bail!(
                "case_collision_detected: path '{}' conflicts in case-insensitive comparison",
                rel
            );
        }

        if ft.is_dir() {
            seen_dirs.insert(rel.clone());
            collect_and_audit_dir(
                root,
                &path,
                root_canon,
                found,
                seen_lower_paths,
                seen_dirs,
                total_bytes,
                total_count,
                max_bytes,
                max_count,
            )?;
        } else if ft.is_file() {
            // 父子路径冲突检查：如果已有同名目录，拒绝
            if seen_dirs.contains(&rel) {
                bail!(
                    "parent_child_path_collision: '{}' exists as both directory and file",
                    rel
                );
            }

            *total_count += 1;
            if *total_count > max_count {
                bail!(
                    "file_count_limit_exceeded: exceeds hard limit of {}",
                    max_count
                );
            }

            let size = symlink_meta.len();
            *total_bytes += size;
            if *total_bytes > max_bytes {
                bail!(
                    "unpacked_size_limit_exceeded: exceeds hard limit of {} bytes",
                    max_bytes
                );
            }

            let mode = symlink_meta.permissions().mode();
            // 拒绝 setuid / setgid / sticky
            if (mode & 0o7000) != 0 {
                bail!(
                    "forbidden_special_permissions: mode {:o} on '{}'",
                    mode,
                    path.display()
                );
            }
            // 拒绝 group/other 可写位
            if (mode & 0o022) != 0 {
                bail!(
                    "forbidden_world_writable_permissions: mode {:o} on '{}'",
                    mode,
                    path.display()
                );
            }

            let sha256 = calculate_file_sha256(&path)?;
            found.insert(
                rel,
                UnpackedFileInfo {
                    size_bytes: size,
                    sha256,
                    mode,
                },
            );
        }
    }
    Ok(())
}

fn calculate_file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
