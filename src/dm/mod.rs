//! The display manager's pure core and its root PAM helper.
//!
//! [`protocol`] is the greeter socket, [`auth`] the lifecycle of one login attempt,
//! [`launch`] the datagrams between `dojo-dm` and its helper, [`account`] and [`session`]
//! who may log in and how their session starts. [`helper`] is the `dojo-dm-pam` process,
//! built on the lock screen's `lock::worker` conversation machinery and libpam loader.

pub mod account;
pub mod auth;
pub mod cycle;
mod codec;
pub mod helper;
pub mod launch;
pub mod look;
pub mod memory;
pub mod process;
pub mod protocol;
pub mod registry;
pub mod session;
pub mod sessions;
pub mod stack;
pub mod vt;

pub use codec::DecodeError;
