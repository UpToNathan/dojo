//! The display manager's pure core and its root PAM helper.
//!
//! [`protocol`] is the greeter socket, [`auth`] the lifecycle of one login attempt,
//! [`launch`] the datagrams between `dojo-dm` and its helper, [`account`] and [`session`]
//! who may log in and how their session starts. [`helper`] is the `dojo-dm-pam` process,
//! built on the lock screen's `lock::worker` conversation machinery and libpam loader.

pub mod account;
pub mod auth;
mod codec;
pub mod launch;
pub mod protocol;
pub mod session;

pub use codec::DecodeError;
