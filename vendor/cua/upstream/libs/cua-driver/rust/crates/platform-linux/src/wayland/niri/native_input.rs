use anyhow::{bail, ensure, Context, Result};
use cua_driver_core::capture_runtime::{
    CaptureActionRequest, CaptureService, CaptureTarget, NativeActionDimensions,
};
use cua_driver_core::protocol::ToolResult;
use dojo_native_seat::{AgentController, InputPaused, InputState};
use niri_ipc::CapturedInputGeometry;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Observation {
    capture_id: String,
    geometries: Vec<CapturedInputGeometry>,
}

#[derive(Default)]
pub(crate) struct NativeInput {
    observations: Mutex<VecDeque<Observation>>,
    controller: Mutex<Option<AgentController>>,
}

impl NativeInput {
    pub fn record(&self, capture_id: &str, evidence: &Value) -> Result<()> {
        let revision = evidence["coordination_revision"]
            .as_u64()
            .context("capture has no native ownership revision")?;
        let compositor_pid = u32::try_from(
            evidence["compositor_pid"]
                .as_u64()
                .context("capture has no compositor identity")?,
        )?;
        let compositor_start_time = evidence["compositor_start_ticks"]
            .as_u64()
            .context("capture has no compositor generation")?;
        let output = evidence["output"]
            .as_str()
            .context("capture has no output")?
            .to_owned();
        let output_geometry = serde_json::from_value(evidence["logical_output"].clone())?;
        let windows: Vec<niri_ipc::CapturedWindow> =
            serde_json::from_value(evidence["rendered_windows"].clone())?;
        let geometries = windows
            .into_iter()
            .filter(|window| !window.blocked_out
                && window.rect.0 == window.output_clip.0
                && window.rect.1 == window.output_clip.1
                && ((window.rect.0 + window.rect.2) - (window.output_clip.0 + window.output_clip.2)).abs() <= 1e-9
                && ((window.rect.1 + window.rect.3) - (window.output_clip.1 + window.output_clip.3)).abs() <= 1e-9)
            .filter_map(|window| {
                Some(CapturedInputGeometry {
                    window_id: window.id,
                    pid: u32::try_from(window.pid?).ok()?,
                    compositor_pid,
                    compositor_start_time,
                    coordination_revision: revision,
                    output: output.clone(),
                    output_geometry,
                    rect: window.rect,
                })
            })
            .collect();
        let mut observations = self
            .observations
            .lock()
            .map_err(|_| anyhow::anyhow!("native capture store is poisoned"))?;
        ensure!(
            !observations
                .iter()
                .any(|item| item.capture_id == capture_id),
            "native capture identity already exists"
        );
        if observations.len() == 128 {
            observations.pop_front();
        }
        observations.push_back(Observation {
            capture_id: capture_id.into(),
            geometries,
        });
        Ok(())
    }

    fn observation(&self, id: &str, pid: u32, window_id: u64) -> Result<CapturedInputGeometry> {
        let observations = self
            .observations
            .lock()
            .map_err(|_| anyhow::anyhow!("native capture store is poisoned"))?;
        observations
            .iter()
            .find(|item| item.capture_id == id)
            .and_then(|item| {
                item.geometries
                    .iter()
                    .find(|geometry| geometry.pid == pid && geometry.window_id == window_id)
            })
            .cloned()
            .context("capture does not prove this native window target")
    }

    fn execute(&self, service: &CaptureService, args: &Value, operation: &str) -> Result<Value> {
        let plan = Plan::parse(operation, args)?;
        let id = args["capture_id"]
            .as_str()
            .context("Niri input requires an immutable capture_id")?;
        let binding = service.binding_from_args(args)?;
        let capture_id = service.parse_capture_id(id)?;
        let capture = service.read_for_perception(capture_id, &binding)?;
        let pid = u32::try_from(
            args["pid"]
                .as_u64()
                .context("Niri input requires an exact pid")?,
        )?;
        let window_id = args["window_id"]
            .as_u64()
            .context("Niri input requires an exact window_id")?;
        let geometry = self.observation(id, pid, window_id)?;
        let target = capture.target().clone();
        let desktop = match target {
            CaptureTarget::Window {
                pid: captured_pid,
                window_id: captured_window,
            } => {
                ensure!(
                    captured_pid == pid && captured_window == window_id,
                    "capture target mismatch"
                );
                false
            }
            CaptureTarget::PrimaryDesktop => true,
        };
        let (_, _, width, height) = super::window_geometry(window_id)?;
        ensure!(
            (f64::from(width), f64::from(height)) == (geometry.rect.2.ceil(), geometry.rect.3.ceil()),
            "native window dimensions changed after capture"
        );
        let dimensions = if desktop {
            NativeActionDimensions::new(
                geometry.output_geometry.width,
                geometry.output_geometry.height,
            )?
        } else {
            NativeActionDimensions::new(width, height)?
        };
        let (x, y) = plan.point().unwrap_or((0.0, 0.0));
        let admission = service.admit_action(CaptureActionRequest {
            capture_id,
            binding,
            target,
            current_native_action_dimensions: dimensions,
            screenshot_x: x,
            screenshot_y: y,
        })?;
        let (output_x, output_y) = if desktop {
            (admission.action_x, admission.action_y)
        } else {
            window_point(geometry.rect, (width, height), (admission.action_x, admission.action_y))
        };
        if plan.point().is_some() {
            ensure!(
                output_x >= geometry.rect.0
                    && output_y >= geometry.rect.1
                    && output_x < geometry.rect.0 + geometry.rect.2
                    && output_y < geometry.rect.1 + geometry.rect.3,
                "input point lies outside the capture-proven target"
            );
        }
        let mut controller = self
            .controller
            .lock()
            .map_err(|_| anyhow::anyhow!("native actor is poisoned"))?;
        if controller.is_none() {
            *controller = Some(AgentController::connect("cua-driver")?);
        }
        let actor = controller.as_mut().unwrap();
        actor.use_coordination_revision(geometry.coordination_revision);
        actor.use_capture_geometry(geometry.clone());
        let result = plan.deliver(actor, &geometry, output_x, output_y);
        if let Err(error) = result {
            let _ = actor.reset();
            controller.take();
            return Err(error);
        }
        Ok(
            json!({"path":"dojo_native_seat","effect":"delivered","capture_id":id,"window_id":window_id,"pid":pid,"peer_id":actor.peer_id()}),
        )
    }
}

fn window_point(rect: (f64, f64, f64, f64), dimensions: (u32, u32), point: (f64, f64)) -> (f64, f64) {
    (rect.0 + point.0 * rect.2 / f64::from(dimensions.0),
     rect.1 + point.1 * rect.3 / f64::from(dimensions.1))
}

pub(crate) async fn invoke(
    native: Arc<NativeInput>,
    service: Arc<CaptureService>,
    args: Value,
    operation: &'static str,
) -> ToolResult {
    use cua_driver_core::action_record::{ActionEffect, ActionExecutionRecord, ActionTransport, ActualDelivery, RequestedDelivery};
    match tokio::task::spawn_blocking(move || native.execute(&service, &args, operation)).await {
        Ok(Ok(evidence)) => {
            ToolResult::text(format!("Native {operation} delivered; UI effect remains unverified. {evidence}"))
                .with_structured(evidence)
                .with_action_record(ActionExecutionRecord::builder(ActionEffect::Unverifiable,
                    ActionTransport::LinuxCuaCompositorInject, RequestedDelivery::Background)
                    .actual_delivery(ActualDelivery::Background).build().expect("native delivery record is valid"))
        }
        Ok(Err(error)) => {
            let mut evidence = json!({"code":"native_input_refused","effect":"refused","retry_automatically":false});
            if let Some(paused) = error.downcast_ref::<InputPaused>() {
                evidence["code"] = json!("input_paused");
                evidence["coordination"] = json!(paused.coordination);
            } else if error
                .downcast_ref::<cua_driver_core::capture_runtime::CaptureLookupError>()
                .is_some()
                || error
                    .downcast_ref::<cua_driver_core::capture_runtime::CaptureActionError>()
                    .is_some()
                || error
                    .downcast_ref::<cua_driver_core::capture_runtime::CaptureIdParseError>()
                    .is_some()
            {
                evidence["code"] = json!(cua_driver_core::capture_runtime::admission_error_code(
                    &error
                ));
            }
            let record = if evidence["code"] == "native_input_refused" || evidence["code"] == "input_paused" {
                evidence["effect"] = json!("unknown");
                ActionExecutionRecord::builder(ActionEffect::Unverifiable, ActionTransport::LinuxCuaCompositorInject, RequestedDelivery::Background)
                    .actual_delivery(ActualDelivery::Unknown).build()
            } else {
                ActionExecutionRecord::builder(ActionEffect::Refused, ActionTransport::LinuxCuaCompositorInject, RequestedDelivery::Background).build()
            }.expect("native failure record is valid");
            ToolResult::error(error.to_string()).with_structured(evidence).with_action_record(record)
        }
        Err(error) => ToolResult::error(error.to_string()).with_action_record(
            ActionExecutionRecord::builder(ActionEffect::Unverifiable,
                ActionTransport::LinuxCuaCompositorInject, RequestedDelivery::Background)
                .actual_delivery(ActualDelivery::Unknown).build().expect("unknown native execution record is valid")),
    }
}

#[derive(Debug)]
enum Plan {
    Click {
        x: f64,
        y: f64,
        button: u32,
    },
    Scroll {
        x: f64,
        y: f64,
        horizontal: f64,
        vertical: f64,
    },
    Key(Vec<u32>),
    Text(Vec<(u32, bool)>),
    Focus,
}

impl Plan {
    fn parse(operation: &str, args: &Value) -> Result<Self> {
        ensure!(
            args.get("element_token").is_none() && args.get("element_index").is_none(),
            "native semantic element input is unavailable"
        );
        ensure!(
            args.get("delivery_mode")
                .and_then(Value::as_str)
                .is_none_or(|value| value == "auto" || value == "background"),
            "native input never activates the human seat"
        );
        let point = || -> Result<(f64, f64)> {
            let x = args["x"]
                .as_f64()
                .context("native pointer input requires x")?;
            let y = args["y"]
                .as_f64()
                .context("native pointer input requires y")?;
            ensure!(x.is_finite() && y.is_finite(), "input point is not finite");
            Ok((x, y))
        };
        match operation {
            "click" => {
                ensure!(
                    args["count"].as_u64().unwrap_or(1) == 1,
                    "native click trains are unavailable"
                );
                ensure!(
                    args["from_zoom"].as_bool() != Some(true),
                    "native clicks require the original capture frame"
                );
                ensure!(
                    args.get("modifier").is_none(),
                    "modified native clicks are unavailable"
                );
                let button = match args["button"].as_str().unwrap_or("left") {
                    "left" => 272,
                    "right" => 273,
                    "middle" => 274,
                    _ => bail!("unknown mouse button"),
                };
                let (x, y) = point()?;
                Ok(Self::Click { x, y, button })
            }
            "scroll" => {
                let (x, y) = point()?;
                let amount = args["amount"].as_u64().unwrap_or(3);
                ensure!(
                    (1..=50).contains(&amount),
                    "scroll amount must be between 1 and 50"
                );
                let delta = amount as f64 * 15.0;
                let (horizontal, vertical) = match args["direction"]
                    .as_str()
                    .context("scroll requires direction")?
                {
                    "up" => (0.0, -delta),
                    "down" => (0.0, delta),
                    "left" => (-delta, 0.0),
                    "right" => (delta, 0.0),
                    _ => bail!("unknown scroll direction"),
                };
                Ok(Self::Scroll {
                    x,
                    y,
                    horizontal,
                    vertical,
                })
            }
            "press_key" => {
                let key = args["key"].as_str().context("press_key requires key")?;
                let mut keys = Vec::new();
                if let Some(modifiers) = args["modifiers"].as_array() {
                    for modifier in modifiers {
                        let modifier = modifier.as_str().context("modifier must be a string")?;
                        ensure!(
                            [
                                "ctrl", "control", "shift", "alt", "super", "meta", "cmd",
                                "command", "win", "windows"
                            ]
                            .contains(&modifier),
                            "unsupported modifier"
                        );
                        keys.push(
                            crate::wayland::key_to_evdev(modifier).context("unknown modifier")?,
                        );
                    }
                }
                for part in key.split('+') {
                    keys.push(crate::wayland::key_to_evdev(part).context("unknown native key")?);
                }
                Ok(Self::Key(keys))
            }
            "type_text" => {
                ensure!(
                    args.get("x").is_none() && args.get("y").is_none(),
                    "native text requires a previously focused target"
                );
                let text = args["text"].as_str().context("type_text requires text")?;
                let text = cua_driver_core::text_sanitize::strip_trailing_agent_protocol_tags(text);
                ensure!(
                    text.chars().count() <= 4096,
                    "native text exceeds 4096 characters"
                );
                Ok(Self::Text(
                    text.chars().map(text_key).collect::<Result<_>>()?,
                ))
            }
            "bring_to_front" => Ok(Self::Focus),
            _ => bail!("native operation is unavailable"),
        }
    }
    fn point(&self) -> Option<(f64, f64)> {
        match self {
            Self::Click { x, y, .. } | Self::Scroll { x, y, .. } => Some((*x, *y)),
            _ => None,
        }
    }
    fn deliver(
        &self,
        actor: &mut AgentController,
        geometry: &CapturedInputGeometry,
        x: f64,
        y: f64,
    ) -> Result<()> {
        let window = geometry.window_id;
        match self {
            Self::Click { button, .. } => {
                actor.move_pointer_for_window(window, &geometry.output, x, y)?;
                actor.button_for_window(window, *button, InputState::Pressed)?;
                actor.button_for_window(window, *button, InputState::Released)
            }
            Self::Scroll {
                horizontal,
                vertical,
                ..
            } => {
                actor.move_pointer_for_window(window, &geometry.output, x, y)?;
                actor.scroll_for_window(window, *horizontal, *vertical)
            }
            Self::Key(keys) => {
                actor.focus_window(window)?;
                for key in keys {
                    actor.key_for_window(window, *key, InputState::Pressed)?;
                }
                for key in keys.iter().rev() {
                    actor.key_for_window(window, *key, InputState::Released)?;
                }
                Ok(())
            }
            Self::Text(keys) => {
                actor.focus_window(window)?;
                for (key, shift) in keys {
                    if *shift {
                        actor.text_key_for_window(window, 42, InputState::Pressed)?;
                    }
                    actor.text_key_for_window(window, *key, InputState::Pressed)?;
                    actor.text_key_for_window(window, *key, InputState::Released)?;
                    if *shift {
                        actor.text_key_for_window(window, 42, InputState::Released)?;
                    }
                }
                Ok(())
            }
            Self::Focus => actor.focus_window(window),
        }
    }
}

fn text_key(character: char) -> Result<(u32, bool)> {
    if character.is_ascii_alphabetic() {
        return Ok((
            crate::wayland::key_to_evdev(&character.to_ascii_lowercase().to_string()).unwrap(),
            character.is_ascii_uppercase(),
        ));
    }
    if character.is_ascii_digit() {
        return Ok((
            crate::wayland::key_to_evdev(&character.to_string()).unwrap(),
            false,
        ));
    }
    let pair = match character {
        ' ' => (57,false), '\n' => (28,false), '\t' => (15,false),
        '-' => (12,false),'_' => (12,true),'=' => (13,false),'+' => (13,true),
        '[' => (26,false),'{' => (26,true),']' => (27,false),'}' => (27,true),
        ';' => (39,false),':' => (39,true),'\'' => (40,false),'"' => (40,true),
        '`' => (41,false),'~' => (41,true),'\\' => (43,false),'|' => (43,true),
        ',' => (51,false),'<' => (51,true),'.' => (52,false),'>' => (52,true),'/' => (53,false),'?' => (53,true),
        '!' => (2,true),'@' => (3,true),'#' => (4,true),'$' => (5,true),'%' => (6,true),
        '^' => (7,true),'&' => (8,true),'*' => (9,true),'(' => (10,true),')' => (11,true),
        _ => bail!("native text supports plain US keyboard characters; unsupported character {character:?}"),
    };
    Ok(pair)
}

#[cfg(test)]
mod tests {
    #[test]
    fn rounded_image_dimensions_map_to_exact_fractional_window_geometry() {
        assert_eq!(super::window_point((10.0,20.0,1.5,2.25),(2,3),(1.0,1.5)),(10.75,21.125));
    }

    use super::*;
    use cua_driver_core::capture_runtime::{
        CapturePublication, EncodedScreenshotDimensions, ScreenshotToActionTransform,
    };

    #[test]
    fn unsupported_text_is_rejected_before_any_delivery() {
        assert!(Plan::parse("type_text", &json!({"text":"abλ"})).is_err());
        assert!(Plan::parse("click", &json!({"x":1,"y":2,"element_token":"1:1"})).is_err());
        assert!(Plan::parse(
            "press_key",
            &json!({"key":"a","delivery_mode":"foreground"})
        )
        .is_err());
        assert!(Plan::parse("click", &json!({"x":1,"y":2,"modifier":"ctrl"})).is_err());
    }

    #[test]
    fn plans_preserve_key_chords_and_plain_us_text() {
        match Plan::parse("press_key", &json!({"key":"ctrl+a"})).unwrap() {
            Plan::Key(keys) => assert_eq!(keys, vec![29, 30]),
            _ => panic!("expected key plan"),
        }
        match Plan::parse("type_text", &json!({"text":"A!?\n"})).unwrap() {
            Plan::Text(keys) => {
                assert_eq!(keys, vec![(30, true), (2, true), (53, true), (28, false)])
            }
            _ => panic!("expected text plan"),
        }
    }

    #[test]
    fn cross_session_capture_cannot_create_a_native_actor() {
        let service = CaptureService::default();
        let binding = service.current_binding("first").unwrap();
        let mut image = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(1, 1)
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
        let capture = service
            .publish(CapturePublication {
                png_bytes: image.into_inner(),
                target: CaptureTarget::Window {
                    pid: 1,
                    window_id: 2,
                },
                encoded_dimensions: EncodedScreenshotDimensions::new(1, 1).unwrap(),
                native_action_dimensions: NativeActionDimensions::new(1, 1).unwrap(),
                screenshot_to_action: ScreenshotToActionTransform::identity(),
                session_id: "first".into(),
                session_generation: binding.session_generation(),
            })
            .unwrap();
        let native = NativeInput::default();
        let error = native.execute(&service,&json!({"capture_id":capture.to_string(),"pid":1,"window_id":2,"x":0,"y":0,"_session_id":"second"}),"click").unwrap_err();
        assert!(matches!(
            error.downcast_ref::<cua_driver_core::capture_runtime::CaptureLookupError>(),
            Some(cua_driver_core::capture_runtime::CaptureLookupError::GenerationMismatch)
        ));
        assert!(native.controller.lock().unwrap().is_none());
        assert!(service.read_for_perception(capture, &binding).is_ok());
    }

    #[test]
    fn native_plans_share_one_geometry_bound_connection_and_drop_releases_it() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = events.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                let reply = if request.pointer("/AgentInput/Create").is_some() {
                    json!({"AgentInputCreated":{"seat_name":"agent-fixture","peer_id":7}})
                } else if request == json!("InputCoordination") {
                    json!({"InputCoordination":{"revision":13,"human_owned_windows":[]}})
                } else {
                    assert_eq!(
                        request.pointer("/AgentInput/Coordinated/revision"),
                        Some(&json!(13))
                    );
                    assert_eq!(
                        request
                            .pointer("/AgentInput/Coordinated/command/Captured/geometry/window_id"),
                        Some(&json!(9))
                    );
                    recorded.lock().unwrap().push(
                        request["AgentInput"]["Coordinated"]["command"]["Captured"]["command"]
                            .clone(),
                    );
                    json!("Handled")
                };
                writeln!(stream, "{}", json!({"Ok":reply})).unwrap();
            }
        });
        let geometry:CapturedInputGeometry=serde_json::from_value(json!({
            "window_id":9,"pid":1,"compositor_pid":2,"compositor_start_time":3,"coordination_revision":13,
            "output":"DP-1","output_geometry":{"x":0,"y":0,"width":100,"height":100,"scale":1.0,"transform":"Normal"},"rect":[0.0,0.0,50.0,50.0]
        })).unwrap();
        let mut actor = AgentController::connect_to(&path, "fixture").unwrap();
        actor.use_coordination_revision(13);
        actor.use_capture_geometry(geometry.clone());
        for (operation, args) in [
            ("click", json!({"x":1,"y":2})),
            ("scroll", json!({"x":1,"y":2,"direction":"down","amount":2})),
            ("press_key", json!({"key":"enter"})),
            ("type_text", json!({"text":"a"})),
            ("bring_to_front", json!({})),
        ] {
            Plan::parse(operation, &args)
                .unwrap()
                .deliver(&mut actor, &geometry, 1.0, 2.0)
                .unwrap();
            assert_eq!(actor.peer_id(), 7);
        }
        drop(actor);
        server.join().unwrap();
        let events = events.lock().unwrap();
        assert_eq!(
            events[0],
            json!({"Targeted":{"window_id":9,"command":{"Move":{"output":"DP-1","x":1.0,"y":2.0}}}})
        );
        assert_eq!(
            events[1],
            json!({"Targeted":{"window_id":9,"command":{"Button":{"button":272,"state":"Pressed"}}}})
        );
        assert_eq!(
            events[2],
            json!({"Targeted":{"window_id":9,"command":{"Button":{"button":272,"state":"Released"}}}})
        );
        assert!(events
            .iter()
            .any(|event| event.pointer("/Targeted/command/TextKey/evdev") == Some(&json!(30))));
        assert_eq!(events.last().unwrap(), &json!({"Focus":{"window_id":9}}));
    }
}
