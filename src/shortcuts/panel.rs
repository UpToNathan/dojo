//! What the binding panel shows and how keys move through it, independent of GPUI.
//!
//! The panel lists an app's shortcuts. Enter on a row waits for the new key combination; Done
//! accepts the list. It opens with Done selected, so approving what an app suggested is one key.

use super::portal::ChoiceRequest;
use super::registry::Choice;
use super::trigger::Trigger;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl Mods {
    fn none(self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.logo)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub description: String,
    pub trigger: Option<Trigger>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Browsing,
    /// The selected row is waiting for a key combination.
    Capturing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    Done(Vec<Choice>),
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub title: String,
    pub rows: Vec<Row>,
    /// A row, or one past the last row for Done.
    pub selected: usize,
    pub mode: Mode,
    pub notice: Option<String>,
}

impl Model {
    pub fn new(request: &ChoiceRequest) -> Self {
        let rows: Vec<Row> = request
            .shortcuts
            .iter()
            .map(|pending| Row {
                id: pending.id.clone(),
                description: if pending.description.is_empty() {
                    pending.id.clone()
                } else {
                    pending.description.clone()
                },
                trigger: pending.current.clone().or_else(|| pending.preferred.clone()),
            })
            .collect();
        let app = if request.app_id.is_empty() {
            "An app".to_owned()
        } else {
            request.app_id.clone()
        };
        let title = if request.reconfigure {
            format!("Shortcuts for {app}")
        } else {
            format!("{app} wants shortcuts")
        };
        Self {
            title,
            selected: rows.len(),
            rows,
            mode: Mode::Browsing,
            notice: None,
        }
    }

    pub fn done_selected(&self) -> bool {
        self.selected == self.rows.len()
    }

    pub fn select(&mut self, index: usize) {
        if index <= self.rows.len() && self.mode == Mode::Browsing {
            self.selected = index;
        }
    }

    /// Enter, or a click on the selected row.
    pub fn activate(&mut self) -> Outcome {
        if self.done_selected() {
            return Outcome::Done(self.choices());
        }
        self.mode = Mode::Capturing;
        self.notice = None;
        Outcome::Pending
    }

    pub fn choices(&self) -> Vec<Choice> {
        self.rows
            .iter()
            .map(|row| Choice {
                id: row.id.clone(),
                trigger: row.trigger.clone(),
            })
            .collect()
    }

    /// A key pressed in the panel. `key` is GPUI's key name.
    pub fn key(&mut self, key: &str, mods: Mods) -> Outcome {
        match self.mode {
            Mode::Browsing => self.browse(key, mods),
            Mode::Capturing => self.capture(key, mods),
        }
    }

    fn browse(&mut self, key: &str, mods: Mods) -> Outcome {
        let count = self.rows.len() + 1;
        match key {
            "escape" => return Outcome::Cancelled,
            "down" => self.selected = (self.selected + 1) % count,
            "up" => self.selected = (self.selected + count - 1) % count,
            "enter" if mods.none() => return self.activate(),
            "backspace" | "delete" if mods.none() && !self.done_selected() => {
                self.rows[self.selected].trigger = None;
                self.notice = None;
            }
            _ => {}
        }
        Outcome::Pending
    }

    fn capture(&mut self, key: &str, mods: Mods) -> Outcome {
        match key {
            "escape" if mods.none() => {
                self.mode = Mode::Browsing;
                self.notice = None;
            }
            "backspace" | "delete" if mods.none() => {
                self.rows[self.selected].trigger = None;
                self.mode = Mode::Browsing;
                self.notice = None;
            }
            _ => {
                // A lone modifier is only the start of a combination: keep waiting.
                let Ok(trigger) =
                    Trigger::from_keystroke(mods.ctrl, mods.alt, mods.shift, mods.logo, key)
                else {
                    return Outcome::Pending;
                };
                let taken = self
                    .rows
                    .iter()
                    .enumerate()
                    .find(|(index, row)| *index != self.selected && row.trigger.as_ref() == Some(&trigger));
                if let Some((_, row)) = taken {
                    self.notice = Some(format!("{trigger} is already used by {}", row.description));
                } else {
                    self.rows[self.selected].trigger = Some(trigger);
                    self.mode = Mode::Browsing;
                    self.notice = None;
                }
            }
        }
        Outcome::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::registry::Pending;

    fn trigger(text: &str) -> Trigger {
        Trigger::from_xdg(text).unwrap()
    }

    fn request(reconfigure: bool) -> ChoiceRequest {
        ChoiceRequest {
            app_id: "org.example.Recorder".into(),
            parent_window: String::new(),
            reconfigure,
            shortcuts: vec![
                Pending {
                    id: "start".into(),
                    description: "Start recording".into(),
                    preferred: Some(trigger("CTRL+F1")),
                    current: None,
                },
                Pending {
                    id: "stop".into(),
                    description: String::new(),
                    preferred: None,
                    current: Some(trigger("CTRL+F2")),
                },
            ],
        }
    }

    const CTRL: Mods = Mods {
        ctrl: true,
        alt: false,
        shift: false,
        logo: false,
    };

    #[test]
    fn opens_on_done_with_suggestions_filled_in() {
        let mut model = Model::new(&request(false));
        assert_eq!(model.title, "org.example.Recorder wants shortcuts");
        assert_eq!(model.rows[0].trigger, Some(trigger("CTRL+F1")));
        assert_eq!(model.rows[1].description, "stop", "a missing description falls back to the id");
        assert!(model.done_selected());
        let Outcome::Done(choices) = model.key("enter", Mods::default()) else {
            panic!("Enter on Done should accept");
        };
        assert_eq!(choices[0].trigger, Some(trigger("CTRL+F1")));
        assert_eq!(choices[1].trigger, Some(trigger("CTRL+F2")));
        assert_eq!(Model::new(&request(true)).title, "Shortcuts for org.example.Recorder");
    }

    #[test]
    fn rebinding_a_row_takes_the_next_combination() {
        let mut model = Model::new(&request(true));
        model.key("up", Mods::default());
        model.key("up", Mods::default());
        assert_eq!(model.selected, 0);
        assert_eq!(model.key("enter", Mods::default()), Outcome::Pending);
        assert_eq!(model.mode, Mode::Capturing);
        // A modifier alone keeps waiting.
        assert_eq!(model.key("control", CTRL), Outcome::Pending);
        assert_eq!(model.mode, Mode::Capturing);
        model.key("k", Mods { alt: true, ..CTRL });
        assert_eq!(model.mode, Mode::Browsing);
        assert_eq!(model.rows[0].trigger, Some(trigger("CTRL+ALT+k")));
    }

    #[test]
    fn a_combination_used_by_another_row_is_refused() {
        let mut model = Model::new(&request(true));
        model.select(0);
        model.key("enter", Mods::default());
        assert_eq!(model.key("f2", CTRL), Outcome::Pending);
        assert_eq!(model.mode, Mode::Capturing);
        assert_eq!(model.notice.as_deref(), Some("Ctrl+F2 is already used by stop"));
        assert_eq!(model.rows[0].trigger, Some(trigger("CTRL+F1")));
        // Choosing the row's own key again is fine.
        model.key("f1", CTRL);
        assert_eq!(model.mode, Mode::Browsing);
        assert_eq!(model.notice, None);
    }

    #[test]
    fn escape_backs_out_one_level_then_cancels() {
        let mut model = Model::new(&request(true));
        model.select(0);
        model.key("enter", Mods::default());
        assert_eq!(model.key("escape", Mods::default()), Outcome::Pending);
        assert_eq!(model.mode, Mode::Browsing);
        assert_eq!(model.rows[0].trigger, Some(trigger("CTRL+F1")), "escape keeps the old key");
        assert_eq!(model.key("escape", Mods::default()), Outcome::Cancelled);
    }

    #[test]
    fn backspace_unbinds_while_browsing_or_capturing() {
        let mut model = Model::new(&request(true));
        model.select(0);
        model.key("backspace", Mods::default());
        assert_eq!(model.rows[0].trigger, None);
        model.select(1);
        model.key("enter", Mods::default());
        model.key("backspace", Mods::default());
        assert_eq!(model.rows[1].trigger, None);
        assert_eq!(model.mode, Mode::Browsing);
        // Done is not a row: backspace there does nothing.
        model.select(2);
        model.key("backspace", Mods::default());
        let Outcome::Done(choices) = model.activate() else {
            panic!()
        };
        assert!(choices.iter().all(|choice| choice.trigger.is_none()));
    }

    #[test]
    fn selection_wraps_and_ignores_hover_while_capturing() {
        let mut model = Model::new(&request(true));
        model.key("down", Mods::default());
        assert_eq!(model.selected, 0);
        model.key("up", Mods::default());
        assert!(model.done_selected());
        model.select(1);
        model.key("enter", Mods::default());
        model.select(0);
        assert_eq!(model.selected, 1, "the row being captured stays selected");
    }
}
