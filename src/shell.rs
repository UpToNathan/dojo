use std::cell::RefCell;
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui::{
    App, AppContext, DisplayChanges, DisplayEvent, DisplayId, Pixels, Point, QuitMode,
    Subscription, px,
};
use gpui_platform::application;

use crate::config::{ConfigStore, ThemeColors};
use gpui::BorrowAppContext;

use crate::control::{ControlCommand, ControlServer};
use crate::frame::DisplayFrame;
use crate::launcher::Launcher;
use crate::niri_layout::NiriLayout;
use crate::state::{Preferences, StateStore};
use crate::tray::{TrayHandle, TrayService};

#[derive(serde::Deserialize)]
struct FocusedOutput {
    logical: OutputLogicalBounds,
}

#[derive(serde::Deserialize)]
struct OutputLogicalBounds {
    x: f32,
    y: f32,
    scale: f32,
}

fn focused_display_id(cx: &App) -> Option<DisplayId> {
    let output = Command::new("niri")
        .args(["msg", "--json", "focused-output"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let focused: FocusedOutput = serde_json::from_slice(&output.stdout).ok()?;
    let displays = cx.displays();
    let wayland_scale = focused.logical.scale.ceil();
    let origin = Point::new(
        px(focused.logical.x / wayland_scale),
        px(focused.logical.y / wayland_scale),
    );
    display_at_origin(
        origin,
        displays
            .into_iter()
            .map(|display| (display.id(), display.bounds().origin)),
    )
}

fn display_at_origin(
    origin: Point<Pixels>,
    displays: impl IntoIterator<Item = (DisplayId, Point<Pixels>)>,
) -> Option<DisplayId> {
    displays.into_iter().find_map(|(id, display_origin)| {
        ((f32::from(display_origin.x) - f32::from(origin.x)).abs() < 2.
            && (f32::from(display_origin.y) - f32::from(origin.y)).abs() < 2.)
            .then_some(id)
    })
}

struct ShellController {
    enabled: bool,
    state: StateStore,
    niri_layout: Rc<RefCell<NiriLayout>>,
    displays: HashMap<DisplayId, DisplayFrame>,
    display_subscription: Option<Subscription>,
    tray_service: Option<TrayService>,
    display_generation: u64,
    launcher: Launcher,
    theme_picker: crate::theme_picker::ThemePicker,
    quick_terminal: crate::quick_terminal::QuickTerminal,
}

impl ShellController {
    fn new(state: StateStore, niri_layout: Rc<RefCell<NiriLayout>>, launcher: Launcher) -> Self {
        Self {
            enabled: false,
            state,
            niri_layout,
            displays: HashMap::new(),
            display_subscription: None,
            tray_service: None,
            display_generation: 0,
            launcher,
            theme_picker: crate::theme_picker::ThemePicker::default(),
            quick_terminal: crate::quick_terminal::QuickTerminal::default(),
        }
    }

    fn initialize(&mut self, enabled: bool, cx: &mut App) -> anyhow::Result<()> {
        if enabled { self.enable(cx) } else { Ok(()) }
    }

    fn set_enabled(&mut self, enabled: bool, cx: &mut App) -> anyhow::Result<()> {
        if self.enabled == enabled {
            return Ok(());
        }
        if enabled {
            self.enable(cx)
        } else {
            self.disable(cx, true)
        }
    }

    fn enable(&mut self, cx: &mut App) -> anyhow::Result<()> {
        self.niri_layout.borrow_mut().capture()?;
        crate::startup::mark("niri-layout-captured");

        let mut tray_service = match TrayService::start() {
            Ok(service) => Some(service),
            Err(error) => {
                eprintln!("Dojo could not start its StatusNotifier host: {error:#}");
                None
            }
        };
        let tray = tray_service
            .as_ref()
            .map(TrayService::handle)
            .unwrap_or_default();
        crate::startup::mark("tray-started");
        let mut opened = HashMap::new();
        if let Err(error) = self.open_active_displays(cx, &mut opened, tray) {
            close_displays(opened, cx);
            if let Some(service) = tray_service.take() {
                service.shutdown();
            }
            if let Err(rollback_error) = self.niri_layout.borrow_mut().disable() {
                eprintln!(
                    "Failed to clear Niri width snapshot after surface-open failure: {rollback_error:#}"
                );
            }
            return Err(error).context("opening the frame on active displays");
        }

        crate::startup::mark("displays-opened");
        if let Err(error) = self.niri_layout.borrow_mut().enable() {
            close_displays(opened, cx);
            if let Some(service) = tray_service.take() {
                service.shutdown();
            }
            return Err(error).context("reserving the Niri tiled area");
        }

        crate::startup::mark("niri-layout-reserved");
        if let Err(error) = self.state.save(Preferences {
            frame_enabled: true,
        }) {
            if let Err(rollback_error) = self.niri_layout.borrow_mut().disable() {
                eprintln!(
                    "Failed to restore Niri widths after state-save failure: {rollback_error:#}"
                );
            }
            close_displays(opened, cx);
            if let Some(service) = tray_service.take() {
                service.shutdown();
            }
            return Err(error).context("persisting the enabled frame state");
        }

        self.displays = opened;
        self.tray_service = tray_service;
        self.enabled = true;
        Ok(())
    }

    fn disable(&mut self, cx: &mut App, persist: bool) -> anyhow::Result<()> {
        crate::bluetooth_ui::close(cx);
        crate::displays_ui::close(cx);
        crate::wifi_ui::close(cx);
        crate::system_menu::close(cx);
        crate::model_picker::close(cx);
        self.quick_terminal.close(cx);
        if !self.enabled {
            if persist {
                self.state.save(Preferences {
                    frame_enabled: false,
                })?;
            }
            return Ok(());
        }

        self.niri_layout
            .borrow_mut()
            .disable()
            .context("restoring Niri tiled columns")?;

        close_displays(std::mem::take(&mut self.displays), cx);
        if let Some(service) = self.tray_service.take() {
            service.shutdown();
        }
        self.enabled = false;

        if persist {
            self.state.save(Preferences {
                frame_enabled: false,
            })?;
        }

        Ok(())
    }

    /// Everything a closing must cover: the launcher, the pickers and the panels.
    fn close_temporary_surfaces(&mut self, cx: &mut App) {
        if let Err(error) = self
            .launcher
            .navigate(crate::launcher::Navigation::Dismiss, cx)
        {
            eprintln!("Dojo could not close the launcher: {error:#}");
        }
        crate::bluetooth_ui::close(cx);
        crate::displays_ui::close(cx);
        crate::wifi_ui::close(cx);
        crate::system_menu::close(cx);
        crate::model_picker::close(cx);
    }

    fn quit(&mut self, cx: &mut App) -> anyhow::Result<()> {
        self.disable(cx, false)
    }

    fn open_active_displays(
        &self,
        cx: &mut App,
        opened: &mut HashMap<DisplayId, DisplayFrame>,
        tray: TrayHandle,
    ) -> anyhow::Result<()> {
        for display in cx.displays() {
            let display_id = display.id();
            self.open_display_into(cx, display_id, display.bounds(), opened, tray.clone())?;
        }
        Ok(())
    }

    fn open_display_into(
        &self,
        cx: &mut App,
        display_id: DisplayId,
        bounds: gpui::Bounds<gpui::Pixels>,
        opened: &mut HashMap<DisplayId, DisplayFrame>,
        tray: TrayHandle,
    ) -> anyhow::Result<()> {
        let frame = DisplayFrame::open(cx, display_id, bounds, tray)
            .with_context(|| format!("opening frame for display {display_id:?}"))?;
        opened.insert(display_id, frame);
        Ok(())
    }

    fn queue_display_reconcile(&mut self, event: DisplayEvent) -> Option<u64> {
        let should_reconcile = match event {
            DisplayEvent::Removed(_) | DisplayEvent::Added(_) if self.enabled => true,
            DisplayEvent::Changed { changes, .. }
                if self.enabled && changes.contains(DisplayChanges::BOUNDS) =>
            {
                true
            }
            _ => false,
        };

        if should_reconcile {
            self.display_generation = self.display_generation.wrapping_add(1);
            Some(self.display_generation)
        } else {
            None
        }
    }

    fn reconcile_if_current(&mut self, generation: u64, cx: &mut App) -> anyhow::Result<()> {
        if !self.enabled || generation != self.display_generation {
            return Ok(());
        }

        self.reconcile_active_displays(cx)
    }

    fn reconcile_active_displays(&mut self, cx: &mut App) -> anyhow::Result<()> {
        let tray = self
            .tray_service
            .as_ref()
            .map(TrayService::handle)
            .unwrap_or_default();

        let mut previous = std::mem::take(&mut self.displays);
        let mut active = HashMap::new();
        for display in cx.displays() {
            let display_id = display.id();
            let bounds = display.bounds();
            if let Some(frame) = previous.remove(&display_id) {
                if frame.can_reuse(cx, bounds) {
                    active.insert(display_id, frame);
                    continue;
                }
                if let Err(error) = frame.close(cx) {
                    eprintln!(
                        "Failed to replace stale Dojo surfaces for {display_id:?}: {error:#}"
                    );
                }
            }

            if let Err(error) =
                self.open_display_into(cx, display_id, bounds, &mut active, tray.clone())
            {
                close_displays(active, cx);
                close_displays(previous, cx);
                self.disable_after_failure(cx, &error);
                return Err(error).context("rebuilding frames after a display change");
            }
        }

        close_displays(previous, cx);
        self.displays = active;
        Ok(())
    }

    fn disable_after_failure(&mut self, cx: &mut App, error: &anyhow::Error) {
        eprintln!("Dojo display reconciliation failed: {error:#}; disabling the frame");
        if let Err(rollback_error) = self.niri_layout.borrow_mut().disable() {
            eprintln!("Failed to restore Niri widths after display failure: {rollback_error:#}");
        }
        close_displays(std::mem::take(&mut self.displays), cx);
        if let Some(service) = self.tray_service.take() {
            service.shutdown();
        }
        self.enabled = false;
        if let Err(state_error) = self.state.save(Preferences {
            frame_enabled: false,
        }) {
            eprintln!("Failed to persist the disabled frame state: {state_error:#}");
        }
    }
}

fn close_displays(displays: HashMap<DisplayId, DisplayFrame>, cx: &mut App) {
    for (display_id, frame) in displays {
        if let Err(error) = frame.close(cx) {
            eprintln!("Failed to close Dojo surfaces for {display_id:?}: {error:#}");
        }
    }
}

pub fn run(force_enabled: Option<bool>) -> anyhow::Result<()> {
    run_with_launcher(force_enabled, false)
}

pub fn run_with_launcher(force_enabled: Option<bool>, show_launcher: bool) -> anyhow::Result<()> {
    run_with_surfaces(force_enabled, show_launcher, false, false)
}

pub fn run_with_theme_picker(force_enabled: Option<bool>) -> anyhow::Result<()> {
    run_with_surfaces(force_enabled, false, true, false)
}

pub fn run_with_models(force_enabled: Option<bool>) -> anyhow::Result<()> {
    run_with_surfaces(force_enabled, false, false, true)
}

fn run_with_surfaces(
    force_enabled: Option<bool>,
    show_launcher: bool,
    show_theme: bool,
    show_models: bool,
) -> anyhow::Result<()> {
    // Connects to the compositor while GPUI initializes; the frame's edges are made on it.
    crate::edge_reservation::prewarm();
    let launcher = Launcher::new()?;
    crate::startup::mark("launcher-built");
    let mut config_store = ConfigStore::user_default()?;
    let initial_colors = config_store.load()?.context("loading initial Dojo theme")?;
    let theme_paths = dojo::theme::Paths::user_default()?;
    let mut watch_paths = config_store.watch_paths();
    watch_paths.extend([
        theme_paths.root.join("current"),
        theme_paths.root.join("selection.json"),
        theme_paths.root.join("current/selection.json"),
        theme_paths.root.join("current/wallpaper.txt"),
        theme_paths
            .root
            .join(dojo::theme::wallpaper_layout::FILE_NAME),
    ]);
    let mut file_watcher = match crate::file_events::Watcher::start(watch_paths) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            eprintln!("Dojo file watches could not start; reconciling every 30 seconds: {error:#}");
            None
        }
    };
    let subscribe = || {
        if let Some(watcher) = &file_watcher {
            watcher.subscribe()
        } else {
            let (sender, receiver) = async_channel::bounded(1);
            invalidate_theme(&sender);
            (sender, receiver)
        }
    };
    let (theme_trigger, theme_events) = subscribe();
    let (wallpaper_trigger, wallpaper_events) = subscribe();
    let watch_fallback = file_watcher.is_none();
    let appearance_task = Rc::new(RefCell::new(None::<gpui::Task<()>>));
    let appearance_task_for_run = Rc::clone(&appearance_task);
    let state = StateStore::user_default()?;
    let initial_enabled = match force_enabled {
        Some(enabled) => enabled,
        None => state
            .load()?
            .map_or(true, |preferences| preferences.frame_enabled),
    };
    let niri_layout = Rc::new(RefCell::new(NiriLayout::default()));
    let niri_layout_for_exit = Rc::clone(&niri_layout);
    let (command_sender, command_receiver) = async_channel::bounded(32);
    let mut control_server = ControlServer::start(command_sender)?;
    crate::startup::mark("control-listening");
    let startup_error = Rc::new(RefCell::new(None::<String>));
    let startup_error_for_run = Rc::clone(&startup_error);

    application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            crate::startup::mark("gpui-running");
            cx.set_global(initial_colors);
            crate::session_overlay::begin_login(cx);
            crate::startup::mark("login-overlay");
            cx.set_global(crate::speech::SpeechUi::default());
            crate::frame::initialize_host_state(cx);
            crate::audio::initialize(cx);
            crate::clock::initialize(cx);
            if let Err(error) = crate::wifi_ui::initialize(cx) {
                eprintln!("Dojo could not start its Wi-Fi controls: {error:#}");
            }
            crate::bluetooth_ui::initialize(cx);
            crate::model_picker::watch_setup(cx);
            crate::startup::mark("services-initialized");
            if let Err(error) = crate::wallpaper::start(cx, wallpaper_events) {
                eprintln!("Dojo could not start its wallpaper surfaces: {error:#}");
            }
            if watch_fallback {
                let theme_trigger = theme_trigger.clone();
                cx.spawn(async move |cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_secs(30))
                            .await;
                        invalidate_theme(&theme_trigger);
                        invalidate_theme(&wallpaper_trigger);
                    }
                })
                .detach();
            }
            *appearance_task_for_run.borrow_mut() = Some(cx.spawn(async move |cx| {
                loop {
                    if let Err(error) = follow_appearance(&theme_trigger).await {
                        eprintln!(
                            "Dojo appearance events disconnected; retrying in 30 seconds: {error:#}"
                        );
                    }
                    cx.background_executor()
                        .timer(Duration::from_secs(30))
                        .await;
                }
            }));
            cx.spawn(async move |cx| {
                let mut theme_store = dojo::theme::ThemeStore::new(theme_paths);
                while theme_events.recv().await.is_ok() {
                    while theme_events.try_recv().is_ok() {}
                    let (store, returned_theme_store, result) = cx
                        .background_executor()
                        .spawn(async move {
                            if let Err(error) = dojo::theme::refresh_system(&theme_store) {
                                eprintln!("Dojo could not propagate appearance change: {error:#}");
                            }
                            let result = config_store.load();
                            (config_store, theme_store, result)
                        })
                        .await;
                    config_store = store;
                    theme_store = returned_theme_store;
                    match result {
                        Ok(Some(colors)) => cx.update(|cx| {
                            if *cx.global::<ThemeColors>() != colors {
                                cx.set_global(colors);
                                cx.refresh_windows();
                            }
                        }),
                        Ok(None) => {}
                        Err(error) => eprintln!("Dojo kept its previous theme: {error:#}"),
                    }
                }
            })
            .detach();
            if let Err(error) = crate::power::register_font(cx) {
                let error = format!("Failed to load Dojo's bundled battery icon font: {error:#}");
                eprintln!("{error}");
                *startup_error_for_run.borrow_mut() = Some(error);
                cx.quit();
                return;
            }

            let controller =
                cx.new(|_| ShellController::new(state, Rc::clone(&niri_layout), launcher));
            if let Err(error) = controller.update(cx, |controller, cx| {
                controller.initialize(initial_enabled, cx)
            }) {
                let error = format!("Failed to initialize Dojo: {error:#}");
                eprintln!("{error}");
                *startup_error_for_run.borrow_mut() = Some(error);
                cx.quit();
                return;
            }

            crate::startup::mark("shell-enabled");
            if show_launcher {
                if let Err(error) =
                    controller.update(cx, |controller, cx| controller.launcher.toggle(None, cx))
                {
                    eprintln!("Dojo could not open launcher: {error:#}");
                }
            }
            if show_theme {
                if let Err(error) = controller.update(cx, |controller, cx| {
                    controller.theme_picker.toggle(None, cx)
                }) {
                    eprintln!("Dojo could not open theme picker: {error:#}");
                }
            }
            if show_models {
                if let Some(display) = cx.primary_display() {
                    if let Err(error) = crate::model_picker::toggle(display.id(), cx) {
                        eprintln!("Dojo could not open model picker: {error:#}");
                    }
                }
            }

            let controller_for_theme = controller.downgrade();
            cx.on_action(move |action: &crate::theme_picker::ToggleThemePicker, cx| {
                if let Err(error) = controller_for_theme
                    .update(cx, |controller, cx| {
                        controller.theme_picker.toggle(action.display_id, cx)
                    })
                    .and_then(|result| result)
                {
                    eprintln!("Dojo could not toggle theme picker: {error:#}");
                }
            });

            let controller_for_launcher = controller.downgrade();
            cx.on_action(move |action: &crate::launcher::ToggleLauncher, cx| {
                match controller_for_launcher.update(cx, |controller, cx| {
                    controller.launcher.toggle(action.display_id, cx)
                }) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) | Err(error) => {
                        eprintln!("Dojo could not open launcher: {error:#}")
                    }
                }
            });

            let controller_for_display_events = controller.downgrade();
            let display_subscription = cx.observe_displays(move |event, cx| {
                let result = controller_for_display_events.update(cx, |controller, _| {
                    controller.queue_display_reconcile(event)
                });
                match result {
                    Ok(Some(generation)) => {
                        let controller_for_reconcile = controller_for_display_events.clone();
                        cx.spawn(async move |cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(350))
                                .await;
                            match controller_for_reconcile.update(cx, |controller, cx| {
                                controller.reconcile_if_current(generation, cx)
                            }) {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    eprintln!("Dojo output reconciliation failed: {error:#}")
                                }
                                Err(error) => {
                                    eprintln!("Dojo output reconciliation ended: {error:#}")
                                }
                            }
                        })
                        .detach();
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("Dojo display observer ended: {error:#}"),
                }
            });
            controller.update(cx, |controller, _| {
                controller.display_subscription = Some(display_subscription);
            });

            let controller_for_commands = controller.clone();
            cx.spawn(async move |cx| {
                while let Ok(command) = command_receiver.recv().await {
                    let quit = command == ControlCommand::Quit;
                    let result =
                        controller_for_commands.update(cx, |controller, cx| match command {
                            ControlCommand::Toggle => {
                                controller.set_enabled(!controller.enabled, cx)
                            }
                            ControlCommand::Enable => controller.set_enabled(true, cx),
                            ControlCommand::Disable => controller.set_enabled(false, cx),
                            ControlCommand::Quit => controller.quit(cx),
                            ControlCommand::Launcher => {
                                controller.launcher.toggle(focused_display_id(cx), cx)
                            }
                            ControlCommand::LauncherNavigation(navigation) => {
                                controller.launcher.navigate(navigation, cx)
                            }
                            ControlCommand::Theme => controller.theme_picker.toggle(None, cx),
                            ControlCommand::Models => {
                                let display = focused_display_id(cx)
                                    .or_else(|| cx.primary_display().map(|display| display.id()))
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("no output available for model picker")
                                    })?;
                                crate::model_picker::toggle(display, cx)
                            }
                            ControlCommand::Displays => {
                                let display = focused_display_id(cx)
                                    .or_else(|| cx.primary_display().map(|display| display.id()))
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("no output available for display settings")
                                    })?;
                                crate::displays_ui::toggle(display, cx)
                            }
                            ControlCommand::Wifi | ControlCommand::SystemMenu => {
                                let display = focused_display_id(cx)
                                    .or_else(|| cx.primary_display().map(|display| display.id()))
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("no output available for shell controls")
                                    })?;
                                if command == ControlCommand::Wifi {
                                    crate::wifi_ui::toggle(display, cx)
                                } else {
                                    crate::system_menu::toggle(display, cx)
                                }
                            }
                            ControlCommand::Leave(departure) => {
                                controller.close_temporary_surfaces(cx);
                                crate::session_overlay::leave(departure, cx)
                            }
                            ControlCommand::Farewell(occasion) => {
                                controller.close_temporary_surfaces(cx);
                                crate::session_overlay::farewell(occasion, cx)
                            }
                            ControlCommand::Terminal => controller
                                .set_enabled(true, cx)
                                .and_then(|()| controller.quick_terminal.toggle(cx)),
                            ControlCommand::TerminalClosed => {
                                controller.quick_terminal.session_ended(cx);
                                Ok(())
                            }
                            ControlCommand::Speech(event) => {
                                cx.update_global::<crate::speech::SpeechUi, _>(|speech, _| {
                                    speech.apply(event, std::time::Instant::now());
                                });
                                cx.refresh_windows();
                                Ok(())
                            }
                        });
                    match result {
                        Ok(()) if quit => {
                            cx.update(|cx| cx.quit());
                            return;
                        }
                        Ok(()) => {}
                        Err(error) => {
                            eprintln!("Dojo control target is unavailable: {error:#}");
                            if quit {
                                cx.update(|cx| cx.quit());
                                return;
                            }
                        }
                    }
                }
            })
            .detach();
            crate::session_overlay::start_login(cx);
            crate::startup::mark("start-up-queued");
        });
    crate::session_overlay::clear_overlay_ready();
    appearance_task.borrow_mut().take();
    if let Some(watcher) = &mut file_watcher {
        watcher.shutdown();
    }
    if let Err(error) = niri_layout_for_exit.borrow_mut().disable() {
        eprintln!("Failed to restore Niri widths after Dojo exited: {error:#}");
    }
    control_server.shutdown();
    if let Some(error) = startup_error.borrow_mut().take() {
        bail!(error);
    }
    Ok(())
}

fn invalidate_theme(sender: &async_channel::Sender<()>) {
    match sender.try_send(()) {
        Ok(())
        | Err(async_channel::TrySendError::Full(()))
        | Err(async_channel::TrySendError::Closed(())) => {}
    }
}

async fn follow_appearance(sender: &async_channel::Sender<()>) -> anyhow::Result<()> {
    use zbus::export::futures_core::Stream as _;
    let connection = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        "nl.whynothugo.darkman",
        "/nl/whynothugo/darkman",
        "nl.whynothugo.darkman",
    )
    .await?;
    let stream = proxy.receive_signal("ModeChanged").await?;
    let mut stream = std::pin::pin!(stream);
    let owners = proxy.receive_owner_changed().await?;
    let mut owners = std::pin::pin!(owners);
    invalidate_theme(sender);
    while std::future::poll_fn(|cx| {
        if let std::task::Poll::Ready(owner) = owners.as_mut().poll_next(cx) {
            return std::task::Poll::Ready(owner.is_some());
        }
        stream.as_mut().poll_next(cx).map(|signal| signal.is_some())
    })
    .await
    {
        invalidate_theme(sender);
    }
    anyhow::bail!("darkman signal stream ended")
}

#[cfg(test)]
mod focused_display_tests {
    use super::{FocusedOutput, display_at_origin};
    use gpui::{DisplayId, Point, px};

    #[test]
    fn focused_output_selects_the_matching_external_display() {
        let focused: FocusedOutput = serde_json::from_str(
            r#"{"name":"eDP-1","logical":{"x":1920,"y":0,"width":1645,"height":1028,"scale":1.75}}"#,
        )
        .unwrap();
        let id = display_at_origin(
            Point::new(
                px(focused.logical.x / focused.logical.scale.ceil()),
                px(focused.logical.y / focused.logical.scale.ceil()),
            ),
            [
                (DisplayId::from(1), Point::new(px(0.), px(0.))),
                (DisplayId::from(2), Point::new(px(960.), px(0.))),
            ],
        );
        assert_eq!(id, Some(DisplayId::from(2)));
    }
}
