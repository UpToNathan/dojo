//! The seams of a mesh method. A method is one way of forming a mesh: it says who the members are
//! (`Membership`), who a connection is (`Identity`) and how a call reaches a peer (`Channel`).
//! Tailscale is the first method; the core knows only these traits, so another method can replace
//! it, and a node with no method at all is a mesh of one.

use crate::channel::Channel;
use crate::config::MeshConfig;
use crate::roster::Roster;

/// How one node reaches a peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// A name or address to connect to, with a port when it is not the usual one.
    Host { name: String, port: Option<u16> },
    /// A command that opens the connection (an SSH `ProxyCommand`), for networks a plain address
    /// cannot cross. A channel that cannot run one refuses the call.
    Command(Vec<String>),
}

/// What a peer's incoming connection shows about itself, as the transport reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    /// The address the connection came from.
    pub address: String,
}

/// Decides who belongs to the mesh and how to reach them. Any failure to read the network leaves
/// this node alone (`Roster::alone`): a mesh of one is a valid mesh.
pub trait Membership: Send + Sync {
    fn roster(&self, config: &MeshConfig, hostname: &str) -> Roster;
}

/// Says which node is on the other end of an incoming connection. The answer comes from the
/// network, never from what the caller claims.
pub trait Identity: Send + Sync {
    fn who(&self, connection: &Connection) -> Result<String, String>;
}

/// One way of forming a mesh.
pub struct Method {
    pub name: &'static str,
    pub membership: Box<dyn Membership>,
    pub identity: Box<dyn Identity>,
    pub channel: Box<dyn Channel>,
}

impl Method {
    /// Tailscale: membership from the local client's report, identity from `tailscale whois`,
    /// calls over `tailscale ssh`.
    pub fn tailscale() -> Self {
        crate::tailscale::method()
    }

    /// No method: every node is a mesh of one, nothing can be identified and no call can be made.
    pub fn none() -> Self {
        Self { name: "none", membership: Box::new(Alone), identity: Box::new(Alone), channel: Box::new(Alone) }
    }
}

const NO_METHOD: &str = "no mesh method is configured";

struct Alone;

impl Membership for Alone {
    fn roster(&self, config: &MeshConfig, hostname: &str) -> Roster {
        Roster::alone(
            crate::device::Device { name: hostname.into(), dns_name: None, tags: Vec::new(), online: true, key_expiry: None, key_expired: false },
            config,
            NO_METHOD.into(),
        )
    }
}

impl Identity for Alone {
    fn who(&self, _: &Connection) -> Result<String, String> {
        Err(NO_METHOD.into())
    }
}

impl Channel for Alone {
    fn run(&self, _: crate::channel::Call) -> Result<crate::channel::Done, String> {
        Err(NO_METHOD.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Call, Done, Ended, Input, Output};
    use crate::cli::{Env, UNAVAILABLE};
    use crate::device::device;
    use crate::roster::{Source, admit_from};
    use crate::transport::Origin;
    use std::sync::{Arc, Mutex};

    /// Members the test names, as a method other than Tailscale would report them.
    struct Canned(Vec<crate::device::Device>);

    impl Membership for Canned {
        fn roster(&self, config: &MeshConfig, hostname: &str) -> Roster {
            admit_from(Source::Method("fake".into()), device(hostname, &[], true), self.0.clone(), config)
        }
    }

    struct Known(Vec<(&'static str, &'static str)>);

    impl Identity for Known {
        fn who(&self, connection: &Connection) -> Result<String, String> {
            self.0.iter().find(|(address, _)| *address == connection.address).map(|(_, name)| name.to_string()).ok_or_else(|| "nobody is known at that address".to_string())
        }
    }

    #[derive(Clone, Default)]
    struct Recording {
        calls: Arc<Mutex<Vec<(Route, String)>>>,
        fail_with: Option<String>,
    }

    impl Channel for Recording {
        fn run(&self, call: Call) -> Result<Done, String> {
            if let Some(reason) = &self.fail_with {
                return Err(reason.clone());
            }
            self.calls.lock().unwrap().push((call.route, call.remote));
            Ok(Done { ended: Ended::Exited(0), stdout: b"done".to_vec() })
        }
    }

    fn env(method: Method) -> Env {
        Env { config_path: None, state_dir: None, home: None, hostname: "alpha".into(), origin: Origin::Operator, session: "s1".into(), method }
    }

    fn fake(devices: Vec<crate::device::Device>, channel: Recording) -> Env {
        env(Method { name: "fake", membership: Box::new(Canned(devices)), identity: Box::new(Known(vec![("100.64.0.9", "beta")])), channel: Box::new(channel) })
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn a_node_with_no_method_is_a_mesh_of_one_and_can_do_nothing_else() {
        let method = Method::none();
        let roster = method.membership.roster(&MeshConfig::default(), "solo");
        assert_eq!(roster.this().name, "solo");
        assert_eq!(roster.peers().count(), 0);
        assert!(matches!(&roster.source, Source::Unavailable(reason) if reason.contains("no mesh method")));
        assert!(method.identity.who(&Connection { address: "100.64.0.9".into() }).is_err());
        let call = Call { route: Route::Host { name: "beta".into(), port: None }, remote: "true".into(), input: Input::Null, output: Output::Capture, timeout: None };
        assert!(method.channel.run(call).unwrap_err().contains("no mesh method"));
    }

    #[test]
    fn a_call_reaches_the_peer_through_the_channel_with_its_route_and_provenance() {
        let channel = Recording::default();
        let calls = channel.calls.clone();
        let config = MeshConfig::parse("allow = [\"beta\"]").unwrap();
        let env = fake(vec![device("beta", &[], true)], channel);
        let roster = env.method.membership.roster(&config, &env.hostname);
        assert_eq!(roster.source, Source::Method("fake".into()));
        let done = crate::cli::call_peer(&env, &roster, "beta", "exec", "echo", &argv(&["echo", "hi"]), Input::Null, Output::Capture, None).unwrap();
        assert_eq!(done.stdout, b"done");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, Route::Host { name: "beta.example.ts.net".into(), port: None });
        assert!(calls[0].1.starts_with("env DOJO_FROM='alpha' DOJO_SESSION='s1' DOJO_ORIGIN=operator 'echo' 'hi'"), "{}", calls[0].1);
    }

    #[test]
    fn an_offline_peer_or_a_stranger_never_reaches_the_channel() {
        let channel = Recording::default();
        let calls = channel.calls.clone();
        let config = MeshConfig::parse("allow = [\"gone\"]").unwrap();
        let env = fake(vec![device("gone", &[], false), device("stranger", &[], true)], channel);
        let roster = env.method.membership.roster(&config, &env.hostname);
        for peer in ["gone", "stranger"] {
            let failure = crate::cli::call_peer(&env, &roster, peer, "exec", "x", &argv(&["true"]), Input::Null, Output::Capture, None).unwrap_err();
            assert_eq!(failure.code, UNAVAILABLE, "{peer}: {}", failure.message);
        }
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_channel_that_cannot_start_is_reported_as_unavailable() {
        let config = MeshConfig::parse("allow = [\"beta\"]").unwrap();
        let env = fake(vec![device("beta", &[], true)], Recording { fail_with: Some("ssh is not installed".into()), ..Recording::default() });
        let roster = env.method.membership.roster(&config, &env.hostname);
        let failure = crate::cli::call_peer(&env, &roster, "beta", "exec", "x", &argv(&["true"]), Input::Null, Output::Capture, None).unwrap_err();
        assert_eq!(failure.code, UNAVAILABLE);
        assert!(failure.message.contains("ssh is not installed"), "{}", failure.message);
    }

    #[test]
    fn identity_comes_from_the_method_and_an_unknown_address_is_refused() {
        let env = fake(vec![], Recording::default());
        assert_eq!(env.method.identity.who(&Connection { address: "100.64.0.9".into() }).unwrap(), "beta");
        assert!(env.method.identity.who(&Connection { address: "10.0.0.1".into() }).is_err());
    }

    #[test]
    fn the_tailscale_channel_refuses_routes_it_cannot_carry() {
        let call = |route| Call { route, remote: "true".into(), input: Input::Null, output: Output::Capture, timeout: None };
        let tailscale = Method::tailscale();
        assert!(tailscale.channel.run(call(Route::Command(vec!["nc".into()]))).unwrap_err().contains("command route"));
        assert!(tailscale.channel.run(call(Route::Host { name: "beta".into(), port: Some(2222) })).unwrap_err().contains("port"));
    }
}
