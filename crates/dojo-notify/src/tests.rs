use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use zbus::zvariant::OwnedValue;

use super::*;
use crate::level::{Volume, parse_brightness, parse_volume, volume_icon};

#[test]
fn a_notice_carries_its_urgency_replace_key_and_progress_as_hints() {
    let notice = Notice::new("volume", "Volume").urgency(Urgency::Low).value(40);
    let hints = notice.hints();
    assert_eq!(hints["urgency"], Value::U8(0));
    assert_eq!(hints["x-canonical-private-synchronous"], Value::Str("volume".into()));
    assert_eq!(hints["value"], Value::I32(40));
    let bare = Notice::new("", "Hello");
    let plain = bare.hints();
    assert!(!plain.contains_key("x-canonical-private-synchronous") && !plain.contains_key("value"));
    assert_eq!(Notice::new("a", "b").value(250).value, Some(100));
}

#[test]
fn urgency_names_parse() {
    assert_eq!("critical".parse::<Urgency>().unwrap(), Urgency::Critical);
    assert!("loud".parse::<Urgency>().is_err());
}

#[test]
fn volume_and_brightness_read_what_the_tools_print() {
    assert_eq!(parse_volume("Volume: 0.45\n"), Some(Volume { percent: 45, muted: false }));
    assert_eq!(parse_volume("Volume: 0.80 [MUTED]"), Some(Volume { percent: 80, muted: true }));
    assert_eq!(parse_volume("Volume: 1.20"), Some(Volume { percent: 100, muted: false }));
    assert_eq!(parse_volume("nonsense"), None);
    assert_eq!(parse_brightness("intel_backlight,backlight,4800,50%,9600\n"), Some(50));
    assert_eq!(parse_brightness("tpacpi::kbd_backlight,leds,1,50%,2"), Some(50));
    assert_eq!(parse_brightness(""), None);
    assert_eq!(volume_icon(Volume { percent: 70, muted: false }), "audio-volume-high-symbolic");
    assert_eq!(volume_icon(Volume { percent: 70, muted: true }), "audio-volume-muted-symbolic");
    assert_eq!(volume_icon(Volume { percent: 20, muted: false }), "audio-volume-low-symbolic");
}

struct PrivateBus {
    child: Child,
    address: String,
    _config: tempfile::NamedTempFile,
}

impl PrivateBus {
    fn start() -> Self {
        let mut config = tempfile::NamedTempFile::new().unwrap();
        config.write_all(b"<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context='default'><allow send_destination='*'/><allow receive_sender='*'/><allow own='*'/></policy></busconfig>").unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.path().display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut address).unwrap();
        Self { child, address: address.trim().to_owned(), _config: config }
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Got {
    app: String,
    icon: String,
    summary: String,
    body: String,
    timeout: i32,
    urgency: Option<u8>,
    sync: Option<String>,
    value: Option<i32>,
}

struct FakeService(Arc<Mutex<Vec<Got>>>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl FakeService {
    #[allow(clippy::too_many_arguments)]
    fn notify(&self, app_name: String, _replaces_id: u32, app_icon: String, summary: String, body: String, _actions: Vec<String>, hints: HashMap<String, OwnedValue>, expire_timeout: i32) -> u32 {
        let mut log = self.0.lock().unwrap();
        log.push(Got {
            app: app_name,
            icon: app_icon,
            summary,
            body,
            timeout: expire_timeout,
            urgency: hints.get("urgency").and_then(|v| u8::try_from(v).ok()),
            sync: hints.get("x-canonical-private-synchronous").and_then(|v| String::try_from(v.try_clone().ok()?).ok()),
            value: hints.get("value").and_then(|v| i32::try_from(v).ok()),
        });
        log.len() as u32
    }
}

#[test]
fn a_notice_reaches_a_notification_service_on_a_private_bus_and_the_connection_is_reused() {
    let bus = PrivateBus::start();
    let log = Arc::new(Mutex::new(Vec::new()));
    let _service = connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at("/org/freedesktop/Notifications", FakeService(log.clone()))
        .unwrap()
        .build()
        .unwrap();

    let first = Notice::new("battery", "Battery low").body("19% left").urgency(Urgency::Normal).icon("battery-caution-symbolic").timeout_ms(8000);
    assert_eq!(send_to(&bus.address, &first).unwrap(), 1);
    let second = Notice::new("volume", "Volume").urgency(Urgency::Low).value(30);
    assert_eq!(send_to(&bus.address, &second).unwrap(), 2);

    let log = log.lock().unwrap();
    assert_eq!(
        log[0],
        Got { app: "Dojo".into(), icon: "battery-caution-symbolic".into(), summary: "Battery low".into(), body: "19% left".into(), timeout: 8000, urgency: Some(1), sync: Some("battery".into()), value: None }
    );
    assert_eq!((log[1].urgency, log[1].sync.as_deref(), log[1].value), (Some(0), Some("volume"), Some(30)));
}

#[test]
fn with_no_notification_service_sending_is_an_error_and_never_a_hang() {
    let bus = PrivateBus::start();
    let started = std::time::Instant::now();
    assert!(send_to(&bus.address, &Notice::new("x", "Nobody is listening")).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}
