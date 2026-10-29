//! Rune key events as Rex key inputs, and the key repeat the layer-shell path lacks.
use std::time::{Duration, Instant};

use dojo_rex::{KeyAction, KeyInput};
use rune::{Key, KeyEvent, PhysicalKeyCode};

pub const REPEAT_DELAY: Duration = Duration::from_millis(600);
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(33);

/// The W3C `KeyboardEvent.code` of a key Rune names only logically. Rune's physical key codes
/// already carry the W3C names, so this is the fallback for keys with no physical origin.
fn logical_code(key: &Key) -> Option<&'static str> {
    Some(match key {
        Key::Enter => "Enter",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Left => "ArrowLeft",
        Key::Right => "ArrowRight",
        Key::Up => "ArrowUp",
        Key::Down => "ArrowDown",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Escape => "Escape",
        Key::Tab => "Tab",
        Key::ContextMenu => "ContextMenu",
        Key::Function(n) => return FUNCTION.get((*n as usize).checked_sub(1)?).copied(),
        Key::Text(text) if text == " " => "Space",
        _ => return None,
    })
}

const FUNCTION: [&str; 24] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16", "F17",
    "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

/// The key input for a Rune key event. `code` is the physical key when the host knows it and
/// `base` the character the key types unmodified. None for keys the encoder can't place.
pub fn key_input(event: &KeyEvent, code: Option<PhysicalKeyCode>, base: Option<char>, action: KeyAction) -> Option<KeyInput> {
    let code = match code {
        Some(code) => format!("{code:?}"),
        None => logical_code(&event.key)?.to_owned(),
    };
    let (text, unshifted) = match &event.key {
        Key::Text(text) if !text.chars().any(char::is_control) => {
            (Some(text.clone()), base.or_else(|| text.chars().next().and_then(|c| c.to_lowercase().next())))
        }
        Key::Command(c) => (None, base.or(Some(*c))),
        _ => (None, base),
    };
    let mods = event.mods;
    Some(KeyInput {
        code,
        text,
        unshifted,
        shift: mods.shift,
        ctrl: mods.ctrl || mods.cmd,
        alt: mods.alt,
        logo: mods.logo,
        action,
    })
}

/// The key being held down, and when it repeats next.
pub struct Held {
    pub input: KeyInput,
    pub next: Instant,
}

impl Held {
    pub fn new(input: KeyInput, now: Instant) -> Self {
        Self { input, next: now + REPEAT_DELAY }
    }

    /// The repeat due at `now`, if any, and the time left until the one after it.
    pub fn due(&mut self, now: Instant) -> (Option<KeyInput>, Duration) {
        if now < self.next {
            return (None, self.next - now);
        }
        self.next = now + REPEAT_INTERVAL;
        let mut input = self.input.clone();
        input.action = KeyAction::Repeat;
        (Some(input), REPEAT_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Mods;

    fn event(key: Key, mods: Mods) -> KeyEvent {
        KeyEvent { key, mods }
    }

    #[test]
    fn typed_text_carries_its_code_and_unshifted_character() {
        let shift = Mods { shift: true, ..Mods::default() };
        let input = key_input(&event(Key::Text("A".into()), shift), Some(PhysicalKeyCode::KeyA), Some('a'), KeyAction::Press).unwrap();
        assert_eq!(input.code, "KeyA");
        assert_eq!(input.text.as_deref(), Some("A"));
        assert_eq!(input.unshifted, Some('a'));
        assert!(input.shift && !input.ctrl);
    }

    #[test]
    fn control_chords_carry_no_text() {
        let ctrl = Mods { ctrl: true, cmd: true, ..Mods::default() };
        let input = key_input(&event(Key::Command('c'), ctrl), Some(PhysicalKeyCode::KeyC), None, KeyAction::Press).unwrap();
        assert_eq!((input.code.as_str(), input.text, input.unshifted, input.ctrl), ("KeyC", None, Some('c'), true));
    }

    #[test]
    fn physical_codes_use_the_w3c_names() {
        let plain = Mods::default();
        for (code, name) in [
            (PhysicalKeyCode::ArrowUp, "ArrowUp"),
            (PhysicalKeyCode::Digit1, "Digit1"),
            (PhysicalKeyCode::BracketLeft, "BracketLeft"),
            (PhysicalKeyCode::Space, "Space"),
            (PhysicalKeyCode::F12, "F12"),
            (PhysicalKeyCode::NumpadEnter, "NumpadEnter"),
        ] {
            assert_eq!(key_input(&event(Key::Up, plain), Some(code), None, KeyAction::Press).unwrap().code, name);
        }
    }

    #[test]
    fn logical_keys_without_a_physical_origin_map_by_name() {
        let plain = Mods::default();
        let code = |key| key_input(&event(key, plain), None, None, KeyAction::Press).map(|input| input.code);
        assert_eq!(code(Key::Left).as_deref(), Some("ArrowLeft"));
        assert_eq!(code(Key::Function(5)).as_deref(), Some("F5"));
        assert_eq!(code(Key::Function(0)), None);
        assert_eq!(code(Key::Text(" ".into())).as_deref(), Some("Space"));
        assert_eq!(code(Key::Text("x".into())), None);
    }

    #[test]
    fn a_held_key_waits_then_repeats_at_the_interval() {
        let start = Instant::now();
        let input = key_input(&event(Key::Enter, Mods::default()), None, None, KeyAction::Press).unwrap();
        let mut held = Held::new(input, start);
        let (none, left) = held.due(start + Duration::from_millis(100));
        assert!(none.is_none());
        assert_eq!(left, Duration::from_millis(500));
        let (repeat, next) = held.due(start + REPEAT_DELAY);
        assert_eq!(repeat.unwrap().action, KeyAction::Repeat);
        assert_eq!(next, REPEAT_INTERVAL);
    }
}
