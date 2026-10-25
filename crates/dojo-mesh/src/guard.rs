//! The pause switch for the agent's peer access: a file in the state directory. It stops the
//! agent's own tools and calls marked as the agent's; it is a brake against a runaway, not a
//! security boundary, because whatever runs as the user can still run `tailscale ssh` itself.

use std::path::{Path, PathBuf};

const FLAG: &str = "agent-paused";

/// `$XDG_STATE_HOME/dojo/mesh`, falling back to `~/.local/state`.
pub fn state_dir(xdg_state_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = xdg_state_home.filter(|path| path.is_absolute()).or_else(|| home.map(|home| home.join(".local").join("state")))?;
    Some(base.join("dojo").join("mesh"))
}

pub fn agent_paused(state_dir: &Path) -> bool {
    state_dir.join(FLAG).exists()
}

pub fn set_agent_paused(state_dir: &Path, paused: bool) -> std::io::Result<()> {
    let flag = state_dir.join(FLAG);
    if paused {
        std::fs::create_dir_all(state_dir)?;
        std::fs::write(flag, b"")
    } else {
        match std::fs::remove_file(flag) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pausing_and_resuming_are_idempotent() {
        let dir = std::env::temp_dir().join(format!("dojo-mesh-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!agent_paused(&dir));
        set_agent_paused(&dir, false).unwrap();
        set_agent_paused(&dir, true).unwrap();
        set_agent_paused(&dir, true).unwrap();
        assert!(agent_paused(&dir));
        set_agent_paused(&dir, false).unwrap();
        assert!(!agent_paused(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn state_dir_prefers_an_absolute_xdg_directory() {
        assert_eq!(state_dir(Some("/s".into()), Some("/h".into())), Some(PathBuf::from("/s/dojo/mesh")));
        assert_eq!(state_dir(Some("rel".into()), Some("/h".into())), Some(PathBuf::from("/h/.local/state/dojo/mesh")));
        assert_eq!(state_dir(None, None), None);
    }
}
