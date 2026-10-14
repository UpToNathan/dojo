//! Native block normalization. Transport and approval remain the caller's responsibility.
use crate::{DecodeError as E, ModelView, Outcome, Profile};
use serde_json::{Map, Value, json};
type ResultStep = Result<(Outcome, Option<String>, Option<String>, Option<Value>), E>;
fn take(map: &mut Map<String, Value>, key: &'static str) -> Result<Value, E> {
    map.remove(key).ok_or(E::Missing(key))
}
fn string(map: &mut Map<String, Value>, key: &'static str) -> Result<String, E> {
    super::profiles::text(&take(map, key)?)
}
fn coordinate(args: &mut Map<String, Value>) -> Result<(), E> {
    if let Some(v) = args.remove("coordinate") {
        let p = v
            .as_array()
            .filter(|p| p.len() == 2)
            .ok_or(E::Invalid("coordinate"))?;
        if args.insert("x".into(), p[0].clone()).is_some()
            || args.insert("y".into(), p[1].clone()).is_some()
        {
            return Err(E::Invalid("duplicate coordinates"));
        }
    }
    Ok(())
}
fn key_string(args: &mut Map<String, Value>, field: &'static str) -> Result<(), E> {
    let value = take(args, field)?;
    let keys = if let Some(s) = value.as_str() {
        json!(s.split('+').collect::<Vec<_>>())
    } else {
        value
    };
    args.insert("keys".into(), keys);
    Ok(())
}
pub(crate) fn decode(profile: Profile, raw: &str, view: &ModelView) -> ResultStep {
    let metadata = crate::json::parse(raw)?;
    let mut block = metadata
        .as_object()
        .cloned()
        .ok_or(E::Malformed("native block"))?;
    if block.get("type").and_then(Value::as_str) == Some("refusal") {
        let message = block
            .get("refusal")
            .or_else(|| block.get("text"))
            .ok_or(E::Missing("refusal"))?;
        return Ok((
            Outcome::Refusal {
                text: super::profiles::text(message)?,
            },
            None,
            None,
            Some(metadata),
        ));
    }
    let (mut args, call_id) = match profile {
        Profile::Anthropic => {
            if string(&mut block, "type")? != "tool_use"
                || string(&mut block, "name")? != "computer"
            {
                return Err(E::Unsupported("Anthropic block/tool".into()));
            }
            let id = string(&mut block, "id")?;
            let mut a = take(&mut block, "input")?
                .as_object()
                .cloned()
                .ok_or(E::Malformed("input"))?;
            coordinate(&mut a)?;
            let kind = string(&mut a, "action")?;
            let mapped = match kind.as_str() {
                "left_click" | "right_click" | "middle_click" | "double_click" | "mouse_move"
                | "type" | "screenshot" => kind,
                "key" => {
                    key_string(&mut a, "text")?;
                    "keypress".into()
                }
                "left_click_drag" => {
                    // Anthropic supplies only the destination. A held-input history is
                    // required to know the start, so a single-block decoder refuses it.
                    return Err(E::Unsupported("Anthropic drag needs explicit start".into()));
                }
                "scroll" => {
                    let amount = take(&mut a, "scroll_amount")?
                        .as_f64()
                        .filter(|x| x.is_finite() && *x >= 0. && *x <= 100.)
                        .ok_or(E::Invalid("scroll_amount"))?;
                    let direction = string(&mut a, "scroll_direction")?;
                    let (x, y) = direction_delta(&direction, amount)?;
                    a.insert("scroll_x".into(), json!(x));
                    a.insert("scroll_y".into(), json!(y));
                    a.insert("__wheel".into(), json!(true));
                    "scroll".into()
                }
                "wait" => {
                    let seconds = take(&mut a, "duration")?
                        .as_f64()
                        .ok_or(E::Invalid("duration"))?;
                    a.insert("milliseconds".into(), json!(seconds * 1000.));
                    "wait".into()
                }
                other => return Err(E::Unsupported(other.into())),
            };
            a.insert("action".into(), json!(mapped));
            (a, Some(id))
        }
        Profile::OpenAiNative => {
            if string(&mut block, "type")? != "computer_call" {
                return Err(E::Unsupported("OpenAI native block".into()));
            }
            let id = string(&mut block, "call_id")?;
            let mut a = take(&mut block, "action")?
                .as_object()
                .cloned()
                .ok_or(E::Malformed("action"))?;
            let kind = string(&mut a, "type")?;
            a.insert("action".into(), json!(kind));
            (a, Some(id))
        }
        Profile::Gemini => {
            let fc = take(&mut block, "functionCall")?;
            let mut fc = fc
                .as_object()
                .cloned()
                .ok_or(E::Malformed("functionCall"))?;
            let id = fc
                .remove("id")
                .map(|v| super::profiles::text(&v))
                .transpose()?;
            let name = string(&mut fc, "name")?;
            let name = name.strip_prefix("web_agent_api:").unwrap_or(&name);
            let mut a = take(&mut fc, "args")?
                .as_object()
                .cloned()
                .ok_or(E::Malformed("args"))?;
            super::profiles::empty(&fc)?;
            // Convert with per-axis dimensions from this exact model view.
            if a.contains_key("x") || a.contains_key("y") {
                let x = take(&mut a, "x")?.as_f64().ok_or(E::Invalid("x"))?;
                let y = take(&mut a, "y")?.as_f64().ok_or(E::Invalid("y"))?;
                let point = view.point(x, y, crate::CoordinateSpace::Normalized1000, true)?;
                a.insert("x".into(), json!(point.model_x));
                a.insert("y".into(), json!(point.model_y));
            }
            let kind = match name {
                "click_at" => "click",
                "hover_at" => "move",
                "type_text_at" => {
                    // Press-enter is a distinct effect. Preserve it by refusing this
                    // compound action until the executor supports an explicit batch.
                    if a.remove("press_enter")
                        .map(|v| v.as_bool().ok_or(E::Invalid("press_enter")))
                        .transpose()?
                        .unwrap_or(false)
                    {
                        return Err(E::Unsupported(
                            "type plus enter requires bounded batch".into(),
                        ));
                    }
                    "type"
                }
                "key_combination" => {
                    key_string(&mut a, "keys")?;
                    "keypress"
                }
                "scroll_document" | "scroll_at" => {
                    if name == "scroll_at" && !a.contains_key("x") {
                        return Err(E::Missing("coordinates"));
                    }
                    let amount = a
                        .remove("magnitude")
                        .unwrap_or(json!(800))
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.)
                        .ok_or(E::Invalid("magnitude"))?;
                    let direction = string(&mut a, "direction")?;
                    let (x, y) = direction_delta(&direction, amount)?;
                    a.insert("scroll_x".into(), json!(x));
                    a.insert("scroll_y".into(), json!(y));
                    "scroll"
                }
                "drag_and_drop" => {
                    let x = take(&mut a, "x")?;
                    let y = take(&mut a, "y")?;
                    let dx = take(&mut a, "destination_x")?
                        .as_f64()
                        .ok_or(E::Invalid("destination_x"))?;
                    let dy = take(&mut a, "destination_y")?
                        .as_f64()
                        .ok_or(E::Invalid("destination_y"))?;
                    let dest = view.point(dx, dy, crate::CoordinateSpace::Normalized1000, true)?;
                    a.insert(
                        "path".into(),
                        json!([{"x":x,"y":y},{"x":dest.model_x,"y":dest.model_y}]),
                    );
                    "drag"
                }
                "wait_5_seconds" => {
                    a.insert("milliseconds".into(), json!(5000));
                    "wait"
                }
                other => return Err(E::Unsupported(other.into())),
            };
            if matches!(kind, "click" | "move" | "type") && !a.contains_key("x") {
                return Err(E::Missing("coordinates"));
            }
            a.insert("action".into(), json!(kind));
            (a, id)
        }
        _ => unreachable!(),
    };
    // Native envelope metadata is opaque and retained in full. Action fields are
    // strictly consumed, even when provider blocks add signatures or safety checks.
    let wheel = args.remove("__wheel").is_some();
    let mut outcome = super::profiles::action(Profile::Canonical, args, view)?;
    if wheel && let Outcome::Action(crate::ComputerAction::Scroll { unit, .. }) = &mut outcome {
        *unit = crate::ScrollUnit::WheelSteps;
    }
    if call_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > 1024)
    {
        return Err(E::Invalid("call id"));
    }
    Ok((outcome, call_id, None, Some(metadata)))
}
fn direction_delta(direction: &str, magnitude: f64) -> Result<(f64, f64), E> {
    match direction {
        "up" => Ok((0., -magnitude)),
        "down" => Ok((0., magnitude)),
        "left" => Ok((-magnitude, 0.)),
        "right" => Ok((magnitude, 0.)),
        _ => Err(E::Invalid("scroll direction")),
    }
}
