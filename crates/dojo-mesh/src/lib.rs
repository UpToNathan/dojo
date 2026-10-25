//! The mesh: which devices a Dojo node treats as its peers, and whether they are well.
//!
//! The core (`config`, `device`, `roster`, `doctor`) is plain data and knows nothing about Tailscale
//! or processes. `method` names the three seams a way of forming a mesh fills (membership,
//! identity and the channel, with `Route` saying how a peer is reached); `tailscale` is the first
//! method. A node that cannot see any network is a mesh of one, which is a valid result everywhere.

pub mod adapter;
pub mod audit;
pub mod channel;
pub mod cli;
pub mod capability;
pub mod config;
pub mod control;
pub mod control_cli;
pub mod diff;
pub mod desktop;
pub mod device;
pub mod doctor;
pub mod guard;
pub mod handoff;
pub mod hujson;
pub mod method;
pub mod niri;
pub mod openssh;
pub mod roster;
pub mod signed;
pub mod send_cli;
pub mod session;
pub mod source;
pub mod ssh;
pub mod tabs;
pub mod tailscale;
pub mod transport;
pub mod remote;
pub mod remote_cli;
pub mod report;
