use chrono::{DateTime, Utc};
use serde::Serialize;

/// A device as the network reports it. Only what the mesh needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Device {
    /// The short host name, as the device calls itself.
    pub name: String,
    /// The fully qualified name on the network, when there is one.
    pub dns_name: Option<String>,
    pub tags: Vec<String>,
    pub online: bool,
    pub key_expiry: Option<DateTime<Utc>>,
    pub key_expired: bool,
}

impl Device {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|candidate| candidate == tag)
    }

    /// Whether a config name (already normalised) refers to this device.
    pub fn answers_to(&self, key: &str) -> bool {
        crate::config::name_key(&self.name) == key
            || self.dns_name.as_deref().is_some_and(|dns| crate::config::name_key(dns) == key)
    }
}

#[cfg(test)]
pub(crate) fn device(name: &str, tags: &[&str], online: bool) -> Device {
    Device {
        name: name.into(),
        dns_name: Some(format!("{name}.example.ts.net.")),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        online,
        key_expiry: None,
        key_expired: false,
    }
}
