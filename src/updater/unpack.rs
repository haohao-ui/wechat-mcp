use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

use super::manifest::{ArtifactSpec, ComponentSpec};

/// 客户端内置硬上限（防止清单自身配置被篡改或放大攻击）
pub const CLIENT_HARD_MAX_UNPACKED_BYTES: u64 = 500 * 1024 * 1024; // 500 MB
pub const CLIENT_HARD_MAX_FILE_COUNT: usize = 2000;

/// 验证归档元数据（若提供了归档文件）
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

/// 首版明确禁止调用系统 tar 进行未知包盲解包。
/// 未经审核的解压库之前，归档解包直接返回 unsupported_archive。
pub fn safe_unpack_and_verify(
    _archive_path: &Path,
    _staging_dir: &Path,
    _expected_artifact: &ArtifactSpec,
    _expected_components: &[ComponentSpec],
) -> Result<()> {
    bail!(
        "unsupported_archive: streaming archive extraction is disabled in this phase (use safe_verify_staged_dir for isolated fixture verification)"
    );
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
