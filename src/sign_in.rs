//! Account sign-in state for the model picker, independent of GPUI and the voice socket.
//!
//! The shell never reads or stores credentials. A typed key lives in an [`InputBuffer`] until it is
//! submitted or the flow is dismissed, and the voice service reports only public progress.

use serde::Deserialize;

use crate::model_picker::{Account, ModelCatalog};

pub const CHATGPT: &str = "openai-codex";
pub const OPENCODE_ACCOUNTS: [&str; 2] = ["opencode-go", "opencode"];
const MAX_KEY_BYTES: usize = 4096;
const MAX_PASTE_BYTES: usize = 8192;

/// Public progress of the latest sign-in, as reported by the voice service's `login-status`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct LoginStatus {
    pub status: String,
    pub active: bool,
    pub login_id: Option<String>,
    pub account: Option<String>,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
    pub url: Option<String>,
    pub instructions: Option<String>,
    pub prompt_type: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Starting,
    DeviceCode {
        code: String,
        address: String,
    },
    /// Browser login: the service opened (or shows) an address and waits for the redirect.
    BrowserAddress {
        url: String,
    },
    /// The service asks for a pasted redirect URL or code.
    NeedsPaste {
        message: String,
    },
    Complete,
    Failed(String),
}

impl Progress {
    /// Maps a service status onto what the panel shows. Unknown statuses keep the flow starting.
    pub fn from_status(status: &LoginStatus) -> Progress {
        match status.status.as_str() {
            "device_code" => match (&status.user_code, &status.verification_uri) {
                (Some(code), Some(address)) => Progress::DeviceCode {
                    code: code.clone(),
                    address: address.clone(),
                },
                _ => Progress::Starting,
            },
            "auth_url" => Progress::BrowserAddress {
                url: status.url.clone().unwrap_or_default(),
            },
            "prompt" => Progress::NeedsPaste {
                message: status
                    .message
                    .clone()
                    .unwrap_or_else(|| "Paste the address you were sent to.".into()),
            },
            "complete" => Progress::Complete,
            "failure" => Progress::Failed(
                status
                    .message
                    .clone()
                    .unwrap_or_else(|| "Sign-in failed.".into()),
            ),
            _ => Progress::Starting,
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, Progress::Complete | Progress::Failed(_))
    }
}

/// A single-line text buffer. Masked buffers never expose their contents for display.
#[derive(Clone, Debug)]
pub struct InputBuffer {
    text: String,
    select_all: bool,
    masked: bool,
    limit: usize,
}

impl InputBuffer {
    pub fn secret() -> Self {
        Self {
            text: String::new(),
            select_all: false,
            masked: true,
            limit: MAX_KEY_BYTES,
        }
    }

    pub fn plain() -> Self {
        Self {
            text: String::new(),
            select_all: false,
            masked: false,
            limit: MAX_PASTE_BYTES,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn insert(&mut self, input: &str) {
        if self.select_all {
            self.text.clear();
        }
        self.select_all = false;
        let available = self.limit.saturating_sub(self.text.len());
        let end = input
            .char_indices()
            .map(|(index, ch)| index + ch.len_utf8())
            .take_while(|end| *end <= available)
            .last()
            .unwrap_or(0);
        self.text.push_str(&input[..end]);
    }

    /// Inserts typed or pasted text, dropping control characters and turning line breaks into spaces.
    pub fn insert_text(&mut self, input: &str) {
        let cleaned: String = input
            .replace(['\n', '\r'], " ")
            .chars()
            .filter(|ch| !ch.is_control())
            .collect();
        self.insert(&cleaned);
    }

    pub fn backspace(&mut self) {
        if self.select_all {
            self.text.clear();
        } else {
            self.text.pop();
        }
        self.select_all = false;
    }

    pub fn select_all(&mut self) {
        self.select_all = true;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.select_all = false;
    }

    /// Takes the trimmed text and leaves the buffer empty. Returns `None` when there is nothing to send.
    pub fn take(&mut self) -> Option<String> {
        let value = std::mem::take(&mut self.text);
        self.select_all = false;
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    }

    /// What the panel draws: bullets for a secret, the text otherwise, or the placeholder when empty.
    pub fn display(&self, placeholder: &str) -> String {
        if self.text.is_empty() {
            placeholder.to_owned()
        } else if self.masked {
            "•".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }
}

impl Drop for InputBuffer {
    fn drop(&mut self) {
        self.text.clear();
    }
}

/// The panel's current sign-in activity.
#[derive(Debug, Default)]
pub enum Flow {
    #[default]
    None,
    ChatGpt {
        login_id: Option<String>,
        progress: Progress,
        paste: InputBuffer,
    },
    ApiKey {
        account: String,
        label: String,
        key: InputBuffer,
        submitting: bool,
        error: Option<String>,
    },
}

/// What the shell should do after a state change.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// The login finished: reload the catalog and close the flow.
    Reload,
}

impl Flow {
    pub fn begin_chatgpt() -> Flow {
        Flow::ChatGpt {
            login_id: None,
            progress: Progress::Starting,
            paste: InputBuffer::plain(),
        }
    }

    pub fn begin_api_key(account: &Account) -> Flow {
        Flow::ApiKey {
            account: account.id.clone(),
            label: account.label.clone(),
            key: InputBuffer::secret(),
            submitting: false,
            error: None,
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self, Flow::None)
    }

    /// The buffer that receives keystrokes right now, if any.
    pub fn input_mut(&mut self) -> Option<&mut InputBuffer> {
        match self {
            Flow::ApiKey {
                key, submitting, ..
            } if !*submitting => Some(key),
            Flow::ChatGpt {
                progress: Progress::NeedsPaste { .. },
                paste,
                ..
            } => Some(paste),
            _ => None,
        }
    }

    pub fn set_login_id(&mut self, id: String) {
        if let Flow::ChatGpt { login_id, .. } = self {
            *login_id = Some(id);
        }
    }

    /// Applies a polled service status to a ChatGPT flow. Statuses of another login are ignored.
    pub fn apply_status(&mut self, status: &LoginStatus) -> Effect {
        let Flow::ChatGpt {
            login_id,
            progress,
            paste,
        } = self
        else {
            return Effect::None;
        };
        if let (Some(ours), Some(theirs)) = (login_id.as_ref(), status.login_id.as_ref()) {
            if ours != theirs {
                return Effect::None;
            }
        }
        if status.account.as_deref().is_some_and(|a| a != CHATGPT) {
            return Effect::None;
        }
        let next = Progress::from_status(status);
        if !matches!(next, Progress::NeedsPaste { .. }) {
            paste.clear();
        }
        let finished = matches!(next, Progress::Complete);
        *progress = next;
        if finished {
            Effect::Reload
        } else {
            Effect::None
        }
    }

    /// Starts submitting the typed OpenCode key. Returns the account and key to send, once.
    pub fn submit_key(&mut self) -> Option<(String, String)> {
        let Flow::ApiKey {
            account,
            key,
            submitting,
            error,
            ..
        } = self
        else {
            return None;
        };
        if *submitting {
            return None;
        }
        match key.take() {
            Some(value) => {
                *submitting = true;
                *error = None;
                Some((account.clone(), value))
            }
            None => {
                *error = Some("Enter an OpenCode API key.".into());
                None
            }
        }
    }

    /// The service rejected the key. The editor stays open, empty, with the reason shown.
    pub fn key_rejected(&mut self, message: String) {
        if let Flow::ApiKey {
            submitting, error, ..
        } = self
        {
            *submitting = false;
            *error = Some(message);
        }
    }

    /// Takes the pasted redirect to send to the service.
    pub fn submit_paste(&mut self) -> Option<(String, String)> {
        let Flow::ChatGpt {
            login_id: Some(id),
            progress: Progress::NeedsPaste { .. },
            paste,
        } = self
        else {
            return None;
        };
        paste.take().map(|value| (id.clone(), value))
    }

    /// The login id to cancel on dismissal, when a ChatGPT login is still waiting.
    pub fn cancel_target(&self) -> Option<String> {
        match self {
            Flow::ChatGpt {
                login_id: Some(id),
                progress,
                ..
            } if !progress.is_finished() => Some(id.clone()),
            _ => None,
        }
    }
}

/// True when no account can carry a request, so the picker shows onboarding and the glyph asks for setup.
pub fn needs_setup(catalog: &ModelCatalog) -> bool {
    !catalog
        .accounts
        .iter()
        .any(|account| account.state == "available")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountAction {
    SignIn,
    SignOut,
}

/// An account that can carry requests offers Sign out; anything else offers Sign in.
/// A login borrowed from another app is read-only here, so it offers Sign in for a Dojo login instead.
pub fn account_action(account: &Account) -> AccountAction {
    if account.state == "available" && account.source.as_deref() != Some("borrowed") {
        AccountAction::SignOut
    } else {
        AccountAction::SignIn
    }
}

/// One line describing the account's state for its row.
pub fn account_status(account: &Account) -> String {
    match (account.state.as_str(), account.source.as_deref()) {
        ("available", Some("borrowed")) => "Using another app's login (read-only).".into(),
        ("available", Some("import")) => "Signed in (imported key).".into(),
        ("available", _) => match account.backend.as_deref() {
            Some("file") => {
                "Signed in. Stored in the private fallback file (no Secret Service).".into()
            }
            Some(_) => "Signed in. Stored in the system password store.".into(),
            None => "Signed in.".into(),
        },
        _ => account
            .reason
            .clone()
            .unwrap_or_else(|| "Not signed in.".into()),
    }
}

pub fn is_chatgpt(account: &Account) -> bool {
    account.id == CHATGPT
}

pub fn is_opencode(account: &Account) -> bool {
    OPENCODE_ACCOUNTS.contains(&account.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_picker::parse_catalog;

    fn account(id: &str, state: &str, source: Option<&str>) -> Account {
        Account {
            id: id.into(),
            label: id.into(),
            state: state.into(),
            reason: None,
            source: source.map(str::to_owned),
            backend: None,
        }
    }

    fn status(json: &str) -> LoginStatus {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn secret_buffer_never_displays_its_text() {
        let mut key = InputBuffer::secret();
        assert_eq!(key.display("API key"), "API key");
        key.insert_text("sk-live-1234");
        let shown = key.display("API key");
        assert!(!shown.contains("sk-") && !shown.contains('1'));
        assert_eq!(shown.chars().count(), 12);
    }

    #[test]
    fn buffer_handles_select_all_paste_and_limits() {
        let mut buffer = InputBuffer::plain();
        buffer.insert_text("line one\nline two\x07");
        assert_eq!(buffer.take().unwrap(), "line one line two");
        buffer.insert_text("old");
        buffer.select_all();
        buffer.insert_text("new");
        assert_eq!(buffer.take().unwrap(), "new");
        let mut key = InputBuffer::secret();
        key.insert(&"x".repeat(MAX_KEY_BYTES + 10));
        assert_eq!(key.take().unwrap().len(), MAX_KEY_BYTES);
        let mut backspaced = InputBuffer::plain();
        backspaced.insert("ab");
        backspaced.backspace();
        assert_eq!(backspaced.take().unwrap(), "a");
        backspaced.insert("zz");
        backspaced.select_all();
        backspaced.backspace();
        assert!(backspaced.take().is_none());
    }

    #[test]
    fn blank_input_is_not_sendable() {
        let mut buffer = InputBuffer::secret();
        buffer.insert("   ");
        assert!(buffer.take().is_none());
    }

    #[test]
    fn statuses_map_to_progress() {
        let device = status(
            r#"{"status":"device_code","active":true,"loginId":"a","userCode":"WXYZ-1","verificationUri":"https://auth.example/device"}"#,
        );
        assert_eq!(
            Progress::from_status(&device),
            Progress::DeviceCode {
                code: "WXYZ-1".into(),
                address: "https://auth.example/device".into()
            }
        );
        assert_eq!(
            Progress::from_status(&status(r#"{"status":"device_code"}"#)),
            Progress::Starting
        );
        assert_eq!(
            Progress::from_status(&status(r#"{"status":"prompt","message":"Paste redirect"}"#)),
            Progress::NeedsPaste {
                message: "Paste redirect".into()
            }
        );
        assert_eq!(
            Progress::from_status(&status(
                r#"{"status":"failure","message":"Login cancelled."}"#
            )),
            Progress::Failed("Login cancelled.".into())
        );
        assert_eq!(
            Progress::from_status(&status(r#"{"status":"idle"}"#)),
            Progress::Starting
        );
    }

    #[test]
    fn chatgpt_flow_follows_the_service_and_reloads_on_completion() {
        let mut flow = Flow::begin_chatgpt();
        flow.set_login_id("a".into());
        assert_eq!(
            flow.apply_status(&status(
                r#"{"status":"device_code","loginId":"a","userCode":"C","verificationUri":"https://x"}"#
            )),
            Effect::None
        );
        assert!(matches!(
            flow,
            Flow::ChatGpt {
                progress: Progress::DeviceCode { .. },
                ..
            }
        ));
        // Another login's status is ignored.
        assert_eq!(
            flow.apply_status(&status(r#"{"status":"complete","loginId":"b"}"#)),
            Effect::None
        );
        assert_eq!(flow.cancel_target().as_deref(), Some("a"));
        assert_eq!(
            flow.apply_status(&status(r#"{"status":"complete","loginId":"a"}"#)),
            Effect::Reload
        );
        assert_eq!(flow.cancel_target(), None);
    }

    #[test]
    fn paste_field_accepts_input_only_when_the_service_asks() {
        let mut flow = Flow::begin_chatgpt();
        flow.set_login_id("a".into());
        assert!(flow.input_mut().is_none());
        flow.apply_status(&status(
            r#"{"status":"prompt","loginId":"a","message":"Paste the redirect URL"}"#,
        ));
        flow.input_mut()
            .unwrap()
            .insert_text("http://localhost:1455/auth/callback?code=abc");
        assert_eq!(
            flow.submit_paste(),
            Some((
                "a".into(),
                "http://localhost:1455/auth/callback?code=abc".into()
            ))
        );
        // Submitting leaves the field empty.
        assert_eq!(flow.submit_paste(), None);
    }

    #[test]
    fn key_is_cleared_on_submit_and_rejection_keeps_the_editor_open() {
        let mut flow = Flow::begin_api_key(&account("opencode-go", "unavailable", None));
        flow.input_mut().unwrap().insert_text("  sk-test  ");
        assert_eq!(
            flow.submit_key(),
            Some(("opencode-go".into(), "sk-test".into()))
        );
        // While validating, the editor takes no input and cannot submit twice.
        assert!(flow.input_mut().is_none());
        assert_eq!(flow.submit_key(), None);
        flow.key_rejected("OpenCode rejected this key.".into());
        let Flow::ApiKey {
            key,
            submitting,
            error,
            ..
        } = &mut flow
        else {
            panic!("editor was dismissed")
        };
        assert!(!*submitting);
        assert!(key.is_empty());
        assert_eq!(error.as_deref(), Some("OpenCode rejected this key."));
        // Submitting an empty editor sends nothing and explains why.
        assert_eq!(flow.submit_key(), None);
        let Flow::ApiKey { error, .. } = &flow else {
            panic!()
        };
        assert_eq!(error.as_deref(), Some("Enter an OpenCode API key."));
    }

    #[test]
    fn setup_is_needed_only_when_no_account_is_available() {
        let none = parse_catalog(
            r#"{"accounts":[{"id":"openai-codex","label":"ChatGPT","state":"unavailable"}],"models":[],"selection":null}"#,
        )
        .unwrap();
        assert!(needs_setup(&none));
        let some = parse_catalog(
            r#"{"accounts":[{"id":"opencode-go","label":"OpenCode Go","state":"available","source":"dojo","backend":"keyring"},{"id":"openai-codex","label":"ChatGPT","state":"unavailable"}],"models":[],"selection":null}"#,
        )
        .unwrap();
        assert!(!needs_setup(&some));
    }

    #[test]
    fn account_rows_offer_sign_in_or_out_and_describe_their_state() {
        let own = account("opencode-go", "available", Some("dojo"));
        assert_eq!(account_action(&own), AccountAction::SignOut);
        let borrowed = account("openai-codex", "available", Some("borrowed"));
        assert_eq!(account_action(&borrowed), AccountAction::SignIn);
        assert!(account_status(&borrowed).contains("read-only"));
        let mut renewal = account("openai-codex", "unavailable", None);
        renewal.reason = Some("Needs renewal.".into());
        assert_eq!(account_action(&renewal), AccountAction::SignIn);
        assert_eq!(account_status(&renewal), "Needs renewal.");
        assert_eq!(
            account_status(&account("opencode", "unavailable", None)),
            "Not signed in."
        );
        let mut keyring = account("opencode-go", "available", Some("dojo"));
        keyring.backend = Some("keyring".into());
        assert!(account_status(&keyring).contains("system password store"));
        keyring.backend = Some("file".into());
        assert!(account_status(&keyring).contains("fallback file"));
    }
}
