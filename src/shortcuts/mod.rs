//! A GlobalShortcuts portal backend. Niri grabs the keys (see `vendor/niri/src/global_shortcuts.rs`);
//! this module decides which keys each app gets, remembers the choice, and speaks the portal's
//! D-Bus interface. Nothing here depends on GPUI.

pub mod compositor;
pub mod portal;
pub mod registry;
pub mod store;
pub mod trigger;
