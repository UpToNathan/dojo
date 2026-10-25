//! The screen outputs of this node, as the compositor names them (Niri's `niri msg outputs`).
//! Boundaries refer to outputs by those names.

use std::process::{Command, Stdio};

/// The output names in `niri msg --json outputs`, sorted.
pub fn parse(json: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| format!("unreadable output list: {error}"))?;
    let object = value.as_object().ok_or("the output list is not an object")?;
    let mut names: Vec<String> = object.keys().cloned().collect();
    names.sort();
    Ok(names)
}

/// This node's outputs, or `None` when there is no compositor to ask (no session, no Niri).
pub fn read() -> Option<Vec<String>> {
    let output = Command::new("niri").args(["msg", "--json", "outputs"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_output_names_sorted() {
        let json = r#"{"HDMI-A-1": {"name": "HDMI-A-1", "logical": null}, "DP-1": {"name": "DP-1"}}"#;
        assert_eq!(parse(json).unwrap(), ["DP-1", "HDMI-A-1"]);
        assert_eq!(parse("{}").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn rejects_anything_but_an_object() {
        assert!(parse("[]").is_err());
        assert!(parse("not json").is_err());
    }
}
