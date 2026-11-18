//! Owner-only directories anchored by descriptors for production updater assets.
use anyhow::{Result, bail};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

pub(crate) struct PrivateDirectory(pub(crate) File);
impl PrivateDirectory {
    pub(crate) fn open_child(&self, name: &str) -> Result<Self> {
        validate_leaf(name)?;
        let name = std::ffi::CString::new(name)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Self::checked(unsafe { File::from_raw_fd(fd) })
    }
    pub(crate) fn read_file(&self, name: &str, limit: u64) -> Result<File> {
        validate_leaf(name)?;
        let name = std::ffi::CString::new(name)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o777 != 0o600
            || meta.len() > limit
        {
            bail!("cached_file_owner_type_permissions_or_size_invalid");
        }
        Ok(file)
    }
    pub(crate) fn read(&self, name: &str, limit: u64) -> Result<Vec<u8>> {
        use std::io::Read;
        let file = self.read_file(name, limit)?;
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            bail!("cached_file_size_limit_exceeded");
        }
        Ok(bytes)
    }
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)?;
        Self::checked(file)
    }
    pub(crate) fn checked(file: File) -> Result<Self> {
        let meta = file.metadata()?;
        if !meta.is_dir()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o777 != 0o700
        {
            bail!("download_directory_requires_current_owner_and_0700");
        }
        Ok(Self(file))
    }
    pub(crate) fn child(&self, name: &str, exclusive: bool) -> Result<Self> {
        validate_leaf(name)?;
        let name = std::ffi::CString::new(name)?;
        let result = unsafe { libc::mkdirat(self.0.as_raw_fd(), name.as_ptr(), 0o700) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if exclusive || error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
        }
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        self.0.sync_all()?;
        Self::checked(unsafe { File::from_raw_fd(fd) })
    }
    pub(crate) fn exclusive_lock(&self, name: &str) -> Result<File> {
        validate_leaf(name)?;
        let name = std::ffi::CString::new(name)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o777 != 0o600
            || meta.nlink() != 1
            || meta.len() != 0
        {
            bail!("download_lock_file_invalid");
        }
        fs2::FileExt::try_lock_exclusive(&file)
            .map_err(|_| anyhow::anyhow!("concurrent_download_in_progress"))?;
        Ok(file)
    }
    pub(crate) fn create_file(&self, name: &str) -> Result<File> {
        validate_leaf(name)?;
        let name = std::ffi::CString::new(name)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let mut file = self.create_file(name)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        self.0.sync_all()?;
        Ok(())
    }
}

fn validate_leaf(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        bail!("private_directory_child_must_be_single_leaf");
    }
    Ok(())
}
