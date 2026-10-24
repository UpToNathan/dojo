use std::io::{BufRead, BufReader, Write};
use std::ops::Range;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Bounds, Context, DisplayId, FocusHandle, IntoElement, KeyDownEvent,
    MouseButton, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, layer_shell, point, prelude::*, px,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::ThemeColors;

const PANEL_WIDTH: f32 = 320.;
const PADDING: f32 = 14.;
const HEADER_HEIGHT: f32 = 34.;
const SECTION_GAP: f32 = 10.;
const ROW_HEIGHT: f32 = 36.;
const ROW_GAP: f32 = 4.;
const CAPTION_HEIGHT: f32 = 20.;
const SEARCH_HEIGHT: f32 = 34.;
const MAX_QUERY: usize = 40;
use crate::picker_menu::{self, Entry, Item, Page};
use crate::sign_in::{self, Effect, Flow, LoginStatus, Progress, needs_setup};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub state: String,
    pub reason: Option<String>,
    pub source: Option<String>,
    pub backend: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ModelRow {
    pub account: String,
    #[serde(rename = "accountLabel")]
    pub account_label: String,
    pub model: String,
    pub name: String,
    pub api: Option<String>,
    pub context: Option<u64>,
    #[serde(rename = "maxTokens")]
    pub max_tokens: Option<u64>,
    pub reasoning: bool,
    pub cost: Cost,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Selection {
    pub account: String,
    pub model: String,
    pub name: String,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ModelCatalog {
    pub accounts: Vec<Account>,
    pub models: Vec<ModelRow>,
    pub selection: Option<Selection>,
}

pub fn parse_catalog(input: &str) -> Result<ModelCatalog, String> {
    let catalog: ModelCatalog =
        serde_json::from_str(input).map_err(|error| format!("Invalid model catalog: {error}"))?;
    if catalog
        .models
        .iter()
        .any(|model| model.account.is_empty() || model.model.is_empty())
    {
        return Err("Invalid model catalog: account and model IDs must be nonempty".into());
    }
    Ok(catalog)
}

/// One request to the Dojo agent runtime's private socket (`dojo-voice.sock`, served by
/// `dojo-voice.service`, which runs with or without a microphone). Sign-in requests are
/// write-only: a key is sent once and never comes back in any response.
fn socket_call(body: &Value) -> Result<Value, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is required")?;
    socket_call_at(
        &std::path::PathBuf::from(runtime).join("dojo-voice.sock"),
        body,
    )
}

fn socket_call_at(path: &std::path::Path, body: &Value) -> Result<Value, String> {
    let mut socket = UnixStream::connect(path).map_err(|error| {
        format!("Could not connect to the Dojo agent runtime (dojo-voice.service): {error}")
    })?;
    socket
        .set_read_timeout(Some(Duration::from_secs(45)))
        .map_err(|error| error.to_string())?;
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let request = serde_json::to_string(body).map_err(|error| error.to_string())?;
    writeln!(socket, "{request}").map_err(|error| error.to_string())?;
    let mut response = String::new();
    BufReader::new(socket)
        .read_line(&mut response)
        .map_err(|error| format!("The Dojo agent runtime did not respond: {error}"))?;
    let value: Value = serde_json::from_str(&response)
        .map_err(|error| format!("Invalid Dojo agent runtime response: {error}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_owned());
    }
    Ok(value)
}

fn catalog_from(value: Value) -> Result<ModelCatalog, String> {
    parse_catalog(&value.to_string())
}

/// Runs a socket request off the UI thread and hands the result back to the picker.
fn dispatch(
    cx: &mut Context<Picker>,
    body: Value,
    done: impl FnOnce(&mut Picker, Result<Value, String>, &mut Context<Picker>) + 'static,
) {
    cx.spawn(async move |this, cx| {
        let result = cx
            .background_executor()
            .spawn(async move {
                std::thread::spawn(move || socket_call(&body))
                    .join()
                    .map_err(|_| "Voice request worker failed".to_string())?
            })
            .await;
        let _ = this.update(cx, |picker: &mut Picker, cx| done(picker, result, cx));
    })
    .detach();
}

/// Whether the agent has no usable account. The top-bar glyph shows a needs-setup state while true.
#[derive(Default)]
pub struct SetupNeeded(pub bool);
impl gpui::Global for SetupNeeded {}

pub fn setup_needed(cx: &App) -> bool {
    cx.try_global::<SetupNeeded>().is_some_and(|state| state.0)
}

fn record_setup(catalog: &ModelCatalog, cx: &mut App) {
    let needed = needs_setup(catalog);
    if setup_needed(cx) != needed {
        cx.set_global(SetupNeeded(needed));
        cx.refresh_windows();
    }
}

/// Whether `dojo-voice.service` is up right now. Asking systemd does not start it; connecting to
/// its socket would, and a poll that woke an idle runtime every few minutes would keep it alive.
fn runtime_active() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "dojo-voice.service"])
        .status()
        .is_ok_and(|status| status.success())
}

/// Reads account state once at startup and then every few minutes, so the glyph reflects reality
/// without opening the menu. The runtime is socket activated and exits when idle; one that is not
/// running is not woken for this and leaves the glyph unchanged until the menu is opened.
pub fn watch_setup(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let result = cx
                .background_executor()
                .spawn(async {
                    std::thread::spawn(|| {
                        if !runtime_active() {
                            return Err("The agent runtime is idle".to_string());
                        }
                        socket_call(&json!({"action": "models"})).and_then(catalog_from)
                    })
                    .join()
                    .map_err(|_| "Voice request worker failed".to_string())?
                })
                .await;
            if let Ok(catalog) = result {
                let _ = cx.update(|cx| record_setup(&catalog, cx));
            }
            cx.background_executor()
                .timer(Duration::from_secs(300))
                .await;
        }
    })
    .detach();
}

#[derive(Default)]
pub struct ModelPicker {
    panel: Option<(DisplayId, WindowHandle<Picker>)>,
}
impl gpui::Global for ModelPicker {}

pub fn close(cx: &mut App) {
    if !cx.has_global::<ModelPicker>() {
        return;
    }
    let panel = cx.update_global::<ModelPicker, _>(|state, _| state.panel.take());
    if let Some((_, panel)) = panel {
        let _ = panel.update(cx, |picker, window, _| {
            picker.forget_flow();
            window.remove_window()
        });
    }
}

pub fn toggle(display_id: DisplayId, cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<ModelPicker>() {
        cx.set_global(ModelPicker::default());
    }
    let previous = cx.update_global::<ModelPicker, _>(|state, _| state.panel.take());
    if let Some((previous_display, panel)) = previous {
        if panel
            .update(cx, |picker, window, _| {
                picker.forget_flow();
                window.remove_window()
            })
            .is_ok()
            && previous_display == display_id
        {
            return Ok(());
        }
    }
    let display = cx
        .displays()
        .into_iter()
        .find(|display| display.id() == display_id)
        .ok_or_else(|| anyhow::anyhow!("Model picker output disappeared"))?;
    let panel = cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-model-picker".into(),
                layer: layer_shell::Layer::Overlay,
                anchor: layer_shell::Anchor::TOP
                    | layer_shell::Anchor::BOTTOM
                    | layer_shell::Anchor::LEFT
                    | layer_shell::Anchor::RIGHT,
                exclusive_zone: Some(px(-1.)),
                keyboard_interactivity: layer_shell::KeyboardInteractivity::Exclusive,
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                let mut picker = Picker {
                    focus,
                    catalog: None,
                    error: None,
                    page: Page::Accounts,
                    cursor: 0,
                    top: 0,
                    query: String::new(),
                    wheel: picker_menu::WheelRows::default(),
                    reveal: dojo::design::MenuReveal::opening_for(
                        Instant::now(),
                        Duration::from_millis(150),
                    ),
                    flow: Flow::None,
                    poll: None,
                };
                picker.reload(cx);
                picker
            })
        },
    )?;
    cx.update_global::<ModelPicker, _>(|state, _| state.panel = Some((display_id, panel)));
    Ok(())
}

struct Picker {
    focus: FocusHandle,
    catalog: Option<ModelCatalog>,
    error: Option<String>,
    page: Page,
    cursor: usize,
    top: usize,
    /// Words typed on a model page to narrow its models.
    query: String,
    wheel: picker_menu::WheelRows,
    reveal: dojo::design::MenuReveal,
    flow: Flow,
    poll: Option<gpui::Task<()>>,
}
impl Picker {
    /// Drops any in-progress sign-in, cancelling a waiting ChatGPT login and discarding a typed key.
    fn forget_flow(&mut self) {
        self.poll = None;
        let target = self.flow.cancel_target();
        self.flow = Flow::None;
        if let Some(login_id) = target {
            std::thread::spawn(move || {
                let _ = socket_call(&json!({"action": "cancel-login", "loginId": login_id}));
            });
        }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.forget_flow();
        self.reveal.set_open(false, Instant::now());
        cx.notify();
    }

    fn entries(&self) -> Vec<Entry> {
        self.catalog
            .as_ref()
            .map(|catalog| picker_menu::entries(catalog, &self.page, &self.query))
            .unwrap_or_default()
    }

    /// Moves to a page with the cursor on the row in use, or on the first choice.
    fn open_page(&mut self, page: Page) {
        self.page = page;
        self.query.clear();
        self.wheel.reset();
        let entries = self.entries();
        self.cursor = entries
            .iter()
            .position(|entry| entry.marked)
            .unwrap_or(usize::from(
                matches!(self.page, Page::Models(_)) && entries.len() > 2,
            ));
        self.top = 0;
        self.keep_cursor_visible();
    }

    /// Whether the model page has a filter field: shown for long lists, and while a filter is applied.
    fn searchable(&self) -> bool {
        let (Page::Models(id), Some(catalog)) = (&self.page, &self.catalog) else {
            return false;
        };
        !self.query.is_empty()
            || picker_menu::model_count(catalog, id) > picker_menu::MAX_MODEL_ROWS
    }

    /// Applies a changed filter: the cursor lands on the first match and the window returns to the top.
    fn filter_changed(&mut self) {
        let entries = self.entries();
        let models = model_span(&entries);
        // With no match the cursor stays on Back, never on the destructive footer row.
        self.cursor = if models.is_empty() { 0 } else { models.start };
        self.top = 0;
    }

    fn keep_cursor_visible(&mut self) {
        let entries = self.entries();
        let models = model_span(&entries);
        if models.contains(&self.cursor) {
            self.top = picker_menu::window_top(
                self.top,
                self.cursor - models.start,
                models.len(),
                picker_menu::MAX_MODEL_ROWS,
            );
        }
    }

    fn apply_catalog(&mut self, catalog: ModelCatalog, cx: &mut Context<Self>) {
        record_setup(&catalog, cx);
        let page = picker_menu::settle(&catalog, &self.page);
        let changed = page != self.page;
        self.catalog = Some(catalog);
        self.error = None;
        if changed || self.catalog.is_some() && self.cursor >= self.entries().len() {
            self.open_page(page);
        }
        cx.notify();
    }

    fn handle_catalog(&mut self, result: Result<Value, String>, cx: &mut Context<Self>) {
        match result.and_then(catalog_from) {
            Ok(catalog) => self.apply_catalog(catalog, cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        dispatch(cx, json!({"action": "models"}), |picker, result, cx| {
            picker.handle_catalog(result, cx)
        });
    }

    /// Picks the model at `index` in the catalog and closes the menu once the service has applied it.
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.models.get(index))
        else {
            return;
        };
        self.error = None;
        let body = json!({"action": "select-model", "account": row.account, "model": row.model});
        dispatch(cx, body, |picker, result, cx| {
            match result.and_then(catalog_from) {
                Ok(catalog) => {
                    picker.apply_catalog(catalog, cx);
                    picker.dismiss(cx);
                }
                Err(error) => {
                    picker.error = Some(error);
                    cx.notify();
                }
            }
        });
        cx.notify();
    }

    fn sign_in(&mut self, account: &Account, cx: &mut Context<Self>) {
        self.error = None;
        if sign_in::is_chatgpt(account) {
            self.begin_chatgpt("device_code", cx);
        } else if sign_in::is_opencode(account) {
            self.flow = Flow::begin_api_key(account);
        }
        cx.notify();
    }

    fn account(&self, id: &str) -> Option<Account> {
        self.catalog
            .as_ref()?
            .accounts
            .iter()
            .find(|account| account.id == id)
            .cloned()
    }

    /// Acts on the row at `index` of the current page.
    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries().get(index).cloned() else {
            return;
        };
        match entry.item {
            Item::Account(id) => {
                let Some(account) = self.account(&id) else {
                    return;
                };
                if account.state == "available" {
                    self.open_page(Page::Models(id));
                } else {
                    self.sign_in(&account, cx);
                }
            }
            Item::Back => {
                let from = match &self.page {
                    Page::Models(id) => Some(id.clone()),
                    Page::Accounts => None,
                };
                self.open_page(Page::Accounts);
                if let Some(position) = from.and_then(|id| {
                    self.entries()
                        .iter()
                        .position(|entry| entry.item == Item::Account(id.clone()))
                }) {
                    self.cursor = position;
                }
            }
            Item::Model(model) => self.choose(model, cx),
            Item::SignOut(id) => {
                self.error = None;
                let body = json!({"action": "sign-out", "account": id});
                dispatch(cx, body, |picker, result, cx| {
                    picker.handle_catalog(result, cx)
                });
            }
            Item::SignInDojo(id) => {
                if let Some(account) = self.account(&id) {
                    self.sign_in(&account, cx);
                }
            }
        }
        cx.notify();
    }

    fn begin_chatgpt(&mut self, method: &'static str, cx: &mut Context<Self>) {
        self.flow = Flow::begin_chatgpt();
        dispatch(
            cx,
            json!({"action": "begin-chatgpt-login", "method": method}),
            |picker, result, cx| {
                match result {
                    Ok(value) => match value.get("loginId").and_then(Value::as_str) {
                        Some(id) => {
                            picker.flow.set_login_id(id.to_owned());
                            picker.start_polling(cx);
                        }
                        None => picker.fail_flow("Dojo voice did not start the sign-in.".into()),
                    },
                    Err(error) => picker.fail_flow(error),
                }
                cx.notify();
            },
        );
    }

    fn fail_flow(&mut self, message: String) {
        if let Flow::ChatGpt { progress, .. } = &mut self.flow {
            *progress = Progress::Failed(message);
        } else {
            self.error = Some(message);
        }
    }

    /// Follows the service's sign-in progress once a second until it finishes or the flow is dropped.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self.poll = Some(cx.spawn(async move |this, cx| {
            let mut wait = Duration::from_millis(300);
            loop {
                cx.background_executor().timer(wait).await;
                wait = Duration::from_secs(1);
                let result = cx
                    .background_executor()
                    .spawn(async {
                        std::thread::spawn(|| socket_call(&json!({"action": "login-status"})))
                            .join()
                            .map_err(|_| "Voice request worker failed".to_string())?
                    })
                    .await;
                let keep_going = this
                    .update(cx, |picker: &mut Picker, cx| {
                        let Ok(value) = result else {
                            return true;
                        };
                        let Ok(status) = serde_json::from_value::<LoginStatus>(value) else {
                            return true;
                        };
                        let effect = picker.flow.apply_status(&status);
                        cx.notify();
                        if effect == Effect::Reload {
                            picker.flow = Flow::None;
                            picker.reload(cx);
                            return false;
                        }
                        !matches!(
                            picker.flow,
                            Flow::ChatGpt { ref progress, .. } if progress.is_finished()
                        ) && picker.flow.is_active()
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    fn switch_to_browser(&mut self, cx: &mut Context<Self>) {
        if let Some(login_id) = self.flow.cancel_target() {
            self.poll = None;
            self.flow = Flow::begin_chatgpt();
            dispatch(
                cx,
                json!({"action": "cancel-login", "loginId": login_id}),
                |picker, _, cx| picker.begin_chatgpt("browser", cx),
            );
        } else {
            self.begin_chatgpt("browser", cx);
        }
        cx.notify();
    }

    fn cancel_flow(&mut self, cx: &mut Context<Self>) {
        self.forget_flow();
        self.reload(cx);
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if let Some((account, key)) = self.flow.submit_key() {
            dispatch(
                cx,
                json!({"action": "submit-opencode-key", "account": account, "key": key}),
                |picker, result, cx| {
                    match result {
                        Ok(_) => {
                            picker.flow = Flow::None;
                            picker.reload(cx);
                        }
                        Err(error) => picker.flow.key_rejected(error),
                    }
                    cx.notify();
                },
            );
        } else if let Some((login_id, value)) = self.flow.submit_paste() {
            dispatch(
                cx,
                json!({"action": "login-input", "loginId": login_id, "value": value}),
                |picker, result, cx| {
                    if let Err(error) = result {
                        picker.error = Some(error);
                    }
                    cx.notify();
                },
            );
        }
        cx.notify();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            if self.flow.is_active() {
                self.cancel_flow(cx);
            } else if !self.query.is_empty() {
                self.query.clear();
                self.filter_changed();
                cx.notify();
            } else if matches!(self.page, Page::Models(_)) {
                self.activate(0, cx);
            } else {
                self.dismiss(cx);
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if self.flow.input_mut().is_some() {
            let modifiers = event.keystroke.modifiers;
            match key {
                "enter" if !event.is_held => self.submit(cx),
                "enter" => {}
                "backspace" => {
                    if let Some(input) = self.flow.input_mut() {
                        input.backspace();
                    }
                }
                "a" if modifiers.control => {
                    if let Some(input) = self.flow.input_mut() {
                        input.select_all();
                    }
                }
                "v" if modifiers.control => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        if let Some(input) = self.flow.input_mut() {
                            input.insert_text(&text);
                        }
                    }
                }
                _ => match &event.keystroke.key_char {
                    Some(chars) if !modifiers.control && !modifiers.alt => {
                        if let Some(input) = self.flow.input_mut() {
                            input.insert_text(chars);
                        }
                    }
                    _ => return,
                },
            }
            cx.notify();
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if self.flow.is_active() {
            return;
        }
        let count = self.entries().len();
        let modifiers = event.keystroke.modifiers;
        let on_models = matches!(self.page, Page::Models(_));
        match key {
            "enter" if !event.is_held => self.activate(self.cursor, cx),
            "enter" => {}
            "down" => self.cursor = picker_menu::step(self.cursor, count, 1),
            "up" => self.cursor = picker_menu::step(self.cursor, count, -1),
            "n" if modifiers.control => self.cursor = picker_menu::step(self.cursor, count, 1),
            "p" if modifiers.control => self.cursor = picker_menu::step(self.cursor, count, -1),
            "pagedown" => {
                self.cursor = picker_menu::clamp_step(
                    self.cursor,
                    count,
                    picker_menu::MAX_MODEL_ROWS as isize,
                )
            }
            "pageup" => {
                self.cursor = picker_menu::clamp_step(
                    self.cursor,
                    count,
                    -(picker_menu::MAX_MODEL_ROWS as isize),
                )
            }
            "home" => self.cursor = 0,
            "end" => self.cursor = count.saturating_sub(1),
            "u" if modifiers.control && on_models => {
                self.query.clear();
                self.filter_changed();
            }
            "backspace" if !self.query.is_empty() => {
                self.query.pop();
                self.filter_changed();
            }
            "left" | "backspace" if on_models && self.query.is_empty() => self.activate(0, cx),
            _ => match &event.keystroke.key_char {
                Some(chars)
                    if on_models
                        && self.searchable()
                        && !modifiers.control
                        && !modifiers.alt
                        && !modifiers.platform
                        && !chars.chars().any(char::is_control) =>
                {
                    if self.query.len() < MAX_QUERY {
                        self.query.push_str(chars);
                    }
                    self.filter_changed();
                }
                _ => return,
            },
        }
        self.keep_cursor_visible();
        cx.notify();
        cx.stop_propagation();
        window.prevent_default();
    }

    fn button(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Picker, &mut Context<Picker>) + 'static,
    ) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        div()
            .id(id.into())
            .flex_shrink_0()
            .px(px(11.))
            .py(px(7.))
            .rounded(px(16.))
            .text_size(px(12.))
            .bg(colors.background.rgb())
            .text_color(colors.foreground.rgb())
            .cursor_pointer()
            .child(label.into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |picker, _, _, cx| {
                    action(picker, cx);
                    cx.stop_propagation();
                }),
            )
            .into_any_element()
    }

    fn card(&self, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.global::<ThemeColors>();
        div()
            .w_full()
            .p(px(12.))
            .rounded(px(12.))
            .bg(colors.surface_raised.rgb())
            .flex()
            .flex_col()
            .gap(px(8.))
    }

    fn field(&self, text: String, placeholder_shown: bool, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.global::<ThemeColors>();
        div()
            .w_full()
            .px(px(10.))
            .py(px(8.))
            .rounded(px(8.))
            .bg(colors.background.rgb())
            .text_size(px(13.))
            .text_color(if placeholder_shown {
                colors.foreground_muted.rgb()
            } else {
                colors.foreground.rgb()
            })
            .child(text)
    }

    fn heading(&self, text: impl Into<String>, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.global::<ThemeColors>();
        div()
            .text_size(px(12.))
            .text_color(colors.foreground.rgb())
            .child(text.into())
    }

    fn notice(&self, text: impl Into<String>, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.global::<ThemeColors>();
        div()
            .text_size(px(11.))
            .text_color(colors.foreground.rgb())
            .child(text.into())
    }

    fn muted(&self, text: impl Into<String>, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.global::<ThemeColors>();
        div()
            .text_size(px(10.))
            .text_color(colors.foreground_muted.rgb())
            .child(text.into())
    }

    fn flow_card(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        match &self.flow {
            Flow::None => None,
            Flow::ApiKey {
                label,
                key,
                submitting,
                error,
                ..
            } => {
                let title = format!("Sign in to {label}");
                let shown = key.display("API key");
                let empty = key.is_empty();
                let submitting = *submitting;
                let error = error.clone();
                let mut card = self
                    .card(cx)
                    .child(self.heading(title, cx))
                    .child(self.muted(
                        "Paste your API key. It is checked with OpenCode, then stored in the system password store.",
                        cx,
                    ))
                    .child(self.field(
                        if submitting {
                            "Checking key…".into()
                        } else {
                            shown
                        },
                        empty || submitting,
                        cx,
                    ));
                if let Some(error) = error {
                    card = card.child(self.notice(error, cx));
                }
                let mut actions = div().flex().justify_end().gap(px(6.)).child(self.button(
                    "key-cancel",
                    "Cancel",
                    cx,
                    |picker, cx| picker.cancel_flow(cx),
                ));
                if !submitting {
                    actions =
                        actions.child(
                            self.button("key-save", "Save key", cx, |picker, cx| picker.submit(cx)),
                        );
                }
                Some(card.child(actions).into_any_element())
            }
            Flow::ChatGpt {
                progress, paste, ..
            } => {
                let progress = progress.clone();
                let paste_shown = paste.display("Paste the address here");
                let paste_empty = paste.is_empty();
                let mut card = self.card(cx).child(self.heading("Sign in to ChatGPT", cx));
                let mut actions = div().flex().justify_end().gap(px(6.));
                match &progress {
                    Progress::Starting => card = card.child(self.muted("Starting sign-in…", cx)),
                    Progress::DeviceCode { code, address } => {
                        let colors = *cx.global::<ThemeColors>();
                        let code_text = code.clone();
                        let address_text = address.clone();
                        card = card
                            .child(self.muted(format!("Open {address} and enter this code:"), cx))
                            .child(
                                div()
                                    .text_size(px(22.))
                                    .line_height(px(30.))
                                    .text_color(colors.foreground.rgb())
                                    .child(code.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(6.))
                                    .child(self.button(
                                        "chatgpt-copy-code",
                                        "Copy code",
                                        cx,
                                        move |_, cx| {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                code_text.clone(),
                                            ))
                                        },
                                    ))
                                    .child(self.button(
                                        "chatgpt-copy-address",
                                        "Copy address",
                                        cx,
                                        move |_, cx| {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                address_text.clone(),
                                            ))
                                        },
                                    )),
                            )
                            .child(self.muted("Waiting for you to finish in the browser…", cx));
                        actions = actions.child(self.button(
                            "chatgpt-browser",
                            "Use browser instead",
                            cx,
                            |picker, cx| picker.switch_to_browser(cx),
                        ));
                    }
                    Progress::BrowserAddress { url } => {
                        let url_text = url.clone();
                        card = card
                            .child(self.muted("Open this address to sign in:", cx))
                            .child(self.field(url.clone(), false, cx).text_size(px(11.)))
                            .child(self.button(
                                "chatgpt-copy-url",
                                "Copy address",
                                cx,
                                move |_, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        url_text.clone(),
                                    ))
                                },
                            ))
                            .child(self.muted("Waiting for you to finish in the browser…", cx));
                    }
                    Progress::NeedsPaste { message } => {
                        card = card
                            .child(self.muted(message.clone(), cx))
                            .child(self.field(paste_shown, paste_empty, cx));
                        actions = actions.child(self.button(
                            "chatgpt-paste-send",
                            "Continue",
                            cx,
                            |picker, cx| picker.submit(cx),
                        ));
                    }
                    Progress::Complete => card = card.child(self.muted("Signed in.", cx)),
                    Progress::Failed(message) => {
                        card = card.child(self.notice(message.clone(), cx));
                        actions = actions.child(self.button(
                            "chatgpt-retry",
                            "Try again",
                            cx,
                            |picker, cx| {
                                picker.begin_chatgpt("device_code", cx);
                                cx.notify();
                            },
                        ));
                    }
                }
                let failed = matches!(progress, Progress::Failed(_));
                actions = actions.child(self.button(
                    "chatgpt-cancel",
                    if failed { "Close" } else { "Cancel" },
                    cx,
                    |picker, cx| picker.cancel_flow(cx),
                ));
                Some(card.child(actions).into_any_element())
            }
        }
    }

    fn menu_row(&self, index: usize, entry: &Entry, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = *cx.global::<ThemeColors>();
        let cursor = self.cursor == index;
        div()
            .id(format!("picker-row-{index}"))
            .aria_label(entry.title.clone())
            .flex_shrink_0()
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(16.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .cursor_pointer()
            .bg(if cursor {
                colors.background_highlight.rgb()
            } else {
                colors.background.rgb()
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .text_size(px(13.))
                    .text_color(if cursor {
                        colors.selection_foreground.rgb()
                    } else {
                        colors.foreground.rgb()
                    })
                    .child(div().w(px(18.)).flex_shrink_0().child(if entry.marked {
                        "✓"
                    } else {
                        ""
                    }))
                    .child(entry.title.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(colors.foreground_muted.rgb())
                    .child(entry.detail.clone()),
            )
            .on_hover(cx.listener(move |picker, hovered, _, cx| {
                if *hovered && picker.cursor != index {
                    picker.cursor = index;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |picker, _, _, cx| {
                    picker.activate(index, cx);
                    cx.stop_propagation();
                }),
            )
            .into_any_element()
    }

    /// Height of the menu body below the header, so the shell extension fits its content.
    fn body_height(&self, entries: &[Entry]) -> f32 {
        let rows = |count: usize| {
            if count == 0 {
                0.
            } else {
                count as f32 * ROW_HEIGHT + (count as f32 - 1.) * ROW_GAP
            }
        };
        let error = if self.error.is_some() { 46. } else { 0. };
        error
            + match &self.flow {
                Flow::None => {
                    let models = model_span(entries);
                    let search = if self.searchable() {
                        SEARCH_HEIGHT + ROW_GAP
                    } else {
                        0.
                    };
                    search
                        + if models.is_empty() {
                            rows(entries.len())
                                + if self.query.is_empty() {
                                    0.
                                } else {
                                    CAPTION_HEIGHT
                                }
                        } else {
                            let shown = models.len().min(picker_menu::MAX_MODEL_ROWS);
                            let caption = if models.len() > shown {
                                CAPTION_HEIGHT
                            } else {
                                0.
                            };
                            rows(entries.len() - models.len() + shown) + caption
                        }
                }
                Flow::ApiKey { error, .. } => 178. + if error.is_some() { 24. } else { 0. },
                Flow::ChatGpt { progress, .. } => match progress {
                    Progress::Starting | Progress::Complete => 102.,
                    Progress::DeviceCode { .. } => 230.,
                    Progress::BrowserAddress { .. } => 250.,
                    Progress::NeedsPaste { .. } => 178.,
                    Progress::Failed(_) => 120.,
                },
            }
    }
}

/// The model rows of a model page: between its Back row and its footer.
fn model_span(entries: &[Entry]) -> Range<usize> {
    let first = entries
        .iter()
        .position(|entry| matches!(entry.item, Item::Model(_)));
    match first {
        Some(first) => {
            let last = entries
                .iter()
                .rposition(|entry| matches!(entry.item, Item::Model(_)))
                .unwrap_or(first);
            first..last + 1
        }
        None => 0..0,
    }
}

impl Render for Picker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (progress, moving) = self
            .reveal
            .sample_with_motion(Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        } else if !self.reveal.is_open() {
            window.remove_window();
        }
        let colors = *cx.global::<ThemeColors>();
        let viewport = window.viewport_size();
        let width = PANEL_WIDTH.min((f32::from(viewport.width) - 16.).max(1.));
        let panel_top = crate::geometry::TOP_BAR_HEIGHT as f32;
        let right = crate::geometry::RIGHT_INSET as f32;
        let catalog = self.catalog.clone();
        let entries = self.entries();
        let full_height = PADDING * 2. + HEADER_HEIGHT + SECTION_GAP + self.body_height(&entries);
        let max_height = full_height.min((f32::from(viewport.height) - 32.).max(1.));
        let panel_bounds = Bounds {
            origin: point(px(f32::from(viewport.width) - width - right), px(panel_top)),
            size: gpui::Size::new(px(width), px(max_height)),
        };
        let setup = catalog.as_ref().is_some_and(needs_setup);
        let summary = match (&catalog, &self.flow, &self.page) {
            (_, flow, _) if flow.is_active() => "Sign in to an account".to_owned(),
            (None, ..) => "Loading…".to_owned(),
            (Some(_), _, _) if setup => "Sign in to use Dojo".to_owned(),
            (Some(_), _, Page::Models(id)) => self
                .account(id)
                .map_or_else(String::new, |account| sign_in::account_status(&account)),
            (Some(catalog), _, _) => catalog.selection.as_ref().map_or_else(
                || "No model selected".to_owned(),
                |selection| {
                    if selection.available {
                        format!("{} · {}", selection.name, selection.account)
                    } else {
                        format!(
                            "{} unavailable: {}",
                            selection.name,
                            selection.reason.as_deref().unwrap_or("account unavailable")
                        )
                    }
                },
            ),
        };
        let mut content = div()
            .w(px(width))
            .h(px(max_height))
            .flex()
            .flex_col()
            .gap(px(SECTION_GAP))
            .p(px(PADDING))
            .opacity(((progress - 0.25) / 0.65).clamp(0., 1.))
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(HEADER_HEIGHT))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(colors.foreground.rgb())
                            .child("Agent"),
                    )
                    .child(self.muted(summary, cx)),
            );
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .flex_shrink_0()
                    .w_full()
                    .p(px(9.))
                    .rounded(px(10.))
                    .bg(colors.surface_raised.rgb())
                    .text_size(px(11.))
                    .text_color(colors.foreground.rgb())
                    .child(error.clone()),
            );
        }
        if let Some(card) = self.flow_card(cx) {
            content = content.child(card);
        } else if catalog.is_some() {
            let models = model_span(&entries);
            let window_rows =
                picker_menu::window(self.top, models.len(), picker_menu::MAX_MODEL_ROWS);
            let mut list = div()
                .id("picker-menu")
                .flex()
                .flex_col()
                .gap(px(ROW_GAP))
                .on_scroll_wheel(
                    cx.listener(|picker, event: &gpui::ScrollWheelEvent, _, cx| {
                        let entries = picker.entries();
                        let models = model_span(&entries);
                        let notch = matches!(event.delta, gpui::ScrollDelta::Lines(_));
                        let up = event.delta.pixel_delta(px(ROW_HEIGHT)).y;
                        let rows = picker
                            .wheel
                            .feed(f32::from(up), notch, ROW_HEIGHT + ROW_GAP);
                        let longest = models.len().saturating_sub(picker_menu::MAX_MODEL_ROWS);
                        let top = (picker.top as isize - rows).clamp(0, longest as isize) as usize;
                        if top != picker.top {
                            picker.top = top;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }),
                );
            if self.searchable() {
                let empty = self.query.is_empty();
                list = list.child(
                    self.field(
                        if empty {
                            "Type to filter models".to_owned()
                        } else {
                            format!("{}▏", self.query)
                        },
                        empty,
                        cx,
                    )
                    .flex_shrink_0()
                    .bg(cx.global::<ThemeColors>().background.rgb())
                    .border_1()
                    .border_color(cx.global::<ThemeColors>().border.rgb())
                    .rounded_full()
                    .px(px(16.))
                    .h(px(SEARCH_HEIGHT))
                    .flex()
                    .items_center(),
                );
            }
            for (index, entry) in entries.iter().enumerate() {
                let hidden =
                    models.contains(&index) && !window_rows.contains(&(index - models.start));
                if models.is_empty()
                    && !self.query.is_empty()
                    && matches!(entry.item, Item::SignOut(_) | Item::SignInDojo(_))
                {
                    list = list.child(self.muted("No models match", cx).h(px(CAPTION_HEIGHT)));
                }
                if !hidden {
                    list = list.child(self.menu_row(index, entry, cx));
                    if index + 1 == models.start + window_rows.end
                        && models.len() > picker_menu::MAX_MODEL_ROWS
                    {
                        list = list.child(
                            self.muted(
                                format!(
                                    "{}–{} of {}{}",
                                    window_rows.start + 1,
                                    window_rows.end,
                                    models.len(),
                                    if self.query.is_empty() {
                                        " · ↑↓ or scroll"
                                    } else {
                                        " matching"
                                    }
                                ),
                                cx,
                            )
                            .h(px(CAPTION_HEIGHT)),
                        );
                    }
                }
            }
            content = content.child(list);
        }
        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|picker, event: &KeyDownEvent, window, cx| {
                picker.handle_key(event, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |picker, event: &gpui::MouseDownEvent, _, cx| {
                    if !panel_bounds.contains(&event.position) {
                        picker.dismiss(cx);
                    }
                }),
            )
            .child(crate::design::shell_extension(
                Bounds::new(
                    panel_bounds.origin,
                    gpui::size(px(width), px(max_height * progress)),
                ),
                colors.background.rgb(),
                colors.border.rgb(),
                true,
            ))
            .child(
                div()
                    .id("model-picker-panel")
                    .absolute()
                    .top(px(panel_top))
                    .right(px(right))
                    .w(px(width))
                    .h(px(max_height * progress))
                    .overflow_hidden()
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socket_in_temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("dojo-picker-{}-{name}", std::process::id()))
    }

    #[test]
    fn a_missing_runtime_is_named_as_the_agent_runtime_not_as_voice() {
        let path = socket_in_temp("missing.sock");
        let error = socket_call_at(&path, &json!({"action": "models"})).unwrap_err();
        assert!(error.contains("Dojo agent runtime"), "{error}");
        assert!(!error.contains("voice service"), "{error}");
    }

    #[test]
    fn the_runtime_socket_carries_one_json_line_each_way() {
        let path = socket_in_temp("runtime.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            for reply in [r#"{"models":[]}"#, r#"{"error":"Nope."}"#] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                assert_eq!(line.trim(), r#"{"action":"models"}"#);
                writeln!(stream, "{reply}").unwrap();
            }
        });
        let body = json!({"action": "models"});
        assert_eq!(socket_call_at(&path, &body).unwrap(), json!({"models": []}));
        assert_eq!(socket_call_at(&path, &body).unwrap_err(), "Nope.");
        server.join().unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn parses_catalog_without_credentials_and_preserves_unavailable_selection() {
        let catalog = parse_catalog(r#"{"accounts":[{"id":"openai-codex","label":"ChatGPT","state":"unavailable","reason":"Needs renewal."}],"models":[],"selection":{"account":"openai-codex","model":"gpt-6-luna","name":"GPT-6 Luna","available":false,"reason":"Needs renewal."}}"#).unwrap();
        assert_eq!(catalog.selection.as_ref().unwrap().model, "gpt-6-luna");
        assert!(!catalog.selection.as_ref().unwrap().available);
        assert!(catalog.models.is_empty());
    }

    #[test]
    fn malformed_catalogs_and_empty_model_identifiers_are_rejected() {
        assert!(parse_catalog("not json").is_err());
        let input = r#"{"accounts":[],"models":[{"account":"","accountLabel":"x","model":"m","name":"m","api":"a","context":1,"maxTokens":1,"reasoning":false,"cost":{"input":0.0,"output":0.0}}],"selection":null}"#;
        assert!(
            parse_catalog(input)
                .unwrap_err()
                .contains("IDs must be nonempty")
        );
    }

    #[test]
    fn model_span_covers_only_the_model_rows() {
        let entry = |item| Entry {
            item,
            title: String::new(),
            detail: String::new(),
            marked: false,
        };
        let rows = [
            entry(Item::Back),
            entry(Item::Model(3)),
            entry(Item::Model(4)),
            entry(Item::SignOut("a".into())),
        ];
        assert_eq!(model_span(&rows), 1..3);
        assert_eq!(model_span(&rows[..1]), 0..0);
    }
}
