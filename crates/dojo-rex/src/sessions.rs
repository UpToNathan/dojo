//! Every session on the server, described for a picker: where its focused terminal is, what it
//! is called, and whether a program in it reported that it finished, failed or needs you.
use anyhow::Result;
use serde_json::{Value, json};

use crate::control::Control;
use crate::layout::SessionView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Working,
    Done,
    Error,
    Blocked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub label: String,
    pub tabs: usize,
    /// The focused terminal's directory, as its shell last reported it.
    pub directory: Option<String>,
    /// The focused terminal's title, which shells set to the running command.
    pub title: Option<String>,
    /// The most pressing status any program in the session reported.
    pub status: Option<Status>,
}

/// Lists the server's sessions in its own order. Each is attached for the queries and then
/// detached again.
pub fn list_sessions(control: &Control) -> Result<Vec<SessionSummary>> {
    let sessions = control.call("session.list", json!({}))?;
    let mut out = Vec::new();
    for session in sessions["sessions"].as_array().into_iter().flatten() {
        let (Some(id), label) = (session["session_id"].as_str(), session["label"].as_str().unwrap_or_default()) else { continue };
        let mut summary =
            SessionSummary { id: id.to_owned(), label: label.to_owned(), tabs: 0, directory: None, title: None, status: None };
        if control.call("session.attach", json!({"session_id": id})).is_ok() {
            if let Ok(view) = control.call("session.view", json!({"session_id": id})) {
                let view = SessionView::parse(&view);
                summary.tabs = view.windows.len();
                if let Some(block) = view.focused_block() {
                    let call = |method: &str| control.block_call(method, id, &block, json!({})).ok();
                    summary.directory = call("com.superlogical.terminal.pwd").and_then(|r| r["pwd"].as_str().map(path_from_pwd));
                    summary.title = call("com.superlogical.terminal.title")
                        .and_then(|r| r["title"].as_str().map(str::to_owned))
                        .filter(|title| !title.is_empty());
                }
                let blocks = view.windows.iter().filter_map(|window| window.layout.as_ref()).flat_map(|layout| layout.blocks());
                summary.status = blocks
                    .filter_map(|block| control.block_call("com.superlogical.terminal.program_status", id, &block, json!({})).ok())
                    .filter_map(|records| most_pressing(&records))
                    .max();
            }
            let _ = control.call("session.detach", json!({"session_id": id}));
        }
        out.push(summary);
    }
    Ok(out)
}

fn most_pressing(result: &Value) -> Option<Status> {
    result["records"]
        .as_array()?
        .iter()
        .filter_map(|record| match record["state"].as_str()? {
            "working" => Some(Status::Working),
            "done" => Some(Status::Done),
            "error" => Some(Status::Error),
            "blocked" => Some(Status::Blocked),
            _ => None,
        })
        .max()
}

/// A reported directory is a plain path or an OSC 7 `file://host/path` URL.
pub fn path_from_pwd(pwd: &str) -> String {
    let Some(rest) = pwd.strip_prefix("file://") else { return pwd.to_owned() };
    let path = rest.find('/').map_or("/", |at| &rest[at..]);
    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&path[at + 1..at + 3], 16) {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_urls_become_paths() {
        assert_eq!(path_from_pwd("file://venus/home/nathan/My%20Dir"), "/home/nathan/My Dir");
        assert_eq!(path_from_pwd("/plain/path"), "/plain/path");
        assert_eq!(path_from_pwd("file:///tmp"), "/tmp");
    }

    #[test]
    fn blocked_outranks_done_and_working() {
        let records = json!({"records": [{"state": "working"}, {"state": "blocked"}, {"state": "done"}]});
        assert_eq!(most_pressing(&records), Some(Status::Blocked));
        assert_eq!(most_pressing(&json!({"records": [{"state": "idle"}]})), None);
    }
}
