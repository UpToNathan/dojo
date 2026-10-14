use super::*;
use serde_json::{json, Value};
use std::os::unix::net::UnixListener;
use std::thread;

fn fixture(
    replies: Vec<(&'static str, Value)>,
) -> (tempfile::TempDir, PathBuf, thread::JoinHandle<()>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("niri.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        for (expected, reply) in replies {
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request, serde_json::from_str::<Value>(expected).unwrap());
            writeln!(stream, "{}", json!({"Ok": reply})).unwrap();
        }
    });
    (directory, path, server)
}

fn logical() -> Value {
    json!({"x": -1280, "y": 100, "width": 1280, "height": 720, "scale": 1.25, "transform": "Normal"})
}

fn output() -> Value {
    json!({"name":"DP-1","make":"fixture","model":"fixture","serial":null,
        "physical_size":null,"modes":[{"width":1600,"height":900,"refresh_rate":60000,"is_preferred":true}],
        "current_mode":0,"is_custom_mode":false,"vrr_supported":false,"vrr_enabled":false,
        "logical":logical(),"max_bpc":null})
}

fn windows() -> Value {
    json!({"Windows":[{"id": 4294967301_u64, "title":"Same title", "app_id":"fixture", "pid":123,
        "workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,
        "layout":{"pos_in_scrolling_layout":null,"tile_size":[500.0,400.0],"window_size":[500,400],
        "tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]},"focus_timestamp":null}]})
}

fn scene(pid: u32) -> Value {
    json!({"WindowPlacements":{"observation_serial":1,"compositor_pid":pid,"output":"DP-1",
        "logical_output":logical(),"width":1600,"height":900,"windows":[{"id":4294967301_u64,
        "pid":123,"is_floating":false,"rect":[20.0,30.0,500.0,400.0],
        "output_clip":[20.0,30.0,500.0,400.0],"blocked_out":false}]}})
}

fn workspaces() -> Value {
    json!({"Workspaces":[
        {"id":1,"idx":1,"name":null,"output":"DP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":4294967301_u64},
        {"id":2,"idx":2,"name":null,"output":"DP-1","is_urgent":false,"is_active":false,"is_focused":false,"active_window_id":4294967302_u64}
    ]})
}

fn window_fixture(scene: Value) -> (tempfile::TempDir, PathBuf, thread::JoinHandle<()>) {
    fixture(vec![
        ("\"Windows\"", windows()),
        ("\"Workspaces\"", workspaces()),
        ("\"Outputs\"", json!({"Outputs":{"DP-1":output()}})),
        ("{\"WindowPlacements\":{\"output\":\"DP-1\"}}", scene),
    ])
}

#[test]
fn preserves_native_id_and_peer_attested_output_geometry() {
    let (_directory, path, server) = window_fixture(scene(std::process::id()));
    let mut connection = Connection::connect(&path).unwrap();
    assert_eq!(connection.peer.pid, std::process::id());
    let windows = list_from_connection(&mut connection).unwrap();
    server.join().unwrap();
    assert_eq!(windows.len(), 1);
    let window = &windows[0];
    assert_eq!(window.xid, 4294967301);
    assert_eq!(window.pid, Some(123));
    assert_eq!(
        (window.x, window.y, window.width, window.height),
        (-1260, 130, 500, 400)
    );
    assert!(window.is_on_screen);
}

#[test]
fn refuses_forged_compositor_identity() {
    let (_directory, path, server) = window_fixture(scene(std::process::id() + 1));
    assert!(
        list_from_connection(&mut Connection::connect(&path).unwrap())
            .unwrap_err()
            .to_string()
            .contains("not the socket peer")
    );
    server.join().unwrap();
}

#[test]
fn refuses_missing_geometry_and_process_substitution() {
    let mut missing = scene(std::process::id());
    missing["WindowPlacements"]["windows"] = json!([]);
    let mut changed = scene(std::process::id());
    changed["WindowPlacements"]["windows"][0]["pid"] = json!(456);
    for (scene, message) in [
        (missing, "no layout position"),
        (changed, "process changed"),
    ] {
        let (_directory, path, server) = window_fixture(scene);
        assert!(
            list_from_connection(&mut Connection::connect(&path).unwrap())
                .unwrap_err()
                .to_string()
                .contains(message)
        );
        server.join().unwrap();
    }
}

#[test]
fn outputs_define_scaled_negative_origin_frame_without_guessing() {
    let (_directory, path, server) =
        fixture(vec![("\"Outputs\"", json!({"Outputs":{"DP-1":output()}}))]);
    let frame = frame_from_outputs(
        outputs(&mut Connection::connect(&path).unwrap())
            .unwrap()
            .into_iter()
            .map(|(_, layout)| layout)
            .collect(),
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        (frame.x, frame.y, frame.width, frame.height),
        (-1280, 100, 1280, 720)
    );
    assert_eq!(frame.to_layout(20, 30), (-1260, 130));
    assert!(frame_from_outputs(Vec::new()).is_err());
}

#[test]
fn bounds_reply_size_and_requires_complete_typed_reply() {
    for reply in [
        vec![b'x'; MAX_REPLY_BYTES as usize + 1],
        b"{\"Ok\":\"Handled\"}".to_vec(),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("niri.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let _ = stream.write_all(&reply);
        });
        let error = Connection::connect(&path)
            .unwrap()
            .query(&Request::Outputs)
            .unwrap_err();
        assert!(
            error.to_string().contains("size limit") || error.to_string().contains("incomplete")
        );
        server.join().unwrap();
    }
}

#[test]
fn niri_input_refuses_before_generic_backend_dispatch() {
    const CHILD: &str = "CUA_NIRI_REFUSAL_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "wayland::niri::tests::niri_input_refuses_before_generic_backend_dispatch",
            ])
            .env(CHILD, "1")
            .env("NIRI_SOCKET", "/nonexistent/niri-readonly.sock")
            .env("CUA_DRIVER_RS_ENABLE_WAYLAND", "1")
            .env("CUA_INJECT_SOCKET", "/nonexistent/inject.sock")
            .env("PATH", "/nonexistent")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let check = |result: Result<()>| {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("facility_unavailable"))
    };
    check(super::super::activate_window_for_input_target(
        4294967301,
        Some(123),
    ));
    check(super::super::with_target_foreground(
        123,
        4294967301,
        || panic!("input body executed"),
    ));
    check(super::super::click_focused(20, 30, 1, 1));
    check(super::super::move_cursor_absolute(Some(4294967301), 20, 30));
    check(super::super::type_text_focused("no input"));
    check(super::super::press_key_focused("enter"));
    check(super::super::hotkey_focused(&["ctrl".into(), "a".into()]));
    assert!(super::super::open_vptr_session(None)
        .err()
        .unwrap()
        .to_string()
        .contains("facility_unavailable"));
    check(
        super::super::DesktopInputSpace::default()
            .input_layout(20, 30)
            .map(|_| ()),
    );
}

#[test]
fn refuses_changed_peer_process_generation_before_query() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("niri.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(std::io::Read::read(&mut stream, &mut byte).unwrap(), 0);
    });
    let mut connection = Connection::connect(&path).unwrap();
    connection.peer.start_ticks += 1;
    assert!(connection
        .query(&Request::Outputs)
        .unwrap_err()
        .to_string()
        .contains("generation changed"));
    drop(connection);
    server.join().unwrap();
}

#[test]
fn times_out_a_peer_that_does_not_complete_its_reply() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("niri.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        stream.write_all(b"{\"Ok\":").unwrap();
        thread::sleep(Duration::from_millis(150));
    });
    let mut connection = Connection::connect(&path).unwrap();
    connection.deadline = Instant::now() + Duration::from_millis(30);
    assert!(connection.query(&Request::Outputs).is_err());
    server.join().unwrap();
}

#[test]
fn lists_visible_and_inactive_tiled_windows_without_capture_authority() {
    let mut inventory = windows();
    let mut inactive = inventory["Windows"][0].clone();
    inactive["id"] = json!(4294967302_u64);
    inactive["workspace_id"] = json!(2);
    inactive["is_focused"] = json!(false);
    inactive["layout"]["tile_pos_in_workspace_view"] = json!([-600.0, 30.0]);
    inactive["layout"]["window_offset_in_tile"] = json!([2.0, 3.0]);
    inactive["layout"]["pos_in_scrolling_layout"] = json!([1, 1]);
    inventory["Windows"].as_array_mut().unwrap().push(inactive);
    let (_directory, path, server) = fixture(vec![
        ("\"Windows\"", inventory),
        ("\"Workspaces\"", workspaces()),
        ("\"Outputs\"", json!({"Outputs":{"DP-1":output()}})),
        (
            "{\"WindowPlacements\":{\"output\":\"DP-1\"}}",
            scene(std::process::id()),
        ),
    ]);
    let windows = list_from_connection(&mut Connection::connect(&path).unwrap()).unwrap();
    server.join().unwrap();
    assert_eq!(windows.len(), 2);
    assert!(windows[0].is_on_screen);
    assert_eq!(windows[1].xid, 4294967302);
    assert_eq!(
        (
            windows[1].x,
            windows[1].y,
            windows[1].width,
            windows[1].height
        ),
        (-1878, 133, 500, 400)
    );
    assert!(!windows[1].is_on_screen);
}

#[test]
fn fractional_rendered_origin_has_integer_inventory_bounds() {
    let mut value = scene(std::process::id());
    value["WindowPlacements"]["windows"][0]["rect"][0] = json!(20.5);
    value["WindowPlacements"]["windows"][0]["rect"][2] = json!(260.5);
    let (_directory, path, server) = window_fixture(value);
    let windows = list_from_connection(&mut Connection::connect(&path).unwrap()).unwrap();
    assert_eq!(windows[0].x, -1260);
    assert_eq!(windows[0].width, 261);
    server.join().unwrap();
}
