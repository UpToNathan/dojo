use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui::{App, AppContext, DisplayChanges, DisplayEvent, DisplayId, QuitMode, Subscription};
use gpui_platform::application;

use crate::control::{ControlCommand, ControlServer};
use crate::frame::DisplayFrame;
use crate::niri_layout::NiriLayout;
use crate::state::{Preferences, StateStore};
use crate::tray::{TrayHandle, TrayService};

struct ShellController {
    enabled: bool,
    state: StateStore,
    niri_layout: Rc<RefCell<NiriLayout>>,
    displays: HashMap<DisplayId, DisplayFrame>,
    display_subscription: Option<Subscription>,
    tray_service: Option<TrayService>,
    display_generation: u64,
}

impl ShellController {
    fn new(state: StateStore, niri_layout: Rc<RefCell<NiriLayout>>) -> Self {
        Self {
            enabled: false,
            state,
            niri_layout,
            displays: HashMap::new(),
            display_subscription: None,
            tray_service: None,
            display_generation: 0,
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

        if let Err(error) = self.niri_layout.borrow_mut().enable() {
            close_displays(opened, cx);
            if let Some(service) = tray_service.take() {
                service.shutdown();
            }
            return Err(error).context("reserving the Niri tiled area");
        }

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
    let state = StateStore::user_default()?;
    let initial_enabled = match force_enabled {
        Some(enabled) => enabled,
        None => state
            .load()?
            .map_or(true, |preferences| preferences.frame_enabled),
    };
    let niri_layout = Rc::new(RefCell::new(NiriLayout::default()));
    let niri_layout_for_exit = Rc::clone(&niri_layout);
    let (command_sender, command_receiver) = mpsc::channel();
    let mut control_server = ControlServer::start(command_sender)?;
    let startup_error = Rc::new(RefCell::new(None::<String>));
    let startup_error_for_run = Rc::clone(&startup_error);

    application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            if let Err(error) = crate::power::register_font(cx) {
                let error = format!("Failed to load Dojo's bundled battery icon font: {error:#}");
                eprintln!("{error}");
                *startup_error_for_run.borrow_mut() = Some(error);
                cx.quit();
                return;
            }

            let controller = cx.new(|_| ShellController::new(state, Rc::clone(&niri_layout)));
            if let Err(error) = controller.update(cx, |controller, cx| {
                controller.initialize(initial_enabled, cx)
            }) {
                let error = format!("Failed to initialize Dojo: {error:#}");
                eprintln!("{error}");
                *startup_error_for_run.borrow_mut() = Some(error);
                cx.quit();
                return;
            }

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
                loop {
                    loop {
                        let command = match command_receiver.try_recv() {
                            Ok(command) => command,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        };
                        let quit = command == ControlCommand::Quit;
                        let result =
                            controller_for_commands.update(cx, |controller, cx| match command {
                                ControlCommand::Toggle => {
                                    controller.set_enabled(!controller.enabled, cx)
                                }
                                ControlCommand::Enable => controller.set_enabled(true, cx),
                                ControlCommand::Disable => controller.set_enabled(false, cx),
                                ControlCommand::Quit => controller.quit(cx),
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
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                }
            })
            .detach();
        });
    if let Err(error) = niri_layout_for_exit.borrow_mut().disable() {
        eprintln!("Failed to restore Niri widths after Dojo exited: {error:#}");
    }
    control_server.shutdown();
    if let Some(error) = startup_error.borrow_mut().take() {
        bail!(error);
    }
    Ok(())
}
