use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::manifest::{ArtifactSpec, ComponentSpec};

/// 检查原始归档文件的 SHA-256 和字节大小
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

/// 安全解包归档并对组件逐一进行严格沙箱检验
pub fn safe_unpack_and_verify(
    archive_path: &Path,
    staging_dir: &Path,
    expected_artifact: &ArtifactSpec,
    expected_components: &[ComponentSpec],
) -> Result<()> {
    // 1. 验证原始归档
    verify_artifact_archive(archive_path, expected_artifact)?;

    // 2. 准备暂存目录，确保 0700 权限
    if staging_dir.exists() {
        fs::remove_dir_all(staging_dir)?;
    }
    fs::create_dir_all(staging_dir)?;
    fs::set_permissions(staging_dir, fs::Permissions::from_mode(0o700))?;

    // 3. 执行解包（通过调用受控的 /usr/bin/tar 进行释放，但解包后由本模块做全量沙箱审计）
    let status = std::process::Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(staging_dir)
        .status()?;

    if !status.success() {
        let _ = fs::remove_dir_all(staging_dir);
        bail!(
            "tar_unpack_failed: /usr/bin/tar exited with status {:?}",
            status.code()
        );
    }

    // 4. 沙箱审计：扫描 staging_dir 下的所有条目
    let audit_res =
        audit_and_verify_staging_dir(staging_dir, expected_artifact, expected_components);
    if let Err(e) = audit_res {
        let _ = fs::remove_dir_all(staging_dir);
        return Err(e);
    }

    Ok(())
}

fn audit_and_verify_staging_dir(
    staging_dir: &Path,
    expected_artifact: &ArtifactSpec,
    expected_components: &[ComponentSpec],
) -> Result<()> {
    let mut found_files = std::collections::HashMap::new();
    let mut total_unpacked_bytes: u64 = 0;
    let mut total_file_count: usize = 0;

    let staging_canon = staging_dir.canonicalize()?;

    // 递归收集与审计
    collect_and_audit_dir(
        staging_dir,
        staging_dir,
        &staging_canon,
        &mut found_files,
        &mut total_unpacked_bytes,
        &mut total_file_count,
        expected_artifact.max_unpacked_bytes,
        expected_artifact.max_file_count,
    )?;

    // 5. 校验条目数与大小上限
    if total_file_count > expected_artifact.max_file_count {
        bail!(
            "file_count_limit_exceeded: found {} files, max is {}",
            total_file_count,
            expected_artifact.max_file_count
        );
    }
    if total_unpacked_bytes > expected_artifact.max_unpacked_bytes {
        bail!(
            "unpacked_size_limit_exceeded: unpacked {} bytes, max is {}",
            total_unpacked_bytes,
            expected_artifact.max_unpacked_bytes
        );
    }

    // 6. 白名单完全匹配比对
    let mut expected_map = std::collections::HashMap::new();
    for comp in expected_components {
        expected_map.insert(comp.path.clone(), comp);
    }

    // 检查是否存在清单中没有的多余未知文件
    for found_path in found_files.keys() {
        if !expected_map.contains_key(found_path) {
            bail!(
                "unexpected_file_in_archive: '{}' is not declared in components whitelist",
                found_path
            );
        }
    }

    // 检查清单中的每个组件是否存在且摘要/mode 完全一致
    for (rel_path, comp) in &expected_map {
        let file_info = match found_files.get(rel_path) {
            Some(info) => info,
            None => bail!(
                "missing_component_in_archive: declared component '{}' not found in unpacked package",
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

        // 校验权限位（规范化为仅比较低 9 位）
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
struct UnpackedFileInfo {
    size_bytes: u64,
    sha256: String,
    mode: u32,
}

fn collect_and_audit_dir(
    root: &Path,
    current: &Path,
    root_canon: &Path,
    found: &mut std::collections::HashMap<String, UnpackedFileInfo>,
    total_bytes: &mut u64,
    total_count: &mut usize,
    max_bytes: u64,
    max_count: usize,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let symlink_meta = fs::symlink_metadata(&path)?;

        // 严格拒绝软链接与任何特殊文件
        if symlink_meta.file_type().is_symlink() {
            bail!(
                "symlink_rejected: symbolic link '{}' is forbidden",
                path.display()
            );
        }

        let ft = symlink_meta.file_type();
        if !ft.is_dir() && !ft.is_file() {
            bail!(
                "special_file_rejected: '{}' is neither regular file nor directory",
                path.display()
            );
        }

        // 路径穿越与绝对路径检查
        let canon = path.canonicalize()?;
        if !canon.starts_with(root_canon) {
            bail!(
                "path_traversal_detected: '{}' escapes root '{}'",
                canon.display(),
                root_canon.display()
            );
        }

        let rel = path.strip_prefix(root)?.to_string_lossy().to_string();
        if rel.contains("..") || rel.contains('\0') {
            bail!("invalid_relative_path: '{}'", rel);
        }

        if ft.is_dir() {
            collect_and_audit_dir(
                root,
                &path,
                root_canon,
                found,
                total_bytes,
                total_count,
                max_bytes,
                max_count,
            )?;
        } else if ft.is_file() {
            *total_count += 1;
            if *total_count > max_count {
                bail!("file_count_limit_exceeded: exceeds limit of {}", max_count);
            }

            let size = symlink_meta.len();
            *total_bytes += size;
            if *total_bytes > max_bytes {
                bail!(
                    "unpacked_size_limit_exceeded: exceeds limit of {} bytes",
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
