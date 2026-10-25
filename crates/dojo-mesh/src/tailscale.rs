//! The Tailscale adapter: turns what the local client reports into devices. It needs no API
//! credentials and makes no network calls of its own; the client already knows the tailnet.

use crate::channel::{Call, Channel, Done};
use crate::config::MeshConfig;
use crate::device::Device;
use crate::method::{Connection, Identity, Membership, Method};
use crate::roster::{Roster, admit};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::process::{Command, Stdio};

#[derive(Deserialize)]
struct Status {
    #[serde(rename = "BackendState")]
    backend_state: Option<String>,
    #[serde(rename = "Self")]
    this: Option<Node>,
    #[serde(rename = "Peer", default)]
    peers: Option<HashMap<String, Node>>,
}

#[derive(Deserialize)]
struct Node {
    #[serde(rename = "HostName", default)]
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "Tags", default)]
    tags: Option<Vec<String>>,
    #[serde(rename = "Online", default)]
    online: Option<bool>,
    #[serde(rename = "KeyExpiry", default)]
    key_expiry: Option<String>,
    #[serde(rename = "Expired", default)]
    expired: Option<bool>,
}

impl Node {
    fn into_device(self, default_online: bool) -> Result<Device, String> {
        let dns_name = Some(self.dns_name.trim().to_string()).filter(|name| !name.is_empty());
        let name = Some(self.host_name.trim().to_string())
            .filter(|name| !name.is_empty())
            .or_else(|| dns_name.as_deref().and_then(|dns| dns.split('.').next()).map(str::to_string))
            .ok_or("a device has neither a host name nor a DNS name")?;
        // Tailscale reports an unset expiry as a zero or absent time; neither is a deadline.
        let key_expiry = self
            .key_expiry
            .as_deref()
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&Utc))
            .filter(|time| time.timestamp() > 0);
        Ok(Device {
            name,
            dns_name,
            tags: self.tags.unwrap_or_default(),
            online: self.online.unwrap_or(default_online),
            key_expiry,
            key_expired: self.expired.unwrap_or(false),
        })
    }
}

/// Reads the report of `tailscale status --json`: this device, then the others.
pub fn parse_status(json: &str) -> Result<(Device, Vec<Device>), String> {
    let status: Status = serde_json::from_str(json).map_err(|error| format!("unreadable Tailscale status: {error}"))?;
    if let Some(state) = status.backend_state.as_deref().filter(|state| *state != "Running") {
        return Err(format!("Tailscale is {state}"));
    }
    let this = status.this.ok_or("Tailscale status has no entry for this device")?.into_device(true)?;
    let others = status.peers.unwrap_or_default().into_values().map(|node| node.into_device(false)).collect::<Result<_, _>>()?;
    Ok((this, others))
}

/// Runs `tailscale status --json`. The error says why there is no report.
pub fn read_status() -> Result<String, String> {
    let output = Command::new("tailscale").args(["status", "--json"]).output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound { "tailscale is not installed".to_string() } else { format!("could not run tailscale: {error}") }
    })?;
    // `tailscale status` exits non-zero while logged out but still prints the report, so the
    // report, not the exit code, decides.
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tailscale gave no status: {}", stderr.trim()));
    }
    Ok(text)
}

/// Builds the roster from a status report. Any failure to read it leaves this node alone,
/// named `fallback_name`, because a mesh of one is a valid mesh.
pub fn roster(report: Result<String, String>, config: &MeshConfig, fallback_name: &str) -> Roster {
    match report.and_then(|json| parse_status(&json)) {
        Ok((this, others)) => admit(this, others, config),
        Err(reason) => Roster::alone(
            Device { name: fallback_name.into(), dns_name: None, tags: Vec::new(), online: true, key_expiry: None, key_expired: false },
            config,
            reason,
        ),
    }
}

/// Who `tailscale whois` says is at this address.
pub fn whois(address: &str) -> Result<String, String> {
    let output = Command::new("tailscale")
        .args(["whois", "--json", address])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("could not ask Tailscale who is connected: {error}"))?;
    if !output.status.success() {
        return Err("Tailscale did not know the connection".into());
    }
    crate::remote::node_name(&String::from_utf8_lossy(&output.stdout))
}

struct TailscaleMembership;

impl Membership for TailscaleMembership {
    fn roster(&self, config: &MeshConfig, hostname: &str) -> Roster {
        roster(read_status(), config, hostname)
    }
}

struct TailscaleIdentity;

impl Identity for TailscaleIdentity {
    fn who(&self, connection: &Connection) -> Result<String, String> {
        whois(&connection.address)
    }
}

struct TailscaleSsh;

impl Channel for TailscaleSsh {
    fn run(&self, call: Call) -> Result<Done, String> {
        crate::ssh::run(call)
    }
}

/// Tailscale as a mesh method: tag membership from the local client, `whois` for identity and
/// `tailscale ssh` for calls.
pub fn method() -> Method {
    Method { name: "tailscale", membership: Box::new(TailscaleMembership), identity: Box::new(TailscaleIdentity), channel: Box::new(TailscaleSsh) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::Source;

    const REPORT: &str = r#"{
      "BackendState": "Running",
      "Self": {"HostName": "alpha", "DNSName": "alpha.example.ts.net.", "Tags": ["tag:mesh"], "Online": true, "KeyExpiry": "0001-01-01T00:00:00Z"},
      "Peer": {
        "nodekey:1": {"HostName": "beta", "DNSName": "beta.example.ts.net.", "Tags": ["tag:mesh"], "Online": true, "KeyExpiry": "2027-03-01T00:00:00Z"},
        "nodekey:2": {"HostName": "", "DNSName": "gamma.example.ts.net.", "Tags": null, "Online": false, "Expired": true}
      }
    }"#;

    #[test]
    fn reads_this_device_and_the_others() {
        let (this, others) = parse_status(REPORT).unwrap();
        assert_eq!(this.name, "alpha");
        assert!(this.online);
        assert_eq!(this.key_expiry, None, "the zero time is no deadline");
        let beta = others.iter().find(|d| d.name == "beta").unwrap();
        assert_eq!(beta.key_expiry.unwrap().to_rfc3339(), "2027-03-01T00:00:00+00:00");
        let gamma = others.iter().find(|d| d.name == "gamma").unwrap();
        assert!(gamma.tags.is_empty() && !gamma.online && gamma.key_expired);
    }

    #[test]
    fn builds_a_roster_through_the_tag() {
        let config = MeshConfig::parse("membership-tag = \"tag:mesh\"").unwrap();
        let roster = roster(Ok(REPORT.into()), &config, "fallback");
        assert_eq!(roster.source, Source::Tailscale);
        assert_eq!(roster.this().name, "alpha");
        assert_eq!(roster.peers().map(|m| m.device.name.as_str()).collect::<Vec<_>>(), ["beta"]);
        assert_eq!(roster.excluded.len(), 1);
    }

    #[test]
    fn a_stopped_or_missing_tailscale_leaves_a_mesh_of_one() {
        let config = MeshConfig::default();
        for report in [
            Err("tailscale is not installed".to_string()),
            Ok(r#"{"BackendState": "NeedsLogin"}"#.to_string()),
            Ok("not json".to_string()),
            Ok(r#"{"BackendState": "Running"}"#.to_string()),
        ] {
            let roster = roster(report, &config, "solo");
            assert_eq!(roster.this().name, "solo");
            assert_eq!(roster.peers().count(), 0);
            assert!(matches!(roster.source, Source::Unavailable(_)));
        }
    }

    #[test]
    fn a_device_with_no_name_at_all_is_an_error() {
        let report = r#"{"BackendState": "Running", "Self": {"HostName": "a"}, "Peer": {"k": {}}}"#;
        assert!(parse_status(report).is_err());
    }
}
