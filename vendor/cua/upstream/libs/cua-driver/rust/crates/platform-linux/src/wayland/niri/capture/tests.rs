use super::*;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;

pub(crate) const WINDOW_ID: u64 = 4294967301;

pub(crate) fn logical() -> Value {
    json!({"x":-8,"y":12,"width":2,"height":2,"scale":2.0,"transform":"Normal"})
}

pub(crate) fn outputs() -> Value {
    json!({"Outputs":{"DP-1":{"name":"DP-1","make":"fixture","model":"fixture","serial":null,
        "physical_size":null,"modes":[{"width":4,"height":4,"refresh_rate":60000,"is_preferred":true}],
        "current_mode":0,"is_custom_mode":false,"vrr_supported":false,"vrr_enabled":false,
        "logical":logical(),"max_bpc":null}}})
}

pub(crate) fn inventory() -> Value {
    json!({"Windows":[{"id":WINDOW_ID,"title":"Fixture","app_id":"fixture","pid":std::process::id(),
        "workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,"focus_timestamp":null,
        "layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[1.0,1.0],"window_size":[1,1],
            "tile_pos_in_workspace_view":[0.0,0.0],"window_offset_in_tile":[0.0,0.0]}}]})
}

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_fn(4, 4, |x, y| {
        image::Rgba(if x < 2 && y < 2 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        })
    });
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    png.into_inner()
}

pub(crate) fn scene() -> Value {
    json!({"CapturedScene":{"frame_serial":7,"compositor_pid":std::process::id(),"output":"DP-1",
        "logical_output":logical(),"width":4,"height":4,"png_base64":STANDARD.encode(png()),
        "windows":[{"id":WINDOW_ID,"pid":std::process::id(),"is_floating":false,
            "rect":[0.0,0.0,1.0,1.0],"output_clip":[0.0,0.0,1.0,1.0],"blocked_out":false}]}})
}

fn fixture(
    replies: Vec<(&'static str, Value)>,
) -> (tempfile::TempDir, PathBuf, thread::JoinHandle<()>) {
    fixture_groups(vec![replies])
}

pub(crate) fn fixture_groups(
    groups: Vec<Vec<(&'static str, Value)>>,
) -> (tempfile::TempDir, PathBuf, thread::JoinHandle<()>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("niri.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        for replies in groups {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for (request, reply) in replies {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&line).unwrap(),
                    serde_json::from_str::<Value>(request).unwrap()
                );
                writeln!(stream, "{}", json!({"Ok":reply})).unwrap();
            }
        }
    });
    (directory, path, server)
}

fn capture_fixture(scene: Value) -> (tempfile::TempDir, PathBuf, thread::JoinHandle<()>) {
    fixture(vec![
        ("\"Outputs\"", outputs()),
        ("\"Windows\"", inventory()),
        ("{\"CaptureScene\":{\"output\":\"DP-1\"}}", scene),
    ])
}

#[test]
fn exact_window_pixels_and_geometry_share_one_authenticated_scene() {
    let (_directory, path, server) = capture_fixture(scene());
    let mut connection = Connection::connect(&path).unwrap();
    let frame =
        capture_from_connection(&mut connection, Some((WINDOW_ID, Some(std::process::id()))))
            .unwrap();
    server.join().unwrap();
    let decoded = image::load_from_memory(&frame.png).unwrap().to_rgba8();
    assert_eq!(decoded.dimensions(), (1, 1));
    assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(frame.evidence.compositor_pid, std::process::id());
    assert_eq!(
        frame.evidence.compositor_start_ticks,
        process_start(std::process::id()).unwrap()
    );
    assert_eq!(frame.evidence.scene_frame_serial, 7);
    assert_eq!(frame.evidence.source_png_sha256, hex_digest(&png()));
    assert_eq!(
        frame.evidence.window.unwrap().rendered_rect,
        (0.0, 0.0, 1.0, 1.0)
    );
    assert!(frame.evidence.image_geometry_atomic);
    assert!(!frame.evidence.image_semantic_atomic);
}

#[test]
fn desktop_keeps_source_pixels_and_cua_resize_pipeline() {
    let (_directory, path, server) = capture_fixture(scene());
    let frame = capture_from_connection(&mut Connection::connect(&path).unwrap(), None).unwrap();
    server.join().unwrap();
    assert_eq!(frame.png, png());
    assert_eq!(
        (
            frame.evidence.logical_output.x,
            frame.evidence.logical_output.y
        ),
        (-8, 12)
    );
    assert_eq!((frame.windows[0].x, frame.windows[0].y), (0, 0));
    let resized = crate::capture::resize_png_if_needed(&frame.png, 2).unwrap();
    assert_eq!(
        crate::capture::png_dimensions_pub(&resized).unwrap(),
        (2, 2)
    );
}

#[test]
fn capture_refuses_forged_identity_and_missing_epoch() {
    let mut wrong_pid = scene();
    wrong_pid["CapturedScene"]["compositor_pid"] = json!(std::process::id() + 1);
    let mut zero_epoch = scene();
    zero_epoch["CapturedScene"]["frame_serial"] = json!(0);
    let mut wrong_output = scene();
    wrong_output["CapturedScene"]["output"] = json!("DP-2");
    let mut changed_geometry = scene();
    changed_geometry["CapturedScene"]["logical_output"]["x"] = json!(0);
    for (response, message) in [
        (wrong_pid, "scene identity"),
        (zero_epoch, "scene identity"),
        (wrong_output, "scene identity"),
        (changed_geometry, "geometry changed"),
    ] {
        let (_directory, path, server) = capture_fixture(response);
        let error = capture_from_connection(&mut Connection::connect(&path).unwrap(), None)
            .err()
            .unwrap();
        assert!(error.to_string().contains(message), "{error}");
        server.join().unwrap();
    }
}

#[test]
fn window_capture_refuses_privacy_clipping_and_pid_substitution() {
    let mut private = scene();
    private["CapturedScene"]["windows"][0]["blocked_out"] = json!(true);
    let mut clipped = scene();
    clipped["CapturedScene"]["windows"][0]["rect"] = json!([-1.0, 0.0, 2.0, 1.0]);
    let mut other_process = scene();
    other_process["CapturedScene"]["windows"][0]["pid"] = json!(123);
    let mut missing = scene();
    missing["CapturedScene"]["windows"] = json!([]);
    for (response, message) in [
        (private, "privacy rules"),
        (clipped, "partially clipped"),
        (other_process, "identity changed"),
        (missing, "not visible"),
    ] {
        let (_directory, path, server) = capture_fixture(response);
        let error = capture_from_connection(
            &mut Connection::connect(&path).unwrap(),
            Some((WINDOW_ID, Some(std::process::id()))),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains(message), "{error}");
        server.join().unwrap();
    }
}

#[test]
fn captures_refuse_png_mismatch_and_image_budget_overflow() {
    let mut mismatch = scene();
    mismatch["CapturedScene"]["width"] = json!(3);
    let mut oversized = scene();
    oversized["CapturedScene"]["width"] = json!(MAX_PIXELS + 1);
    let mut corrupt = scene();
    corrupt["CapturedScene"]["png_base64"] = json!(STANDARD.encode(b"not a PNG"));
    for (response, message) in [
        (mismatch, "dimensions mismatch"),
        (oversized, "image budget"),
        (corrupt, "decode captured PNG"),
    ] {
        let (_directory, path, server) = capture_fixture(response);
        let error = capture_from_connection(&mut Connection::connect(&path).unwrap(), None)
            .err()
            .unwrap();
        assert!(error.to_string().contains(message), "{error}");
        server.join().unwrap();
    }
}

#[test]
fn captures_refuse_multimonitor_and_unknown_target_before_capture_request() {
    let mut oversized_layout = outputs();
    oversized_layout["Outputs"]["DP-1"]["logical"]["width"] = json!(32769);
    let (_directory, path, server) = fixture(vec![("\"Outputs\"", oversized_layout)]);
    assert!(
        capture_from_connection(&mut Connection::connect(&path).unwrap(), None)
            .err()
            .unwrap()
            .to_string()
            .contains("logical capture frame exceeds image budget")
    );
    server.join().unwrap();
    let mut multi = outputs();
    let mut second = multi["Outputs"]["DP-1"].clone();
    second["name"] = json!("DP-2");
    multi["Outputs"]["DP-2"] = second;
    let (_directory, path, server) = fixture(vec![("\"Outputs\"", multi)]);
    assert!(
        capture_from_connection(&mut Connection::connect(&path).unwrap(), None)
            .err()
            .unwrap()
            .to_string()
            .contains("exactly one")
    );
    server.join().unwrap();
    let (_directory, path, server) = fixture(vec![
        ("\"Outputs\"", outputs()),
        ("\"Windows\"", inventory()),
    ]);
    assert!(capture_from_connection(
        &mut Connection::connect(&path).unwrap(),
        Some((WINDOW_ID + 1, None))
    )
    .err()
    .unwrap()
    .to_string()
    .contains("absent or ambiguous"));
    server.join().unwrap();
}

#[test]
fn action_capture_pins_the_revision_bracketing_its_pixels() {
    let (_directory, path, server) = fixture(vec![
        ("\"Outputs\"", outputs()),
        ("\"Windows\"", inventory()),
        (
            "\"InputCoordination\"",
            json!({"InputCoordination":{"revision":13,"human_owned_windows":[]}}),
        ),
        ("{\"CaptureScene\":{\"output\":\"DP-1\"}}", scene()),
        (
            "\"InputCoordination\"",
            json!({"InputCoordination":{"revision":13,"human_owned_windows":[]}}),
        ),
    ]);
    let frame = capture_scoped(
        &mut Connection::connect(&path).unwrap(),
        Some((WINDOW_ID, Some(std::process::id()))),
        true,
    )
    .unwrap();
    assert_eq!(frame.evidence.coordination_revision, Some(13));
    server.join().unwrap();
}

#[test]
fn ownership_change_during_capture_refuses_an_action_frame() {
    let (_directory, path, server) = fixture(vec![
        ("\"Outputs\"", outputs()),
        ("\"Windows\"", inventory()),
        (
            "\"InputCoordination\"",
            json!({"InputCoordination":{"revision":13,"human_owned_windows":[]}}),
        ),
        ("{\"CaptureScene\":{\"output\":\"DP-1\"}}", scene()),
        (
            "\"InputCoordination\"",
            json!({"InputCoordination":{"revision":14,"human_owned_windows":[WINDOW_ID]}}),
        ),
    ]);
    let error = capture_scoped(&mut Connection::connect(&path).unwrap(), None, true)
        .err()
        .unwrap();
    assert!(error.to_string().contains("ownership changed"));
    server.join().unwrap();
}

#[test]
fn fractional_origin_keeps_exact_capture_geometry_and_window_dimensions() {
    let mut value = scene();
    value["CapturedScene"]["windows"][0]["rect"] = json!([0.5,0.0,0.75,0.75]);
    value["CapturedScene"]["windows"][0]["output_clip"] = json!([0.5,0.0,0.75,0.75]);
    let (_directory, path, server) = capture_fixture(value);
    let frame = capture_from_connection(&mut Connection::connect(&path).unwrap(), Some((WINDOW_ID, Some(std::process::id())))).unwrap();
    assert_eq!(frame.evidence.window.unwrap().rendered_rect, (0.5,0.0,0.75,0.75));
    assert_eq!(png_dimensions(&frame.png).unwrap(), (1,1));
    assert_eq!(frame.windows[0].x, 0);
    server.join().unwrap();
}

#[test]
fn window_capture_selects_its_workspace_output_with_multiple_monitors() {
    let mut multi = outputs();
    let mut second = multi["Outputs"]["DP-1"].clone();
    second["name"] = json!("DP-2");
    multi["Outputs"]["DP-2"] = second;
    let workspaces = json!({"Workspaces":[{"id":1,"idx":1,"name":null,"output":"DP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":WINDOW_ID}]});
    let (_directory,path,server) = fixture(vec![("\"Outputs\"",multi),("\"Windows\"",inventory()),("\"Workspaces\"",workspaces),("{\"CaptureScene\":{\"output\":\"DP-1\"}}",scene())]);
    let frame = capture_from_connection(&mut Connection::connect(&path).unwrap(), Some((WINDOW_ID,Some(std::process::id())))).unwrap();
    assert_eq!(frame.evidence.output,"DP-1");
    assert_eq!(png_dimensions(&frame.png).unwrap(),(1,1));
    server.join().unwrap();
}
