//! The sessions this machine's terminals opened, most recent first, in
//! `$XDG_STATE_HOME/dojo/terminal/recent` (one label per line).
use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(state.join("dojo/terminal/recent"))
}

/// Most recent first.
pub fn load() -> Vec<String> {
    path().and_then(|path| std::fs::read_to_string(path).ok()).map(|text| parse(&text)).unwrap_or_default()
}

/// Moves `label` to the front.
pub fn touch(label: &str) {
    let Some(path) = path() else { return };
    let labels = touched(load(), label);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, labels.join("\n") + "\n");
}

fn parse(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
}

fn touched(mut labels: Vec<String>, label: &str) -> Vec<String> {
    labels.retain(|existing| existing != label);
    labels.insert(0, label.to_owned());
    labels.truncate(64);
    labels
}

/// Orders `labels` by recency; ones never opened here keep their order after the rest.
pub fn sort(labels: &mut [String]) {
    let recent = load();
    labels.sort_by_key(|label| recent.iter().position(|r| r == label).unwrap_or(usize::MAX));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_moves_a_label_to_the_front_once() {
        let labels = touched(parse("a\nb\n\nc\n"), "c");
        assert_eq!(labels, ["c", "a", "b"]);
        assert_eq!(touched(labels, "d"), ["d", "c", "a", "b"]);
    }
}
