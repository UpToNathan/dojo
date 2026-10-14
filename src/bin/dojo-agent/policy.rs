use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use anyhow::{Context, Result, ensure};
use dojo_cua_core::host_policy::HostPolicy;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) struct DesktopPolicy {
    policy: HostPolicy,
    path: std::path::PathBuf,
    pub(super) digest: String,
}

pub(super) fn authorize(tool: &str, arguments: &Value) -> Result<()> {
    if let Some(policy) = DesktopPolicy::load()? {
        policy.authorize(tool, arguments)?;
    }
    Ok(())
}

impl DesktopPolicy {
    pub(super) fn load() -> Result<Option<Self>> {
        let Some(path) = std::env::var_os("DOJO_DESKTOP_POLICY") else {
            return Ok(None);
        };
        Self::load_path(std::path::Path::new(&path)).map(Some)
    }
    fn load_path(path: &std::path::Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .context("open operator desktop policy")?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.len() <= 65536,
            "desktop policy must be a regular file of at most 64 KiB"
        );
        ensure!(
            (metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() })
                && metadata.mode() & 0o022 == 0,
            "desktop policy must be owned by root or this user and not writable by group or others"
        );
        let mut source = String::new();
        file.take(65537).read_to_string(&mut source)?;
        ensure!(source.len() <= 65536, "desktop policy grew beyond 64 KiB");
        Ok(Self {
            policy: HostPolicy::from_yaml(&source)?,
            path: path.to_owned(),
            digest: format!("{:x}", Sha256::digest(source.as_bytes())),
        })
    }
    pub(super) fn authorize(&self, tool: &str, arguments: &Value) -> Result<()> {
        ensure!(
            Self::load_path(&self.path)?.digest == self.digest,
            "operator desktop policy changed; reconnect before dispatch"
        );
        self.policy
            .authorize(tool, arguments)
            .context("operator desktop policy denied dispatch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn private_policy_restricts_arguments_and_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("policy.yaml");
        std::fs::write(&path,"allow:\n  rules:\n    - tool: desktop_capture_action\n      constraints:\n        x: { min: 0, max: 100 }\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let policy = DesktopPolicy::load_path(&path).unwrap();
        assert!(
            policy
                .authorize("desktop_capture_action", &serde_json::json!({"x":0}))
                .is_ok()
        );
        assert!(
            policy
                .authorize("desktop_capture_action", &serde_json::json!({"x":101}))
                .is_err()
        );
        assert!(
            policy
                .authorize("desktop_capture_action", &serde_json::json!({}))
                .is_err()
        );
        assert!(
            policy
                .authorize("desktop_observe", &serde_json::json!({}))
                .is_err()
        );
        assert_eq!(policy.digest.len(), 64);
        std::fs::write(&path, "allow: [desktop_observe]").unwrap();
        assert!(
            policy
                .authorize("desktop_capture_action", &serde_json::json!({"x":0}))
                .is_err()
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(DesktopPolicy::load_path(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("link.yaml");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(DesktopPolicy::load_path(&link).is_err());
        std::fs::write(&path, "allow: : invalid").unwrap();
        assert!(DesktopPolicy::load_path(&path).is_err());
    }
}
