//! The mesh: which devices a Dojo node treats as its peers, and whether they are well.
//!
//! The core (`config`, `device`, `roster`, `doctor`) is plain data and knows nothing about Tailscale
//! or processes. `tailscale` is the one adapter that turns the local Tailscale client's report into
//! devices. A node that cannot see any network is a mesh of one, which is a valid result everywhere.

pub mod config;
pub mod device;
pub mod doctor;
pub mod roster;
pub mod tailscale;
pub mod report;
