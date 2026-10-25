//! Remote input: the operator's pointer and keyboard driving a peer (ADR-0007). The sending side
//! captures local input and writes one JSON event per line; the receiving side replays them
//! through an input seat of its own. This module is the part that needs no compositor: the events,
//! the chords that bring input home, how the receiver's cursor moves, and who is allowed to send.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Event {
    /// Relative pointer motion in logical pixels.
    Motion { dx: f64, dy: f64 },
    /// A Linux evdev button code (272 is the left button).
    Button { code: u32, pressed: bool },
    /// Scrolling in logical pixels.
    Axis { h: f64, v: f64 },
    /// A Linux evdev key code.
    Key { code: u32, pressed: bool },
}

impl Event {
    pub fn parse(line: &str) -> Result<Self, String> {
        serde_json::from_str(line.trim()).map_err(|error| format!("not an input event: {error}"))
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("events serialize")
    }
}

const KEY_ESC: u32 = 1;
const KEY_LEFT_CTRL: u32 = 29;
const KEY_LEFT_SHIFT: u32 = 42;
const KEY_RIGHT_SHIFT: u32 = 54;
const KEY_LEFT_ALT: u32 = 56;
const KEY_F11: u32 = 87;
const KEY_RIGHT_CTRL: u32 = 97;
const KEY_RIGHT_ALT: u32 = 100;
const KEY_LEFT_META: u32 = 125;
const KEY_RIGHT_META: u32 = 126;

/// Watches the keys going by for the chords that bring input back: Super+F11 (the key that sent
/// the window, pressed again) and Ctrl+Alt+Shift+Esc, which is always there for when it goes wrong.
#[derive(Default)]
pub struct ReleaseChords {
    down: HashSet<u32>,
}

impl ReleaseChords {
    /// Feeds one key event; true when it completes a chord.
    pub fn feed(&mut self, code: u32, pressed: bool) -> bool {
        if !pressed {
            self.down.remove(&code);
            return false;
        }
        self.down.insert(code);
        let held = |codes: &[u32]| codes.iter().any(|code| self.down.contains(code));
        (code == KEY_F11 && held(&[KEY_LEFT_META, KEY_RIGHT_META]))
            || (code == KEY_ESC && held(&[KEY_LEFT_CTRL, KEY_RIGHT_CTRL]) && held(&[KEY_LEFT_ALT, KEY_RIGHT_ALT]) && held(&[KEY_LEFT_SHIFT, KEY_RIGHT_SHIFT]))
    }
}

/// What the receiver does to its input seat for an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Move the cursor to a place on the output.
    Move { x: f64, y: f64 },
    /// Let a movement finish, before something happens where the cursor is going.
    Wait,
    Button { code: u32, pressed: bool },
    Axis { h: f64, v: f64 },
    Key { code: u32, pressed: bool },
}

/// The receiver's cursor on one output, in logical pixels. Remote motion is relative, so the cursor
/// starts in the middle and stays on the output.
pub struct Cursor {
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    /// A movement was asked for and not waited for yet.
    moving: bool,
}

impl Cursor {
    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height, x: width / 2., y: height / 2., moving: false }
    }

    pub fn position(&self) -> (f64, f64) {
        (self.x, self.y)
    }

    /// The seat actions for an event. A button waits for the cursor to arrive first, so a click
    /// lands where the cursor is, not where it was a moment ago.
    pub fn plan(&mut self, event: &Event) -> Vec<Action> {
        match event {
            Event::Motion { dx, dy } => {
                self.x = (self.x + dx).clamp(0., (self.width - 1.).max(0.));
                self.y = (self.y + dy).clamp(0., (self.height - 1.).max(0.));
                self.moving = true;
                vec![Action::Move { x: self.x, y: self.y }]
            }
            Event::Button { code, pressed } => {
                let mut actions = Vec::new();
                if *pressed && std::mem::take(&mut self.moving) {
                    actions.push(Action::Wait);
                }
                actions.push(Action::Button { code: *code, pressed: *pressed });
                actions
            }
            Event::Axis { h, v } => vec![Action::Axis { h: *h, v: *v }],
            Event::Key { code, pressed } => vec![Action::Key { code: *code, pressed: *pressed }],
        }
    }
}

/// Adds the motion of consecutive events into one, so a fast mouse is one cursor movement per
/// frame, not hundreds.
pub fn coalesce(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::new();
    for event in events {
        match (&event, out.last_mut()) {
            (Event::Motion { dx, dy }, Some(Event::Motion { dx: sum_x, dy: sum_y })) => {
                *sum_x += dx;
                *sum_y += dy;
            }
            (Event::Axis { h, v }, Some(Event::Axis { h: sum_h, v: sum_v })) => {
                *sum_h += h;
                *sum_v += v;
            }
            _ => out.push(event),
        }
    }
    out
}

/// The address a connection came from, out of `SSH_CONNECTION` ("client_ip client_port server_ip
/// server_port"), which sshd (and Tailscale SSH) sets for the session.
pub fn caller_address(ssh_connection: Option<&str>) -> Option<String> {
    ssh_connection?.split_whitespace().next().filter(|address| !address.is_empty()).map(String::from)
}

/// The node name in `tailscale whois --json` output.
pub fn node_name(whois_json: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(whois_json).map_err(|error| format!("unreadable whois answer: {error}"))?;
    let name = ["Name", "ComputedName"].iter().find_map(|key| value["Node"][*key].as_str().filter(|name| !name.is_empty()));
    name.map(|name| name.trim_end_matches('.').to_string()).ok_or_else(|| "Tailscale did not name the sender".to_string())
}

/// Whether `accept_input` (normalised names) names the sender. Short and full names are the same
/// node, as everywhere else in the mesh.
pub fn accepts(accept_input: &[String], sender: &str) -> bool {
    let label = |name: &str| crate::config::name_key(name).split('.').next().unwrap_or_default().to_string();
    let sender_key = crate::config::name_key(sender);
    accept_input.iter().any(|entry| *entry == sender_key || label(entry) == label(sender))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_as_one_json_line() {
        for event in [
            Event::Motion { dx: 1.5, dy: -2. },
            Event::Button { code: 272, pressed: true },
            Event::Axis { h: 0., v: 15. },
            Event::Key { code: 30, pressed: false },
        ] {
            assert_eq!(Event::parse(&event.encode()).unwrap(), event);
        }
        assert_eq!(Event::Key { code: 1, pressed: true }.encode(), r#"{"t":"key","code":1,"pressed":true}"#);
        assert!(Event::parse("{\"t\":\"run\",\"command\":\"rm -rf ~\"}").is_err());
        assert!(Event::parse("garbage").is_err());
    }

    #[test]
    fn super_f11_and_the_panic_chord_bring_input_home_and_nothing_else_does() {
        let mut chords = ReleaseChords::default();
        assert!(!chords.feed(KEY_F11, true), "F11 alone is just F11");
        chords.feed(KEY_F11, false);
        assert!(!chords.feed(KEY_LEFT_META, true));
        assert!(chords.feed(KEY_F11, true));

        let mut panic = ReleaseChords::default();
        for key in [KEY_RIGHT_CTRL, KEY_LEFT_ALT, KEY_RIGHT_SHIFT] {
            assert!(!panic.feed(key, true));
        }
        assert!(panic.feed(KEY_ESC, true));

        let mut partial = ReleaseChords::default();
        partial.feed(KEY_LEFT_CTRL, true);
        partial.feed(KEY_LEFT_ALT, true);
        assert!(!partial.feed(KEY_ESC, true), "Ctrl+Alt+Esc without Shift is left to the peer");
    }

    #[test]
    fn a_released_modifier_no_longer_counts() {
        let mut chords = ReleaseChords::default();
        chords.feed(KEY_LEFT_META, true);
        chords.feed(KEY_LEFT_META, false);
        assert!(!chords.feed(KEY_F11, true));
    }

    #[test]
    fn the_cursor_starts_in_the_middle_and_stays_on_the_output() {
        let mut cursor = Cursor::new(1000., 500.);
        assert_eq!(cursor.position(), (500., 250.));
        assert_eq!(cursor.plan(&Event::Motion { dx: 10., dy: -5. }), [Action::Move { x: 510., y: 245. }]);
        assert_eq!(cursor.plan(&Event::Motion { dx: 5000., dy: 5000. }), [Action::Move { x: 999., y: 499. }]);
        assert_eq!(cursor.plan(&Event::Motion { dx: -5000., dy: -5000. }), [Action::Move { x: 0., y: 0. }]);
    }

    #[test]
    fn a_click_waits_for_the_cursor_to_arrive_once() {
        let mut cursor = Cursor::new(100., 100.);
        assert_eq!(cursor.plan(&Event::Button { code: 272, pressed: true }), [Action::Button { code: 272, pressed: true }], "no wait when it has not moved");
        cursor.plan(&Event::Motion { dx: 1., dy: 1. });
        assert_eq!(cursor.plan(&Event::Button { code: 272, pressed: true }), [Action::Wait, Action::Button { code: 272, pressed: true }]);
        assert_eq!(cursor.plan(&Event::Button { code: 272, pressed: false }), [Action::Button { code: 272, pressed: false }], "a release never waits");
        assert_eq!(cursor.plan(&Event::Axis { h: 0., v: 8. }), [Action::Axis { h: 0., v: 8. }]);
        assert_eq!(cursor.plan(&Event::Key { code: 30, pressed: true }), [Action::Key { code: 30, pressed: true }]);
    }

    #[test]
    fn a_burst_of_motion_becomes_one_movement_without_reordering_clicks() {
        let events = vec![
            Event::Motion { dx: 1., dy: 1. },
            Event::Motion { dx: 2., dy: 3. },
            Event::Button { code: 272, pressed: true },
            Event::Motion { dx: 4., dy: 0. },
            Event::Axis { h: 0., v: 1. },
            Event::Axis { h: 0., v: 2. },
        ];
        assert_eq!(
            coalesce(events),
            [
                Event::Motion { dx: 3., dy: 4. },
                Event::Button { code: 272, pressed: true },
                Event::Motion { dx: 4., dy: 0. },
                Event::Axis { h: 0., v: 3. },
            ]
        );
    }

    #[test]
    fn the_sender_is_known_from_the_connection_and_tailscales_answer() {
        assert_eq!(caller_address(Some("100.64.0.1 51234 100.64.0.2 22")).as_deref(), Some("100.64.0.1"));
        assert_eq!(caller_address(Some("")), None);
        assert_eq!(caller_address(None), None);
        assert_eq!(node_name(r#"{"Node": {"Name": "alpha.example.ts.net.", "ComputedName": "alpha"}}"#).unwrap(), "alpha.example.ts.net");
        assert_eq!(node_name(r#"{"Node": {"Name": "", "ComputedName": "alpha"}}"#).unwrap(), "alpha");
        assert!(node_name(r#"{"Node": null}"#).is_err());
        assert!(node_name("nope").is_err());
    }

    #[test]
    fn only_named_peers_may_send_input() {
        let accepted = vec!["alpha".to_string(), "gamma.example.ts.net".to_string()];
        assert!(accepts(&accepted, "alpha.example.ts.net"));
        assert!(accepts(&accepted, "ALPHA"));
        assert!(accepts(&accepted, "gamma"));
        assert!(!accepts(&accepted, "beta.example.ts.net"));
        assert!(!accepts(&[], "alpha"));
    }
}
