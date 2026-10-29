//! The window's own key bindings. Everything else goes to the focused pane.
use rune::{Key, KeyEvent, PhysicalKeyCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    /// Tab number, from 1.
    GoToTab(usize),
    SplitRight,
    SplitDown,
    Focus(Direction),
    Zoom,
    ClosePane,
    Picker,
    Paste,
}

/// The W3C name of the key: the physical one when the host knows it, else from the logical key.
fn code(event: &KeyEvent, physical: Option<PhysicalKeyCode>) -> Option<String> {
    if let Some(physical) = physical {
        return Some(format!("{physical:?}"));
    }
    Some(match &event.key {
        Key::Command(c) if c.is_ascii_alphabetic() => format!("Key{}", c.to_ascii_uppercase()),
        Key::Command(c) if c.is_ascii_digit() => format!("Digit{c}"),
        Key::Tab => "Tab".into(),
        Key::Left => "ArrowLeft".into(),
        Key::Right => "ArrowRight".into(),
        Key::Up => "ArrowUp".into(),
        Key::Down => "ArrowDown".into(),
        _ => return None,
    })
}

/// The action a key press is bound to. Bindings need Ctrl; Alt and Logo make a key the
/// terminal's own.
pub fn action(event: &KeyEvent, physical: Option<PhysicalKeyCode>) -> Option<Action> {
    let mods = event.mods;
    if !(mods.ctrl || mods.cmd) || mods.alt || mods.logo {
        return None;
    }
    let code = code(event, physical)?;
    if mods.shift {
        return Some(match code.as_str() {
            "KeyT" => Action::NewTab,
            "KeyW" => Action::CloseTab,
            "Tab" => Action::PreviousTab,
            "KeyD" => Action::SplitRight,
            "KeyE" => Action::SplitDown,
            "ArrowLeft" => Action::Focus(Direction::Left),
            "ArrowRight" => Action::Focus(Direction::Right),
            "ArrowUp" => Action::Focus(Direction::Up),
            "ArrowDown" => Action::Focus(Direction::Down),
            "KeyZ" => Action::Zoom,
            "KeyX" => Action::ClosePane,
            "KeyS" => Action::Picker,
            "KeyV" => Action::Paste,
            _ => return None,
        });
    }
    if code == "Tab" {
        return Some(Action::NextTab);
    }
    let digit = code.strip_prefix("Digit")?.parse::<usize>().ok()?;
    (1..=9).contains(&digit).then_some(Action::GoToTab(digit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Mods;

    fn press(key: Key, shift: bool, physical: Option<PhysicalKeyCode>) -> Option<Action> {
        action(&KeyEvent { key, mods: Mods { ctrl: true, cmd: true, shift, ..Mods::default() } }, physical)
    }

    #[test]
    fn the_spec_bindings() {
        let k = |code| Some(code);
        assert_eq!(press(Key::Command('t'), true, k(PhysicalKeyCode::KeyT)), Some(Action::NewTab));
        assert_eq!(press(Key::Command('w'), true, k(PhysicalKeyCode::KeyW)), Some(Action::CloseTab));
        assert_eq!(press(Key::Tab, false, k(PhysicalKeyCode::Tab)), Some(Action::NextTab));
        assert_eq!(press(Key::Tab, true, k(PhysicalKeyCode::Tab)), Some(Action::PreviousTab));
        assert_eq!(press(Key::Command('3'), false, k(PhysicalKeyCode::Digit3)), Some(Action::GoToTab(3)));
        assert_eq!(press(Key::Command('d'), true, k(PhysicalKeyCode::KeyD)), Some(Action::SplitRight));
        assert_eq!(press(Key::Command('e'), true, k(PhysicalKeyCode::KeyE)), Some(Action::SplitDown));
        assert_eq!(press(Key::Left, true, k(PhysicalKeyCode::ArrowLeft)), Some(Action::Focus(Direction::Left)));
        assert_eq!(press(Key::Down, true, k(PhysicalKeyCode::ArrowDown)), Some(Action::Focus(Direction::Down)));
        assert_eq!(press(Key::Command('z'), true, k(PhysicalKeyCode::KeyZ)), Some(Action::Zoom));
        assert_eq!(press(Key::Command('x'), true, k(PhysicalKeyCode::KeyX)), Some(Action::ClosePane));
        assert_eq!(press(Key::Command('s'), true, k(PhysicalKeyCode::KeyS)), Some(Action::Picker));
        assert_eq!(press(Key::Command('v'), true, k(PhysicalKeyCode::KeyV)), Some(Action::Paste));
    }

    #[test]
    fn logical_keys_bind_without_a_physical_code() {
        assert_eq!(press(Key::Command('t'), true, None), Some(Action::NewTab));
        assert_eq!(press(Key::Command('5'), false, None), Some(Action::GoToTab(5)));
        assert_eq!(press(Key::Up, true, None), Some(Action::Focus(Direction::Up)));
    }

    #[test]
    fn everything_else_belongs_to_the_terminal() {
        // Ctrl+C, Ctrl+D (no shift), Ctrl+0, plain keys, Alt chords.
        assert_eq!(press(Key::Command('c'), false, Some(PhysicalKeyCode::KeyC)), None);
        assert_eq!(press(Key::Command('d'), false, Some(PhysicalKeyCode::KeyD)), None);
        assert_eq!(press(Key::Command('0'), false, Some(PhysicalKeyCode::Digit0)), None);
        let plain = KeyEvent { key: Key::Text("t".into()), mods: Mods::default() };
        assert_eq!(action(&plain, Some(PhysicalKeyCode::KeyT)), None);
        let alt = KeyEvent { key: Key::Command('t'), mods: Mods { ctrl: true, cmd: true, shift: true, alt: true, ..Mods::default() } };
        assert_eq!(action(&alt, Some(PhysicalKeyCode::KeyT)), None);
    }
}
