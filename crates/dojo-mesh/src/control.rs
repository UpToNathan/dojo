//! The Tailscale control plane: minting enrolment keys and reading, checking and replacing the
//! policy. Operator-only in use (see `control_cli`); this module only knows how to ask.
//!
//! Credentials come from secret references, are held in memory for the one command, and travel to
//! `curl` on its standard input so they never appear in a process listing.

use crate::config::{ControlPlane, Credential, SecretRef};
use std::cell::OnceCell;
use std::io::Write;
use std::process::{Command, Stdio};

const API: &str = "https://api.tailscale.com/api/v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

pub trait Http {
    fn send(&self, request: &Request) -> Result<Response, String>;
}

pub trait Secrets {
    fn read(&self, reference: &SecretRef) -> Result<String, String>;
}

// ---- the API client ---------------------------------------------------------------------------

pub struct Client<'a> {
    http: &'a dyn Http,
    secrets: &'a dyn Secrets,
    plane: &'a ControlPlane,
    token: OnceCell<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NewKey {
    pub key: String,
    pub expires: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Policy {
    pub text: String,
    pub etag: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Validation {
    Valid,
    Invalid(String),
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The message in an API error body, or the body cut short.
fn explain(response: &Response) -> String {
    let message = serde_json::from_str::<serde_json::Value>(&response.body)
        .ok()
        .and_then(|value| value.get("message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| response.body.trim().chars().take(300).collect());
    format!("Tailscale answered {}: {}", response.status, if message.is_empty() { "no message" } else { &message })
}

impl<'a> Client<'a> {
    pub fn new(http: &'a dyn Http, secrets: &'a dyn Secrets, plane: &'a ControlPlane) -> Self {
        Self { http, secrets, plane, token: OnceCell::new() }
    }

    fn token(&self) -> Result<&str, String> {
        if let Some(token) = self.token.get() {
            return Ok(token);
        }
        let token = match &self.plane.credential {
            Credential::ApiToken(reference) => self.secrets.read(reference)?,
            Credential::OauthClient { client_id, client_secret } => {
                let id = self.secrets.read(client_id)?;
                let secret = self.secrets.read(client_secret)?;
                let response = self.http.send(&Request {
                    method: Method::Post,
                    url: format!("{API}/oauth/token"),
                    headers: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())],
                    body: Some(format!("client_id={}&client_secret={}&grant_type=client_credentials", encode(&id), encode(&secret))),
                })?;
                if response.status != 200 {
                    return Err(format!("could not sign in with the OAuth client: {}", explain(&response)));
                }
                serde_json::from_str::<serde_json::Value>(&response.body)
                    .ok()
                    .and_then(|value| value.get("access_token").and_then(|t| t.as_str()).map(str::to_string))
                    .ok_or("the OAuth sign-in answered without an access token")?
            }
        };
        if token.trim().is_empty() {
            return Err("the credential is empty".into());
        }
        Ok(self.token.get_or_init(|| token.trim().to_string()))
    }

    fn send(&self, method: Method, path: &str, mut headers: Vec<(String, String)>, body: Option<String>) -> Result<Response, String> {
        headers.push(("Authorization".into(), format!("Bearer {}", self.token()?)));
        self.http.send(&Request { method, url: format!("{API}/tailnet/{}/{path}", encode(&self.plane.tailnet)), headers, body })
    }

    /// A one-use, pre-approved key that enrols a device carrying `tags`.
    pub fn create_auth_key(&self, tags: &[String], expiry_seconds: u64) -> Result<NewKey, String> {
        let body = serde_json::json!({
            "keyType": "auth",
            "description": "dojo mesh enrolment",
            "expirySeconds": expiry_seconds,
            "capabilities": {"devices": {"create": {"reusable": false, "ephemeral": false, "preauthorized": true, "tags": tags}}},
        });
        let response = self.send(Method::Post, "keys", vec![("Content-Type".into(), "application/json".into())], Some(body.to_string()))?;
        if !(200..300).contains(&response.status) {
            return Err(explain(&response));
        }
        let value: serde_json::Value = serde_json::from_str(&response.body).map_err(|error| format!("unreadable key response: {error}"))?;
        Ok(NewKey {
            key: value.get("key").and_then(|k| k.as_str()).ok_or("the response held no key")?.to_string(),
            expires: value.get("expires").and_then(|e| e.as_str()).map(str::to_string),
        })
    }

    /// The current policy as written (comments included), and the tag that guards replacing it.
    pub fn policy(&self) -> Result<Policy, String> {
        let response = self.send(Method::Get, "acl", vec![("Accept".into(), "application/hujson".into())], None)?;
        if response.status != 200 {
            return Err(explain(&response));
        }
        Ok(Policy { etag: response.header("ETag").map(str::to_string), text: response.body })
    }

    /// Checks a candidate policy (plain JSON) and its tests without changing anything.
    pub fn validate(&self, json: &str) -> Result<Validation, String> {
        let response = self.send(Method::Post, "acl/validate", vec![("Content-Type".into(), "application/json".into())], Some(json.to_string()))?;
        match response.status {
            200 => {
                let value: serde_json::Value = serde_json::from_str(&response.body).unwrap_or(serde_json::Value::Null);
                match value.get("message") {
                    None => Ok(Validation::Valid),
                    Some(_) => Ok(Validation::Invalid(describe_failure(&value))),
                }
            }
            400 => {
                let detail = describe_failure(&serde_json::from_str(&response.body).unwrap_or(serde_json::Value::Null));
                Ok(Validation::Invalid(if detail.is_empty() { explain(&response) } else { detail }))
            }
            _ => Err(explain(&response)),
        }
    }

    /// Replaces the policy, only if nobody changed it since `etag` was read.
    pub fn set_policy(&self, hujson: &str, etag: &str) -> Result<(), String> {
        let response = self.send(
            Method::Post,
            "acl",
            vec![("Content-Type".into(), "application/hujson".into()), ("If-Match".into(), etag.to_string()), ("Accept".into(), "application/json".into())],
            Some(hujson.to_string()),
        )?;
        match response.status {
            200 => Ok(()),
            412 => Err("the policy changed since it was read; nothing was written, run the command again".into()),
            _ => Err(explain(&response)),
        }
    }
}

/// A validation failure's message and any per-test errors, on separate lines.
fn describe_failure(value: &serde_json::Value) -> String {
    let mut lines = Vec::new();
    if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
        lines.push(message.to_string());
    }
    if let Some(items) = value.get("data").and_then(|d| d.as_array()) {
        for item in items {
            let who = item.get("user").and_then(|u| u.as_str()).unwrap_or("policy");
            for error in item.get("errors").and_then(|e| e.as_array()).into_iter().flatten().filter_map(|e| e.as_str()) {
                lines.push(format!("  {who}: {error}"));
            }
        }
    }
    lines.join("\n")
}

// ---- the real world: curl and secret stores ---------------------------------------------------

/// Quotes a value for a curl config file.
pub fn config_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The curl configuration for a request; it carries the credential, so it only ever goes to curl's stdin.
pub fn curl_config(request: &Request, body_file: Option<&std::path::Path>) -> String {
    let mut config = String::from("silent\nshow-error\ninclude\nmax-time = 60\nproto = \"=https\"\n");
    config.push_str(&format!("url = {}\n", config_quote(&request.url)));
    config.push_str(&format!("request = {}\n", config_quote(if request.method == Method::Post { "POST" } else { "GET" })));
    for (name, value) in &request.headers {
        config.push_str(&format!("header = {}\n", config_quote(&format!("{name}: {value}"))));
    }
    if let Some(path) = body_file {
        config.push_str(&format!("data-binary = {}\n", config_quote(&format!("@{}", path.display()))));
    }
    config
}

/// Splits `curl --include` output into status, headers and body, skipping any interim 1xx blocks.
pub fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let text = String::from_utf8_lossy(raw);
    let mut rest: &str = &text;
    loop {
        let (head, after) = rest.split_once("\r\n\r\n").ok_or("curl gave no HTTP response")?;
        let mut lines = head.lines();
        let status_line = lines.next().unwrap_or_default();
        let status: u16 = status_line.split_whitespace().nth(1).and_then(|code| code.parse().ok()).ok_or_else(|| format!("unreadable HTTP status line {status_line:?}"))?;
        if (100..200).contains(&status) {
            rest = after;
            continue;
        }
        let headers = lines.filter_map(|line| line.split_once(':')).map(|(name, value)| (name.trim().to_string(), value.trim().to_string())).collect();
        return Ok(Response { status, headers, body: after.to_string() });
    }
}

pub struct Curl;

impl Http for Curl {
    fn send(&self, request: &Request) -> Result<Response, String> {
        // The body goes through a private file: the configuration is on stdin, and a policy can
        // be larger than a configuration line is allowed to be.
        let body_file = match &request.body {
            Some(body) => Some(PrivateFile::create(body.as_bytes())?),
            None => None,
        };
        let config = curl_config(request, body_file.as_ref().map(|file| file.path.as_path()));
        let mut child = Command::new("curl")
            .args(["--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { "curl is not installed".to_string() } else { format!("could not run curl: {error}") })?;
        child.stdin.take().expect("stdin was piped").write_all(config.as_bytes()).map_err(|error| format!("could not hand curl its configuration: {error}"))?;
        let output = child.wait_with_output().map_err(|error| format!("curl failed: {error}"))?;
        if !output.status.success() {
            return Err(format!("curl failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        parse_response(&output.stdout)
    }
}

/// A mode-0600 file holding a request body, removed when dropped.
struct PrivateFile {
    path: std::path::PathBuf,
}

impl PrivateFile {
    fn create(contents: &[u8]) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt;
        let directory = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).filter(|dir| dir.is_dir()).unwrap_or_else(std::env::temp_dir);
        let mut suffix = [0u8; 8];
        if let Ok(mut random) = std::fs::File::open("/dev/urandom") {
            let _ = std::io::Read::read_exact(&mut random, &mut suffix);
        }
        let name: String = suffix.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = directory.join(format!("dojo-mesh-{}-{name}", std::process::id()));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|error| format!("could not stage the request: {error}"))?;
        file.write_all(contents).map_err(|error| format!("could not stage the request: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for PrivateFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct SystemSecrets;

impl Secrets for SystemSecrets {
    fn read(&self, reference: &SecretRef) -> Result<String, String> {
        let value = match reference {
            SecretRef::Env(name) => std::env::var(name).map_err(|_| format!("the environment variable {name} is not set"))?,
            SecretRef::File(path) => std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?,
            SecretRef::Op(uri) => {
                let output = Command::new("op").args(["read", "--no-newline", uri]).stdin(Stdio::null()).output().map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound { "the 1Password CLI (op) is not installed".to_string() } else { format!("could not run op: {error}") }
                })?;
                if !output.status.success() {
                    return Err(format!("op could not read {uri}: {}", String::from_utf8_lossy(&output.stderr).trim()));
                }
                String::from_utf8_lossy(&output.stdout).into_owned()
            }
        };
        Ok(value.trim_end_matches(['\n', '\r']).to_string())
    }
}

/// Stand-ins for the network and the secret stores, shared by the tests of this module and `control_cli`.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub struct FakeHttp {
        pub sent: RefCell<Vec<Request>>,
        pub replies: RefCell<Vec<Response>>,
    }

    impl FakeHttp {
        pub fn new(replies: Vec<Response>) -> Self {
            Self { sent: RefCell::new(Vec::new()), replies: RefCell::new(replies.into_iter().rev().collect()) }
        }
    }

    impl Http for FakeHttp {
        fn send(&self, request: &Request) -> Result<Response, String> {
            self.sent.borrow_mut().push(request.clone());
            self.replies.borrow_mut().pop().ok_or_else(|| "no reply left".to_string())
        }
    }

    pub struct FakeSecrets(pub HashMap<SecretRef, String>);

    impl Secrets for FakeSecrets {
        fn read(&self, reference: &SecretRef) -> Result<String, String> {
            self.0.get(reference).cloned().ok_or_else(|| format!("no secret {reference:?}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::testing::{FakeHttp, FakeSecrets};
    use std::collections::HashMap;

    fn reply(status: u16, body: &str) -> Response {
        Response { status, headers: vec![("ETag".into(), "\"abc\"".into())], body: body.into() }
    }

    fn token_plane() -> (ControlPlane, FakeSecrets) {
        let reference = SecretRef::Env("TOKEN".into());
        (ControlPlane { tailnet: "example.com".into(), credential: Credential::ApiToken(reference.clone()) }, FakeSecrets(HashMap::from([(reference, "tskey-api-secret\n".to_string())])))
    }

    #[test]
    fn a_token_credential_is_sent_as_a_bearer_header_to_the_named_tailnet() {
        let (plane, secrets) = token_plane();
        let http = FakeHttp::new(vec![reply(200, "{\"key\": \"tskey-auth-new\", \"expires\": \"2026-02-01T00:00:00Z\"}")]);
        let key = Client::new(&http, &secrets, &plane).create_auth_key(&["tag:mesh".into()], 3600).unwrap();
        assert_eq!(key, NewKey { key: "tskey-auth-new".into(), expires: Some("2026-02-01T00:00:00Z".into()) });
        let sent = http.sent.borrow();
        assert_eq!(sent[0].url, "https://api.tailscale.com/api/v2/tailnet/example.com/keys");
        assert!(sent[0].headers.contains(&("Authorization".to_string(), "Bearer tskey-api-secret".to_string())));
        let body: serde_json::Value = serde_json::from_str(sent[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["capabilities"]["devices"]["create"]["tags"], serde_json::json!(["tag:mesh"]));
        assert_eq!(body["capabilities"]["devices"]["create"]["reusable"], false);
        assert_eq!(body["capabilities"]["devices"]["create"]["preauthorized"], true);
        assert_eq!(body["expirySeconds"], 3600);
    }

    #[test]
    fn an_oauth_client_is_exchanged_for_a_token_once() {
        let id = SecretRef::Env("ID".into());
        let secret = SecretRef::Env("SECRET".into());
        let plane = ControlPlane { tailnet: "-".into(), credential: Credential::OauthClient { client_id: id.clone(), client_secret: secret.clone() } };
        let secrets = FakeSecrets(HashMap::from([(id, "id with space".to_string()), (secret, "s&cret=".to_string())]));
        let http = FakeHttp::new(vec![reply(200, "{\"access_token\": \"minted\"}"), reply(200, "{}"), reply(200, "{}")]);
        let client = Client::new(&http, &secrets, &plane);
        assert_eq!(client.validate("{}").unwrap(), Validation::Valid);
        assert_eq!(client.validate("{}").unwrap(), Validation::Valid);
        let sent = http.sent.borrow();
        assert_eq!(sent.len(), 3, "one sign-in, then the two calls");
        assert_eq!(sent[0].url, "https://api.tailscale.com/api/v2/oauth/token");
        assert_eq!(sent[0].body.as_deref(), Some("client_id=id%20with%20space&client_secret=s%26cret%3D&grant_type=client_credentials"));
        assert!(sent[1].headers.contains(&("Authorization".to_string(), "Bearer minted".to_string())));
        assert!(sent[1].url.contains("/tailnet/-/acl/validate"));
    }

    #[test]
    fn a_rejected_sign_in_or_missing_secret_stops_before_any_call() {
        let (plane, _) = token_plane();
        let http = FakeHttp::new(vec![]);
        let empty = FakeSecrets(HashMap::new());
        assert!(Client::new(&http, &empty, &plane).policy().unwrap_err().contains("no secret"));
        assert!(http.sent.borrow().is_empty());

        let id = SecretRef::Env("ID".into());
        let oauth = ControlPlane { tailnet: "-".into(), credential: Credential::OauthClient { client_id: id.clone(), client_secret: id.clone() } };
        let secrets = FakeSecrets(HashMap::from([(id, "x".to_string())]));
        let http = FakeHttp::new(vec![reply(401, "{\"message\": \"invalid client\"}")]);
        let error = Client::new(&http, &secrets, &oauth).policy().unwrap_err();
        assert!(error.contains("401") && error.contains("invalid client"), "{error}");
        assert_eq!(http.sent.borrow().len(), 1);
    }

    #[test]
    fn reads_the_policy_with_its_etag() {
        let (plane, secrets) = token_plane();
        let http = FakeHttp::new(vec![reply(200, "{ // comment\n}")]);
        let policy = Client::new(&http, &secrets, &plane).policy().unwrap();
        assert_eq!(policy, Policy { text: "{ // comment\n}".into(), etag: Some("\"abc\"".into()) });
        assert!(http.sent.borrow()[0].headers.contains(&("Accept".to_string(), "application/hujson".to_string())));
    }

    #[test]
    fn validation_reports_failures_and_failed_tests() {
        let (plane, secrets) = token_plane();
        let failed_tests = "{\"message\": \"test(s) failed\", \"data\": [{\"user\": \"a@example.com\", \"errors\": [\"want deny, got accept\"]}]}";
        let http = FakeHttp::new(vec![reply(200, "{}"), reply(200, failed_tests), reply(400, "{\"message\": \"bad syntax\"}"), reply(500, "oops")]);
        let client = Client::new(&http, &secrets, &plane);
        assert_eq!(client.validate("{}").unwrap(), Validation::Valid);
        assert_eq!(client.validate("{}").unwrap(), Validation::Invalid("test(s) failed\n  a@example.com: want deny, got accept".into()));
        assert_eq!(client.validate("{}").unwrap(), Validation::Invalid("bad syntax".into()));
        assert!(client.validate("{}").unwrap_err().contains("500"));
    }

    #[test]
    fn replacing_the_policy_is_guarded_by_the_etag() {
        let (plane, secrets) = token_plane();
        let http = FakeHttp::new(vec![reply(200, "{}"), reply(412, "{\"message\": \"precondition failed\"}")]);
        let client = Client::new(&http, &secrets, &plane);
        client.set_policy("{ /* new */ }", "\"abc\"").unwrap();
        let error = client.set_policy("{}", "\"stale\"").unwrap_err();
        assert!(error.contains("changed since it was read"), "{error}");
        let sent = http.sent.borrow();
        assert!(sent[0].headers.contains(&("If-Match".to_string(), "\"abc\"".to_string())));
        assert!(sent[0].headers.contains(&("Content-Type".to_string(), "application/hujson".to_string())));
        assert_eq!(sent[0].body.as_deref(), Some("{ /* new */ }"));
    }

    #[test]
    fn the_curl_configuration_escapes_values_and_keeps_the_body_out_of_it() {
        let request = Request {
            method: Method::Post,
            url: "https://api.tailscale.com/api/v2/x".into(),
            headers: vec![("Authorization".into(), "Bearer a\"b\\c".into())],
            body: Some("secret body".into()),
        };
        let config = curl_config(&request, Some(std::path::Path::new("/run/user/1/body")));
        assert!(config.contains("header = \"Authorization: Bearer a\\\"b\\\\c\"\n"), "{config}");
        assert!(config.contains("data-binary = \"@/run/user/1/body\"\n"));
        assert!(!config.contains("secret body"));
        assert!(config.contains("proto = \"=https\""));
        assert_eq!(config_quote("a\nb\tc"), "\"a\\nb\\tc\"");
    }

    #[test]
    fn parses_curl_output_skipping_interim_responses() {
        let raw = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/2 200 \r\nContent-Type: application/json\r\nETag: \"e1\"\r\n\r\n{\"a\": 1}\r\n\r\nstill body";
        let response = parse_response(raw).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.header("etag"), Some("\"e1\""));
        assert_eq!(response.body, "{\"a\": 1}\r\n\r\nstill body");
        assert!(parse_response(b"nonsense").is_err());
    }
}
