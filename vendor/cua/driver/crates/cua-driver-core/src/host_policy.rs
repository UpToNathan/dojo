//! Host-owned policy values. No environment, process globals or CUA lifecycle.
use crate::policy::{PolicyDecision, PolicyEngine, YamlPolicy};
use anyhow::{Result, ensure};
use serde_json::Value;

pub struct HostPolicy(PolicyEngine);
impl HostPolicy {
    pub fn from_yaml(source: &str) -> Result<Self> {
        ensure!(source.len() <= 65536, "policy exceeds 64 KiB");
        Ok(Self(PolicyEngine::Yaml(YamlPolicy::from_str(source)?)))
    }
    pub fn authorize(&self, tool: &str, arguments: &Value) -> Result<()> {
        match self.0.evaluate(tool, arguments) {
            PolicyDecision::Allow => Ok(()),
            PolicyDecision::Deny(reason) | PolicyDecision::Error(reason) => {
                anyhow::bail!("{reason}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_values_are_independent_and_denial_wins() {
        let reads =
            HostPolicy::from_yaml("allow: [desktop_observe]\ndeny: [desktop_capture_action]")
                .unwrap();
        let blocked =
            HostPolicy::from_yaml("allow: [desktop_observe]\ndeny: [desktop_observe]").unwrap();
        assert!(
            reads
                .authorize("desktop_observe", &serde_json::json!({}))
                .is_ok()
        );
        assert!(
            reads
                .authorize("desktop_capture_action", &serde_json::json!({}))
                .is_err()
        );
        assert!(reads.authorize("unknown", &serde_json::json!({})).is_err());
        assert!(
            blocked
                .authorize("desktop_observe", &serde_json::json!({}))
                .is_err()
        );
    }
    #[test]
    fn invalid_and_oversized_policy_cannot_disable_enforcement() {
        assert!(HostPolicy::from_yaml("allow: : invalid").is_err());
        assert!(HostPolicy::from_yaml(&" ".repeat(65537)).is_err());
    }
}
