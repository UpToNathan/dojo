//! The Mesh command: this machine and the peers it admits, read from the local Tailscale client.
//! Read-only; reaching a peer is `dojo-mesh exec` and the agent's mesh tools.

use std::path::PathBuf;

use dojo::launcher_core::rows::{Row, RowAction, RowIcon};
use dojo_mesh::config::{MeshConfig, default_path};
use dojo_mesh::device::Device;
use dojo_mesh::roster::{Admission, Roster, Source};

use super::loader::{LoadState, Loader};
use super::{Host, Notify, Outcome, Secondary, View};

type Load = fn() -> Roster;

/// What a row stands for, by the position its `Item` carries.
struct Shown {
    host: String,
    this_node: bool,
}

pub struct MeshView {
    load: Load,
    loader: Option<Loader<Roster>>,
    shown: Vec<Shown>,
}

impl MeshView {
    pub fn new() -> Self {
        Self::with(read_roster)
    }

    fn with(load: Load) -> Self {
        Self {
            load,
            loader: None,
            shown: Vec::new(),
        }
    }
}

/// The mesh as this node sees it now. A configuration that cannot be read leaves the mesh empty
/// rather than failing the view, so the view still shows this machine.
fn read_roster() -> Roster {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let config = default_path(var("XDG_CONFIG_HOME"), var("HOME"))
        .and_then(|path| MeshConfig::load(&path).ok())
        .unwrap_or_default();
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|name| name.trim().to_owned())
        .unwrap_or_default();
    dojo_mesh::tailscale::roster(
        dojo_mesh::tailscale::read_status(),
        &config,
        if hostname.is_empty() {
            "this-node"
        } else {
            &hostname
        },
    )
}

fn host(device: &Device) -> String {
    device
        .dns_name
        .as_deref()
        .map(|dns| dns.trim_end_matches('.').to_owned())
        .unwrap_or_else(|| device.name.clone())
}

fn matches(device: &Device, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || device.name.to_lowercase().contains(&query)
        || host(device).to_lowercase().contains(&query)
        || device
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(&query))
}

impl View for MeshView {
    fn placeholder(&self) -> &str {
        "Search your machines…"
    }

    fn open(&mut self, notify: Notify) {
        let load = self.load;
        self.loader = Some(Loader::start(notify, move || Ok(load())));
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        self.shown.clear();
        let Some(loader) = &self.loader else {
            return Vec::new();
        };
        let shown = &mut self.shown;
        loader.with(|state| {
            let LoadState::Ready(roster) = state else {
                return Vec::new();
            };
            let mut rows = Vec::new();
            for member in &roster.members {
                if !matches(&member.device, query) {
                    continue;
                }
                let this_node = member.admitted_by == Admission::This;
                let state = if member.device.online {
                    "online"
                } else {
                    "offline"
                };
                let subtitle = match member.admitted_by {
                    Admission::This => format!("This machine · {state}"),
                    Admission::Allowed => format!("{state} · allowed by name"),
                    Admission::Tag => state.to_owned(),
                };
                shown.push(Shown {
                    host: host(&member.device),
                    this_node,
                });
                rows.push(Row {
                    id: String::new(),
                    title: member.device.name.clone(),
                    subtitle,
                    icon: RowIcon::Glyph(
                        if this_node {
                            "◆"
                        } else if member.device.online {
                            "●"
                        } else {
                            "○"
                        }
                        .into(),
                    ),
                    action: RowAction::Item(shown.len() - 1),
                });
            }
            if query.trim().is_empty() && roster.peers().next().is_none() {
                rows.push(Row::header(match &roster.source {
                    Source::Unavailable(_) => "Tailscale is not available, so there are no peers",
                    Source::Tailscale => {
                        "No peers: set membership-tag or allow in ~/.config/dojo/mesh.toml"
                    }
                }));
            }
            rows
        })
    }

    /// Enter copies the command that opens a shell on the peer.
    fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
        match row.action {
            RowAction::Item(index) => match self.shown.get(index) {
                Some(shown) if shown.this_node => Outcome::Notice("That is this machine".into()),
                Some(shown) => Outcome::Copy(format!("tailscale ssh {}", shown.host)),
                None => Outcome::Stay,
            },
            _ => Outcome::Stay,
        }
    }

    fn secondary(&mut self, key: Secondary, row: &Row, _host: &Host) -> Option<Outcome> {
        let RowAction::Item(index) = row.action else {
            return None;
        };
        match key {
            Secondary::Alternate => self
                .shown
                .get(index)
                .map(|shown| Outcome::Copy(shown.host.clone())),
            _ => None,
        }
    }

    fn hint(&self) -> &str {
        "Shift+Enter copy name"
    }

    fn empty_text(&self, query: &str) -> String {
        let loading = self
            .loader
            .as_ref()
            .is_none_or(|loader| loader.with(|state| matches!(state, LoadState::Loading)));
        if loading {
            "Reading the network…".into()
        } else if query.trim().is_empty() {
            "No machines found".into()
        } else {
            "No matching machines".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use dojo_mesh::config::MeshConfig;
    use dojo_mesh::roster::admit;

    use super::*;

    fn device(name: &str, tags: &[&str], online: bool) -> Device {
        Device {
            name: name.into(),
            dns_name: Some(format!("{name}.example.ts.net.")),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            online,
            key_expiry: None,
            key_expired: false,
        }
    }

    fn mesh() -> Roster {
        let config = MeshConfig::parse("membership-tag = \"tag:mesh\"\nallow = [\"odd\"]").unwrap();
        admit(
            device("alpha", &["tag:mesh"], true),
            vec![
                device("beta", &["tag:mesh"], true),
                device("gone", &["tag:mesh"], false),
                device("odd", &[], true),
                device("stranger", &[], true),
            ],
            &config,
        )
    }

    fn view(roster: fn() -> Roster) -> MeshView {
        let mut view = MeshView::with(roster);
        view.loader = Some(Loader::ready(roster()));
        view
    }

    fn lines(rows: &[Row]) -> Vec<(String, String)> {
        rows.iter()
            .map(|row| (row.title.clone(), row.subtitle.clone()))
            .collect()
    }

    #[test]
    fn lists_this_machine_first_then_the_admitted_peers_with_their_state() {
        let mut view = view(mesh);
        assert_eq!(
            lines(&view.rows("")),
            [
                ("alpha".to_owned(), "This machine · online".to_owned()),
                ("beta".to_owned(), "online".to_owned()),
                ("gone".to_owned(), "offline".to_owned()),
                ("odd".to_owned(), "online · allowed by name".to_owned()),
            ],
            "the stranger is not in the mesh and is not listed"
        );
    }

    #[test]
    fn the_search_matches_names_and_tags() {
        let mut view = view(mesh);
        assert_eq!(
            lines(&view.rows("BET")),
            [("beta".to_owned(), "online".to_owned())]
        );
        assert_eq!(view.rows("tag:mesh").len(), 3);
        assert!(view.rows("nothing").is_empty());
    }

    #[test]
    fn enter_copies_the_ssh_command_and_alt_the_name() {
        let mut view = view(mesh);
        let rows = view.rows("");
        let host = Host::default();
        assert_eq!(
            view.activate(&rows[1], "", &host),
            Outcome::Copy("tailscale ssh beta.example.ts.net".into())
        );
        assert_eq!(
            view.secondary(Secondary::Alternate, &rows[1], &host),
            Some(Outcome::Copy("beta.example.ts.net".into()))
        );
        assert_eq!(
            view.activate(&rows[0], "", &host),
            Outcome::Notice("That is this machine".into())
        );
        assert_eq!(view.secondary(Secondary::Delete, &rows[1], &host), None);
    }

    #[test]
    fn a_mesh_of_one_still_shows_this_machine_and_says_how_to_add_peers() {
        let mut view = view(|| {
            Roster::alone(
                device("solo", &[], true),
                &MeshConfig::default(),
                "tailscale is not installed".into(),
            )
        });
        let rows = view.rows("");
        assert_eq!(rows[0].title, "solo");
        assert_eq!(rows[1].title, "Tailscale is not available, so there are no peers");
        assert!(!rows[1].is_selectable());
        assert_eq!(view.rows("so").len(), 1, "no guidance while searching");
    }

    #[test]
    fn the_status_line_says_what_the_view_is_waiting_for() {
        let mut view = MeshView::with(mesh);
        assert_eq!(view.empty_text(""), "Reading the network…");
        view.loader = Some(Loader::ready(mesh()));
        assert_eq!(view.empty_text(""), "No machines found");
        assert_eq!(view.empty_text("zz"), "No matching machines");
    }
}
