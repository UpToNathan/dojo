//! The lock screen's pure core and its PAM boundary. `LockModel` decides; the
//! [`edge`] that `dojo-lock` and `dojo-greeter` share carries out its effects and paints its scenes.

pub mod edge;
mod login;
mod model;
pub mod pam;
mod power;
mod scene;
pub mod secret;
pub mod worker;

#[cfg(test)]
mod tests;

pub use login::{
    Choice, FarewellSet, Leave, Login, LoginError, MAX_LOGIN_BYTES, NameError, UserLook, Username,
};
pub use model::{
    Effect, Event, EventKind, ExitReason, GreetingSet, HostChips, Key, LockModel, Now, Output,
    OutputId, Settings,
};
pub use scene::{
    Chips, Dial, Line, LoginChips, LookMix, NameEdit, Panel, Pill, Ring, Role, Scene, Tone,
};
