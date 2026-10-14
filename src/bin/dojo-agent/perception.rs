use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct VisualParseInput(pub dojo_cua_contract::ParseVisualRegionsInput);

pub(super) fn pinned_input_schema() -> std::sync::Arc<serde_json::Map<String, serde_json::Value>> {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../provenance/contracts/cua-driver.manifest.json"
    ))
    .expect("pinned Driver manifest");
    let tool = manifest["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "parse_visual_regions")
        .expect("pinned perception contract");
    std::sync::Arc::new(
        tool["input_schema"]
            .as_object()
            .expect("pinned schema object")
            .clone(),
    )
}

impl<'de> Deserialize<'de> for VisualParseInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let Some(options) = value.get("options").and_then(serde_json::Value::as_object) {
            for name in ["kinds", "min_confidence", "max_regions"] {
                if options.get(name).is_some_and(serde_json::Value::is_null) {
                    return Err(serde::de::Error::custom(format!(
                        "{name} must be omitted rather than null"
                    )));
                }
            }
        }
        let input: dojo_cua_contract::ParseVisualRegionsInput =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        input.validate().map_err(serde::de::Error::custom)?;
        Ok(Self(input))
    }
}

impl schemars::JsonSchema for VisualParseInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "parse_visual_regions".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::Schema::try_from(serde_json::Value::Object((*pinned_input_schema()).clone()))
            .expect("pinned perception schema")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pinned_perception_input_rejects_null_unknown_and_invalid_filters() {
        let good: VisualParseInput = serde_json::from_value(json!({"capture_id":"one"})).unwrap();
        assert_eq!(good.0.capture_id, "one");
        for value in [
            json!({"capture_id":"one","image":"caller pixels"}),
            json!({"capture_id":""}),
            json!({"capture_id":"one","options":null}),
            json!({"capture_id":"one","options":{"max_regions":null}}),
            json!({"capture_id":"one","options":{"max_regions":0}}),
            json!({"capture_id":"one","options":{"min_confidence":1.1}}),
            json!({"capture_id":"one","options":{"kinds":[]}}),
            json!({"capture_id":"one","options":{"kinds":["button"]}}),
        ] {
            assert!(serde_json::from_value::<VisualParseInput>(value).is_err());
        }
    }
}
