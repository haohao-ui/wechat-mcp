use anyhow::{Context, Result, bail};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

pub struct ResourceLock {
    _file: File,
}
impl ResourceLock {
    pub fn try_acquire(path: &Path) -> Result<Option<Self>> {
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)?;
            }
            #[cfg(not(unix))]
            std::fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).context("打开资源锁失败")?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error).context("获取资源锁失败"),
        }
    }
    pub async fn desktop() -> Result<Self> {
        let path =
            std::path::PathBuf::from(std::env::var_os("HOME").context("桌面资源锁需要 HOME")?)
                .join("Library/Application Support/message-gateway/desktop.lock");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(lock) = Self::try_acquire(&path)? {
                return Ok(lock);
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("桌面被另一个 MCP 进程占用，等待超过 30 秒");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusive_lock_is_released_when_owner_drops() {
        let path = std::env::temp_dir().join(format!("gateway-lock-{}", crate::storage::new_id()));
        let first = ResourceLock::try_acquire(&path).unwrap().unwrap();
        assert!(ResourceLock::try_acquire(&path).unwrap().is_none());
        drop(first);
        assert!(ResourceLock::try_acquire(&path).unwrap().is_some());
        std::fs::remove_file(path).unwrap();
    }
}
