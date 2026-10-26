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

/// Watches the keys going by for the chords that bring input back: Super+Alt+F11, and
/// Ctrl+Alt+Shift+Esc, which is always there for when it goes wrong. The keys may go down in any order;
/// the chord completes when the last of them does. Super+F11 alone is not one, since that is the key
/// that opens the send panel and it should not end a session by accident.
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
        let home = held(&[KEY_F11]) && held(&[KEY_LEFT_META, KEY_RIGHT_META]) && held(&[KEY_LEFT_ALT, KEY_RIGHT_ALT]);
        let panic = held(&[KEY_ESC]) && held(&[KEY_LEFT_CTRL, KEY_RIGHT_CTRL]) && held(&[KEY_LEFT_ALT, KEY_RIGHT_ALT]) && held(&[KEY_LEFT_SHIFT, KEY_RIGHT_SHIFT]);
        // Only a key that is part of a chord can complete it.
        let part_of_home = [KEY_F11, KEY_LEFT_META, KEY_RIGHT_META, KEY_LEFT_ALT, KEY_RIGHT_ALT].contains(&code);
        let part_of_panic = [KEY_ESC, KEY_LEFT_CTRL, KEY_RIGHT_CTRL, KEY_LEFT_ALT, KEY_RIGHT_ALT, KEY_LEFT_SHIFT, KEY_RIGHT_SHIFT].contains(&code);
        (home && part_of_home) || (panic && part_of_panic)
    }
}

/// What the receiver does to its input seat for an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Put the cursor at a place on an output (coordinates are within that output).
    Move { output: String, x: f64, y: f64 },
    /// Let a movement finish, before something happens where the cursor is going. Only needed when
    /// the seat animates its cursor; one that places it at once has nothing to wait for.
    Wait,
    Button { code: u32, pressed: bool },
    Axis { h: f64, v: f64 },
    Key { code: u32, pressed: bool },
}

/// One output's place in the layout, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Rect {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// The receiver's cursor on the whole layout of outputs, in global logical pixels. Remote motion is
/// relative, so it starts in the middle of the first output and moves wherever the layout allows:
/// across to a neighbouring output when it leaves one, and stopping at the layout's outer edge.
pub struct Cursor {
    outputs: Vec<Rect>,
    x: f64,
    y: f64,
    /// A movement was asked for and not waited for yet.
    moving: bool,
}

impl Cursor {
    /// `None` when there are no outputs.
    pub fn new(outputs: Vec<Rect>) -> Option<Self> {
        let first = outputs.first()?;
        let (x, y) = (first.x + first.width / 2., first.y + first.height / 2.);
        Some(Self { outputs, x, y, moving: false })
    }

    pub fn position(&self) -> (f64, f64) {
        (self.x, self.y)
    }

    /// The output the cursor is on, with the cursor's place on it.
    fn locate(&self) -> Action {
        let rect = self.outputs.iter().find(|rect| rect.contains(self.x, self.y)).unwrap_or(&self.outputs[0]);
        Action::Move { output: rect.name.clone(), x: self.x - rect.x, y: self.y - rect.y }
    }

    fn on_an_output(&self, x: f64, y: f64) -> bool {
        self.outputs.iter().any(|rect| rect.contains(x, y))
    }

    /// The furthest point toward `(tx, ty)` that is on an output, found by halving: all the way if
    /// the target is on one, otherwise right up to the edge of the layout.
    fn reach(&self, x: f64, y: f64, tx: f64, ty: f64) -> (f64, f64) {
        if self.on_an_output(tx, ty) {
            return (tx, ty);
        }
        let (mut near, mut far) = (0., 1.);
        for _ in 0..16 {
            let middle = (near + far) / 2.;
            if self.on_an_output(x + (tx - x) * middle, y + (ty - y) * middle) {
                near = middle;
            } else {
                far = middle;
            }
        }
        (x + (tx - x) * near, y + (ty - y) * near)
    }

    /// Moves by `(dx, dy)` in small steps, so the path is followed, not just its end: it crosses from
    /// one output to the next where they touch, and where a step would leave every output it goes as
    /// far as the edge and then slides along it on whichever axis is still free, as a mouse does
    /// against the side of a screen. A big jump never lands the cursor on some other monitor.
    fn place(&mut self, dx: f64, dy: f64) {
        const STEP: f64 = 8.;
        let steps = (dx.abs().max(dy.abs()) / STEP).ceil().max(1.);
        let (sx, sy) = (dx / steps, dy / steps);
        for _ in 0..steps as usize {
            let (x, y) = (self.x, self.y);
            if self.on_an_output(x + sx, y + sy) {
                (self.x, self.y) = (x + sx, y + sy);
                continue;
            }
            // Blocked: slide, one axis at a time, each as far as it goes.
            let (x, y) = self.reach(x, y, x + sx, y);
            let (x, y) = self.reach(x, y, x, y + sy);
            (self.x, self.y) = (x, y);
        }
    }

    /// The first position as an action, so the seat's cursor starts where this one does.
    pub fn start(&mut self) -> Action {
        self.locate()
    }

    /// The seat actions for an event. A button waits for the cursor to arrive first, so a click
    /// lands where the cursor is, not where it was a moment ago.
    pub fn plan(&mut self, event: &Event) -> Vec<Action> {
        match event {
            Event::Motion { dx, dy } => {
                self.place(*dx, *dy);
                self.moving = true;
                vec![self.locate()]
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
    fn super_alt_f11_and_the_panic_chord_bring_input_home_in_any_order_and_nothing_else_does() {
        for order in [
            [KEY_LEFT_META, KEY_LEFT_ALT, KEY_F11],
            [KEY_F11, KEY_LEFT_META, KEY_LEFT_ALT],
            [KEY_LEFT_ALT, KEY_F11, KEY_RIGHT_META],
        ] {
            let mut chords = ReleaseChords::default();
            assert!(!chords.feed(order[0], true) && !chords.feed(order[1], true), "{order:?}");
            assert!(chords.feed(order[2], true), "the last key of {order:?} completes it");
        }
        // Not chords: F11 alone, Super+F11 (the send panel's own key), Alt+F11.
        let mut other = ReleaseChords::default();
        assert!(!other.feed(KEY_F11, true), "F11 alone is just F11");
        other.feed(KEY_F11, false);
        assert!(!other.feed(KEY_LEFT_META, true));
        assert!(!other.feed(KEY_F11, true), "Super+F11 no longer ends a session");
        other.feed(KEY_LEFT_META, false);
        assert!(!other.feed(KEY_RIGHT_ALT, true), "Alt+F11 is left to the peer");

        for order in [
            [KEY_RIGHT_CTRL, KEY_LEFT_ALT, KEY_RIGHT_SHIFT, KEY_ESC],
            [KEY_ESC, KEY_LEFT_SHIFT, KEY_LEFT_CTRL, KEY_RIGHT_ALT],
        ] {
            let mut panic = ReleaseChords::default();
            for key in &order[..3] {
                assert!(!panic.feed(*key, true), "{order:?}");
            }
            assert!(panic.feed(order[3], true), "{order:?}");
        }

        let mut partial = ReleaseChords::default();
        partial.feed(KEY_LEFT_CTRL, true);
        partial.feed(KEY_LEFT_ALT, true);
        assert!(!partial.feed(KEY_ESC, true), "Ctrl+Alt+Esc without Shift is left to the peer");
    }

    #[test]
    fn a_released_modifier_no_longer_counts() {
        let mut chords = ReleaseChords::default();
        chords.feed(KEY_LEFT_META, true);
        chords.feed(KEY_LEFT_ALT, true);
        chords.feed(KEY_LEFT_META, false);
        assert!(!chords.feed(KEY_F11, true));
    }

    fn rect(name: &str, x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { name: name.into(), x, y, width, height }
    }

    fn at(output: &str, x: f64, y: f64) -> Action {
        Action::Move { output: output.into(), x, y }
    }

    /// Whether `actions` is one move to `output` within a hundredth of a pixel of `(x, y)`.
    fn is_at(actions: &[Action], output: &str, x: f64, y: f64) -> bool {
        matches!(actions, [Action::Move { output: name, x: ax, y: ay }] if name == output && (ax - x).abs() < 0.01 && (ay - y).abs() < 0.01)
    }

    #[test]
    fn the_cursor_starts_in_the_middle_of_the_first_output_and_stays_on_a_lone_output() {
        let mut cursor = Cursor::new(vec![rect("A", 0., 0., 1000., 500.)]).unwrap();
        assert_eq!(cursor.position(), (500., 250.));
        assert_eq!(cursor.start(), at("A", 500., 250.));
        assert_eq!(cursor.plan(&Event::Motion { dx: 10., dy: -5. }), [at("A", 510., 245.)]);
        assert!(is_at(&cursor.plan(&Event::Motion { dx: 5000., dy: 5000. }), "A", 1000., 500.), "stops at the edge");
        assert!(is_at(&cursor.plan(&Event::Motion { dx: -5000., dy: -5000. }), "A", 0., 0.));
        assert!(Cursor::new(vec![]).is_none());
    }

    /// A layout like a real desk: two portrait outputs at the sides and two landscape ones stacked
    /// between them, none sharing an origin.
    fn desk() -> Vec<Rect> {
        vec![
            rect("LEFT", 1827., 355., 1080., 1920.),
            rect("TOP", 2907., -112., 1920., 1080.),
            rect("BOTTOM", 2907., 968., 1920., 1080.),
            rect("RIGHT", 4827., 355., 1080., 1920.),
        ]
    }

    #[test]
    fn the_cursor_crosses_from_one_output_to_the_next_instead_of_stopping_at_the_edge() {
        let mut cursor = Cursor::new(desk()).unwrap();
        assert_eq!(cursor.start(), at("LEFT", 540., 960.), "the first output is the leftmost");
        // Right across the border into the output beside it (here the lower one, where y 1315 is).
        let crossing = cursor.plan(&Event::Motion { dx: 600., dy: 0. });
        assert_eq!(crossing, [at("BOTTOM", 540. + 600. - (2907. - 1827.), 1315. - 968.)]);
        // Across it and into the output beyond.
        let beyond = cursor.plan(&Event::Motion { dx: 1900., dy: 0. });
        assert!(matches!(&beyond[0], Action::Move { output, .. } if output == "RIGHT"), "{beyond:?}");
        // Back the way it came.
        let back = cursor.plan(&Event::Motion { dx: -1900., dy: 0. });
        assert!(matches!(&back[0], Action::Move { output, .. } if output == "BOTTOM"), "{back:?}");
    }

    #[test]
    fn the_cursor_moves_between_stacked_outputs_and_stops_at_the_layouts_outer_edge() {
        let mut cursor = Cursor::new(desk()).unwrap();
        cursor.plan(&Event::Motion { dx: 1200., dy: -300. });
        assert_eq!(cursor.position(), (1827. + 540. + 1200., 960. + 355. - 300.));
        let up = cursor.plan(&Event::Motion { dx: 0., dy: -2000. });
        assert!(is_at(&up, "TOP", 660., 0.), "stops at the top of the top output: {up:?}");
        let down = cursor.plan(&Event::Motion { dx: 0., dy: 5000. });
        assert!(is_at(&down, "BOTTOM", 660., 1080.), "stops at the bottom of the bottom one, not on some other monitor: {down:?}");
        let left = cursor.plan(&Event::Motion { dx: -9000., dy: 0. });
        assert!(matches!(&left[..], [Action::Move { output, x, .. }] if output == "LEFT" && x.abs() < 0.01), "{left:?}");
    }

    #[test]
    fn a_step_into_a_gap_lands_on_the_nearest_output() {
        // Two outputs of different heights: the area beside the shorter one is not part of the desk.
        let mut cursor = Cursor::new(vec![rect("TALL", 0., 0., 1000., 2000.), rect("SHORT", 1000., 0., 1000., 500.)]).unwrap();
        cursor.plan(&Event::Motion { dx: 0., dy: 700. });
        let step = cursor.plan(&Event::Motion { dx: 800., dy: 0. });
        assert!(is_at(&step, "TALL", 1000., 1700.), "pushed against the edge of the tall output, not lost beyond it: {step:?}");
        let on = cursor.plan(&Event::Motion { dx: 0., dy: -1500. });
        assert!(is_at(&on, "TALL", 1000., 200.), "{on:?}");
        let over = cursor.plan(&Event::Motion { dx: 5., dy: 0. });
        assert!(is_at(&over, "SHORT", 5., 200.), "and over once it is level with the short one: {over:?}");
    }

    #[test]
    fn a_click_waits_for_the_cursor_to_arrive_once() {
        let mut cursor = Cursor::new(vec![rect("A", 0., 0., 100., 100.)]).unwrap();
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
