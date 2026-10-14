//! Bounded, opt-in recording projections. Pi owns canonical intents and conversations.
//! CUA-derived trajectory/reward formats; see provenance.json and LICENSE.md.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub mod benchmark;
pub mod general_metrics;
pub mod processors;
pub mod usage;

pub const TRACE_SCHEMA: &str = "dojo.computer.trace/1";
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const REDACTED: &str = "[redacted]";
const OMITTED: &str = "[omitted]";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("recording limit exceeded: {0}")]
    Limit(&'static str),
    #[error("invalid recording: {0}")]
    Invalid(&'static str),
    #[error("invalid JSON")]
    Json(#[from] serde_json::Error),
    #[error("transcript: {0}")]
    Transcript(#[from] cua_spaces_transcript::TranscriptError),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingPolicy {
    pub max_events: usize,
    pub max_event_bytes: usize,
    pub max_total_bytes: usize,
    /// Image artifacts require explicit capture consent from the session owner.
    pub retain_images: bool,
    pub max_images: usize,
    pub max_image_bytes: usize,
    pub max_image_pixels: u64,
    /// Values supplied by the caller are removed from every string and map key.
    #[serde(default, skip_serializing)]
    pub secret_values: Vec<String>,
}
impl Default for RecordingPolicy {
    fn default() -> Self {
        Self {
            max_events: 1000,
            max_event_bytes: 64 * 1024,
            max_total_bytes: MAX_INPUT_BYTES,
            retain_images: false,
            max_images: 8,
            max_image_bytes: 1024 * 1024,
            max_image_pixels: 16_777_216,
            secret_values: vec![],
        }
    }
}
impl RecordingPolicy {
    fn validate(&self) -> Result<(), Error> {
        if self.max_events > 10_000
            || self.max_event_bytes > MAX_INPUT_BYTES
            || self.max_total_bytes > MAX_INPUT_BYTES
            || self.max_image_bytes > MAX_INPUT_BYTES
            || self.max_images > 100
            || self.max_image_pixels > 33_554_432
        {
            return Err(Error::Limit("policy exceeds hard limits"));
        }
        if self.secret_values.len() > 256
            || self
                .secret_values
                .iter()
                .any(|v| v.is_empty() || v.len() > 8192)
        {
            return Err(Error::Invalid(
                "secret filters must be nonempty and bounded",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Intent,
    Receipt,
    Observation,
    Verification,
    RunEnd,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Confirmed,
    Partial,
    Unknown,
    SuspectedNoop,
    Refused,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceEvent {
    pub time_ms: u64,
    pub kind: EventKind,
    pub pi_intent_id: Option<String>,
    pub call_id: Option<String>,
    pub route: Option<String>,
    pub observation_id: Option<String>,
    pub effect: Option<Effect>,
    #[serde(default)]
    pub data: Value,
}
impl TraceEvent {
    fn validate(&self) -> Result<(), Error> {
        if self.kind == EventKind::Intent || self.kind == EventKind::Receipt {
            if self.pi_intent_id.as_deref().is_none_or(str::is_empty)
                || self.call_id.as_deref().is_none_or(str::is_empty)
            {
                return Err(Error::Invalid(
                    "intent/receipt requires Pi intent and call IDs",
                ));
            }
        }
        if (self.kind == EventKind::Receipt) != self.effect.is_some() {
            return Err(Error::Invalid(
                "only receipts require an effect classification",
            ));
        }
        for value in [
            &self.pi_intent_id,
            &self.call_id,
            &self.route,
            &self.observation_id,
        ]
        .into_iter()
        .flatten()
        {
            if value.len() > 256 || value.chars().any(char::is_control) {
                return Err(Error::Invalid(
                    "event identifiers must be bounded printable strings",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageArtifact {
    pub sha256: String,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes_base64: String,
}
/// Fields are private: persisted traces can only originate from the redacting recorder.
#[derive(Clone, Debug, Serialize)]
pub struct Trace {
    schema_version: String,
    session_id: String,
    events: Vec<TraceEvent>,
    images: Vec<ImageArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<usage::UsageTotals>,
}
impl Trace {
    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }
    pub fn images(&self) -> &[ImageArtifact] {
        &self.images
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn usage(&self) -> Option<&usage::UsageTotals> {
        self.usage.as_ref()
    }
}

pub struct Recorder {
    policy: RecordingPolicy,
    trace: Trace,
    total_bytes: usize,
}
impl Recorder {
    pub fn new(session_id: &str, policy: RecordingPolicy) -> Result<Self, Error> {
        policy.validate()?;
        if session_id.is_empty()
            || session_id.len() > 256
            || session_id.chars().any(char::is_control)
        {
            return Err(Error::Invalid("invalid session ID"));
        }
        let session_id = scrub_string(session_id, &policy.secret_values);
        let trace = Trace {
            schema_version: TRACE_SCHEMA.into(),
            session_id,
            events: vec![],
            images: vec![],
            usage: None,
        };
        let total_bytes = serde_json::to_vec(&trace)?.len();
        if total_bytes > policy.max_total_bytes {
            return Err(Error::Limit("total bytes"));
        }
        Ok(Self {
            policy,
            trace,
            total_bytes,
        })
    }
    pub fn push(&mut self, event: TraceEvent) -> Result<(), Error> {
        event.validate()?;
        if self.trace.events.len() >= self.policy.max_events {
            return Err(Error::Limit("event count"));
        }
        if self
            .trace
            .events
            .last()
            .is_some_and(|last| last.time_ms > event.time_ms)
        {
            return Err(Error::Invalid("event timestamps are not monotonic"));
        }
        if serde_json::to_vec(&event)?.len() > self.policy.max_event_bytes {
            return Err(Error::Limit("event bytes"));
        }
        let value = redact_value(&serde_json::to_value(event)?, &self.policy.secret_values)?;
        let event: TraceEvent = serde_json::from_value(value)?;
        let bytes = serde_json::to_vec(&event)?.len() + 1;
        self.reserve(bytes)?;
        self.trace.events.push(event);
        self.total_bytes += bytes;
        Ok(())
    }
    fn reserve(&self, bytes: usize) -> Result<(), Error> {
        if bytes > self.policy.max_total_bytes.saturating_sub(self.total_bytes) {
            Err(Error::Limit("total bytes"))
        } else {
            Ok(())
        }
    }
    /// Only in-memory PNG bytes; no paths, network fetches, call-ID filenames or decoding.
    /// Capture consent is explicit. Visual secrets cannot be filtered by text redaction.
    pub fn add_png(
        &mut self,
        bytes: &[u8],
        capture_consent: bool,
    ) -> Result<Option<String>, Error> {
        if !self.policy.retain_images {
            return Ok(None);
        }
        if !capture_consent {
            return Err(Error::Invalid("image capture consent required"));
        }
        if bytes.len() > self.policy.max_image_bytes {
            return Err(Error::Limit("image bytes"));
        }
        let (width, height) = validate_png(bytes, self.policy.max_image_pixels)?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        if self.trace.images.iter().any(|a| a.sha256 == digest) {
            return Ok(Some(digest));
        }
        if self.trace.images.len() >= self.policy.max_images {
            return Err(Error::Limit("image count"));
        }
        use base64::Engine;
        let artifact = ImageArtifact {
            sha256: digest.clone(),
            media_type: "image/png".into(),
            width,
            height,
            bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        let size = serde_json::to_vec(&artifact)?.len() + 1;
        self.reserve(size)?;
        self.trace.images.push(artifact);
        self.total_bytes += size;
        Ok(Some(digest))
    }
    pub fn set_usage(&mut self, samples: &[usage::UsageSample]) -> Result<(), Error> {
        self.set_usage_totals(usage::aggregate(samples)?)
    }
    fn set_usage_totals(&mut self, totals: usage::UsageTotals) -> Result<(), Error> {
        totals.validate()?;
        if self.trace.usage.is_some() {
            return Err(Error::Invalid("usage already supplied"));
        }
        let size = serde_json::to_vec(&totals)?.len() + 9;
        self.reserve(size)?;
        self.trace.usage = Some(totals);
        self.total_bytes += size;
        Ok(())
    }
    pub fn finish(self) -> Trace {
        self.trace
    }
}

fn sensitive_key(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "authorization",
        "cookie",
        "apikey",
        "credential",
        "clipboard",
        "imageurl",
        "screenshot",
        "bytesbase64",
        "dataimages",
        "text",
        "content",
        "raw",
        "outputtail",
        "html",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}
fn scrub_string(value: &str, secrets: &[String]) -> String {
    let mut result = value.to_string();
    // Longest first so overlapping secrets cannot leave a suffix behind.
    let mut secrets: Vec<_> = secrets.iter().collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for secret in secrets {
        if !secret.is_empty() {
            result = result.replace(secret, REDACTED);
        }
    }
    let lower = result.to_ascii_lowercase();
    if lower.contains("bearer ")
        || lower.contains("basic ")
        || lower.contains("sk-")
        || lower.contains("password=")
        || lower.contains("token=")
        || lower.starts_with("data:image/")
    {
        return REDACTED.into();
    }
    result
}
/// Conservative field filtering. Callers must still deny sensitive capture scopes.
pub fn redact_value(value: &Value, secrets: &[String]) -> Result<Value, Error> {
    fn visit(value: &Value, secrets: &[String], depth: usize) -> Result<Value, Error> {
        if depth > 32 {
            return Err(Error::Limit("JSON depth"));
        }
        Ok(match value {
            Value::String(text) => Value::String(scrub_string(text, secrets)),
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|v| visit(v, secrets, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(values) => {
                let mut out = serde_json::Map::new();
                for (key, value) in values {
                    let sanitized = scrub_string(key, secrets);
                    if out.contains_key(&sanitized) {
                        return Err(Error::Invalid("redacted map key collision"));
                    }
                    out.insert(sanitized, if sensitive_key(key) {
                        Value::String(if key.to_ascii_lowercase().contains("image") || key == "screenshot" { OMITTED } else { REDACTED }.into())
                    } else { visit(value, secrets, depth + 1)? });
                }
                Value::Object(out)
            }
            _ => value.clone(),
        })
    }
    if secrets.len() > 256 || secrets.iter().any(|s| s.is_empty() || s.len() > 8192) {
        return Err(Error::Invalid("invalid secret filters"));
    }
    if serde_json::to_vec(value)?.len() > MAX_INPUT_BYTES {
        return Err(Error::Limit("JSON bytes"));
    }
    visit(value, secrets, 0)
}

/// Validate the complete PNG chunk envelope before retaining opaque image bytes.
fn validate_png(bytes: &[u8], max_pixels: u64) -> Result<(u32, u32), Error> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(Error::Invalid("PNG signature"));
    }
    let mut offset = 8usize;
    let mut dimensions = None;
    let mut saw_data = false;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or(Error::Invalid("PNG chunk header"))?;
        let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(len)
            .and_then(|n| n.checked_add(12))
            .ok_or(Error::Invalid("PNG chunk size"))?;
        let chunk = bytes
            .get(offset..end)
            .ok_or(Error::Invalid("truncated PNG chunk"))?;
        let expected_crc = u32::from_be_bytes(chunk[len + 8..len + 12].try_into().unwrap());
        if crc32fast::hash(&chunk[4..len + 8]) != expected_crc {
            return Err(Error::Invalid("PNG chunk checksum"));
        }
        if offset == 8 {
            if &header[4..8] != b"IHDR" || len != 13 {
                return Err(Error::Invalid("PNG IHDR"));
            }
            let width = u32::from_be_bytes(chunk[8..12].try_into().unwrap());
            let height = u32::from_be_bytes(chunk[12..16].try_into().unwrap());
            if width == 0 || height == 0 || u64::from(width) * u64::from(height) > max_pixels {
                return Err(Error::Limit("image dimensions"));
            }
            dimensions = Some((width, height));
        } else if &header[4..8] == b"IHDR" {
            return Err(Error::Invalid("duplicate PNG IHDR"));
        }
        if &header[4..8] == b"IDAT" {
            saw_data = true;
        }
        if &header[4..8] == b"IEND" {
            if len != 0 || end != bytes.len() || !saw_data {
                return Err(Error::Invalid("PNG end"));
            }
            return dimensions.ok_or(Error::Invalid("PNG dimensions"));
        }
        offset = end;
    }
    Err(Error::Invalid("PNG lacks IEND"))
}

/// Ingest untrusted persisted JSON through the same privacy and size boundary.
pub fn ingest(bytes: &[u8], policy: RecordingPolicy) -> Result<Trace, Error> {
    if bytes.len() > MAX_INPUT_BYTES || bytes.len() > policy.max_total_bytes {
        return Err(Error::Limit("trace input bytes"));
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        schema_version: String,
        session_id: String,
        events: Vec<TraceEvent>,
        #[serde(default)]
        images: Vec<ImageArtifact>,
        #[serde(default)]
        usage: Option<usage::UsageTotals>,
    }
    let input: Input = serde_json::from_slice(bytes)?;
    if input.schema_version != TRACE_SCHEMA {
        return Err(Error::Invalid("trace schema"));
    }
    let mut recorder = Recorder::new(&input.session_id, policy)?;
    if let Some(totals) = input.usage {
        recorder.set_usage_totals(totals)?;
    }
    for event in input.events {
        recorder.push(event)?;
    }
    for image in input.images {
        if !recorder.policy.retain_images {
            continue;
        }
        if image.media_type != "image/png"
            || image.bytes_base64.len() > recorder.policy.max_image_bytes.saturating_mul(4) / 3 + 4
        {
            return Err(Error::Invalid("image format/size"));
        }
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image.bytes_base64)
            .map_err(|_| Error::Invalid("image base64"))?;
        let dimensions = validate_png(&bytes, recorder.policy.max_image_pixels)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != image.sha256 || dimensions != (image.width, image.height) {
            return Err(Error::Invalid("image digest/dimensions"));
        }
        recorder.add_png(&bytes, true)?;
    }
    Ok(recorder.finish())
}

#[derive(Debug, Serialize)]
pub struct ReplayReport {
    pub events: usize,
    pub duration_ms: u64,
    pub confirmed: usize,
    pub partial: usize,
    pub unknown: usize,
    pub refused: usize,
    pub suspected_noop: usize,
    pub unresolved_intents: Vec<String>,
    pub invalid_receipts: Vec<String>,
    pub safe_to_replay_effects: bool,
}
/// Classify receipt gaps, never execute or schedule recorded input.
pub fn analyze(trace: &Trace) -> ReplayReport {
    let mut pending = BTreeSet::new();
    let mut completed = BTreeSet::new();
    let mut report = ReplayReport {
        events: trace.events.len(),
        duration_ms: trace.events.last().map_or(0, |last| {
            last.time_ms
                .saturating_sub(trace.events.first().unwrap().time_ms)
        }),
        confirmed: 0,
        partial: 0,
        unknown: 0,
        refused: 0,
        suspected_noop: 0,
        unresolved_intents: vec![],
        invalid_receipts: vec![],
        safe_to_replay_effects: false,
    };
    for event in &trace.events {
        let id = (
            event.pi_intent_id.clone().unwrap_or_default(),
            event.call_id.clone().unwrap_or_default(),
        );
        match event.kind {
            EventKind::Intent => {
                if completed.contains(&id) || !pending.insert(id.clone()) {
                    report.invalid_receipts.push(id.0);
                }
            }
            EventKind::Receipt => {
                if !pending.remove(&id) || !completed.insert(id.clone()) {
                    report.invalid_receipts.push(id.0);
                    report.unknown += 1;
                    continue;
                }
                match event.effect.unwrap() {
                    Effect::Confirmed => report.confirmed += 1,
                    Effect::Partial => report.partial += 1,
                    Effect::Unknown => report.unknown += 1,
                    Effect::Refused => report.refused += 1,
                    Effect::SuspectedNoop => report.suspected_noop += 1,
                }
            }
            _ => {}
        }
    }
    report.unresolved_intents = pending.into_iter().map(|(intent, _)| intent).collect();
    report
}

pub fn export_atif(trace: &Trace) -> Value {
    let steps: Vec<Value> = trace.events.iter().enumerate().map(|(index, event)| {
        let source = if event.kind == EventKind::Intent { "agent" } else { "system" };
        let mut step = json!({"step_id": index + 1, "timestamp": null, "source": source,
            "message": "", "extra": {"time_ms": event.time_ms, "event": event}});
        if event.kind == EventKind::Intent {
            step["tool_calls"] = json!([{"tool_call_id": event.call_id, "function_name": "computer", "arguments": event.data}]);
        } else {
            step["observation"] = json!({"results": [{"content": [{"type": "text", "text": serde_json::to_string(&event.data).unwrap()}]}]});
        }
        step
    }).collect();
    let mut output = json!({"schema_version": "ATIF-v1.8", "session_id": trace.session_id,
        "agent": {"name": "dojo", "version": env!("CARGO_PKG_VERSION")}, "steps": steps,
        "final_metrics": {"total_steps": trace.events.iter().filter(|e| e.kind == EventKind::Intent).count(),
            "extra": {"replay_analysis": analyze(trace)}}});
    if let Some(totals) = &trace.usage {
        output["final_metrics"]["extra"]["usage"] = json!(totals);
    }
    output
}

/// Scalar reward port of cua_bench.runner.episode.reward_of.
pub fn reward_of(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
        Value::Number(v) => v.as_f64().filter(|v| v.is_finite()),
        Value::Array(values) => {
            let rewards: Vec<_> = values.iter().filter_map(reward_of).collect();
            if rewards.is_empty() {
                None
            } else {
                Some(rewards.iter().map(|v| v / rewards.len() as f64).sum())
            }
        }
        _ => None,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialResult {
    pub task: String,
    pub variant: u32,
    pub reward: Option<f64>,
}
/// Harbor unbiased estimator, grouped by task+variant. Missing reward counts as failure.
pub fn pass_at_k(results: &[TrialResult]) -> Option<BTreeMap<String, f64>> {
    let mut groups: BTreeMap<(&str, u32), Vec<bool>> = BTreeMap::new();
    for result in results {
        if result.reward.is_some_and(|r| r != 0.0 && r != 1.0) {
            return None;
        }
        groups
            .entry((&result.task, result.variant))
            .or_default()
            .push(result.reward == Some(1.0));
    }
    let min = groups.values().map(Vec::len).min()?;
    if min < 2 {
        return None;
    }
    let mut ks = BTreeSet::from([1usize]);
    let mut k = 2;
    while k <= min {
        ks.insert(k);
        k = k.checked_mul(2)?;
    }
    for k in (5..=min).step_by(5) {
        ks.insert(k);
    }
    Some(
        ks.into_iter()
            .map(|k| {
                let mean = groups
                    .values()
                    .map(|group| {
                        let n = group.len();
                        let failures = group.iter().filter(|v| !**v).count();
                        if failures < k {
                            1.0
                        } else {
                            1.0 - (0..k)
                                .map(|i| (failures - i) as f64 / (n - i) as f64)
                                .product::<f64>()
                        }
                    })
                    .sum::<f64>()
                    / groups.len() as f64;
                (k.to_string(), mean)
            })
            .collect(),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Evaluator {
    Exact { expected: Value },
    Contains { expected: String },
    JsonField { pointer: String, expected: Value },
}
#[derive(Debug, Serialize)]
pub struct Evaluation {
    pub outcome: &'static str,
    pub reward: Option<f64>,
}
/// Deterministic fixture evaluation. Missing evidence is unknown, never success.
pub fn evaluate(evaluator: &Evaluator, evidence: Option<&Value>) -> Evaluation {
    let Some(evidence) = evidence else {
        return Evaluation {
            outcome: "unknown",
            reward: None,
        };
    };
    let matched = match evaluator {
        Evaluator::Exact { expected } => Some(expected == evidence),
        Evaluator::Contains { expected } => evidence.as_str().map(|value| value.contains(expected)),
        Evaluator::JsonField { pointer, expected } => {
            evidence.pointer(pointer).map(|value| value == expected)
        }
    };
    match matched {
        Some(true) => Evaluation {
            outcome: "success",
            reward: Some(1.0),
        },
        Some(false) => Evaluation {
            outcome: "failure",
            reward: Some(0.0),
        },
        None => Evaluation {
            outcome: "unknown",
            reward: None,
        },
    }
}

/// Bound and strictly validate casts before invoking the unchanged upstream renderer.
/// Result uses CUA's byte-stable frame schema, with all strings privacy-filtered.
pub fn observe_transcript(cast: &str, time_ms: u64, secrets: &[String]) -> Result<Value, Error> {
    if cast.len() > 1024 * 1024 || time_ms > i64::MAX as u64 {
        return Err(Error::Limit("transcript bytes/time"));
    }
    let mut lines = cast.lines().filter(|line| !line.is_empty());
    let header: Value =
        serde_json::from_str(lines.next().ok_or(Error::Invalid("cast header missing"))?)?;
    let width = header
        .get("width")
        .and_then(Value::as_u64)
        .ok_or(Error::Invalid("cast width"))?;
    let height = header
        .get("height")
        .and_then(Value::as_u64)
        .ok_or(Error::Invalid("cast height"))?;
    if header.get("version").and_then(Value::as_u64) != Some(2)
        || width == 0
        || height == 0
        || width > 512
        || height > 256
    {
        return Err(Error::Limit("cast dimensions/version"));
    }
    let mut last = 0.0;
    let mut output = String::new();
    for (i, line) in lines.enumerate() {
        if i >= 10_000 {
            return Err(Error::Limit("cast event count"));
        }
        let value: Value = serde_json::from_str(line)?;
        let array = value.as_array().ok_or(Error::Invalid("cast event"))?;
        if array.len() != 3 || !array[2].is_string() {
            return Err(Error::Invalid("cast event shape"));
        }
        let time = array[0].as_f64().ok_or(Error::Invalid("cast timestamp"))?;
        if !time.is_finite() || time < last || time < 0.0 {
            return Err(Error::Invalid("cast event order"));
        }
        if !matches!(array[1].as_str(), Some("o" | "i" | "m" | "r")) {
            return Err(Error::Invalid("cast event kind"));
        }
        if array[1].as_str() == Some("o") {
            output.push_str(array[2].as_str().unwrap());
        }
        last = time;
    }
    validate_terminal_budget(&output, width, height)?;
    let frame = cua_spaces_transcript::render_frame_document(
        cast,
        "recording.cast",
        "requested",
        time_ms as i64,
    )?;
    let mut frame: Value = serde_json::from_str(&frame)?;
    let joined = frame
        .get("lines")
        .and_then(Value::as_array)
        .map(|lines| lines.iter().filter_map(Value::as_str).collect::<String>())
        .unwrap_or_default();
    if secrets
        .iter()
        .any(|secret| !secret.is_empty() && (output.contains(secret) || joined.contains(secret)))
    {
        if let Some(lines) = frame.get_mut("lines").and_then(Value::as_array_mut) {
            for line in lines {
                *line = Value::String(REDACTED.into());
            }
        }
        frame["window_title"] = Value::String(REDACTED.into());
    }
    redact_value(&frame, secrets)
}

fn validate_terminal_budget(stream: &str, width: u64, height: u64) -> Result<(), Error> {
    let bytes = stream.as_bytes();
    let controls = bytes
        .iter()
        .filter(|b| matches!(b, 0x1b | 0x0a..=0x0c))
        .count() as u64;
    let work = controls
        .saturating_mul(width * height)
        .saturating_add(bytes.len() as u64 * height);
    if bytes.len() > 256 * 1024 || work > 32_000_000 {
        return Err(Error::Limit("terminal work budget"));
    }
    let mut offset = 0;
    while offset + 1 < bytes.len() {
        if bytes[offset] == 0x1b && bytes[offset + 1] == b'[' {
            offset += 2;
            let start = offset;
            while offset < bytes.len() && !(0x40..=0x7e).contains(&bytes[offset]) {
                offset += 1;
            }
            if offset - start > 128 {
                return Err(Error::Limit("terminal control length"));
            }
            if offset < bytes.len() && matches!(bytes[offset], b'M' | b'S') {
                return Err(Error::Invalid("unsafe upstream scroll-up sequence"));
            }
            let parameters = &bytes[start..offset];
            for digits in parameters
                .split(|b| !b.is_ascii_digit())
                .filter(|run| !run.is_empty())
            {
                if digits.len() > 4
                    || std::str::from_utf8(digits).unwrap().parse::<u32>().unwrap() > 4096
                {
                    return Err(Error::Limit("terminal control parameter"));
                }
            }
        }
        offset += 1;
    }
    Ok(())
}
