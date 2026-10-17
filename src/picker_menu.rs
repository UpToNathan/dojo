//! Pages and rows of the model picker menu, independent of GPUI and the voice socket.
//!
//! The menu has two steps: accounts (each with its current model, or Sign in), then the chosen
//! account's models with Back and Sign out. Only the model step can outgrow the menu, and it shows
//! a short window of rows that follows the cursor instead of a scrolling list. Long model pages can be
//! narrowed by typing, and the window moves by keys, pages, or wheel notches and smooth-scroll distance.

use std::ops::Range;

use crate::model_picker::{Account, ModelCatalog};
use crate::sign_in;

/// Most model rows shown at once.
pub const MAX_MODEL_ROWS: usize = 7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Accounts,
    Models(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Account(String),
    Back,
    /// Index into the catalog's model list.
    Model(usize),
    SignOut(String),
    /// Replace a login borrowed from another app with a Dojo login.
    SignInDojo(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub item: Item,
    pub title: String,
    pub detail: String,
    /// The account or model currently in use.
    pub marked: bool,
}

pub fn model_count(catalog: &ModelCatalog, account: &str) -> usize {
    catalog
        .models
        .iter()
        .filter(|row| row.account == account)
        .count()
}

/// Accounts the menu offers: the sign-in methods this shell supports, and any other account that works.
fn listed(account: &Account) -> bool {
    sign_in::is_chatgpt(account) || sign_in::is_opencode(account) || account.state == "available"
}

/// The page to show after the catalog changed: a model page whose account is gone falls back to accounts.
pub fn settle(catalog: &ModelCatalog, page: &Page) -> Page {
    match page {
        Page::Models(id)
            if catalog
                .accounts
                .iter()
                .any(|account| &account.id == id && account.state == "available") =>
        {
            page.clone()
        }
        _ => Page::Accounts,
    }
}

/// Whether a model matches every word of `query`, ignoring case, in its name or identifier.
fn matches(row: &crate::model_picker::ModelRow, query: &str) -> bool {
    let haystack = format!("{} {}", row.name, row.model).to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

/// Rows of `page`. On a model page only models matching `query` are listed; Back and the footer always stay.
pub fn entries(catalog: &ModelCatalog, page: &Page, query: &str) -> Vec<Entry> {
    match page {
        Page::Accounts => {
            let active = catalog
                .selection
                .as_ref()
                .filter(|selection| selection.available);
            catalog
                .accounts
                .iter()
                .filter(|account| listed(account))
                .map(|account| {
                    let available = account.state == "available";
                    let current = active.filter(|selection| selection.account == account.id);
                    let detail = if let Some(selection) = current {
                        selection.name.clone()
                    } else if available {
                        match model_count(catalog, &account.id) {
                            1 => "1 model".to_owned(),
                            count => format!("{count} models"),
                        }
                    } else {
                        "Sign in".to_owned()
                    };
                    Entry {
                        item: Item::Account(account.id.clone()),
                        title: account.label.clone(),
                        detail,
                        marked: current.is_some(),
                    }
                })
                .collect()
        }
        Page::Models(id) => {
            let Some(account) = catalog.accounts.iter().find(|account| &account.id == id) else {
                return Vec::new();
            };
            let active = catalog.selection.as_ref();
            let mut rows = vec![Entry {
                item: Item::Back,
                title: "‹ Accounts".to_owned(),
                detail: account.label.clone(),
                marked: false,
            }];
            for (index, row) in catalog
                .models
                .iter()
                .enumerate()
                .filter(|(_, row)| &row.account == id && matches(row, query))
            {
                let in_use = active.is_some_and(|selection| {
                    selection.account == row.account && selection.model == row.model
                });
                rows.push(Entry {
                    item: Item::Model(index),
                    title: row.name.clone(),
                    detail: format!(
                        "{}${:.2}/${:.2}",
                        if in_use { "In use · " } else { "" },
                        row.cost.input,
                        row.cost.output
                    ),
                    marked: in_use,
                });
            }
            rows.push(
                if sign_in::account_action(account) == sign_in::AccountAction::SignOut {
                    Entry {
                        item: Item::SignOut(id.clone()),
                        title: "Sign out".to_owned(),
                        detail: String::new(),
                        marked: false,
                    }
                } else {
                    Entry {
                        item: Item::SignInDojo(id.clone()),
                        title: "Sign in with Dojo".to_owned(),
                        detail: "Replace the shared login".to_owned(),
                        marked: false,
                    }
                },
            );
            rows
        }
    }
}

/// The cursor after moving by `delta`, wrapping at both ends.
pub fn step(cursor: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (cursor as isize + delta).rem_euclid(len as isize) as usize
}

/// The cursor after moving by `delta` without wrapping, for page and edge keys.
pub fn clamp_step(cursor: usize, len: usize, delta: isize) -> usize {
    (cursor as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize
}

/// Turns wheel events into whole rows: a notch moves one row, and smooth scrolling (touchpads report many
/// small pixel deltas) accumulates until a full row has been travelled.
#[derive(Default)]
pub struct WheelRows {
    residue: f32,
}

impl WheelRows {
    /// Rows to move for an event of `delta` (positive moves up), given the pixels one row spans.
    pub fn feed(&mut self, delta: f32, notch: bool, row: f32) -> isize {
        if notch {
            self.residue = 0.;
            return if delta > 0. {
                1
            } else if delta < 0. {
                -1
            } else {
                0
            };
        }
        if delta * self.residue < 0. {
            self.residue = 0.;
        }
        self.residue += delta;
        let rows = (self.residue / row).trunc();
        self.residue -= rows * row;
        rows as isize
    }

    pub fn reset(&mut self) {
        self.residue = 0.;
    }
}

/// The first row of a window of `max` rows over `len` rows that keeps `cursor` visible and moves as little
/// as possible from `top`.
pub fn window_top(top: usize, cursor: usize, len: usize, max: usize) -> usize {
    if len <= max {
        return 0;
    }
    let top = top.min(len - max);
    if cursor < top {
        cursor
    } else if cursor >= top + max {
        cursor + 1 - max
    } else {
        top
    }
}

/// Rows `top..top + max` clamped to `len`.
pub fn window(top: usize, len: usize, max: usize) -> Range<usize> {
    top.min(len.saturating_sub(max))..(top + max).min(len).max(top.min(len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_picker::parse_catalog;

    fn catalog(selection: &str) -> ModelCatalog {
        parse_catalog(&format!(
            r#"{{"accounts":[
              {{"id":"opencode-go","label":"OpenCode Go","state":"available","source":"dojo","backend":"keyring"}},
              {{"id":"opencode","label":"OpenCode Zen","state":"unavailable","reason":"Add an OpenCode API key."}},
              {{"id":"openai-codex","label":"ChatGPT","state":"available","source":"borrowed"}},
              {{"id":"openai","label":"OpenAI","state":"unavailable"}}],
             "models":[
              {{"account":"opencode-go","accountLabel":"OpenCode Go","model":"a","name":"Alpha","reasoning":false,"cost":{{"input":0.15,"output":0.6}}}},
              {{"account":"opencode-go","accountLabel":"OpenCode Go","model":"b","name":"Beta","reasoning":false,"cost":{{"input":1.0,"output":2.0}}}},
              {{"account":"openai-codex","accountLabel":"ChatGPT","model":"c","name":"Gamma","reasoning":false,"cost":{{"input":1.0,"output":4.0}}}}],
             "selection":{selection}}}"#
        ))
        .unwrap()
    }
    const GO_ALPHA: &str =
        r#"{"account":"opencode-go","model":"a","name":"Alpha","available":true}"#;

    #[test]
    fn accounts_page_shows_the_current_model_model_counts_and_sign_in() {
        let rows = entries(&catalog(GO_ALPHA), &Page::Accounts, "");
        let summary: Vec<_> = rows
            .iter()
            .map(|row| (row.title.as_str(), row.detail.as_str(), row.marked))
            .collect();
        assert_eq!(
            summary,
            [
                ("OpenCode Go", "Alpha", true),
                ("OpenCode Zen", "Sign in", false),
                ("ChatGPT", "1 model", false),
            ]
        );
    }

    #[test]
    fn unsupported_accounts_are_listed_only_when_they_work() {
        let mut available = catalog(GO_ALPHA);
        available.accounts[3].state = "available".into();
        assert_eq!(entries(&available, &Page::Accounts, "").len(), 4);
    }

    #[test]
    fn an_unavailable_selection_is_not_marked_current() {
        let rows = entries(
            &catalog(r#"{"account":"opencode-go","model":"a","name":"Alpha","available":false}"#),
            &Page::Accounts,
            "",
        );
        assert!(rows.iter().all(|row| !row.marked));
        assert_eq!(rows[0].detail, "2 models");
    }

    #[test]
    fn model_page_has_back_models_and_a_footer() {
        let rows = entries(&catalog(GO_ALPHA), &Page::Models("opencode-go".into()), "");
        let items: Vec<_> = rows.iter().map(|row| row.item.clone()).collect();
        assert_eq!(
            items,
            [
                Item::Back,
                Item::Model(0),
                Item::Model(1),
                Item::SignOut("opencode-go".into())
            ]
        );
        assert_eq!(rows[1].detail, "In use · $0.15/$0.60");
        assert!(rows[1].marked && !rows[2].marked);
    }

    #[test]
    fn a_borrowed_login_offers_a_dojo_login_instead_of_sign_out() {
        let rows = entries(&catalog(GO_ALPHA), &Page::Models("openai-codex".into()), "");
        assert_eq!(
            rows.last().unwrap().item,
            Item::SignInDojo("openai-codex".into())
        );
    }

    #[test]
    fn a_model_page_returns_to_accounts_when_its_account_goes_away() {
        let mut changed = catalog(GO_ALPHA);
        let page = Page::Models("opencode-go".into());
        assert_eq!(settle(&changed, &page), page);
        changed.accounts[0].state = "unavailable".into();
        assert_eq!(settle(&changed, &page), Page::Accounts);
        assert_eq!(settle(&changed, &Page::Accounts), Page::Accounts);
    }

    #[test]
    fn cursor_wraps_and_empty_menus_stay_at_zero() {
        assert_eq!(step(0, 3, -1), 2);
        assert_eq!(step(2, 3, 1), 0);
        assert_eq!(step(0, 0, 1), 0);
    }

    #[test]
    fn the_window_follows_the_cursor_with_the_least_movement() {
        assert_eq!(window_top(0, 3, 5, 7), 0);
        assert_eq!(window_top(0, 6, 20, 7), 0);
        assert_eq!(window_top(0, 7, 20, 7), 1);
        assert_eq!(window_top(4, 4, 20, 7), 4);
        assert_eq!(window_top(4, 2, 20, 7), 2);
        assert_eq!(window_top(30, 19, 20, 7), 13);
        assert_eq!(window(13, 20, 7), 13..20);
        assert_eq!(window(0, 3, 7), 0..3);
    }

    #[test]
    fn typing_narrows_the_model_page_but_keeps_back_and_the_footer() {
        let page = Page::Models("opencode-go".into());
        let items = |query: &str| -> Vec<Item> {
            entries(&catalog(GO_ALPHA), &page, query)
                .into_iter()
                .map(|row| row.item)
                .collect()
        };
        assert_eq!(
            items("BET"),
            [
                Item::Back,
                Item::Model(1),
                Item::SignOut("opencode-go".into())
            ]
        );
        assert_eq!(items("  alp  "), items("alp"));
        assert_eq!(items("zzz").len(), 2);
        assert_eq!(items("alpha b").len(), 2);
    }

    #[test]
    fn page_keys_stop_at_the_ends() {
        assert_eq!(clamp_step(2, 20, 7), 9);
        assert_eq!(clamp_step(15, 20, 7), 19);
        assert_eq!(clamp_step(3, 20, -7), 0);
        assert_eq!(clamp_step(0, 0, 7), 0);
    }

    #[test]
    fn a_notch_moves_one_row_and_smooth_scrolling_accumulates() {
        let mut wheel = WheelRows::default();
        assert_eq!(wheel.feed(-120., true, 40.), -1);
        assert_eq!(wheel.feed(300., true, 40.), 1);
        // A touchpad drag of many small deltas moves a row per 40px, not per event.
        let moved: isize = (0..10).map(|_| wheel.feed(-3., false, 40.)).sum();
        assert_eq!(moved, 0);
        let moved: isize = (0..10).map(|_| wheel.feed(-3., false, 40.)).sum();
        assert_eq!(moved, -1);
        // Reversing direction discards the leftover instead of cancelling against it.
        wheel.feed(30., false, 40.);
        assert_eq!(wheel.feed(-45., false, 40.), -1);
    }
}
