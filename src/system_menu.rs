use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Bounds, Context, DisplayId, FocusHandle, IntoElement, KeyDownEvent,
    MouseButton, Render, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, layer_shell, point, prelude::*, px,
};

use crate::config::ThemeColors;
use dojo::session::Occasion;
use dojo::session::action::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PowerAction {
    Lock,
    Suspend,
    Hibernate,
    Logout,
    Reboot,
    PowerOff,
}
impl PowerAction {
    const ALL: [Self; 6] = [
        Self::Lock,
        Self::Suspend,
        Self::Hibernate,
        Self::Logout,
        Self::Reboot,
        Self::PowerOff,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Lock => "Lock",
            Self::Suspend => "Suspend",
            Self::Hibernate => "Hibernate",
            Self::Logout => "Log out",
            Self::Reboot => "Restart",
            Self::PowerOff => "Shut down",
        }
    }
    fn confirmation(self) -> Option<&'static str> {
        match self {
            Self::Lock | Self::Suspend => None,
            Self::Hibernate => Some("Hibernate this computer?"),
            Self::Logout => Some("Log out of this session?"),
            Self::Reboot => Some("Restart this computer?"),
            Self::PowerOff => Some("Shut down this computer?"),
        }
    }
    fn method(self) -> Option<&'static str> {
        match self {
            Self::Suspend => Some("Suspend"),
            Self::Hibernate => Some("Hibernate"),
            Self::Lock | Self::Logout => None,
            Self::Reboot => Some("Reboot"),
            Self::PowerOff => Some("PowerOff"),
        }
    }
    fn capability(self) -> Option<&'static str> {
        match self {
            Self::Suspend => Some("CanSuspend"),
            Self::Hibernate => Some("CanHibernate"),
            Self::Lock | Self::Logout => None,
            Self::Reboot => Some("CanReboot"),
            Self::PowerOff => Some("CanPowerOff"),
        }
    }
    /// The occasion the shell closes for when this action is chosen.
    fn occasion(self) -> Occasion {
        match self {
            Self::Lock => Occasion::Lock,
            Self::Suspend => Occasion::Suspend,
            Self::Hibernate => Occasion::Hibernate,
            Self::Logout => Occasion::Logout,
            Self::Reboot => Occasion::Restart,
            Self::PowerOff => Occasion::Shutdown,
        }
    }
    /// The power action behind a session action. The locker is not one.
    #[cfg_attr(feature = "record-session-actions", allow(dead_code))]
    fn from_action(action: Action) -> Option<Self> {
        match action {
            Action::StartLocker => None,
            Action::Suspend => Some(Self::Suspend),
            Action::Hibernate => Some(Self::Hibernate),
            Action::Logout => Some(Self::Logout),
            Action::Restart => Some(Self::Reboot),
            Action::Shutdown => Some(Self::PowerOff),
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Choosing,
    Confirming(PowerAction),
    Running(PowerAction),
}

#[derive(Default)]
pub struct SystemMenu {
    panel: Option<(DisplayId, WindowHandle<Menu>)>,
}
impl gpui::Global for SystemMenu {}

pub fn close(cx: &mut App) {
    if !cx.has_global::<SystemMenu>() {
        return;
    }
    let panel = cx.update_global::<SystemMenu, _>(|state, _| state.panel.take());
    if let Some((_, panel)) = panel {
        let _ = panel.update(cx, |_, window, _| window.remove_window());
    }
}

pub fn toggle(display_id: DisplayId, cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<SystemMenu>() {
        cx.set_global(SystemMenu::default());
    }
    let previous = cx.update_global::<SystemMenu, _>(|state, _| state.panel.take());
    if let Some((previous_display, panel)) = previous {
        if panel
            .update(cx, |_, window, _| window.remove_window())
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
        .ok_or_else(|| anyhow::anyhow!("System menu output disappeared"))?;
    let panel = cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-system-menu".into(),
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
                let task = cx.spawn(async move |this, cx| {
                    let capabilities = cx
                        .background_executor()
                        .spawn(async { read_capabilities() })
                        .await;
                    let _ = this.update(cx, |menu: &mut Menu, cx| {
                        match capabilities {
                            Ok(capabilities) => menu.capabilities = Some(capabilities),
                            Err(error) => menu.error = Some(error.to_string()),
                        }
                        cx.notify();
                    });
                });
                Menu {
                    focus,
                    capabilities: None,
                    phase: Phase::Choosing,
                    selected: 0,
                    reveal: dojo::design::MenuReveal::opening(Instant::now()),
                    error: None,
                    _probe: task,
                    action: None,
                }
            })
        },
    )?;
    cx.update_global::<SystemMenu, _>(|state, _| state.panel = Some((display_id, panel)));
    Ok(())
}

fn read_capabilities() -> anyhow::Result<[bool; PowerAction::ALL.len()]> {
    let connection = zbus::blocking::connection::Builder::system()?
        .method_timeout(Duration::from_secs(5))
        .build()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    let mut capabilities = [false; PowerAction::ALL.len()];
    for (index, action) in PowerAction::ALL.iter().enumerate() {
        capabilities[index] = match action.capability() {
            _ if *action == PowerAction::Lock => true,
            Some(method) => {
                let value: String = proxy.call(method, &())?;
                matches!(value.as_str(), "yes" | "challenge")
            }
            None => std::env::var_os("NIRI_SOCKET").is_some(),
        };
    }
    Ok(capabilities)
}

/// Performs a session action the way the menu always has: logind for machine
/// power, Niri IPC for logout. Blocking; the caller keeps it off the UI thread.
#[cfg_attr(feature = "record-session-actions", allow(dead_code))]
pub fn perform(action: Action) -> anyhow::Result<()> {
    let power = PowerAction::from_action(action)
        .ok_or_else(|| anyhow::anyhow!("{} is not a power action", action.key()))?;
    execute(power)
}

fn execute(action: PowerAction) -> anyhow::Result<()> {
    anyhow::ensure!(
        action != PowerAction::Lock,
        "locking goes through the locker, not logind"
    );
    if let Some(method) = action.method() {
        let connection = zbus::blocking::connection::Builder::system()?
            .method_timeout(Duration::from_secs(15))
            .build()?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        proxy.call::<_, _, ()>(method, &(true,))?;
    } else {
        let mut socket = niri_ipc::socket::Socket::connect()?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        let response = socket
            .send(niri_ipc::Request::Action(niri_ipc::Action::Quit {
                skip_confirmation: true,
            }))?
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            matches!(response, niri_ipc::Response::Handled),
            "Niri did not accept logout"
        );
    }
    Ok(())
}

struct Menu {
    focus: FocusHandle,
    capabilities: Option<[bool; PowerAction::ALL.len()]>,
    phase: Phase,
    selected: usize,
    reveal: dojo::design::MenuReveal,
    error: Option<String>,
    _probe: gpui::Task<()>,
    action: Option<gpui::Task<()>>,
}
impl Menu {
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.reveal.set_open(false, Instant::now());
        cx.notify();
    }
    fn select(&mut self, action: PowerAction, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Running(_)) {
            return;
        }
        let available = PowerAction::ALL
            .iter()
            .position(|candidate| *candidate == action)
            .is_some_and(|index| {
                self.capabilities
                    .is_some_and(|capabilities| capabilities[index])
            });
        if !available {
            return;
        }
        self.error = None;
        if action.confirmation().is_some() {
            self.phase = Phase::Confirming(action);
            cx.notify();
        } else {
            self.run(action, cx);
        }
    }
    /// Hands the action to the shell's closing, which draws the transition and
    /// then performs it. The menu closes once the closing has begun.
    fn run(&mut self, action: PowerAction, cx: &mut Context<Self>) {
        self.phase = Phase::Running(action);
        cx.notify();
        self.action = Some(cx.spawn(async move |this, cx| {
            let _ = this.update_in(cx, |menu, window, cx| {
                match crate::session_overlay::leave(action.occasion(), cx) {
                    Ok(()) => window.remove_window(),
                    Err(error) => {
                        menu.phase = Phase::Choosing;
                        menu.error = Some(error.to_string());
                        cx.notify();
                    }
                }
            });
        }));
    }
}

impl Render for Menu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let viewport = window.viewport_size();
        let (progress, moving) = self
            .reveal
            .sample_with_motion(Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        }
        if !self.reveal.is_open() && progress < 0.001 {
            window.remove_window();
        }
        let width = 264_f32.min((f32::from(viewport.width) - 24.).max(100.));
        let action_count = PowerAction::ALL.len() as f32;
        let padding = 18.;
        let gap = 8.;
        let row_padding = 8.;
        let action_line_height = 20.;
        let title_line_height = 24.;
        let content_height = padding * 2.
            + title_line_height
            + (gap + row_padding * 2. + action_line_height) * action_count;
        let height = content_height.min((f32::from(viewport.height) - 80.).max(80.));
        let right = crate::geometry::RIGHT_INSET as f32;
        let bottom = crate::geometry::BOTTOM_INSET as f32;
        let bounds = Bounds::new(
            point(
                viewport.width - px(right + width),
                viewport.height - px(bottom + height * progress),
            ),
            gpui::size(px(width), px(height * progress)),
        );
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(gap))
            .p(px(padding))
            .text_size(px(13.))
            .line_height(px(action_line_height))
            .child(
                div()
                    .text_size(px(16.))
                    .line_height(px(title_line_height))
                    .child("Power"),
            );
        match self.phase {
            Phase::Choosing => {
                for (index, action) in PowerAction::ALL.iter().copied().enumerate() {
                    let available = self
                        .capabilities
                        .is_some_and(|capabilities| capabilities[index]);
                    content = content.child(
                        div()
                            .id(format!("power-action-{index}"))
                            .aria_label(action.label())
                            .px(px(12.))
                            .py(px(row_padding))
                            .rounded(px(10.))
                            .bg(if self.selected == index {
                                colors.background_highlight.rgb()
                            } else {
                                colors.background.rgb()
                            })
                            .text_color(if available {
                                if self.selected == index {
                                    colors.selection_foreground.rgb()
                                } else {
                                    colors.foreground.rgb()
                                }
                            } else {
                                colors.foreground_muted.rgb()
                            })
                            .child(action.label())
                            .on_hover(cx.listener(move |menu, hovered, _, cx| {
                                if *hovered && menu.selected != index {
                                    menu.selected = index;
                                    cx.notify();
                                }
                            }))
                            .when(available, |row| {
                                row.cursor_pointer().on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |menu, _, _, cx| {
                                        menu.select(action, cx);
                                        cx.stop_propagation();
                                    }),
                                )
                            }),
                    );
                }
                if self.capabilities.is_none() && self.error.is_none() {
                    content =
                        content.child(div().text_size(px(11.)).child("Checking power controls…"));
                }
            }
            Phase::Confirming(action) => {
                content = content
                    .child(action.confirmation().unwrap_or(action.label()))
                    .child(
                        div()
                            .id("power-confirm")
                            .px(px(12.))
                            .py(px(10.))
                            .rounded(px(10.))
                            .bg(colors.background_highlight.rgb())
                            .text_color(colors.selection_foreground.rgb())
                            .cursor_pointer()
                            .child(action.label())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |menu, _, _, cx| {
                                    menu.run(action, cx);
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id("power-cancel")
                            .px(px(12.))
                            .py(px(10.))
                            .cursor_pointer()
                            .child("Cancel")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|menu, _, _, cx| {
                                    menu.phase = Phase::Choosing;
                                    cx.notify();
                                    cx.stop_propagation();
                                }),
                            ),
                    );
            }
            Phase::Running(action) => content = content.child(format!("{}…", action.label())),
        }
        if let Some(error) = &self.error {
            content = content.child(div().text_size(px(11.)).child(error.clone()));
        }
        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .text_color(colors.foreground.rgb())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|menu, event: &KeyDownEvent, _window, cx| {
                if event.keystroke.key == "escape" {
                    if matches!(menu.phase, Phase::Confirming(_)) {
                        menu.phase = Phase::Choosing;
                        cx.notify();
                    } else {
                        menu.dismiss(cx);
                    }
                    cx.stop_propagation();
                } else if !event.is_held && event.keystroke.key == "enter" {
                    match menu.phase {
                        Phase::Choosing => menu.select(PowerAction::ALL[menu.selected], cx),
                        Phase::Confirming(action) => menu.run(action, cx),
                        Phase::Running(_) => (),
                    }
                    cx.stop_propagation();
                } else if matches!(menu.phase, Phase::Choosing) {
                    match event.keystroke.key.as_str() {
                        "down" => menu.selected = (menu.selected + 1) % PowerAction::ALL.len(),
                        "up" => {
                            menu.selected = (menu.selected + PowerAction::ALL.len() - 1)
                                % PowerAction::ALL.len()
                        }
                        _ => return,
                    }
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |menu, event: &gpui::MouseDownEvent, _window, cx| {
                    if !bounds.contains(&event.position) {
                        menu.dismiss(cx);
                    }
                }),
            )
            .child(dojo::design::bottom_right_shell_extension(
                bounds,
                colors.background.rgb(),
                colors.border.rgb(),
            ))
            .child(
                div()
                    .id("system-menu-panel")
                    .absolute()
                    .right(px(right))
                    .bottom(px(bottom))
                    .w(px(width))
                    .h(px(height * progress))
                    .opacity(progress)
                    .overflow_y_scroll()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_menu_action_leaves_through_its_own_occasion_and_back() {
        for action in PowerAction::ALL {
            let session = action.occasion().action().unwrap();
            let back = PowerAction::from_action(session);
            if action == PowerAction::Lock {
                assert_eq!(session, Action::StartLocker);
                assert_eq!(back, None);
            } else {
                assert_eq!(back, Some(action), "{action:?}");
            }
        }
        let occasions: Vec<_> = PowerAction::ALL.map(PowerAction::occasion).to_vec();
        for (index, occasion) in occasions.iter().enumerate() {
            assert!(!occasions[..index].contains(occasion), "{occasion:?}");
        }
    }
    #[test]
    fn locking_is_never_executed_as_logout() {
        assert!(execute(PowerAction::Lock).is_err());
        assert!(perform(Action::StartLocker).is_err());
        assert_eq!(PowerAction::Lock.method(), None);
        assert!(PowerAction::Lock.confirmation().is_none());
    }
    #[test]
    fn power_actions_keep_log_out_separate_from_machine_power() {
        assert_eq!(PowerAction::Suspend.method(), Some("Suspend"));
        assert_eq!(PowerAction::Reboot.method(), Some("Reboot"));
        assert_eq!(PowerAction::PowerOff.method(), Some("PowerOff"));
        assert_eq!(PowerAction::Logout.method(), None);
        assert!(PowerAction::Logout.confirmation().is_some());
        assert!(PowerAction::Reboot.confirmation().is_some());
        assert!(PowerAction::PowerOff.confirmation().is_some());
        assert!(PowerAction::Suspend.confirmation().is_none());
    }
    #[test]
    #[ignore = "requires the live system logind bus; read-only capability probe"]
    fn live_power_capabilities_are_available() {
        assert!(
            read_capabilities()
                .unwrap()
                .iter()
                .any(|available| *available)
        );
    }
    #[gpui::test]
    fn close_without_a_menu_is_safe(cx: &mut gpui::TestAppContext) {
        cx.update(close);
    }
}
