//! The lock screen's pure core and its PAM boundary. `LockModel` decides; the
//! `dojo-lock` edge carries out its effects and paints its scenes.

mod model;
pub mod pam;
mod power;
mod scene;
pub mod secret;
pub mod worker;

#[cfg(test)]
mod tests;

pub use model::{
    Effect, Event, EventKind, ExitReason, GreetingSet, HostChips, Key, LockModel, Now, Output,
    OutputId, Settings,
};
pub use scene::{Chips, Dial, Line, Panel, Pill, Ring, Role, Scene, Tone};
