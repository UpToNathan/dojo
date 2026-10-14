use super::*;
use dojo_computer_models::{ModelView, PARSER_VERSION, Profile};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelProfile {
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
impl From<ModelProfile> for Profile {
    fn from(value: ModelProfile) -> Self {
        match value {
            ModelProfile::Canonical => Self::Canonical,
            ModelProfile::UiTars => Self::UiTars,
            ModelProfile::Qwen35 => Self::Qwen35,
            ModelProfile::GenericVlm => Self::GenericVlm,
            ModelProfile::Anthropic => Self::Anthropic,
            ModelProfile::OpenAiNative => Self::OpenAiNative,
            ModelProfile::Gemini => Self::Gemini,
            ModelProfile::UiTars2 => Self::UiTars2,
            ModelProfile::Fara => Self::Fara,
            ModelProfile::OpenCua => Self::OpenCua,
        }
    }
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelDecode {
    pub capture_id: String,
    pub profile: ModelProfile,
    /// One model step referring to the unchanged PNG delivered by Dojo.
    pub raw: String,
}

/// Prediction provenance carried into Pi's tool intent, never authority.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelSource {
    pub profile: ModelProfile,
    pub parser_version: String,
    pub raw: String,
    pub capture_digest: String,
    pub call_id: Option<String>,
}
impl ModelSource {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.raw.len() <= 8192 && self.parser_version.len() <= 128,
            "model provenance exceeds limits"
        );
        ensure!(
            self.capture_digest.len() == 64
                && self.capture_digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid prediction capture digest"
        );
        ensure!(
            self.call_id.as_ref().is_none_or(|id| id.len() <= 128),
            "model call id exceeds limits"
        );
        Ok(())
    }
}

impl ModelDecode {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.raw.len() <= 8192,
            "model tool output exceeds 8192 bytes"
        );
        ensure!(self.capture_id.len() <= 128, "invalid capture id");
        self.capture_id.parse::<CaptureId>()?;
        Ok(())
    }
}

impl CaptureSession {
    /// Decoding never admits an action, consumes a capture, or creates input.
    /// View geometry comes from retained pixels, never caller supplied transforms.
    pub fn decode_model(&self, input: ModelDecode) -> Result<serde_json::Value> {
        input.validate()?;
        let scope = self.scope(&input.capture_id)?;
        let capture = self
            .authority
            .captures
            .read_for_perception(input.capture_id.parse()?, &self.binding)?;
        let dimensions = capture.encoded_dimensions();
        let profile = Profile::from(input.profile);
        let view = ModelView {
            view_id: input.capture_id.clone(),
            capture_id: input.capture_id.clone(),
            capture_digest: capture.digest().hex(),
            session_id: self.binding.session_id().into(),
            runtime_generation: self.binding.runtime_generation(),
            geometry_generation: self.binding.session_generation(),
            profile,
            parser_version: PARSER_VERSION.into(),
            preprocessing: "unchanged_dojo_delivered_png/1".into(),
            model_width: dimensions.width(),
            model_height: dimensions.height(),
            delivered_width: dimensions.width(),
            delivered_height: dimensions.height(),
            scale_x: 1.,
            scale_y: 1.,
            offset_x: 0.,
            offset_y: 0.,
        };
        let decoded = dojo_computer_models::decode(profile, &input.raw, &view)?;
        Ok(
            serde_json::json!({"capture_id":input.capture_id,"output":scope.output,"parser_version":PARSER_VERSION,"capture_digest":view.capture_digest,"profile":input.profile,"preprocessing":view.preprocessing,"image_width":dimensions.width(),"image_height":dimensions.height(),"outcome":decoded.outcome,"call_id":decoded.call_id,"thought":decoded.thought,"provider_metadata":decoded.provider_metadata,"authorized":false,"capture_consumed":false}),
        )
    }
}
