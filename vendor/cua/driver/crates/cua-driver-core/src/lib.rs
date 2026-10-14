#![allow(dead_code, unused_imports)]
//! Platform-independent Driver core and dual-era MCP dispatch.
//!
//! Stdio supports legacy initialization and modern per-request negotiation.
//! Protocol metadata does not grant Driver permissions or session ownership.

// `ToolResult` is the tool reply itself: fallible helpers return
// `Result<_, ToolResult>` so an early exit is already the finished response,
// built once per call and handed straight to the caller. clippy's
// result_large_err assumes an error that propagates through hot paths; boxing
// here would churn every adapter's call sites for no runtime gain.
#![allow(clippy::result_large_err)]

pub(crate) mod mcp_skills;
pub(crate) mod mcp_wire;

pub const RESPONSIBILITY_DISCLAIMED_ENV: &str = "CUA_DRIVER_RS_RESPONSIBILITY_DISCLAIMED";

/// Embedded mode (`CUA_DRIVER_EMBEDDED=1` / `--embedded`): the daemon runs as
/// a direct child of a host app and stays in its TCC responsibility chain —
/// no disclaim re-exec, standalone-app relaunch, or permission prompts.
/// See `Skills/cua-driver/EMBEDDING.md`.
///
/// Caller-controlled, which is safe only because embedded mode strictly
/// REMOVES capability claims; it must never feed into the `driver-daemon`
/// attribution decision (`permission_source` in platform-macos).
pub const EMBEDDED_ENV: &str = "CUA_DRIVER_EMBEDDED";

/// Advisory label for the embedding host's bundle id, echoed in
/// `check_permissions` output. NOT a trust signal — trust comes from the
/// OS responsibility chain.
pub const HOST_BUNDLE_ID_ENV: &str = "CUA_DRIVER_HOST_BUNDLE_ID";

/// Internal embedded-host contract: when set to the exact value `1`, the
/// daemon treats EOF on stdin as proof that its owning host has exited. The
/// Rust SDK sets this only on the directly-spawned `serve` child; MCP proxies
/// continue to use stdin for JSON-RPC and never set it.
pub const PARENT_LIVENESS_STDIN_ENV: &str = "CUA_DRIVER_PARENT_LIVENESS_STDIN";

/// Only the exact value `1` counts — fail-safe for anything else.
pub fn embedded_mode() -> bool {
    std::env::var_os(EMBEDDED_ENV).is_some_and(|v| v == "1")
}

/// Parent-EOF shutdown is valid only for a directly embedded daemon. Requiring
/// both sentinels prevents an ambient variable from changing ordinary
/// standalone or MCP stdin behavior.
pub fn parent_liveness_stdin_enabled() -> bool {
    embedded_mode() && std::env::var_os(PARENT_LIVENESS_STDIN_ENV).is_some_and(|value| value == "1")
}

pub(crate) mod action_record;
pub(crate) mod action_target;
pub(crate) mod agent_cursor;
pub(crate) mod authorization;
pub(crate) mod background_input;
pub(crate) mod batch_tools;
pub(crate) mod browser;
pub(crate) mod build_info;
pub(crate) mod capture_mode;
pub(crate) mod capture_registry;
pub(crate) mod capture_runtime;
pub(crate) mod capture_scope;
pub(crate) mod cdp;
pub(crate) mod clipboard;
pub(crate) mod consent;
pub(crate) mod cursor_events;
pub(crate) mod cursor_hook;
pub(crate) mod cursor_sampler;
pub(crate) mod cursor_shape;
pub(crate) mod daemon;
pub(crate) mod delivery;
pub(crate) mod desktop_capture_scale;
pub(crate) mod element_frame;
pub(crate) mod element_query;
pub(crate) mod element_token;
pub(crate) mod expectation;
pub(crate) mod ffmpeg_install;
pub(crate) mod health_report;
pub(crate) mod history;
pub(crate) mod image_utils;
pub(crate) mod interactive_input;
pub(crate) mod key_pacing;
pub(crate) mod launch_guard;
pub(crate) mod mcp_result;
pub(crate) mod page;
pub(crate) mod perception_client;
#[cfg(target_os = "linux")]
pub mod capture_perception;
pub(crate) mod perception_tools;
pub(crate) mod pip_hook;
pub(crate) mod pointer_shape;
pub(crate) mod policy;
pub(crate) mod protocol;
pub(crate) mod recording;
pub(crate) mod recording_loader;
pub(crate) mod recording_render;
pub(crate) mod recording_tools;
pub(crate) mod recording_zoom;
#[cfg(feature = "script")]
pub(crate) mod script_tool;
pub(crate) mod server;
pub(crate) mod session;
pub(crate) mod session_authorization;
pub(crate) mod session_manifest;
pub(crate) mod session_tools;
pub(crate) mod single_flight;
pub(crate) mod snapshot_store;
#[cfg(test)]
pub(crate) mod snapshot_test_support;
pub(crate) mod socket_io;
pub(crate) mod text_sanitize;
pub(crate) mod timestamp;
pub use timestamp::unix_secs_to_rfc3339;
pub(crate) mod tool;
pub(crate) mod tool_args;
pub(crate) mod tool_schema;
pub(crate) mod video;
pub(crate) mod video_ffmpeg;
pub(crate) mod walk_budget;
pub(crate) mod window_inspection;
pub(crate) mod window_observation;
pub(crate) mod window_state_view;
pub(crate) mod window_target;

pub use cua_driver_contract::{CaptureScope, EscalationReason, TOOL_INVOCATION_FAILED_CODE};
pub(crate) use recording::RecordingSession;

/// The only public execution API of the ownership experiment.
pub mod embedded;

/// Read-only engine integration. No engine, transport, or registry is exported.
pub mod browser_observation;

#[cfg(feature = "yaml")]
pub mod host_policy;
