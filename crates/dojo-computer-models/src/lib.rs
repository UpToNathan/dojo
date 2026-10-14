//! Pure, fail-closed model-output decoding. Decoding does not authorize execution.
mod json;
mod native;
pub mod policy;
pub mod preprocessing;
mod profiles;
pub mod s1;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PARSER_VERSION: &str = "dojo-computer-models/1";
pub const MAX_RAW_BYTES: usize = 65_536;
pub const MAX_TEXT_BYTES: usize = 16_384;
pub const MAX_PATH_POINTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Canonical,
    UiTars,
    Qwen35,
    GenericVlm,
    Anthropic,
    OpenAiNative,
    Gemini,
    UiTars2,
    Fara,
    OpenCua,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoordinateSpace {
    ModelPixels,
    Normalized1000,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelView {
    pub view_id: String,
    pub capture_id: String,
    pub capture_digest: String,
    pub session_id: String,
    pub runtime_generation: u64,
    pub geometry_generation: u64,
    pub profile: Profile,
    pub parser_version: String,
    /// Description/version of preprocessing performed by the caller; not inferred.
    pub preprocessing: String,
    pub model_width: u32,
    pub model_height: u32,
    pub delivered_width: u32,
    pub delivered_height: u32,
    /// Axis-aligned model-view to delivered-image transform (crop + resize).
    pub scale_x: f64,
    pub scale_y: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub source_x: f64,
    pub source_y: f64,
    pub source_space: CoordinateSpace,
    pub model_x: f64,
    pub model_y: f64,
    /// Fractional delivered pixels retained; no desktop mapping is performed here.
    pub delivered_x: f64,
    pub delivered_y: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Button {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollUnit {
    Pixels,
    WheelSteps,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComputerAction {
    Click {
        at: Point,
        button: Button,
        count: u8,
    },
    Move {
        at: Point,
    },
    Drag {
        path: Vec<Point>,
    },
    Keypress {
        keys: Vec<String>,
    },
    Type {
        text: String,
        at: Option<Point>,
    },
    /// Positive x = right, positive y = down. Units must not be interchanged.
    Scroll {
        at: Option<Point>,
        dx: f64,
        dy: f64,
        unit: ScrollUnit,
    },
    Wait {
        milliseconds: u32,
    },
    Screenshot,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Outcome {
    Action(ComputerAction),
    Done { text: String },
    AskHuman { text: String },
    Refusal { text: String },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedStep {
    pub profile: Profile,
    pub parser_version: String,
    pub raw: String,
    pub observation: ModelView,
    pub call_id: Option<String>,
    pub thought: Option<String>,
    pub outcome: Outcome,
    /// Complete bounded native block, including safety checks and signatures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<serde_json::Value>,
}
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DecodeError {
    #[error("input or collection limit exceeded: {0}")]
    Limit(&'static str),
    #[error("malformed output: {0}")]
    Malformed(&'static str),
    #[error("required field missing: {0}")]
    Missing(&'static str),
    #[error("invalid field or range: {0}")]
    Invalid(&'static str),
    #[error("unsupported capability: {0}")]
    Unsupported(String),
    #[error("invalid or mismatched model-view binding")]
    Binding,
}

/// Decode exactly one step, preserving source and the explicit model-view binding.
/// The executor must independently resolve/check session, freshness, capabilities,
/// consent and single-use capture admission. IDs here are not authorization grants.
pub fn decode(
    profile: Profile,
    raw: &str,
    observation: &ModelView,
) -> Result<DecodedStep, DecodeError> {
    if raw.len() > MAX_RAW_BYTES {
        return Err(DecodeError::Limit("raw"));
    }
    observation.validate(profile)?;
    let native = matches!(
        profile,
        Profile::Anthropic | Profile::OpenAiNative | Profile::Gemini
    );
    let (outcome, call_id, thought, provider_metadata) = if native {
        native::decode(profile, raw, observation)?
    } else {
        let (outcome, call_id, thought) = profiles::decode(profile, raw, observation)?;
        (outcome, call_id, thought, None)
    };
    Ok(DecodedStep {
        profile,
        parser_version: PARSER_VERSION.into(),
        raw: raw.into(),
        observation: observation.clone(),
        call_id,
        thought,
        outcome,
        provider_metadata,
    })
}

impl ModelView {
    fn validate(&self, profile: Profile) -> Result<(), DecodeError> {
        let strings = [
            &self.view_id,
            &self.capture_id,
            &self.capture_digest,
            &self.session_id,
            &self.preprocessing,
        ];
        if self.profile != profile
            || self.parser_version != PARSER_VERSION
            || strings.iter().any(|s| s.is_empty() || s.len() > 1024)
        {
            return Err(DecodeError::Binding);
        }
        if [
            self.model_width,
            self.model_height,
            self.delivered_width,
            self.delivered_height,
        ]
        .iter()
        .any(|&d| d == 0 || d > 32768)
        {
            return Err(DecodeError::Binding);
        }
        if ![self.scale_x, self.scale_y, self.offset_x, self.offset_y]
            .iter()
            .all(|v| v.is_finite())
            || self.scale_x <= 0.0
            || self.scale_y <= 0.0
            || self.offset_x < 0.0
            || self.offset_y < 0.0
            || self.offset_x + f64::from(self.model_width) * self.scale_x
                > f64::from(self.delivered_width)
            || self.offset_y + f64::from(self.model_height) * self.scale_y
                > f64::from(self.delivered_height)
        {
            return Err(DecodeError::Binding);
        }
        Ok(())
    }
    pub(crate) fn point(
        &self,
        x: f64,
        y: f64,
        space: CoordinateSpace,
        floor: bool,
    ) -> Result<Point, DecodeError> {
        let (mut mx, mut my) = match space {
            CoordinateSpace::ModelPixels => (x, y),
            CoordinateSpace::Normalized1000 => {
                if !(0.0..=1000.0).contains(&x) || !(0.0..=1000.0).contains(&y) {
                    return Err(DecodeError::Invalid("coordinate"));
                }
                (
                    x * f64::from(self.model_width) / 1000.0,
                    y * f64::from(self.model_height) / 1000.0,
                )
            }
        };
        if !mx.is_finite() || !my.is_finite() {
            return Err(DecodeError::Invalid("coordinate"));
        }
        if space == CoordinateSpace::Normalized1000 {
            mx = if floor {
                mx.floor()
            } else {
                mx.round_ties_even()
            };
            my = if floor {
                my.floor()
            } else {
                my.round_ties_even()
            };
        }
        if mx < 0.0
            || my < 0.0
            || mx >= f64::from(self.model_width)
            || my >= f64::from(self.model_height)
        {
            return Err(DecodeError::Invalid("coordinate"));
        }
        Ok(Point {
            source_x: x,
            source_y: y,
            source_space: space,
            model_x: mx,
            model_y: my,
            delivered_x: self.offset_x + mx * self.scale_x,
            delivered_y: self.offset_y + my * self.scale_y,
        })
    }
}
