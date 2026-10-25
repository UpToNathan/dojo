use crate::config::MeshConfig;
use crate::device::Device;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Admission {
    /// This node. It belongs to its own mesh whatever else is true.
    This,
    Tag,
    Allowed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Exclusion {
    Denied,
    /// Not allowed by name, and no matching tag (or no tag is configured).
    NotAdmitted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Member {
    pub device: Device,
    pub admitted_by: Admission,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Excluded {
    pub device: Device,
    pub reason: Exclusion,
}

/// Where the devices came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "kebab-case")]
pub enum Source {
    Tailscale,
    /// No network could be read; the roster is this node alone.
    Unavailable(String),
}

/// The mesh as one node sees it. `members[0]` is always this node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Roster {
    pub source: Source,
    pub membership_tag: Option<String>,
    pub members: Vec<Member>,
    pub excluded: Vec<Excluded>,
}

impl Roster {
    pub fn this(&self) -> &Device {
        &self.members[0].device
    }

    pub fn peers(&self) -> impl Iterator<Item = &Member> {
        self.members.iter().skip(1)
    }

    /// This node alone, for when no network can be read.
    pub fn alone(this: Device, config: &MeshConfig, reason: String) -> Self {
        Self {
            source: Source::Unavailable(reason),
            membership_tag: config.membership_tag.clone(),
            members: vec![Member { device: this, admitted_by: Admission::This }],
            excluded: Vec::new(),
        }
    }
}

/// Decides who belongs. A deny entry beats everything, an allow entry beats the tag, and the tag
/// admits the rest. This node is always a member and is never excluded by its own deny list.
pub fn admit(this: Device, others: Vec<Device>, config: &MeshConfig) -> Roster {
    let mut members = vec![Member { device: this, admitted_by: Admission::This }];
    let mut excluded = Vec::new();
    for device in others {
        let named = |list: &[String]| list.iter().any(|key| device.answers_to(key));
        if named(&config.deny) {
            excluded.push(Excluded { device, reason: Exclusion::Denied });
        } else if named(&config.allow) {
            members.push(Member { device, admitted_by: Admission::Allowed });
        } else if config.membership_tag.as_deref().is_some_and(|tag| device.has_tag(tag)) {
            members.push(Member { device, admitted_by: Admission::Tag });
        } else {
            excluded.push(Excluded { device, reason: Exclusion::NotAdmitted });
        }
    }
    members[1..].sort_by_key(|member| crate::config::name_key(&member.device.name));
    excluded.sort_by_key(|entry| crate::config::name_key(&entry.device.name));
    Roster { source: Source::Tailscale, membership_tag: config.membership_tag.clone(), members, excluded }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::device;

    fn config(text: &str) -> MeshConfig {
        MeshConfig::parse(text).unwrap()
    }

    fn names(members: &[Member]) -> Vec<&str> {
        members.iter().map(|member| member.device.name.as_str()).collect()
    }

    #[test]
    fn the_tag_admits_and_untagged_devices_are_left_out() {
        let roster = admit(
            device("me", &["tag:mesh"], true),
            vec![device("zed", &["tag:mesh"], true), device("amy", &["tag:mesh"], false), device("other", &["tag:else"], true)],
            &config("membership-tag = \"tag:mesh\""),
        );
        assert_eq!(names(&roster.members), ["me", "amy", "zed"]);
        assert_eq!(roster.members[1].admitted_by, Admission::Tag);
        assert_eq!(roster.excluded.len(), 1);
        assert_eq!(roster.excluded[0].reason, Exclusion::NotAdmitted);
    }

    #[test]
    fn deny_beats_both_the_tag_and_allow() {
        let roster = admit(
            device("me", &[], true),
            vec![device("bad", &["tag:mesh"], true)],
            &config("membership-tag = \"tag:mesh\"\ndeny = [\"bad\"]"),
        );
        assert_eq!(names(&roster.members), ["me"]);
        assert_eq!(roster.excluded[0].reason, Exclusion::Denied);
    }

    #[test]
    fn allow_admits_without_the_tag_by_short_or_full_name() {
        let roster = admit(
            device("me", &[], true),
            vec![device("one", &[], true), device("two", &[], true)],
            &config("allow = [\"ONE\", \"two.example.ts.net\"]"),
        );
        assert_eq!(names(&roster.members), ["me", "one", "two"]);
        assert!(roster.peers().all(|member| member.admitted_by == Admission::Allowed));
    }

    #[test]
    fn without_a_tag_or_allow_list_nobody_joins() {
        let roster = admit(device("me", &[], true), vec![device("x", &["tag:mesh"], true)], &MeshConfig::default());
        assert_eq!(roster.peers().count(), 0);
        assert_eq!(roster.excluded.len(), 1);
    }

    #[test]
    fn a_node_cannot_deny_itself_out_of_its_own_mesh() {
        let roster = admit(device("me", &[], true), vec![], &config("deny = [\"me\"]"));
        assert_eq!(names(&roster.members), ["me"]);
        assert_eq!(roster.this().name, "me");
    }

    #[test]
    fn alone_is_a_valid_roster() {
        let roster = Roster::alone(device("me", &[], true), &MeshConfig::default(), "no network".into());
        assert_eq!(roster.peers().count(), 0);
        assert_eq!(roster.source, Source::Unavailable("no network".into()));
    }
}
