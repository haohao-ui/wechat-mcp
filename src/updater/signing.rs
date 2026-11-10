use anyhow::{Result, bail};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct LocalSigner {
    pub keychain_path: Option<std::path::PathBuf>,
    pub keychain_pass: Option<String>,
    pub identity: String,
    pub simulate_failure: Option<String>,
}

impl LocalSigner {
    pub fn new(identity: impl Into<String>) -> Self {
        Self {
            keychain_path: None,
            keychain_pass: None,
            identity: identity.into(),
            simulate_failure: None,
        }
    }

    pub fn with_keychain(
        mut self,
        keychain_path: impl Into<std::path::PathBuf>,
        keychain_pass: impl Into<String>,
    ) -> Self {
        self.keychain_path = Some(keychain_path.into());
        self.keychain_pass = Some(keychain_pass.into());
        self
    }

    pub fn with_simulation(mut self, failure_reason: impl Into<String>) -> Self {
        self.simulate_failure = Some(failure_reason.into());
        self
    }

    /// 对单个二进制或 app 结构重签名并严格验证
    pub fn sign_and_verify(&self, target_path: &Path) -> Result<()> {
        if let Some(ref fail) = self.simulate_failure {
            bail!("local_signing_failed: simulated failure ({})", fail);
        }

        if !target_path.exists() {
            bail!("target_path_not_found: {}", target_path.display());
        }

        // 若配置了解锁钥匙串
        if let (Some(kc), Some(pass)) = (&self.keychain_path, &self.keychain_pass) {
            if !kc.exists() {
                bail!("signing_keychain_missing: {}", kc.display());
            }
            let unlock_res = Command::new("security")
                .arg("unlock-keychain")
                .arg("-p")
                .arg(pass)
                .arg(kc)
                .status();
            match unlock_res {
                Ok(st) if st.success() => {}
                Ok(st) => bail!("unlock_keychain_failed: security exit code {:?}", st.code()),
                Err(e) => bail!("unlock_keychain_exec_failed: {}", e),
            }
        }

        // 构造 codesign 命令
        let mut cmd = Command::new("codesign");
        cmd.arg("--force");
        cmd.arg("--sign").arg(&self.identity);
        if let Some(ref kc) = self.keychain_path {
            cmd.arg("--keychain").arg(kc);
        }
        cmd.arg(target_path);

        let status = cmd.status()?;
        if !status.success() {
            bail!(
                "codesign_failed: signing '{}' failed with status {:?}",
                target_path.display(),
                status.code()
            );
        }

        // 立即验证代码签名
        let verify_status = Command::new("codesign")
            .arg("--verify")
            .arg("--verbose=2")
            .arg(target_path)
            .status()?;

        if !verify_status.success() {
            bail!(
                "codesign_verify_failed: verification on '{}' failed with status {:?}",
                target_path.display(),
                verify_status.code()
            );
        }

        Ok(())
    }
}
