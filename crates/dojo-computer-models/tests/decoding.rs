use dojo_computer_models::*;
fn view(profile: Profile) -> ModelView {
    ModelView {
        view_id: "view-one".into(),
        capture_id: "capture-one".into(),
        capture_digest: "a".repeat(64),
        session_id: "one".into(),
        runtime_generation: 1,
        geometry_generation: 1,
        profile,
        parser_version: PARSER_VERSION.into(),
        preprocessing: "crop-and-scale/1".into(),
        model_width: 400,
        model_height: 200,
        delivered_width: 1000,
        delivered_height: 500,
        scale_x: 2.,
        scale_y: 2.,
        offset_x: 100.,
        offset_y: 50.,
    }
}
#[test]
fn uitars_zero_point_and_non_square_transform_are_preserved() {
    let step = decode(
        Profile::UiTars,
        "Thought: select origin\nAction: click(start_box='(0,0)')",
        &view(Profile::UiTars),
    )
    .unwrap();
    let Outcome::Action(ComputerAction::Click { at, button, count }) = step.outcome else {
        panic!("click")
    };
    assert_eq!(
        (at.model_x, at.model_y, at.delivered_x, at.delivered_y),
        (0., 0., 100., 50.)
    );
    assert_eq!((button, count), (Button::Left, 1));
    assert_eq!(step.thought.as_deref(), Some("select origin"));
}
#[test]
fn qwen_box_center_is_not_assumed_and_ties_round_even() {
    let step=decode(Profile::Qwen35,r#"<tool_call>{"name":"computer","arguments":{"action":"left_click","coordinate":[6.25,12.5]}}</tool_call>"#,&view(Profile::Qwen35)).unwrap();
    let Outcome::Action(ComputerAction::Click { at, .. }) = step.outcome else {
        panic!("click")
    };
    assert_eq!(
        (at.model_x, at.model_y, at.delivered_x, at.delivered_y),
        (2., 2., 104., 54.)
    );
}
#[test]
fn uitars_box_center_floors_each_axis() {
    let step = decode(
        Profile::UiTars,
        "Action: double_click(start_box='(100,200,300,400)')",
        &view(Profile::UiTars),
    )
    .unwrap();
    let Outcome::Action(ComputerAction::Click { at, count, .. }) = step.outcome else {
        panic!("click")
    };
    assert_eq!((at.model_x, at.model_y, count), (80., 60., 2));
}
#[test]
fn qwen_xml_and_json_match() {
    let a=decode(Profile::Qwen35,"<tool_call><function=computer><parameter=type>mouse_move</parameter><parameter=coordinate>[500,250]</parameter></function></tool_call>",&view(Profile::Qwen35)).unwrap();
    let b=decode(Profile::Qwen35,r#"<tool_call>{"name":"computer","arguments":{"action":"mouse_move","coordinate":[500,250]}}</tool_call>"#,&view(Profile::Qwen35)).unwrap();
    assert_eq!(a.outcome, b.outcome);
}
#[test]
fn targeted_type_drag_and_call_identity_survive_normalization() {
    let step = decode(
        Profile::Canonical,
        r#"{"action":"type","text":"a\nb","x":0,"y":1,"call_id":"call-one"}"#,
        &view(Profile::Canonical),
    )
    .unwrap();
    assert_eq!(step.call_id.as_deref(), Some("call-one"));
    let Outcome::Action(ComputerAction::Type { text, at: Some(at) }) = step.outcome else {
        panic!("targeted type")
    };
    assert_eq!(text, "a\nb");
    assert_eq!(at.delivered_y, 52.);
    let step = decode(
        Profile::UiTars,
        "Action: drag(start_box='(0,0)', end_box='(500,500)')",
        &view(Profile::UiTars),
    )
    .unwrap();
    let Outcome::Action(ComputerAction::Drag { path }) = step.outcome else {
        panic!("drag")
    };
    assert_eq!(path.len(), 2);
    assert_eq!((path[1].delivered_x, path[1].delivered_y), (500., 250.));
}
#[test]
fn outcomes_are_distinct_and_do_not_disappear() {
    assert_eq!(
        decode(
            Profile::UiTars,
            "Action: call_user(content='choose')",
            &view(Profile::UiTars)
        )
        .unwrap()
        .outcome,
        Outcome::AskHuman {
            text: "choose".into()
        }
    );
    assert_eq!(
        decode(
            Profile::Canonical,
            r#"{"action":"refusal","text":"denied"}"#,
            &view(Profile::Canonical)
        )
        .unwrap()
        .outcome,
        Outcome::Refusal {
            text: "denied".into()
        }
    );
    assert_eq!(decode(Profile::Qwen35,r#"<tool_call>{"name":"computer","arguments":{"action":"answer","text":"done"}}</tool_call>"#,&view(Profile::Qwen35)).unwrap().outcome,Outcome::Done{text:"done".into()});
}
#[test]
fn keys_wait_and_scroll_units_are_explicit() {
    assert_eq!(
        decode(
            Profile::UiTars,
            "Action: hotkey(key='ctrl shift a')",
            &view(Profile::UiTars)
        )
        .unwrap()
        .outcome,
        Outcome::Action(ComputerAction::Keypress {
            keys: vec!["CTRL".into(), "SHIFT".into(), "a".into()]
        })
    );
    assert_eq!(decode(Profile::Qwen35,r#"<tool_call>{"name":"computer","arguments":{"action":"wait","time":0.25}}</tool_call>"#,&view(Profile::Qwen35)).unwrap().outcome,Outcome::Action(ComputerAction::Wait{milliseconds:250}));
    let step = decode(
        Profile::UiTars,
        "Action: scroll(direction='up')",
        &view(Profile::UiTars),
    )
    .unwrap();
    let Outcome::Action(ComputerAction::Scroll { dx, dy, unit, .. }) = step.outcome else {
        panic!("scroll")
    };
    assert_eq!((dx, dy, unit), (0., -5., ScrollUnit::WheelSteps));
}
#[test]
fn code_nonliteral_boxes_and_multiple_actions_refuse() {
    for raw in [
        "Action: __import__('os').system('touch /tmp/dojo-test')",
        "Action: click(start_box=__import__('os'))",
        "Action: click(start_box='(0,0)'); wait()",
        "Action: click(start_box='(False,0)')",
        "Action: click(start_box='(NaN,0)')",
        "Action: click(start_box='(-1,0)')",
        "Action: click(start_box='(1000,1000)')",
    ] {
        assert!(
            decode(Profile::UiTars, raw, &view(Profile::UiTars)).is_err(),
            "{raw}"
        );
    }
    assert!(decode(Profile::Qwen35,r#"<tool_call>{"name":"computer","arguments":{"action":"wait"}}</tool_call><tool_call>{"name":"computer","arguments":{"action":"wait"}}</tool_call>"#,&view(Profile::Qwen35)).is_err());
}
#[test]
fn missing_duplicate_unknown_and_boolean_fields_refuse() {
    for raw in [
        r#"{"action":"click"}"#,
        r#"{"action":"move","x":false,"y":0}"#,
        r#"{"action":"wait","action":"click"}"#,
        r#"{"action":"wait","milliseconds":1,"execute":"code"}"#,
        r#"{"action":"drag","path":[{"x":0,"x":1,"y":0},{"x":1,"y":1}]}"#,
    ] {
        assert!(
            decode(Profile::Canonical, raw, &view(Profile::Canonical)).is_err(),
            "{raw}"
        );
    }
    assert!(decode(Profile::Qwen35,"<tool_call><function=computer><parameter=type>wait</parameter><parameter=action>wait</parameter></function></tool_call>",&view(Profile::Qwen35)).is_err());
}
#[test]
fn binding_limits_and_pixel_edges_refuse() {
    let mut invalid = view(Profile::Canonical);
    invalid.scale_x = 3.;
    assert!(matches!(
        decode(Profile::Canonical, r#"{"action":"screenshot"}"#, &invalid),
        Err(DecodeError::Binding)
    ));
    assert!(
        decode(
            Profile::Canonical,
            r#"{"action":"move","x":400,"y":0}"#,
            &view(Profile::Canonical)
        )
        .is_err()
    );
    assert!(
        decode(
            Profile::Canonical,
            r#"{"action":"wait","milliseconds":30001}"#,
            &view(Profile::Canonical)
        )
        .is_err()
    );
    assert!(
        decode(
            Profile::Canonical,
            &" ".repeat(MAX_RAW_BYTES + 1),
            &view(Profile::Canonical)
        )
        .is_err()
    );
}
#[test]
fn generic_json_and_qwen_scroll_keep_their_unit_and_direction() {
    let step=decode(Profile::GenericVlm,r#"<tool_call>{"name":"computer","arguments":{"action":"scroll","pixels":12,"coordinate":[500,500]}}</tool_call>"#,&view(Profile::GenericVlm)).unwrap();
    let Outcome::Action(ComputerAction::Scroll {
        at: Some(at),
        dx,
        dy,
        unit,
    }) = step.outcome
    else {
        panic!("scroll")
    };
    assert_eq!(
        (at.model_x, at.model_y, dx, dy, unit),
        (200., 100., 0., -12., ScrollUnit::Pixels)
    );
    assert!(
        decode(
            Profile::GenericVlm,
            "<tool_call><function=computer><parameter=type>wait</parameter></function></tool_call>",
            &view(Profile::GenericVlm)
        )
        .is_err()
    );
}
#[test]
fn modifier_parameters_and_invalid_buttons_are_not_silently_discarded() {
    for raw in [
        r#"<tool_call>{"name":"computer","arguments":{"action":"left_click","coordinate":[500,500],"text":"ctrl"}}</tool_call>"#,
        r#"<tool_call>{"name":"computer","arguments":{"action":"left_click_drag","coordinate":[500,500]}}</tool_call>"#,
        r#"<tool_call>{"name":"computer","arguments":{"action":"triple_click","coordinate":[500,500]}}</tool_call>"#,
    ] {
        assert!(decode(Profile::Qwen35, raw, &view(Profile::Qwen35)).is_err());
    }
    assert!(
        decode(
            Profile::Canonical,
            r#"{"action":"click","x":0,"y":0,"button":false}"#,
            &view(Profile::Canonical)
        )
        .is_err()
    );
}

#[test]
fn native_blocks_keep_safety_signatures_and_call_identity() {
    let raw = r#"{"type":"computer_call","call_id":"call-a","id":"native-a","action":{"type":"click","x":0,"y":0,"button":"left"},"pending_safety_checks":[{"id":"check-a","code":"sensitive","message":"approval required"}]}"#;
    let step = decode(Profile::OpenAiNative, raw, &view(Profile::OpenAiNative)).unwrap();
    assert_eq!(step.call_id.as_deref(), Some("call-a"));
    assert_eq!(
        step.provider_metadata.unwrap()["pending_safety_checks"][0]["id"],
        "check-a"
    );
    let raw = r#"{"functionCall":{"name":"click_at","args":{"x":250,"y":500},"id":"gemini-a"},"thoughtSignature":"signed-value"}"#;
    let step = decode(Profile::Gemini, raw, &view(Profile::Gemini)).unwrap();
    let Outcome::Action(ComputerAction::Click { at, .. }) = step.outcome else {
        panic!("click")
    };
    assert_eq!((at.model_x, at.model_y), (100., 100.));
    assert_eq!(
        step.provider_metadata.unwrap()["thoughtSignature"],
        "signed-value"
    );
}
#[test]
fn anthropic_native_preserves_wheel_units_and_axis_restoration() {
    let raw = r#"{"type":"tool_use","id":"a","name":"computer","input":{"action":"scroll","coordinate":[20,10],"scroll_direction":"up","scroll_amount":3}}"#;
    let step = decode(Profile::Anthropic, raw, &view(Profile::Anthropic)).unwrap();
    let Outcome::Action(ComputerAction::Scroll {
        at: Some(at),
        dx,
        dy,
        unit,
    }) = step.outcome
    else {
        panic!("scroll")
    };
    assert_eq!(
        (at.delivered_x, at.delivered_y, dx, dy, unit),
        (140., 70., 0., -3., ScrollUnit::WheelSteps)
    );
    let key=decode(Profile::Anthropic,r#"{"type":"tool_use","id":"a","name":"computer","input":{"action":"key","text":"ctrl+Return"}}"#,&view(Profile::Anthropic)).unwrap();
    assert_eq!(
        key.outcome,
        Outcome::Action(ComputerAction::Keypress {
            keys: vec!["CTRL".into(), "ENTER".into()]
        })
    );
}
#[test]
fn native_missing_targets_compound_effects_code_and_unknown_fields_refuse() {
    for raw in [
        r#"{"functionCall":{"name":"click_at","args":{}}}"#,
        r#"{"functionCall":{"name":"type_text_at","args":{"x":0,"y":0,"text":"hi","press_enter":true}}}"#,
        r#"{"functionCall":{"name":"playwright_exec","args":{"code":"evil()"}}}"#,
        r#"{"functionCall":{"name":"click_at","args":{"x":0,"y":0,"mystery":1}}}"#,
    ] {
        assert!(decode(Profile::Gemini, raw, &view(Profile::Gemini)).is_err());
    }
    let step = decode(
        Profile::OpenAiNative,
        r#"{"type":"refusal","refusal":"cannot proceed"}"#,
        &view(Profile::OpenAiNative),
    )
    .unwrap();
    assert_eq!(
        step.outcome,
        Outcome::Refusal {
            text: "cannot proceed".into()
        }
    );
}
#[test]
fn remaining_dialects_are_literal_and_preserve_zero() {
    for (profile, raw) in [
        (
            Profile::UiTars2,
            "<gui_think>origin</gui_think><seed:tool_call><function=click><parameter=point><point>0 0</point></parameter></function></seed:tool_call>",
        ),
        (Profile::OpenCua, "pyautogui.click(x=0, y=0)"),
        (
            Profile::Fara,
            r#"<tool_call>{"name":"computer_use","arguments":{"action":"left_click","coordinate":[0,0]}}</tool_call>"#,
        ),
    ] {
        let step = decode(profile, raw, &view(profile)).unwrap();
        let Outcome::Action(ComputerAction::Click { at, .. }) = step.outcome else {
            panic!("click")
        };
        assert_eq!((at.model_x, at.model_y), (0., 0.));
    }
    for raw in [
        "click(x=0,y=0);system('bad')",
        "click(x=0,y=0) click(x=1,y=1)",
    ] {
        assert!(decode(Profile::OpenCua, raw, &view(Profile::OpenCua)).is_err());
    }
    let raw = "<seed:tool_call><function=click><parameter=point>1 1</parameter><parameter=point>2 2</parameter></function></seed:tool_call>";
    assert!(decode(Profile::UiTars2, raw, &view(Profile::UiTars2)).is_err());
}
#[test]
fn native_wait_and_endpoint_drag_keep_semantics() {
    let step = decode(
        Profile::Gemini,
        r#"{"functionCall":{"name":"wait_5_seconds","args":{}}}"#,
        &view(Profile::Gemini),
    )
    .unwrap();
    assert_eq!(
        step.outcome,
        Outcome::Action(ComputerAction::Wait { milliseconds: 5000 })
    );
    let step=decode(Profile::Gemini,r#"{"functionCall":{"name":"drag_and_drop","args":{"x":0,"y":0,"destination_x":500,"destination_y":500}}}"#,&view(Profile::Gemini)).unwrap();
    let Outcome::Action(ComputerAction::Drag { path }) = step.outcome else {
        panic!("drag")
    };
    assert_eq!(
        (
            path[0].model_x,
            path[0].model_y,
            path[1].model_x,
            path[1].model_y
        ),
        (0., 0., 200., 100.)
    );
}
