//! Translated from pinned CUA response normalization and VLM profile parsers.
//! See README.md for exact source paths and deliberate behavior differences.
mod uitars;
use crate::{
    Button, ComputerAction, CoordinateSpace, DecodeError, MAX_PATH_POINTS, MAX_TEXT_BYTES,
    ModelView, Outcome, Profile, ScrollUnit,
};
use serde_json::{Map, Value};

type Decoded = (Outcome, Option<String>, Option<String>);
pub(crate) fn decode(
    profile: Profile,
    raw: &str,
    view: &ModelView,
) -> Result<Decoded, DecodeError> {
    if profile == Profile::UiTars2 {
        return uitars::seed(raw, view);
    }
    if profile == Profile::OpenCua {
        return uitars::opencua(raw, view);
    }
    if profile == Profile::UiTars {
        return uitars::decode(raw, view);
    }
    let value = if profile == Profile::Canonical {
        crate::json::parse(raw)?
    } else {
        tool_call(raw, profile)?
    };
    let mut envelope = value
        .as_object()
        .cloned()
        .ok_or(DecodeError::Malformed("object"))?;
    let call_id = envelope.remove("call_id").map(|v| text(&v)).transpose()?;
    let thought = envelope.remove("thought").map(|v| text(&v)).transpose()?;
    let args = if profile == Profile::Canonical {
        envelope
    } else {
        if envelope.remove("name").as_ref().and_then(Value::as_str)
            != Some(if profile == Profile::Fara {
                "computer_use"
            } else {
                "computer"
            })
        {
            return Err(DecodeError::Unsupported("tool name".into()));
        }
        let args = envelope
            .remove("arguments")
            .ok_or(DecodeError::Missing("arguments"))?;
        empty(&envelope)?;
        args.as_object()
            .cloned()
            .ok_or(DecodeError::Malformed("arguments"))?
    };
    Ok((action(profile, args, view)?, call_id, thought))
}
pub(crate) fn tool_call(raw: &str, profile: Profile) -> Result<Value, DecodeError> {
    let (before, rest) = raw
        .split_once("<tool_call>")
        .ok_or(DecodeError::Missing("tool_call"))?;
    let (body, after) = rest
        .split_once("</tool_call>")
        .ok_or(DecodeError::Malformed("tool_call"))?;
    if before.contains("</tool_call>")
        || after.contains("<tool_call>")
        || body.contains("<tool_call>")
    {
        return Err(DecodeError::Unsupported("multiple tool calls".into()));
    }
    let body = body.trim();
    if body.starts_with('{') {
        return crate::json::parse(body);
    }
    if profile != Profile::Qwen35 {
        return Err(DecodeError::Malformed("JSON tool_call"));
    }
    let body = body
        .strip_prefix("<function=computer>")
        .and_then(|s| s.strip_suffix("</function>"))
        .ok_or(DecodeError::Malformed("function"))?;
    let mut rest = body.trim();
    let mut args = Map::new();
    while !rest.is_empty() {
        let field = rest
            .strip_prefix("<parameter=")
            .ok_or(DecodeError::Malformed("parameter"))?;
        let (key, field) = field
            .split_once('>')
            .ok_or(DecodeError::Malformed("parameter"))?;
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(DecodeError::Malformed("parameter name"));
        }
        let key = if key == "type" { "action" } else { key };
        if args.contains_key(key) || args.len() >= 32 {
            return Err(DecodeError::Malformed("duplicate parameter"));
        }
        let (value, tail) = field
            .split_once("</parameter>")
            .ok_or(DecodeError::Malformed("parameter"))?;
        let value = value.trim();
        let value = if value.starts_with(['[', '{', '"'])
            || value.parse::<f64>().is_ok()
            || matches!(value, "true" | "false" | "null")
        {
            crate::json::parse(value)?
        } else {
            Value::String(value.into())
        };
        args.insert(key.into(), value);
        rest = tail.trim();
    }
    Ok(serde_json::json!({"name":"computer","arguments":args}))
}
pub(super) fn text(value: &Value) -> Result<String, DecodeError> {
    let text = value.as_str().ok_or(DecodeError::Invalid("text"))?;
    if text.len() > MAX_TEXT_BYTES {
        return Err(DecodeError::Limit("text"));
    }
    Ok(text.into())
}
pub(super) fn empty(args: &Map<String, Value>) -> Result<(), DecodeError> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(DecodeError::Invalid("unknown action fields"))
    }
}
fn number(value: &Value) -> Result<f64, DecodeError> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or(DecodeError::Invalid("finite number"))
}
fn point(
    args: &mut Map<String, Value>,
    view: &ModelView,
    normalized: bool,
) -> Result<Option<crate::Point>, DecodeError> {
    if normalized {
        let Some(value) = args.remove("coordinate") else {
            return Ok(None);
        };
        let values = value
            .as_array()
            .filter(|v| v.len() == 2)
            .ok_or(DecodeError::Invalid("coordinate pair"))?;
        return view
            .point(
                number(&values[0])?,
                number(&values[1])?,
                CoordinateSpace::Normalized1000,
                false,
            )
            .map(Some);
    }
    match (args.remove("x"), args.remove("y")) {
        (None, None) => Ok(None),
        (Some(x), Some(y)) => view
            .point(
                number(&x)?,
                number(&y)?,
                CoordinateSpace::ModelPixels,
                false,
            )
            .map(Some),
        _ => Err(DecodeError::Missing("coordinate pair")),
    }
}
fn required(point: Option<crate::Point>) -> Result<crate::Point, DecodeError> {
    point.ok_or(DecodeError::Missing("coordinates"))
}
pub(super) fn keys(value: &Value) -> Result<Vec<String>, DecodeError> {
    let values = value
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 16)
        .ok_or(DecodeError::Invalid("keys"))?;
    values
        .iter()
        .map(|v| {
            let key = text(v)?;
            if key.trim().is_empty()
                || key.chars().count() > 64
                || key.chars().any(char::is_control)
            {
                return Err(DecodeError::Invalid("key"));
            }
            Ok(match key.to_ascii_lowercase().as_str() {
                "control" | "ctrl" => "CTRL".into(),
                "return" | "enter" => "ENTER".into(),
                "escape" | "esc" => "ESC".into(),
                "shift" => "SHIFT".into(),
                "alt" | "option" => "ALT".into(),
                "super" | "meta" | "win" | "command" => "META".into(),
                _ => key,
            })
        })
        .collect()
}
pub(crate) fn action(
    profile: Profile,
    mut args: Map<String, Value>,
    view: &ModelView,
) -> Result<Outcome, DecodeError> {
    let normalized = profile != Profile::Canonical;
    let kind = args
        .remove("action")
        .ok_or(DecodeError::Missing("action"))?;
    let kind = kind.as_str().ok_or(DecodeError::Invalid("action"))?;
    let at = point(&mut args, view, normalized)?;
    let outcome = match kind {
        "done" | "answer" | "terminate" | "ask_human" | "refusal" => {
            if at.is_some() {
                return Err(DecodeError::Invalid("terminal coordinates"));
            }
            let message = args
                .remove("text")
                .map(|v| text(&v))
                .transpose()?
                .unwrap_or_default();
            match kind {
                "ask_human" => Outcome::AskHuman { text: message },
                "refusal" => Outcome::Refusal { text: message },
                _ => Outcome::Done { text: message },
            }
        }
        "left_click" | "right_click" | "middle_click" | "double_click" | "triple_click"
        | "click" => {
            if kind == "triple_click" {
                return Err(DecodeError::Unsupported("triple click".into()));
            }
            let button = match kind {
                "right_click" => Button::Right,
                "middle_click" => Button::Middle,
                _ => match args
                    .remove("button")
                    .map(|v| text(&v))
                    .transpose()?
                    .as_deref()
                    .unwrap_or("left")
                {
                    "left" => Button::Left,
                    "right" => Button::Right,
                    "middle" => Button::Middle,
                    "back" => Button::Back,
                    "forward" => Button::Forward,
                    _ => return Err(DecodeError::Invalid("button")),
                },
            };
            Outcome::Action(ComputerAction::Click {
                at: required(at)?,
                button,
                count: if kind == "double_click" { 2 } else { 1 },
            })
        }
        "move" | "mouse_move" => Outcome::Action(ComputerAction::Move { at: required(at)? }),
        "type" => Outcome::Action(ComputerAction::Type {
            text: text(&args.remove("text").ok_or(DecodeError::Missing("text"))?)?,
            at,
        }),
        "key" | "keypress" => {
            if at.is_some() {
                return Err(DecodeError::Invalid("key coordinates"));
            }
            Outcome::Action(ComputerAction::Keypress {
                keys: keys(&args.remove("keys").ok_or(DecodeError::Missing("keys"))?)?,
            })
        }
        "scroll" | "hscroll" => {
            let (dx, dy) = if normalized {
                // Qwen's documented pixels sign is up/left; canonical sign is down/right.
                let amount = number(
                    &args
                        .remove("pixels")
                        .ok_or(DecodeError::Missing("pixels"))?,
                )?;
                if kind == "hscroll" {
                    (-amount, 0.)
                } else {
                    (0., -amount)
                }
            } else {
                (
                    number(
                        &args
                            .remove("scroll_x")
                            .ok_or(DecodeError::Missing("scroll_x"))?,
                    )?,
                    number(
                        &args
                            .remove("scroll_y")
                            .ok_or(DecodeError::Missing("scroll_y"))?,
                    )?,
                )
            };
            if dx.abs() > 10000. || dy.abs() > 10000. {
                return Err(DecodeError::Invalid("scroll range"));
            }
            Outcome::Action(ComputerAction::Scroll {
                at,
                dx,
                dy,
                unit: ScrollUnit::Pixels,
            })
        }
        "drag" => {
            if at.is_some() {
                return Err(DecodeError::Invalid("drag coordinates"));
            }
            let value = args.remove("path").ok_or(DecodeError::Missing("path"))?;
            let path = value
                .as_array()
                .filter(|p| p.len() >= 2 && p.len() <= MAX_PATH_POINTS)
                .ok_or(DecodeError::Invalid("path"))?;
            let path = path
                .iter()
                .map(|p| {
                    let mut p = p
                        .as_object()
                        .cloned()
                        .ok_or(DecodeError::Invalid("path point"))?;
                    let at = required(point(&mut p, view, normalized)?)?;
                    empty(&p)?;
                    Ok(at)
                })
                .collect::<Result<_, DecodeError>>()?;
            Outcome::Action(ComputerAction::Drag { path })
        }
        "wait" => {
            if at.is_some() {
                return Err(DecodeError::Invalid("wait coordinates"));
            }
            let milliseconds = if normalized {
                number(&args.remove("time").unwrap_or(Value::from(1.)))? * 1000.
            } else {
                number(&args.remove("milliseconds").unwrap_or(Value::from(1000)))?
            };
            if !(0.0..=30000.0).contains(&milliseconds) || milliseconds.fract() != 0. {
                return Err(DecodeError::Invalid("wait duration"));
            }
            Outcome::Action(ComputerAction::Wait {
                milliseconds: milliseconds as u32,
            })
        }
        "screenshot" => {
            if at.is_some() {
                return Err(DecodeError::Invalid("screenshot coordinates"));
            }
            Outcome::Action(ComputerAction::Screenshot)
        }
        other => return Err(DecodeError::Unsupported(other.into())),
    };
    empty(&args)?;
    Ok(outcome)
}
