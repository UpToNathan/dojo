//! Engines behind the launcher's commands and inline rows (`.scratch/launcher-features/spec.md`).
//!
//! Each engine is independent of GPUI and of the launcher window; the launcher only calls into it.

pub mod bangs;
pub mod calculator;
pub mod clipboard_history;
pub mod color;
pub mod commands;
pub mod devutil;
pub mod emoji;
pub mod inline;
pub mod processes;
pub mod root;
pub mod rows;
pub mod services;
pub mod snippets;
pub mod windows;
