use super::{Decoded, empty, keys, text};
use crate::{Button, ComputerAction, CoordinateSpace, DecodeError, ModelView, Outcome, ScrollUnit};
use serde_json::{Map, Value};

pub(super) fn decode(raw: &str, view: &ModelView) -> Result<Decoded, DecodeError> {
    let (prefix, call) = raw
        .split_once("Action:")
        .ok_or(DecodeError::Missing("Action"))?;
    let thought = prefix
        .trim()
        .strip_prefix("Thought:")
        .map(|t| t.trim().to_owned());
    let (name, mut args) = parse_call(call.trim())?;
    let has_start_box = args.contains_key("start_box");
    let mut box_point = |key: &str| -> Result<crate::Point, DecodeError> {
        let value = args.remove(key).ok_or(DecodeError::Missing("box"))?;
        let string = text(&value)?;
        let s = string
            .trim()
            .strip_prefix("<|box_start|>")
            .unwrap_or(string.trim());
        let s = s.strip_suffix("<|box_end|>").unwrap_or(s).trim();
        let s = s
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .or_else(|| s.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
            .ok_or(DecodeError::Malformed("box"))?;
        let values = s
            .split(',')
            .map(|n| {
                n.trim()
                    .parse::<f64>()
                    .map_err(|_| DecodeError::Invalid("box number"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (x, y) = match values.as_slice() {
            [x, y] => (*x, *y),
            [x1, y1, x2, y2] => {
                if !values
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1000.0).contains(v))
                    || x1 > x2
                    || y1 > y2
                {
                    return Err(DecodeError::Invalid("box"));
                }
                ((x1 + x2) / 2., (y1 + y2) / 2.)
            }
            _ => return Err(DecodeError::Invalid("box size")),
        };
        view.point(x, y, CoordinateSpace::Normalized1000, true)
    };
    let outcome = match name.as_str() {
        "click" | "left_single" | "double_click" | "left_double" | "right_click"
        | "right_single" => Outcome::Action(ComputerAction::Click {
            at: box_point("start_box")?,
            button: if name.starts_with("right") {
                Button::Right
            } else {
                Button::Left
            },
            count: if name.contains("double") { 2 } else { 1 },
        }),
        "drag" => Outcome::Action(ComputerAction::Drag {
            path: vec![box_point("start_box")?, box_point("end_box")?],
        }),
        "type" => Outcome::Action(ComputerAction::Type {
            text: text(
                &args
                    .remove("content")
                    .ok_or(DecodeError::Missing("content"))?,
            )?,
            at: None,
        }),
        "hotkey" | "press" => {
            let key = text(&args.remove("key").ok_or(DecodeError::Missing("key"))?)?;
            let list = if name == "press" {
                vec![Value::from(key)]
            } else {
                key.split_whitespace().map(Value::from).collect()
            };
            Outcome::Action(ComputerAction::Keypress {
                keys: keys(&Value::Array(list))?,
            })
        }
        "scroll" => {
            let at = if has_start_box {
                box_point("start_box")?
            } else {
                view.point(500., 500., CoordinateSpace::Normalized1000, true)?
            };
            let direction = text(
                &args
                    .remove("direction")
                    .ok_or(DecodeError::Missing("direction"))?,
            )?;
            let (dx, dy) = match direction.as_str() {
                "up" => (0., -5.),
                "down" => (0., 5.),
                "left" => (-5., 0.),
                "right" => (5., 0.),
                _ => return Err(DecodeError::Invalid("scroll direction")),
            };
            Outcome::Action(ComputerAction::Scroll {
                at: Some(at),
                dx,
                dy,
                unit: ScrollUnit::WheelSteps,
            })
        }
        "wait" => Outcome::Action(ComputerAction::Wait { milliseconds: 1000 }),
        "finished" | "call_user" | "refuse" => {
            let message = args
                .remove("content")
                .map(|v| text(&v))
                .transpose()?
                .unwrap_or_default();
            match name.as_str() {
                "call_user" => Outcome::AskHuman { text: message },
                "refuse" => Outcome::Refusal { text: message },
                _ => Outcome::Done { text: message },
            }
        }
        other => return Err(DecodeError::Unsupported(other.into())),
    };
    empty(&args)?;
    Ok((outcome, None, thought))
}

// Accept only a bare function name and named, quoted string literals. No AST/evaluation.
fn parse_call(raw: &str) -> Result<(String, Map<String, Value>), DecodeError> {
    let (name, body) = raw.split_once('(').ok_or(DecodeError::Malformed("call"))?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(DecodeError::Malformed("function name"));
    }
    let body = body
        .strip_suffix(')')
        .ok_or(DecodeError::Malformed("call"))?;
    let mut rest = body.trim();
    let mut args = Map::new();
    while !rest.is_empty() {
        let (key, value) = rest
            .split_once('=')
            .ok_or(DecodeError::Malformed("argument"))?;
        let key = key.trim();
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || args.contains_key(key)
            || args.len() >= 16
        {
            return Err(DecodeError::Malformed("argument name"));
        }
        let value = value.trim_start();
        let quote = value
            .chars()
            .next()
            .ok_or(DecodeError::Malformed("string"))?;
        if !matches!(quote, '\'' | '"') {
            return Err(DecodeError::Unsupported(
                "nonliteral UITARS argument".into(),
            ));
        }
        let mut chars = value.char_indices().skip(1);
        let mut decoded = String::new();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            if c == quote {
                end = Some(i + c.len_utf8());
                break;
            }
            if c == '\\' {
                let (_, escape) = chars.next().ok_or(DecodeError::Malformed("escape"))?;
                decoded.push(match escape {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    _ => return Err(DecodeError::Malformed("escape")),
                });
            } else {
                decoded.push(c);
            }
        }
        let end = end.ok_or(DecodeError::Malformed("unclosed string"))?;
        args.insert(key.into(), Value::String(decoded));
        rest = value[end..].trim();
        if !rest.is_empty() {
            rest = rest
                .strip_prefix(',')
                .ok_or(DecodeError::Malformed("argument separator"))?
                .trim();
            if rest.is_empty() {
                return Err(DecodeError::Malformed("trailing comma"));
            }
        }
    }
    Ok((name.into(), args))
}

pub(super) fn opencua(raw: &str, view: &ModelView) -> Result<Decoded, DecodeError> {
    let raw = raw.trim().strip_prefix("pyautogui.").unwrap_or(raw.trim());
    let body = raw
        .strip_prefix("click(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or(DecodeError::Malformed("click"))?;
    let mut args = Map::new();
    for field in body.split(',') {
        let (key, value) = field
            .split_once('=')
            .ok_or(DecodeError::Malformed("click argument"))?;
        let key = key.trim();
        if !matches!(key, "x" | "y") || args.contains_key(key) {
            return Err(DecodeError::Invalid("click argument"));
        }
        let value = value.trim();
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(DecodeError::Invalid("click number"));
        }
        let value = value
            .parse::<u32>()
            .map_err(|_| DecodeError::Invalid("click number"))?;
        args.insert(key.into(), Value::from(value));
    }
    let x = args
        .remove("x")
        .and_then(|v| v.as_f64())
        .ok_or(DecodeError::Missing("x"))?;
    let y = args
        .remove("y")
        .and_then(|v| v.as_f64())
        .ok_or(DecodeError::Missing("y"))?;
    empty(&args)?;
    let at = view.point(x, y, CoordinateSpace::ModelPixels, false)?;
    Ok((
        Outcome::Action(ComputerAction::Click {
            at,
            button: Button::Left,
            count: 1,
        }),
        None,
        None,
    ))
}

pub(super) fn seed(raw: &str, view: &ModelView) -> Result<Decoded, DecodeError> {
    let (before, rest) = raw
        .split_once("<seed:tool_call>")
        .ok_or(DecodeError::Missing("seed tool call"))?;
    let (body, after) = rest
        .split_once("</seed:tool_call>")
        .ok_or(DecodeError::Malformed("seed tool call"))?;
    if after.contains('<') || before.contains("<seed:tool_call>") {
        return Err(DecodeError::Unsupported("multiple seed calls".into()));
    }
    let function = body
        .trim()
        .strip_prefix("<function=")
        .ok_or(DecodeError::Malformed("function"))?;
    let (name, rest) = function
        .split_once('>')
        .ok_or(DecodeError::Malformed("function"))?;
    let mut rest = rest
        .strip_suffix("</function>")
        .ok_or(DecodeError::Malformed("function"))?
        .trim();
    let mut args = Map::new();
    while !rest.is_empty() {
        let param = rest
            .strip_prefix("<parameter=")
            .ok_or(DecodeError::Malformed("parameter"))?;
        let (key, param) = param
            .split_once('>')
            .ok_or(DecodeError::Malformed("parameter"))?;
        let (value, tail) = param
            .split_once("</parameter>")
            .ok_or(DecodeError::Malformed("parameter"))?;
        if args.len() >= 16
            || args
                .insert(key.into(), Value::String(value.into()))
                .is_some()
        {
            return Err(DecodeError::Invalid("duplicate parameter"));
        }
        rest = tail.trim();
    }
    let parse_point = |v: Value| -> Result<crate::Point, DecodeError> {
        let s = text(&v)?;
        let s = s.trim();
        let s = s
            .strip_prefix("<point>")
            .and_then(|s| s.strip_suffix("</point>"))
            .unwrap_or(s);
        let p = s
            .split_whitespace()
            .map(|s| s.parse::<f64>().map_err(|_| DecodeError::Invalid("point")))
            .collect::<Result<Vec<_>, _>>()?;
        if p.len() != 2 {
            return Err(DecodeError::Invalid("point pair"));
        }
        view.point(p[0], p[1], CoordinateSpace::Normalized1000, true)
    };
    let point =
        |args: &mut Map<String, Value>, key: &'static str| -> Result<crate::Point, DecodeError> {
            parse_point(args.remove(key).ok_or(DecodeError::Missing(key))?)
        };
    let outcome = match name {
        "click" | "left_double" | "right_single" => Outcome::Action(ComputerAction::Click {
            at: point(&mut args, "point")?,
            button: if name == "right_single" {
                Button::Right
            } else {
                Button::Left
            },
            count: if name == "left_double" { 2 } else { 1 },
        }),
        "move_to" => Outcome::Action(ComputerAction::Move {
            at: point(&mut args, "point")?,
        }),
        "drag" => Outcome::Action(ComputerAction::Drag {
            path: vec![
                point(&mut args, "start_point")?,
                point(&mut args, "end_point")?,
            ],
        }),
        "type" => Outcome::Action(ComputerAction::Type {
            text: text(
                &args
                    .remove("content")
                    .ok_or(DecodeError::Missing("content"))?,
            )?,
            at: None,
        }),
        "press" | "hotkey" => {
            let key = text(&args.remove("key").ok_or(DecodeError::Missing("key"))?)?;
            let values = if name == "hotkey" {
                key.split_whitespace()
                    .map(|s| Value::String(s.into()))
                    .collect()
            } else {
                vec![Value::String(key)]
            };
            Outcome::Action(ComputerAction::Keypress {
                keys: keys(&Value::Array(values))?,
            })
        }
        "scroll" => {
            let at = args.remove("point").map(parse_point).transpose()?;
            let direction = text(
                &args
                    .remove("direction")
                    .ok_or(DecodeError::Missing("direction"))?,
            )?;
            let (dx, dy) = match direction.as_str() {
                "up" => (0., -5.),
                "down" => (0., 5.),
                "left" => (-5., 0.),
                "right" => (5., 0.),
                _ => return Err(DecodeError::Invalid("direction")),
            };
            Outcome::Action(ComputerAction::Scroll {
                at,
                dx,
                dy,
                unit: ScrollUnit::WheelSteps,
            })
        }
        "wait" => Outcome::Action(ComputerAction::Wait { milliseconds: 1000 }),
        "take_screenshot" | "open_computer" => Outcome::Action(ComputerAction::Screenshot),
        "finished" => Outcome::Done {
            text: args
                .remove("content")
                .map(|v| text(&v))
                .transpose()?
                .unwrap_or_default(),
        },
        _ => return Err(DecodeError::Unsupported(name.into())),
    };
    empty(&args)?;
    let thought = before
        .trim()
        .strip_prefix("<gui_think>")
        .and_then(|s| s.strip_suffix("</gui_think>"))
        .map(|s| s.to_owned());
    if thought
        .as_ref()
        .is_some_and(|s| s.len() > crate::MAX_TEXT_BYTES)
    {
        return Err(DecodeError::Limit("thought"));
    }
    Ok((outcome, None, thought))
}
