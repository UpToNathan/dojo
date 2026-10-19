//! The display manager: its pure cores, its root PAM helper and its service loop.
//!
//! [`protocol`] is the greeter socket, [`auth`] the lifecycle of one login attempt and
//! [`registry`] the attempts in flight, [`cycle`] what the service does next. [`launch`]
//! is the datagrams between `dojo-dm` and its helper, [`account`] and [`session`] who may
//! log in and how their session starts, [`sessions`] the desktop entries on offer and
//! [`memory`] what the login screen remembers. [`helper`] is the `dojo-dm-pam` process,
//! built on the lock screen's `lock::worker` conversation machinery and libpam loader.
//! [`service`] is `dojo-dm` itself; [`process`], [`greeter`], [`vt`], [`host`] and [`look`]
//! are the thin parts that touch the machine, each behind a seam the tests replace.

pub mod account;
pub mod auth;
mod codec;
pub mod config;
pub mod cycle;
pub mod greeter;
pub mod helper;
pub mod host;
pub mod launch;
pub mod look;
pub mod memory;
pub mod process;
pub mod protocol;
pub mod registry;
pub mod service;
pub mod session;
pub mod sessions;
pub mod stack;
pub mod vt;

pub use codec::DecodeError;
