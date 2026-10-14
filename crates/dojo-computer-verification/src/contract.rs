// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Cua AI, Inc.

//! Cross-platform deterministic state-verification contract.
//!
//! The caller owns the task-level meaning of success. The driver only checks
//! bounded predicates against one explicitly authorized window.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

pub const VERIFY_STATE_DEFAULT_TIMEOUT_MS: u64 = 5_000;

fn timeout_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "integer",
        "minimum": 0,
        "maximum": 10000,
        "default": VERIFY_STATE_DEFAULT_TIMEOUT_MS
    })
}

fn stable_samples_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "integer", "minimum": 1, "maximum": 5, "default": 2 })
}

fn tolerance_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "number", "minimum": 0, "maximum": 100 })
}

fn number_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "number" })
}

fn integer_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "integer" })
}

fn positive_integer_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "integer", "minimum": 1 })
}

fn nonempty_string_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "minLength": 1 })
}

fn boolean_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "boolean" })
}

fn nullable_string_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "anyOf": [{ "type": "string" }, { "type": "null" }] })
}

fn nullable_unknown_reason_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "anyOf": [
            {
                "type": "string",
                "enum": [
                    "invalid_predicate",
                    "unsupported_predicate",
                    "untrusted_source",
                    "multi_match",
                    "target_missing",
                    "observation_unavailable",
                    "stability_unproven"
                ]
            },
            { "type": "null" }
        ]
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoundsExpectation {
    #[schemars(schema_with = "number_schema")]
    pub x: f64,
    #[schemars(schema_with = "number_schema")]
    pub y: f64,
    #[schemars(schema_with = "number_schema")]
    pub width: f64,
    #[schemars(schema_with = "number_schema")]
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "tolerance_schema")]
    pub tolerance_px: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ElementSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "nonempty_string_schema")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "nonempty_string_schema")]
    pub label_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WindowPredicate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_equals: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_equals: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoundsExpectation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ElementPredicate {
    pub selector: ElementSelector,
    /// Assert that at least one trusted element matches the selector.
    ///
    /// Negative existence requires complete coverage of the exact bound window.
    /// Partial trees, ref-only browser lists and unbound accessibility are unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "boolean_schema")]
    pub exists: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_equals: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StatePredicate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<ElementPredicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VerifyStateInput {
    /// Exact process whose window may be observed.
    #[schemars(schema_with = "positive_integer_schema")]
    pub pid: i64,
    /// Exact native window identifier.
    #[schemars(schema_with = "integer_schema")]
    pub window_id: u64,
    /// One to eight predicates, combined with logical AND.
    #[schemars(length(min = 1, max = 8))]
    pub expect: Vec<StatePredicate>,
    /// Bounded wait. Zero performs one sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "timeout_schema")]
    pub timeout_ms: Option<u64>,
    /// Consecutive satisfied samples required before returning success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "stable_samples_schema")]
    pub stable_samples: Option<u64>,
    /// Return the final window screenshot as image content for a multimodal
    /// caller. The driver does not interpret that image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_screenshot: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Satisfied,
    Unsatisfied,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    InvalidPredicate,
    UnsupportedPredicate,
    UntrustedSource,
    MultiMatch,
    TargetMissing,
    ObservationUnavailable,
    StabilityUnproven,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct PredicateOutcome {
    pub index: u64,
    pub status: VerificationStatus,
    #[schemars(required, schema_with = "nullable_unknown_reason_schema")]
    pub unknown_reason: Option<UnknownReason>,
    /// Normalized, bounded JSON projection of the matched state.
    #[schemars(required, schema_with = "nullable_string_schema")]
    pub observed_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct VerifyStateOutput {
    pub status: VerificationStatus,
    pub stable: bool,
    pub elapsed_ms: u64,
    pub samples: u64,
    pub predicates: Vec<PredicateOutcome>,
}
