use dojo_computer_models::{DecodeError, policy::*, preprocessing::*, s1::*};
fn element(token: &str, index: i64, role: &str, label: &str) -> Element {
    Element {
        element_token: token.into(),
        index,
        role: role.into(),
        label: label.into(),
        value: String::new(),
        actions: vec![],
        checked: None,
    }
}
fn decision(token: &str, action: Action, p: f64) -> Decision {
    Decision {
        element_token: token.into(),
        action,
        probability: p,
        distribution: vec![],
    }
}
#[test]
fn s1_options_order_confidence_and_single_submit() {
    let entities = vec![Entity {
        label: "name".into(),
        value: "Ada".into(),
    }];
    assert_eq!(
        render_options(&entities).unwrap(),
        vec!["fill name: Ada", "check", "click", "skip"]
    );
    for index in 0..4 {
        assert_eq!(
            encode_option(decode_option(index, &entities).unwrap(), &entities).unwrap(),
            index
        );
    }
    assert!(decode_option(4, &entities).is_err());
    let elements = vec![
        element("a", 1, "TextField", "Name"),
        element("b", 2, "CheckBox", "Consent"),
        element("c", 3, "Button", "Submit"),
        element("d", 4, "Button", "submit form"),
    ];
    let decisions = vec![
        decision("d", Action::Click, 0.8),
        decision("b", Action::Check, 0.9),
        decision("c", Action::Click, 0.95),
        decision("a", Action::Fill(0), 0.6),
    ];
    assert_eq!(
        order_decisions(&elements, &entities, &decisions, 0.5, true)
            .unwrap()
            .iter()
            .map(|d| d.element_token.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert_eq!(
        order_decisions(&elements, &entities, &decisions, 0.7, false).unwrap(),
        vec![decision("b", Action::Check, 0.9)]
    );
    for p in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert!(order_decisions(&elements, &entities, &decisions, p, true).is_err());
    }
    let mut missing = decisions.clone();
    missing.pop();
    assert!(validate_plan(&elements, &entities, &missing).is_err());
    let mut duplicate = decisions.clone();
    duplicate[0] = duplicate[1].clone();
    assert!(validate_plan(&elements, &entities, &duplicate).is_err());
}
#[test]
fn unicode_context_uses_characters_but_collator_uses_utf8_bytes() {
    let mut e = element("a", 1, "TextField", &"é".repeat(73));
    e.value = "界".repeat(49);
    let context = render_context(&"界".repeat(65), &e, "");
    assert!(context.contains(&format!("FORM {}\n", "界".repeat(64))));
    assert!(context.contains(&format!("value=\"{}\"", "界".repeat(48))));
    let batch = collate_bytes(
        &[
            ByteExample {
                context: "é".into(),
                options: vec!["a".into(), "".into()],
            },
            ByteExample {
                context: "".into(),
                options: vec!["bc".into()],
            },
        ],
        1,
        2,
    )
    .unwrap();
    assert_eq!(batch.context_ids, vec![vec![196], vec![0]]);
    assert_eq!(batch.context_mask, vec![vec![true], vec![false]]);
    assert_eq!(
        batch.option_ids,
        vec![
            vec![vec![98, 0], vec![0, 0]],
            vec![vec![99, 100], vec![0, 0]]
        ]
    );
    assert_eq!(batch.option_mask, vec![vec![true, true], vec![true, false]]);
    assert_eq!(normalize_title("Form - Google Chrome"), "Form");
}
#[test]
fn snapshot_policy_refuses_ambiguity_unknown_checkbox_and_missing_capability() {
    let e = element("token", 1, "CheckBox", "Agree");
    let mut snapshot = Snapshot {
        snapshot_id: "snapshot".into(),
        window_id: 1,
        pid: 1,
        process_start_time: 5,
        session_id: "s".into(),
        runtime_generation: 1,
        complete: true,
        elements: vec![e],
    };
    let d = decision("token", Action::Check, 0.9);
    assert!(mutation(&snapshot, &d, &[], false).is_err());
    snapshot.elements[0].checked = Some(true);
    assert_eq!(
        mutation(&snapshot, &d, &[], false).unwrap(),
        Mutation::AlreadySatisfied
    );
    snapshot.elements[0].checked = Some(false);
    assert_eq!(
        mutation(&snapshot, &d, &[], false).unwrap(),
        Mutation::Click {
            element_token: "token".into()
        }
    );
    assert!(
        mutation(
            &snapshot,
            &decision("token", Action::Fill(0), 0.9),
            &[Entity {
                label: "a".into(),
                value: "b".into()
            }],
            false
        )
        .is_err()
    );
    snapshot.elements.push(snapshot.elements[0].clone());
    assert!(snapshot.resolve(&d).is_err());
    snapshot.elements.pop();
    snapshot.complete = false;
    assert!(snapshot.resolve(&d).is_err());
    for effect in [Effect::Confirmed, Effect::Unknown, Effect::Refused] {
        assert!(requires_observation_after_dispatch(effect));
    }
}
#[test]
fn resizing_records_axis_mapping_and_rejects_invalid_bounds() {
    let p = plan(
        1000,
        500,
        ResizePolicy::Area {
            minimum: 100,
            maximum: 125000,
        },
    )
    .unwrap();
    assert_eq!(
        (p.width, p.height, p.restore_x, p.restore_y),
        (500, 250, 2., 2.)
    );
    let p = plan(
        100,
        100,
        ResizePolicy::Factor {
            factor: 28,
            minimum: 3136,
            maximum: 12845056,
        },
    )
    .unwrap();
    assert_eq!((p.width, p.height), (112, 112));
    let p = plan(
        1000,
        500,
        ResizePolicy::Fit {
            maximum_width: 800,
            maximum_height: 600,
        },
    )
    .unwrap();
    assert_eq!((p.width, p.height), (800, 400));
    assert!(
        plan(
            0,
            1,
            ResizePolicy::Area {
                minimum: 1,
                maximum: 2
            }
        )
        .is_err()
    );
    assert!(
        plan(
            2,
            2,
            ResizePolicy::Area {
                minimum: 5,
                maximum: 4
            }
        )
        .is_err()
    );
}
#[test]
fn budgets_are_checked_in_integer_units_and_zero_retention_means_none() {
    let used = Usage {
        steps: 1,
        input_tokens: 10,
        output_tokens: 5,
        cost_microunits: 100,
    };
    assert_eq!(
        used.admit(
            used,
            Usage {
                steps: 2,
                input_tokens: 20,
                output_tokens: 10,
                cost_microunits: 200
            }
        )
        .unwrap(),
        Usage {
            steps: 2,
            input_tokens: 20,
            output_tokens: 10,
            cost_microunits: 200
        }
    );
    assert_eq!(
        used.admit(used, used).unwrap_err(),
        DecodeError::Limit("budget")
    );
    let mut overflow = used;
    overflow.steps = u64::MAX;
    assert!(overflow.add(used).is_err());
    assert_eq!(retained_image_indices(5, 0), 5..5);
    assert_eq!(retained_image_indices(5, 1), 4..5);
    assert_eq!(retained_image_indices(5, 10), 0..5);
}
#[test]
fn grounding_cache_is_bound_to_model_capture_and_session_and_failed_result_retires() {
    use dojo_computer_models::*;
    let view = ModelView {
        view_id: "v".into(),
        capture_id: "c".into(),
        capture_digest: "digest".into(),
        session_id: "s".into(),
        runtime_generation: 1,
        geometry_generation: 1,
        profile: Profile::Canonical,
        parser_version: PARSER_VERSION.into(),
        preprocessing: "identity".into(),
        model_width: 20,
        model_height: 10,
        delivered_width: 20,
        delivered_height: 10,
        scale_x: 1.,
        scale_y: 1.,
        offset_x: 0.,
        offset_y: 0.,
    };
    let key = GroundingKey::new(&view, "model-a".into(), "button".into()).unwrap();
    let step = decode(
        Profile::Canonical,
        r#"{"action":"click","x":0,"y":0}"#,
        &view,
    )
    .unwrap();
    let Outcome::Action(ComputerAction::Click { at, .. }) = step.outcome else {
        panic!("click")
    };
    let mut entry = GroundingEntry {
        key: key.clone(),
        point: Some(at),
    };
    assert!(entry.lookup(&key).is_some());
    let mut other = key.clone();
    other.capture_id = "other".into();
    assert!(entry.lookup(&other).is_none());
    other = key.clone();
    other.model = "model-b".into();
    assert!(entry.lookup(&other).is_none());
    other = key.clone();
    other.session_id = "other".into();
    assert!(entry.lookup(&other).is_none());
    entry.point = None;
    assert!(entry.lookup(&key).is_none());
}
#[test]
fn refresh_requires_new_complete_exact_target_and_confirmed_effect() {
    let before = Snapshot {
        snapshot_id: "old".into(),
        window_id: 1,
        pid: 1,
        process_start_time: 5,
        session_id: "s".into(),
        runtime_generation: 1,
        complete: true,
        elements: vec![],
    };
    let mut after = before.clone();
    after.snapshot_id = "fresh".into();
    assert!(validate_refresh(&before, &after, Effect::Confirmed).is_ok());
    assert!(validate_refresh(&before, &after, Effect::Unknown).is_err());
    assert!(validate_refresh(&before, &before, Effect::Confirmed).is_err());
    after.process_start_time = 6;
    assert!(validate_refresh(&before, &after, Effect::Confirmed).is_err());
}

#[test]
fn actionable_roles_match_observer_names_and_pid_refresh_does_not_alias() {
    for role in ["AX_Button", "text field", "CheckBox", "Edit"] {
        assert!(is_actionable_role(role));
    }
    assert!(!is_actionable_role("Label"));
    let entities = vec![];
    let elements = vec![element("a", 1, "AX_Button", "Submit form!")];
    let plan = vec![decision("a", Action::Click, 0.9)];
    assert_eq!(
        order_decisions(&elements, &entities, &plan, 0.5, true).unwrap(),
        plan
    );
    let before = Snapshot {
        snapshot_id: "before".into(),
        window_id: 1,
        pid: 1,
        process_start_time: 1,
        session_id: "s".into(),
        runtime_generation: 1,
        complete: true,
        elements: vec![],
    };
    let mut after = before.clone();
    after.snapshot_id = "after".into();
    after.pid = 2;
    assert!(validate_refresh(&before, &after, Effect::Confirmed).is_err());
}
