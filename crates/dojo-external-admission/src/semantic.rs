//! Native actor handoff contract. The application reply is distinct from dispatch.
use niri_ipc::AccessibilityTarget;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Dispatch {
        id: String,
        peer: u64,
        revision: u64,
        input_generation: u64,
        target: AccessibilityTarget,
        process_start: u64,
        lifetime_ms: u32,
        payload: Vec<u8>,
    },
    Terminal {
        id: String,
        peer: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: String,
    pub effect: Effect,
    pub accepted_bytes: usize,
    pub message: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Refused,
    Unknown,
    Terminal,
}

pub fn socket_path(native: &std::path::Path) -> std::path::PathBuf {
    let mut path = native.as_os_str().to_os_string();
    path.push(".semantic");
    path.into()
}
