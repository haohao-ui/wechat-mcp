//! Bounded release transport. Downloaded bytes remain untrusted until the caller
//! verifies the publisher envelope and the complete artifact digest.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
use std::time::Duration;

pub(crate) const ENVELOPE_LIMIT: u64 = 1024 * 1024;
pub(crate) const ARTIFACT_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReleaseSource {
    Local { directory: PathBuf },
    Https { manifest_url: String },
}

pub(crate) enum Transport {
    Local(File),
    Https {
        client: reqwest::Client,
        manifest: reqwest::Url,
    },
}

fn validate_leaf(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        bail!("release_filename_invalid");
    }
    Ok(())
}

fn validate_https(url: &reqwest::Url) -> Result<()> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
        || url.path().ends_with('/')
    {
        bail!("release_source_requires_fixed_https_url_without_credentials");
    }
    Ok(())
}

impl ReleaseSource {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::Local { directory } if !directory.is_absolute() => {
                bail!("release_directory_must_be_absolute")
            }
            Self::Local { .. } => Ok(()),
            Self::Https { manifest_url } => {
                let url = reqwest::Url::parse(manifest_url)
                    .map_err(|_| anyhow::anyhow!("release_source_url_invalid"))?;
                validate_https(&url)
            }
        }
    }
}

impl Transport {
    pub(crate) fn open(source: &ReleaseSource) -> Result<Self> {
        source.validate()?;
        match source {
            ReleaseSource::Local { directory } => {
                if !directory.is_absolute() {
                    bail!("release_directory_must_be_absolute");
                }
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(directory)
                    .context("cannot_open_release_directory")?;
                let meta = file.metadata()?;
                if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
                    bail!("release_directory_owner_or_permissions_invalid");
                }
                Ok(Self::Local(file))
            }
            ReleaseSource::Https { manifest_url } => {
                let manifest = reqwest::Url::parse(manifest_url)
                    .map_err(|_| anyhow::anyhow!("release_source_url_invalid"))?;
                validate_https(&manifest)?;
                let client = reqwest::Client::builder()
                    .https_only(true)
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(10))
                    .timeout(Duration::from_secs(300))
                    .build()?;
                Ok(Self::Https { client, manifest })
            }
        }
    }

    pub(crate) async fn envelope(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.transfer(None, ENVELOPE_LIMIT, &mut bytes).await?;
        Ok(bytes)
    }

    /// The caller supplies a newly created, private staging file and must not
    /// expose it as a candidate until size, hash and signature checks succeed.
    pub(crate) async fn artifact(
        &self,
        filename: &str,
        expected_size: u64,
        out: &mut File,
    ) -> Result<()> {
        validate_leaf(filename)?;
        if expected_size == 0 || expected_size > ARTIFACT_LIMIT {
            bail!("artifact_size_out_of_bounds");
        }
        let size = self.transfer(Some(filename), expected_size, out).await?;
        if size != expected_size {
            bail!("artifact_size_mismatch");
        }
        out.sync_all()?;
        Ok(())
    }

    async fn transfer(
        &self,
        filename: Option<&str>,
        limit: u64,
        out: &mut impl Write,
    ) -> Result<u64> {
        match self {
            Self::Local(root) => {
                let name = std::ffi::CString::new(filename.unwrap_or("envelope.json"))?;
                let fd = unsafe {
                    libc::openat(
                        root.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    bail!(
                        "cannot_open_release_file: {}",
                        std::io::Error::last_os_error()
                    );
                }
                let file = unsafe { File::from_raw_fd(fd) };
                let meta = file.metadata()?;
                if !meta.is_file()
                    || meta.uid() != unsafe { libc::geteuid() }
                    || meta.mode() & 0o022 != 0
                {
                    bail!("release_file_owner_or_type_invalid");
                }
                if meta.len() > limit {
                    bail!("release_download_limit_exceeded");
                }
                copy_bounded(file, out, limit)
            }
            Self::Https { client, manifest } => {
                let url = match filename {
                    Some(name) => manifest.join(name).context("cannot_resolve_artifact_url")?,
                    None => manifest.clone(),
                };
                if url.origin() != manifest.origin() {
                    bail!("artifact_origin_changed");
                }
                let mut response = client.get(url).send().await.map_err(|e| e.without_url())?;
                if response.status() != reqwest::StatusCode::OK {
                    bail!("release_http_status: {}", response.status().as_u16());
                }
                if response.content_length().is_some_and(|size| size > limit) {
                    bail!("release_download_limit_exceeded");
                }
                let mut count = 0u64;
                while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
                    count = count
                        .checked_add(chunk.len() as u64)
                        .context("release_download_overflow")?;
                    if count > limit {
                        bail!("release_download_limit_exceeded");
                    }
                    out.write_all(&chunk)?;
                }
                Ok(count)
            }
        }
    }
}

fn copy_bounded(reader: impl Read, writer: &mut impl Write, limit: u64) -> Result<u64> {
    let mut bounded = reader.take(limit + 1);
    let mut count = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = bounded.read(&mut buffer)?;
        if read == 0 {
            return Ok(count);
        }
        count += read as u64;
        if count > limit {
            bail!("release_download_limit_exceeded");
        }
        writer.write_all(&buffer[..read])?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn source_urls_and_artifact_names_cannot_escape_policy() {
        for url in [
            "http://host/feed.json",
            "https://u:p@host/feed.json",
            "https://host/feed.json?token=secret",
            "https://host/",
            "https://host/feed.json#x",
        ] {
            assert!(
                Transport::open(&ReleaseSource::Https {
                    manifest_url: url.into()
                })
                .is_err()
            );
        }
        for name in [
            "../payload",
            "/payload",
            "a/b",
            "..",
            "x?redirect",
            "x\\y",
            "",
        ] {
            assert!(validate_leaf(name).is_err(), "{name}");
        }
        assert!(validate_leaf("release-1.tar.gz").is_ok());
        assert!(
            Transport::open(&ReleaseSource::Https {
                manifest_url: "https://example.com/releases/envelope.json".into()
            })
            .is_ok()
        );
    }

    #[test]
    fn streaming_limit_rejects_growth_without_writing_excess() {
        let mut out = Vec::new();
        assert!(copy_bounded(&b"12345"[..], &mut out, 4).is_err());
        assert!(out.len() <= 4);
        let mut out = Vec::new();
        assert_eq!(copy_bounded(&b"1234"[..], &mut out, 4).unwrap(), 4);
    }

    #[tokio::test]
    async fn local_transport_rejects_links_permissions_and_truncated_artifacts() {
        let dir = std::env::temp_dir().join(format!("release-source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let root = dir.join("feed");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let transport = Transport::open(&ReleaseSource::Local {
            directory: root.clone(),
        })
        .unwrap();
        std::fs::write(root.join("envelope.json"), b"signed bytes").unwrap();
        assert_eq!(transport.envelope().await.unwrap(), b"signed bytes");
        std::fs::write(root.join("payload.tar"), b"123").unwrap();
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dir.join("output"))
            .unwrap();
        assert!(
            transport
                .artifact("payload.tar", 4, &mut output)
                .await
                .is_err()
        );
        assert!(
            transport
                .artifact("payload.tar", 2, &mut output)
                .await
                .is_err()
        );
        transport
            .artifact("payload.tar", 3, &mut output)
            .await
            .unwrap();
        symlink(root.join("payload.tar"), root.join("link.tar")).unwrap();
        assert!(
            transport
                .artifact("link.tar", 3, &mut output)
                .await
                .is_err()
        );
        std::fs::set_permissions(
            root.join("payload.tar"),
            std::fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        assert!(
            transport
                .artifact("payload.tar", 3, &mut output)
                .await
                .is_err()
        );
        let link = dir.join("linked-feed");
        symlink(&root, &link).unwrap();
        assert!(Transport::open(&ReleaseSource::Local { directory: link }).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn isolated_https_enforces_status_stream_limits_and_truncation() {
        use std::process::{Command, Stdio};
        struct Server {
            child: std::process::Child,
            root: PathBuf,
        }
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let root = std::env::temp_dir().join(format!("updater-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let generated = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-keyout",
                "key.pem",
                "-out",
                "cert.pem",
            ])
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(generated.success());
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new("openssl")
            .args([
                "s_server",
                "-quiet",
                "-HTTP",
                "-accept",
                &format!("127.0.0.1:{port}"),
                "-cert",
                "cert.pem",
                "-key",
                "key.pem",
            ])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let server = Server { child, root };
        let mut ready = false;
        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(ready, "isolated TLS server failed to start");
        let cert =
            reqwest::Certificate::from_pem(&std::fs::read(server.root.join("cert.pem")).unwrap())
                .unwrap();
        // A test-only local root; production has no custom certificate or TLS bypass option.
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .add_root_certificate(cert)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let transport = Transport::Https {
            client,
            manifest: reqwest::Url::parse(&format!("https://localhost:{port}/envelope.json"))
                .unwrap(),
        };
        for (response, limit, succeeds) in [
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody",
                4,
                true,
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/leak\r\nContent-Length: 0\r\n\r\n",
                4,
                false,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nbody",
                4,
                false,
            ),
            (
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ntoo many bytes",
                4,
                false,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nbody",
                100,
                false,
            ),
        ] {
            std::fs::write(server.root.join("envelope.json"), response).unwrap();
            let mut out = Vec::new();
            let result = transport.transfer(None, limit, &mut out).await;
            assert_eq!(result.is_ok(), succeeds, "{result:?}");
            assert!(out.len() <= limit as usize);
        }
    }
}
