use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use gpui::{
    AnyElement, App, Bounds, Context, DisplayId, ImageSource, MouseButton, Size, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, img,
    layer_shell::Anchor, point, prelude::*, px,
};

use crate::config::ThemeColors;
use crate::geometry::{BOTTOM_INSET, LEFT_PANEL_WIDTH, RIGHT_INSET, TOP_BAR_HEIGHT};
use crate::power::{self, PowerStatus};
use crate::tray::{TrayAction, TrayHandle, TrayImageCache, TrayItem, TrayMenu, TrayMenuItem};

const LAUNCHER_BUTTON_SIZE: f32 = 36.;
const TOP_BAR_GAP: f32 = 3.;
const TOP_BAR_BUTTON_WIDTH: f32 = 18.;
const TOP_BAR_ICON_SIZE: f32 = 12.;
const SYSTEM_BUTTON_SIZE: f32 = 20.;

struct FrameView {
    tray: TrayHandle,
    tray_items: Arc<Vec<TrayItem>>,
    tray_images: TrayImageCache,
    _tray_updates: gpui::Task<()>,
    _host_observer: gpui::Subscription,
    speech_wake: Option<(Instant, gpui::Task<()>)>,
    open_menu: Option<(String, TrayItem)>,
    menu_reveal: crate::design::MenuReveal,
    menu_data: Option<TrayMenu>,
    active_submenu: Option<i32>,
    hovered_menu_item: Option<i32>,
    pending_menu_hover: Option<(i32, Instant)>,
    submenu_bounds: Option<Bounds<gpui::Pixels>>,
    last_menu_pointer: Option<gpui::Point<gpui::Pixels>>,
    launcher_hovered: bool,
    wlsunset_hovered: bool,
    theme_hovered: bool,
    model_hovered: bool,
    audio_hovered: bool,
    audio_drag: Option<(f32, u8)>,
    audio_dragged: bool,
    audio_wave: crate::audio::Wave,
    _audio_observer: gpui::Subscription,
    _bluetooth_observer: gpui::Subscription,
    _clock_observer: gpui::Subscription,
    _wifi_observer: gpui::Subscription,
}

#[derive(Clone, PartialEq, Eq)]
struct HostState {
    power: PowerStatus,
    vigiland_running: bool,
}

impl gpui::Global for HostState {}

struct HostObservation {
    _task: gpui::Task<()>,
}

impl gpui::Global for HostObservation {}

struct PowerEvents {
    _connection: zbus::Connection,
    properties: zbus::MessageStream,
    owners: zbus::MessageStream,
    available: bool,
}
impl PowerEvents {
    async fn subscribe(connection: zbus::Connection) -> zbus::Result<Self> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg(0, "org.freedesktop.UPower")?
            .build();
        let mut owners = zbus::MessageStream::for_match_rule(rule, &connection, Some(4)).await?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.UPower")?
            .interface("org.freedesktop.DBus.Properties")?
            .path_namespace("/org/freedesktop/UPower")?
            .build();
        // Installing the second match is itself a bus round-trip. Drain the
        // first stream concurrently so its bounded queue cannot block replies.
        let properties = futures_lite::future::race(
            zbus::MessageStream::for_match_rule(rule, &connection, Some(4)),
            async {
                use futures_lite::StreamExt as _;
                while let Some(Ok(_)) = owners.next().await {}
                Err(zbus::Error::Failure("power owner stream closed".into()))
            },
        )
        .await?;
        let proxy = zbus::Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await?;
        let mut events = Self {
            _connection: connection.clone(),
            properties,
            owners,
            available: false,
        };
        events.available = events
            .while_draining(proxy.call("NameHasOwner", &("org.freedesktop.UPower",)))
            .await?;
        Ok(events)
    }
    async fn while_draining<T>(
        &mut self,
        operation: impl std::future::Future<Output = zbus::Result<T>>,
    ) -> zbus::Result<T> {
        futures_lite::future::race(operation, async {
            while self.next().await {}
            Err(zbus::Error::Failure("power event stream closed".into()))
        })
        .await
    }
    async fn next(&mut self) -> bool {
        use futures_lite::StreamExt as _;
        let event =
            futures_lite::future::race(async { (false, self.properties.next().await) }, async {
                (true, self.owners.next().await)
            })
            .await;
        let (owner_changed, Some(Ok(message))) = event else {
            return false;
        };
        if owner_changed {
            let Ok((_, _, owner)) = message.body().deserialize::<(String, String, String)>() else {
                return false;
            };
            self.available = !owner.is_empty();
        }
        true
    }
}

pub fn initialize_host_state(cx: &mut App) {
    let mut vigiland = VigilandProbe::default();
    cx.set_global(HostState {
        power: power::read_status(),
        vigiland_running: vigiland.read(),
    });
    let task = cx.spawn(async move |cx| {
        let mut power_events = None;
        let mut schedule = HostRefreshSchedule::new(Instant::now());
        loop {
            if power_events.is_none() {
                let subscribe =
                    async { PowerEvents::subscribe(zbus::Connection::system().await?).await };
                let result = futures_lite::future::race(async { Some(subscribe.await) }, async {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    None
                })
                .await;
                match result {
                    Some(Ok(events)) => power_events = Some(events),
                    Some(Err(error)) => {
                        eprintln!("Dojo power event subscription unavailable: {error}")
                    }
                    None => eprintln!("Dojo power event subscription timed out"),
                }
            }
            let wake = cx.background_executor().timer(
                schedule
                    .next_deadline()
                    .saturating_duration_since(Instant::now()),
            );
            let event = futures_lite::future::race(
                async {
                    wake.await;
                    None
                },
                async {
                    match power_events.as_mut() {
                        Some(events) => Some(events.next().await),
                        None => futures_lite::future::pending().await,
                    }
                },
            )
            .await;
            if event == Some(false) {
                power_events = None;
            }
            let (read_power, read_vigiland) = schedule.due(
                Instant::now(),
                event.is_some(),
                power_events.as_ref().is_some_and(|events| events.available),
            );
            let (returned_probe, power, running) = cx
                .background_executor()
                .spawn(async move {
                    let power = read_power.then(power::read_status);
                    let running = read_vigiland.then(|| vigiland.read());
                    (vigiland, power, running)
                })
                .await;
            vigiland = returned_probe;
            cx.update(|cx| {
                let mut next = cx.global::<HostState>().clone();
                if let Some(power) = power {
                    next.power = power;
                }
                if let Some(running) = running {
                    next.vigiland_running = running;
                }
                if cx.global::<HostState>() != &next {
                    cx.set_global(next);
                }
            });
        }
    });
    cx.set_global(HostObservation { _task: task });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryTrayAction {
    ShowMenu,
    Activate,
    ContextMenu,
}

struct ReservationView {
    exclusive_zone: f32,
}

/// How a display's four edges are held: by the direct client, or by GPUI windows when that is
/// unavailable.
enum Reservations {
    Direct(crate::edge_reservation::Reservation),
    Windows(Vec<gpui::WindowHandle<ReservationView>>),
}

impl Reservations {
    fn is_open(&self, cx: &App) -> bool {
        match self {
            Self::Direct(reservation) => reservation.is_active(),
            Self::Windows(windows) => windows.iter().all(|window| window.read(cx).is_ok()),
        }
    }

    fn close(self, cx: &mut App) -> Vec<String> {
        match self {
            Self::Direct(_) => Vec::new(),
            Self::Windows(windows) => close_windows(windows, cx),
        }
    }
}

pub struct DisplayFrame {
    display_id: DisplayId,
    bounds: Bounds<gpui::Pixels>,
    frame: gpui::WindowHandle<FrameView>,
    reservations: Reservations,
}

impl DisplayFrame {
    pub fn open(
        cx: &mut App,
        display_id: DisplayId,
        bounds: Bounds<gpui::Pixels>,
        tray: TrayHandle,
    ) -> anyhow::Result<Self> {
        let reservations = match Self::reserve_directly(cx, display_id) {
            Some(reservation) => Reservations::Direct(reservation),
            None => Reservations::Windows(Self::open_reservation_windows(cx, display_id, bounds)?),
        };
        crate::startup::mark("reservations-requested");

        let frame = match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                app_id: Some("dojo".to_string()),
                display_id: Some(display_id),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(gpui::layer_shell::LayerShellOptions {
                    namespace: format!("dojo-frame-{}", u64::from(display_id)),
                    layer: gpui::layer_shell::Layer::Top,
                    anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                    exclusive_zone: Some(px(-1.)),
                    keyboard_interactivity: gpui::layer_shell::KeyboardInteractivity::None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| FrameView::new(tray.clone(), window, cx)),
        ) {
            Ok(window) => window,
            Err(error) => {
                let errors = reservations.close(cx);
                if !errors.is_empty() {
                    eprintln!(
                        "Failed to close partial edge reservations: {}",
                        errors.join("; ")
                    );
                }
                return Err(error).context("opening desktop frame surface");
            }
        };

        crate::startup::mark("frame-window-opened");
        Ok(Self {
            display_id,
            bounds,
            frame,
            reservations,
        })
    }

    /// The four edges through the direct client, or `None` when it is not available here.
    fn reserve_directly(
        cx: &App,
        display_id: DisplayId,
    ) -> Option<crate::edge_reservation::Reservation> {
        use crate::edge_reservation::Edge;
        let uuid = cx.find_display(display_id)?.uuid().ok()?;
        crate::edge_reservation::reserve(
            uuid,
            &[
                (Edge::Top, TOP_BAR_HEIGHT as u32),
                (Edge::Bottom, BOTTOM_INSET as u32),
                (Edge::Left, LEFT_PANEL_WIDTH as u32),
                (Edge::Right, RIGHT_INSET as u32),
            ],
        )
    }

    fn open_reservation_windows(
        cx: &mut App,
        display_id: DisplayId,
        bounds: Bounds<gpui::Pixels>,
    ) -> anyhow::Result<Vec<gpui::WindowHandle<ReservationView>>> {
        let reservations = [
            (
                Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                Anchor::TOP,
                Size::new(bounds.size.width, px(TOP_BAR_HEIGHT as f32)),
                "top",
                TOP_BAR_HEIGHT as f32,
            ),
            (
                Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                Anchor::BOTTOM,
                Size::new(bounds.size.width, px(BOTTOM_INSET as f32)),
                "bottom",
                BOTTOM_INSET as f32,
            ),
            (
                Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
                Anchor::LEFT,
                Size::new(px(LEFT_PANEL_WIDTH as f32), bounds.size.height),
                "left",
                LEFT_PANEL_WIDTH as f32,
            ),
            (
                Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
                Anchor::RIGHT,
                Size::new(px(RIGHT_INSET as f32), bounds.size.height),
                "right",
                RIGHT_INSET as f32,
            ),
        ];

        let mut opened_reservations = Vec::with_capacity(reservations.len());
        for (anchor, edge, size, name, exclusive_zone) in reservations {
            match cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: bounds.origin,
                        size,
                    })),
                    titlebar: None,
                    app_id: Some("dojo".to_string()),
                    display_id: Some(display_id),
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(gpui::layer_shell::LayerShellOptions {
                        namespace: format!("dojo-reservation-{name}-{}", u64::from(display_id)),
                        layer: gpui::layer_shell::Layer::Top,
                        anchor,
                        exclusive_zone: Some(px(exclusive_zone)),
                        exclusive_edge: Some(edge),
                        keyboard_interactivity: gpui::layer_shell::KeyboardInteractivity::None,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| ReservationView { exclusive_zone }),
            ) {
                Ok(window) => {
                    opened_reservations.push(window);
                    crate::startup::mark(&format!("reservation-{name}-opened"));
                }
                Err(error) => {
                    let errors = close_windows(opened_reservations, cx);
                    if !errors.is_empty() {
                        eprintln!(
                            "Failed to close partial edge reservations: {}",
                            errors.join("; ")
                        );
                    }
                    return Err(error).context(format!("opening {name} edge reservation"));
                }
            }
        }
        Ok(opened_reservations)
    }

    pub fn can_reuse(&self, cx: &App, bounds: Bounds<gpui::Pixels>) -> bool {
        self.bounds.size == bounds.size
            && self.frame.read(cx).is_ok()
            && self.reservations.is_open(cx)
    }

    pub fn close(self, cx: &mut App) -> anyhow::Result<()> {
        let mut errors = self.reservations.close(cx);
        if let Err(error) = self.frame.update(cx, |_, window, _| window.remove_window()) {
            if !is_already_closed_window(&error) {
                errors.push(error.to_string());
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(
                "failed to close display {} surfaces: {}",
                u64::from(self.display_id),
                errors.join("; ")
            )
        }
    }
}

fn close_windows(
    windows: impl IntoIterator<Item = gpui::WindowHandle<ReservationView>>,
    cx: &mut App,
) -> Vec<String> {
    windows
        .into_iter()
        .filter_map(|window| {
            window
                .update(cx, |_, window, _| window.remove_window())
                .err()
                .filter(|error| !is_already_closed_window(error))
                .map(|error| error.to_string())
        })
        .collect()
}

fn is_already_closed_window(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains("window not found"))
}

fn tray_menu_height(items: &[TrayMenuItem]) -> f32 {
    16. + items
        .iter()
        .filter(|item| item.visible)
        .map(tray_menu_row_height)
        .sum::<f32>()
}

fn toggle_wlsunset() {
    let result = std::thread::Builder::new()
        .name("dojo-wlsunset-toggle".into())
        .spawn(
            || match std::process::Command::new("toggle-wlsunset").output() {
                Ok(output) if output.status.success() => {}
                Ok(output) => eprintln!(
                    "Dojo could not toggle wlsunset: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                Err(error) => eprintln!("Dojo could not start the wlsunset toggle: {error}"),
            },
        );
    if let Err(error) = result {
        eprintln!("Dojo could not start the wlsunset toggle thread: {error}");
    }
}

fn tray_menu_row_height(item: &TrayMenuItem) -> f32 {
    if item.separator {
        9.
    } else {
        30. + item.label.lines().count().saturating_sub(1) as f32 * 18.
    }
}

fn tray_menu_width(items: &[TrayMenuItem], window: &Window, colors: ThemeColors) -> f32 {
    let text_system = window.text_system();
    let widest = items
        .iter()
        .filter(|item| item.visible && !item.separator)
        .flat_map(|item| item.label.lines())
        .map(|line| {
            let label = line.replace('_', "");
            let run = gpui::TextRun {
                len: label.len(),
                font: power::font(),
                color: colors.foreground.rgb().into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            f32::from(
                text_system
                    .shape_line(label.into(), px(12.), &[run], None)
                    .width,
            )
        })
        .fold(0., f32::max);
    (widest + 60.).max(224.)
}

fn primary_tray_action(item: &TrayItem) -> PrimaryTrayAction {
    if !item.item_is_menu {
        PrimaryTrayAction::Activate
    } else if item.menu_path.is_some() {
        PrimaryTrayAction::ShowMenu
    } else {
        PrimaryTrayAction::ContextMenu
    }
}

fn top_bar_input_region(viewport: Size<gpui::Pixels>) -> Bounds<gpui::Pixels> {
    Bounds {
        origin: point(px(0.), px(0.)),
        size: Size::new(viewport.width, px(TOP_BAR_HEIGHT as f32)),
    }
}

fn launcher_button_bounds(viewport: Size<gpui::Pixels>) -> Bounds<gpui::Pixels> {
    Bounds {
        origin: point(
            px((LEFT_PANEL_WIDTH as f32 - LAUNCHER_BUTTON_SIZE) / 2. - 1.),
            viewport.height - px(BOTTOM_INSET as f32 + 6. + LAUNCHER_BUTTON_SIZE),
        ),
        size: Size::new(px(LAUNCHER_BUTTON_SIZE), px(LAUNCHER_BUTTON_SIZE)),
    }
}

fn system_button_bounds(viewport: Size<gpui::Pixels>) -> Bounds<gpui::Pixels> {
    Bounds::new(
        point(
            viewport.width - px(SYSTEM_BUTTON_SIZE / 2. + 13.),
            viewport.height - px(SYSTEM_BUTTON_SIZE / 2. + 13.),
        ),
        Size::new(px(SYSTEM_BUTTON_SIZE), px(SYSTEM_BUTTON_SIZE)),
    )
}

fn menu_aim_contains(
    previous: gpui::Point<gpui::Pixels>,
    current: gpui::Point<gpui::Pixels>,
    submenu: Bounds<gpui::Pixels>,
) -> bool {
    let edge = if previous.x >= submenu.right() {
        submenu.right()
    } else if previous.x <= submenu.left() {
        submenu.left()
    } else {
        return submenu.contains(&current);
    };
    let distance = f32::from(edge - previous.x);
    if distance.abs() < f32::EPSILON {
        return submenu.contains(&current);
    }
    let progress = f32::from(current.x - previous.x) / distance;
    if !(0. ..=1.).contains(&progress) || progress == 0. {
        return false;
    }
    let upper = previous.y + (submenu.top() - px(8.) - previous.y) * progress;
    let lower = previous.y + (submenu.bottom() + px(8.) - previous.y) * progress;
    current.y >= upper && current.y <= lower
}

fn submenu_hover_target(
    active: Option<i32>,
    is_submenu: bool,
    item_id: i32,
    has_submenu: bool,
) -> Option<i32> {
    if is_submenu {
        active
    } else {
        has_submenu.then_some(item_id)
    }
}

// Independent deadlines: frequent UPower signals must not trigger process scans
// or postpone the process reconciliation indefinitely.
struct HostRefreshSchedule {
    power: Instant,
    vigiland: Instant,
}
impl HostRefreshSchedule {
    fn new(now: Instant) -> Self {
        Self {
            power: now + Duration::from_secs(10),
            vigiland: now + Duration::from_secs(10),
        }
    }
    fn next_deadline(&self) -> Instant {
        self.power.min(self.vigiland)
    }
    fn due(&mut self, now: Instant, power_event: bool, subscribed: bool) -> (bool, bool) {
        let power = power_event || now >= self.power;
        let vigiland = now >= self.vigiland;
        if power {
            self.power = now + Duration::from_secs(if subscribed { 60 } else { 10 });
        }
        if vigiland {
            self.vigiland = now + Duration::from_secs(10);
        }
        (power, vigiland)
    }
}

#[derive(Default)]
struct VigilandProbe {
    process: Option<std::path::PathBuf>,
}
impl VigilandProbe {
    fn read(&mut self) -> bool {
        self.read_at(std::path::Path::new("/proc"))
    }
    fn read_at(&mut self, root: &std::path::Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        let Ok(metadata) = std::fs::metadata(root.join("self")) else {
            self.process = None;
            return false;
        };
        let matches = |path: &std::path::Path| {
            path.metadata()
                .is_ok_and(|entry| entry.uid() == metadata.uid())
                && std::fs::read_to_string(path.join("comm"))
                    .is_ok_and(|name| name.trim_end() == "vigiland")
        };
        // Revalidate UID and name, not just PID existence (PIDs can be reused).
        if self.process.as_deref().is_some_and(matches) {
            return true;
        }
        self.process = std::fs::read_dir(root).ok().and_then(|processes| {
            processes.flatten().find_map(|process| {
                let name = process.file_name();
                (!name.is_empty()
                    && name.as_encoded_bytes().iter().all(u8::is_ascii_digit)
                    && matches(&process.path()))
                .then(|| process.path())
            })
        });
        self.process.is_some()
    }
}

impl FrameView {
    fn new(tray: TrayHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tray_for_updates = tray.clone();
        let changes = tray.subscribe_changes();
        let tray_updates = cx.spawn_in(window, async move |this, cx| {
            while changes.recv().await.is_ok() {
                if this
                    .update_in(cx, |this, window, cx| {
                        let items = tray_for_updates.items();
                        let menu = this
                            .open_menu
                            .as_ref()
                            .and_then(|(key, _)| tray_for_updates.menu(key));
                        if Arc::ptr_eq(&this.tray_items, &items) && menu == this.menu_data {
                            return;
                        }
                        this.tray_items = items;
                        this.tray_images.update(&this.tray_items, window);
                        this.menu_data = menu;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let bluetooth_observer =
            cx.observe_global::<crate::bluetooth_ui::BluetoothUi>(|_, cx| cx.notify());
        let clock_observer = cx.observe_global::<crate::clock::Clock>(|_, cx| cx.notify());
        let wifi_observer = cx.observe_global::<crate::wifi_ui::WifiUi>(|_, cx| cx.notify());
        let audio_observer = cx.observe_global::<crate::audio::AudioState>(|_, cx| cx.notify());
        let host_observer = cx.observe_global::<HostState>(|_, cx| cx.notify());
        let tray_items = tray.items();
        let mut tray_images = TrayImageCache::default();
        tray_images.update(&tray_items, window);
        Self {
            _clock_observer: clock_observer,
            _wifi_observer: wifi_observer,
            tray_items,
            tray_images,
            _tray_updates: tray_updates,
            _host_observer: host_observer,
            speech_wake: None,
            open_menu: None,
            menu_reveal: crate::design::MenuReveal::opening(Instant::now()),
            menu_data: None,
            active_submenu: None,
            hovered_menu_item: None,
            pending_menu_hover: None,
            submenu_bounds: None,
            last_menu_pointer: None,
            launcher_hovered: false,
            wlsunset_hovered: false,
            theme_hovered: false,
            model_hovered: false,
            audio_hovered: false,
            audio_drag: None,
            audio_dragged: false,
            audio_wave: crate::audio::Wave::default(),
            _audio_observer: audio_observer,
            _bluetooth_observer: bluetooth_observer,
            tray,
        }
    }
}

impl FrameView {
    fn open_tray_menu(&mut self, item: TrayItem, _window: &mut Window, cx: &mut Context<Self>) {
        let key = item.key.clone();
        if self
            .open_menu
            .as_ref()
            .is_some_and(|(open_key, _)| open_key == &key)
        {
            self.close_tray_menu(cx);
            return;
        }
        self.tray.open_menu(&item);
        self.menu_data = self.tray.menu(&key);
        self.open_menu = Some((key, item));
        self.menu_reveal = crate::design::MenuReveal::opening(Instant::now());
        self.active_submenu = None;
        self.hovered_menu_item = None;
        self.pending_menu_hover = None;
        self.last_menu_pointer = None;

        cx.notify();
    }

    fn close_tray_menu(&mut self, cx: &mut Context<Self>) {
        self.menu_reveal.set_open(false, Instant::now());
        cx.notify();
    }

    fn render_menu_items(
        &self,
        items: &[TrayMenuItem],
        menu_owner: &TrayItem,
        menu_width: f32,
        is_submenu: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = *cx.global::<ThemeColors>();
        items
            .iter()
            .filter(|item| item.visible)
            .map(|item| {
                if item.separator {
                    return div()
                        .mx(px(10.))
                        .my(px(4.))
                        .h(px(1.))
                        .bg(colors.border.rgb().opacity(crate::design::BORDER_OPACITY))
                        .into_any_element();
                }

                let item_id = item.id;
                let enabled = item.enabled;
                let has_submenu = item.has_submenu;
                let mut row =
                    div()
                        .id(format!("dbus-menu-item-{item_id}"))
                        .mx(px(8.))
                        .w(px(menu_width - 16.))
                        .h(px(tray_menu_row_height(item)))
                        .flex_shrink_0()
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(12.))
                        .rounded(px(15.))
                        .bg(if self.hovered_menu_item == Some(item_id) {
                            colors.background_highlight.rgb()
                        } else {
                            colors.background.rgb()
                        })
                        .text_color(if self.hovered_menu_item == Some(item_id) {
                            colors.selection_foreground.rgb()
                        } else if enabled {
                            colors.foreground.rgb()
                        } else {
                            colors.foreground_muted.rgb()
                        })
                        .child(div().flex().flex_col().children(item.label.split('\n').map(
                            |line| {
                                div().h(px(18.)).child(if line.is_empty() {
                                    " ".to_string()
                                } else {
                                    line.replace('_', "")
                                })
                            },
                        )));

                if item.checked {
                    row = row.child("✓");
                } else if has_submenu {
                    row = row.child("›");
                }

                row = row.on_hover(cx.listener(move |this, hovered, window, cx| {
                    if *hovered {
                        this.hovered_menu_item = Some(item_id);
                    } else if this.hovered_menu_item == Some(item_id) {
                        this.hovered_menu_item = None;
                    }
                    if *hovered {
                        let pointer = window.mouse_position();
                        let target = submenu_hover_target(
                            this.active_submenu,
                            is_submenu,
                            item_id,
                            has_submenu,
                        );
                        if is_submenu {
                            this.pending_menu_hover = None;
                            this.active_submenu = target;
                        } else {
                            let heading_to_submenu = this.active_submenu != target
                                && this.last_menu_pointer.zip(this.submenu_bounds).is_some_and(
                                    |(previous, bounds)| {
                                        menu_aim_contains(previous, pointer, bounds)
                                    },
                                );
                            if heading_to_submenu {
                                let request = (item_id, Instant::now());
                                this.pending_menu_hover = Some(request);
                                cx.spawn_in(window, async move |this, cx| {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(300))
                                        .await;
                                    if let Err(error) = this.update_in(cx, |this, _, cx| {
                                        if this.pending_menu_hover == Some(request) {
                                            this.pending_menu_hover = None;
                                            this.active_submenu = target;
                                            cx.notify();
                                        }
                                    }) {
                                        eprintln!("Dojo could not update submenu hover: {error}");
                                    }
                                })
                                .detach();
                            } else {
                                this.pending_menu_hover = None;
                                this.active_submenu = target;
                            }
                        }
                        this.last_menu_pointer = Some(pointer);
                    }
                    cx.notify();
                }));

                if has_submenu {
                    row = row.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.active_submenu = Some(item_id);
                            this.pending_menu_hover = None;
                            cx.notify();
                        }),
                    );
                } else if enabled {
                    let menu_owner = menu_owner.clone();
                    row = row.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.tray.menu_event(&menu_owner, item_id);
                            this.close_tray_menu(cx);
                        }),
                    );
                }

                row.into_any_element()
            })
            .collect()
    }
}

impl Render for FrameView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let power = cx.global::<HostState>().power.clone();
        let vigiland_running = cx.global::<HostState>().vigiland_running;
        let reduced_motion = cx.reduce_motion();
        let now = Instant::now();
        let speech = cx.global::<crate::speech::SpeechUi>();
        let animation = speech.animation_schedule(now, reduced_motion);
        let speech_phase = speech.phase;
        let speech_glyph = speech_phase.glyph();
        let speech_icon_size = speech_phase.icon_size();
        let speech_label = speech_phase.label();
        let speech_bubbles: Vec<_> = speech
            .messages
            .iter()
            .rev()
            .filter(|bubble| !bubble.expired(now))
            .map(|bubble| {
                (
                    bubble.id,
                    bubble.text.clone(),
                    bubble.is_response,
                    if reduced_motion {
                        1.
                    } else {
                        bubble.opacity(now)
                    },
                    if reduced_motion {
                        0.
                    } else {
                        bubble.rise_offset(now)
                    },
                    if reduced_motion {
                        1.
                    } else {
                        bubble.entry_progress(now)
                    },
                )
            })
            .collect();
        match animation {
            crate::speech::AnimationSchedule::Animate => {
                self.speech_wake = None;
                window.request_animation_frame();
            }
            crate::speech::AnimationSchedule::Idle => self.speech_wake = None,
            crate::speech::AnimationSchedule::WakeAt(deadline) => {
                if self
                    .speech_wake
                    .as_ref()
                    .is_none_or(|(scheduled, _)| *scheduled != deadline)
                {
                    let task = cx.spawn_in(window, async move |view, cx| {
                        cx.background_executor()
                            .timer(deadline.saturating_duration_since(Instant::now()))
                            .await;
                        if let Err(error) = view.update_in(cx, |_, _, cx| cx.notify()) {
                            eprintln!("Dojo speech deadline target closed: {error}");
                        }
                    });
                    self.speech_wake = Some((deadline, task));
                }
            }
        }
        let viewport = window.viewport_size();
        let top_bar_input = [top_bar_input_region(viewport)];
        let launcher_bounds = launcher_button_bounds(viewport);
        let system_bounds = system_button_bounds(viewport);
        let launcher_hovered = self.launcher_hovered && window.is_window_hovered();
        let wlsunset_hovered = self.wlsunset_hovered && window.is_window_hovered();
        let theme_hovered = self.theme_hovered && window.is_window_hovered();
        let model_hovered = self.model_hovered && window.is_window_hovered();

        let battery_glyph = power::battery_glyph(power.capacity, power.battery_status.as_deref());
        let charging = power.battery_status.as_deref() == Some("Charging");
        let battery_percent = power
            .capacity
            .map_or("?".to_string(), |capacity| format!("{capacity}%"));
        let visible_items = self
            .tray_items
            .iter()
            .filter(|item| item.status != "Passive")
            .collect::<Vec<_>>();
        let mut tray_icons = Vec::with_capacity(visible_items.len());

        for (index, item) in visible_items.iter().enumerate() {
            let icon: AnyElement = if let Some(image) = self.tray_images.image(&item.key) {
                img(ImageSource::Render(image))
                    .size(px(TOP_BAR_ICON_SIZE))
                    .into_any_element()
            } else {
                div()
                    .text_size(px(11.))
                    .child(fallback_tray_glyph(item))
                    .into_any_element()
            };
            let left_item = (*item).clone();
            let mut button = div()
                .id(format!("tray-item-{index}"))
                .aria_label(if item.title.is_empty() {
                    item.icon_name.clone()
                } else {
                    item.title.clone()
                })
                .w(px(TOP_BAR_BUTTON_WIDTH))
                .h(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(
                        move |this, _, window, cx| match primary_tray_action(&left_item) {
                            PrimaryTrayAction::ShowMenu => {
                                this.open_tray_menu(left_item.clone(), window, cx);
                            }
                            PrimaryTrayAction::Activate => {
                                this.tray.activate(&left_item, TrayAction::Activate);
                            }
                            PrimaryTrayAction::ContextMenu => {
                                this.tray.activate(&left_item, TrayAction::ContextMenu);
                            }
                        },
                    ),
                );

            let right_item = (*item).clone();
            button = button.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    if right_item.menu_path.is_some() {
                        this.open_tray_menu(right_item.clone(), window, cx);
                    } else {
                        this.tray.activate(&right_item, TrayAction::ContextMenu);
                    }
                }),
            );
            let middle_item = (*item).clone();
            button = button.on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, _, _| {
                    this.tray
                        .activate(&middle_item, TrayAction::SecondaryActivate);
                }),
            );
            tray_icons.push(button.into_any_element());
        }

        let audio_volume = cx.global::<crate::audio::AudioState>().volume;
        let audio_expanded =
            (self.audio_hovered && window.is_window_hovered()) || self.audio_drag.is_some();
        let (wave, moving) = self.audio_wave.update(audio_volume, audio_expanded, now);
        if moving {
            window.request_animation_frame();
        }
        let audio_width = wave[7];
        let mut audio = div()
            .id("audio-wave")
            .aria_label(match audio_volume {
                Some(v) => format!(
                    "Volume {} percent{}. Drag or scroll to adjust; click to mute",
                    v.percent,
                    if v.muted { ", muted" } else { "" }
                ),
                None => "Audio output unavailable".into(),
            })
            .w(px(audio_width))
            .h(px(22.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px(px(4.))
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered, _, cx| {
                this.audio_hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if let Some(volume) = cx.global::<crate::audio::AudioState>().volume {
                        this.audio_drag = Some((f32::from(event.position.x), volume.percent));
                        this.audio_dragged = false;
                        this.close_tray_menu(cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|_, event: &gpui::ScrollWheelEvent, _, cx| {
                let delta = match event.delta {
                    gpui::ScrollDelta::Lines(p) => p.y,
                    gpui::ScrollDelta::Pixels(p) => f32::from(p.y) / 12.,
                };
                if delta != 0. {
                    let audio = cx.global::<crate::audio::AudioState>();
                    if audio.volume.is_some() {
                        audio.adjust_volume(delta * 2.);
                    }
                    cx.stop_propagation();
                }
            }));
        let reveal = ((audio_width - 36.) / 44.).clamp(0., 1.);
        audio = audio.child(audio_volume_label(reveal, audio_volume, colors));
        let flatness = (1. - (wave[3] - 2.) / 3.).clamp(0., 1.);
        let wave_color = if audio_volume.is_none_or(|v| v.muted) {
            colors.foreground_muted.rgb()
        } else {
            colors.foreground.rgb()
        };
        audio = audio.child(
            div()
                .relative()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .right(px(0.))
                        .top(px(10.))
                        .h(px(2.))
                        .rounded(px(2.))
                        .bg(wave_color)
                        .opacity(flatness),
                )
                .children(wave[..7].iter().enumerate().map(|(index, height)| {
                    div()
                        .id(("audio-crest", index))
                        .w(px(2.5))
                        .h(px(*height))
                        .rounded(px(2.))
                        .opacity(1. - flatness)
                        .bg(wave_color)
                })),
        );
        let tray_count = tray_icons.len();
        let indicator_count = usize::from(vigiland_running);
        let top_bar_width = 124.
            + audio_width
            + TOP_BAR_BUTTON_WIDTH
            + TOP_BAR_BUTTON_WIDTH
            + TOP_BAR_BUTTON_WIDTH
            + TOP_BAR_BUTTON_WIDTH
            + TOP_BAR_GAP
            + TOP_BAR_GAP * 3.
            + (tray_count + indicator_count) as f32 * (TOP_BAR_BUTTON_WIDTH + TOP_BAR_GAP);
        let clock_bounds = crate::geometry::top_bar_clock_bounds(viewport, top_bar_width);
        let tray_group = div()
            .flex()
            .items_center()
            .gap(px(TOP_BAR_GAP))
            .children(tray_icons);

        let mut top_bar = div()
            .absolute()
            .top(px(0.))
            .right(px(RIGHT_INSET as f32 - 3.))
            .w(px(top_bar_width))
            .h(px(TOP_BAR_HEIGHT as f32))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(TOP_BAR_GAP))
            .text_color(colors.foreground.rgb())
            .child(audio)
            .child(
                div()
                    .id("displays-button")
                    .aria_label("Open display settings")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(14.))
                    .cursor_pointer()
                    .child(top_bar_icon("\u{f0379}", 0.545))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_tray_menu(cx);
                            cx.stop_propagation();
                            if let Some(display) = window.display(cx) {
                                if let Err(error) = crate::displays_ui::toggle(display.id(), cx) {
                                    eprintln!("Dojo could not open display settings: {error:#}");
                                }
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .id("mesh-button")
                    .aria_label("Open the mesh panel")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(14.))
                    .cursor_pointer()
                    .child(top_bar_icon("\u{f0318}", 0.545))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_tray_menu(cx);
                            cx.stop_propagation();
                            if let Some(display) = window.display(cx) {
                                if let Err(error) = crate::mesh_ui::toggle(display.id(), crate::control::MeshTab::Machines, cx) {
                                    eprintln!("Dojo could not open the mesh panel: {error:#}");
                                }
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .id("wifi-button")
                    .aria_label("Open network menu")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(14.))
                    .child(top_bar_icon(
                        if cx.has_global::<crate::wifi_ui::WifiUi>() {
                            cx.global::<crate::wifi_ui::WifiUi>().snapshot.icon()
                        } else {
                            "\u{f05aa}"
                        },
                        0.545,
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_tray_menu(cx);
                            cx.stop_propagation();
                            if let Some(display) = window.display(cx) {
                                if let Err(error) = crate::wifi_ui::toggle(display.id(), cx) {
                                    eprintln!("Dojo could not open Wi-Fi: {error:#}");
                                }
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .id("bluetooth-button")
                    .aria_label("Open Bluetooth dock")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(14.))
                    .text_color(
                        if cx
                            .global::<crate::bluetooth_ui::BluetoothUi>()
                            .snapshot
                            .adapters
                            .iter()
                            .any(|a| a.powered)
                        {
                            colors.foreground.rgb()
                        } else {
                            colors.foreground_muted.rgb()
                        },
                    )
                    .cursor_pointer()
                    .child(top_bar_icon("\u{f00af}", 0.858))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_tray_menu(cx);
                            cx.stop_propagation();
                            if let Some(display) = window.display(cx) {
                                if let Err(error) = crate::bluetooth_ui::toggle(display.id(), cx) {
                                    eprintln!("Dojo could not open Bluetooth: {error:#}");
                                }
                            }
                        }),
                    ),
            )
            .when(vigiland_running, |bar| {
                bar.child(
                    div()
                        .id("vigiland-indicator")
                        .aria_label("Vigiland is inhibiting idle")
                        .w(px(TOP_BAR_BUTTON_WIDTH))
                        .h(px(18.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(18.5))
                        .child(top_bar_icon("\u{f05e2}", 0.680)),
                )
            })
            .child(
                div()
                    .id("wlsunset-button")
                    .aria_label("Toggle wlsunset eye-comfort filter")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .rounded(px(crate::design::PILL_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(18.5))
                    .bg(if wlsunset_hovered {
                        colors.background_highlight.rgb().opacity(0.35)
                    } else {
                        colors.background.rgb()
                    })
                    .text_color(colors.foreground.rgb())
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if this.wlsunset_hovered != *hovered {
                            this.wlsunset_hovered = *hovered;
                            cx.notify();
                        }
                    }))
                    .child(top_bar_icon("\u{f2a8}", 0.547))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.wlsunset_hovered = false;
                            this.close_tray_menu(cx);
                            cx.notify();
                            cx.stop_propagation();
                            toggle_wlsunset();
                        }),
                    ),
            )
            .child(
                div()
                    .id("theme-picker-button")
                    .aria_label("Choose theme")
                    .w(px(TOP_BAR_BUTTON_WIDTH))
                    .h(px(18.))
                    .flex_shrink_0()
                    .rounded(px(crate::design::PILL_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(18.5))
                    .bg(if theme_hovered {
                        colors.background_highlight.rgb().opacity(0.35)
                    } else {
                        colors.background.rgb()
                    })
                    .text_color(colors.foreground.rgb())
                    .on_hover(cx.listener(|this, hovered, _, cx| {
                        if this.theme_hovered != *hovered {
                            this.theme_hovered = *hovered;
                            cx.notify();
                        }
                    }))
                    .child(top_bar_icon("\u{f1fc}", 0.547))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.theme_hovered = false;
                            this.close_tray_menu(cx);
                            cx.notify();
                            cx.stop_propagation();
                            window.dispatch_action(
                                Box::new(crate::theme_picker::ToggleThemePicker {
                                    display_id: window.display(cx).map(|display| display.id()),
                                }),
                                cx,
                            );
                        }),
                    ),
            );

        top_bar = top_bar.child(
            div()
                .id("model-picker-button")
                .aria_label(if crate::model_picker::setup_needed(cx) {
                    "Choose agent model (sign-in needed)"
                } else {
                    "Choose agent model"
                })
                .relative()
                .w(px(TOP_BAR_BUTTON_WIDTH))
                .h(px(18.))
                .flex_shrink_0()
                .rounded(px(crate::design::PILL_RADIUS))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_size(px(18.5))
                .bg(if model_hovered {
                    colors.background_highlight.rgb().opacity(0.35)
                } else {
                    colors.background.rgb()
                })
                .text_color(colors.foreground.rgb())
                .on_hover(cx.listener(|this, hovered, _, cx| {
                    if this.model_hovered != *hovered {
                        this.model_hovered = *hovered;
                        cx.notify();
                    }
                }))
                .child(top_bar_icon("\u{f06a9}", 0.62))
                .when(crate::model_picker::setup_needed(cx), |button| {
                    button.child(
                        div()
                            .absolute()
                            .top(px(1.))
                            .right(px(3.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(colors.primary.rgb()),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.model_hovered = false;
                        this.close_tray_menu(cx);
                        cx.stop_propagation();
                        if let Some(display) = window.display(cx) {
                            if let Err(error) = crate::model_picker::toggle(display.id(), cx) {
                                eprintln!("Dojo could not open model picker: {error:#}");
                            }
                        }
                    }),
                ),
        );

        if tray_count > 0 {
            top_bar = top_bar.child(tray_group);
        }
        // Desktops have no battery; an unknown-battery chip there would only mislead.
        if power.has_battery() {
            top_bar = top_bar.child(
                div()
                    .id("battery-status")
                    .aria_label(if power.battery_status.as_deref() == Some("Charging") {
                        "Battery charging"
                    } else {
                        "Battery status"
                    })
                    .flex()
                    .items_center()
                    .ml(px(2.))
                    .gap(px(0.))
                    .text_size(px(11.))
                    .child(top_bar_icon(
                        &battery_glyph.to_string(),
                        if charging { 0.545 } else { 0.907 },
                    ))
                    .child(div().ml(px(-1.)).child(battery_percent)),
            );
        }

        let (menu_progress, menu_moving) = self.menu_reveal.sample_with_motion(now, reduced_motion);
        if self.open_menu.is_some() {
            if menu_moving {
                window.request_animation_frame();
            } else if !self.menu_reveal.is_open() {
                self.open_menu = None;
                self.menu_data = None;
                self.active_submenu = None;
                self.hovered_menu_item = None;
                self.pending_menu_hover = None;
                self.submenu_bounds = None;
                self.last_menu_pointer = None;
            }
        }
        let active_item = self.open_menu.as_ref().map(|(_, item)| item);
        let active_menu = self.menu_data.as_ref();
        let menu_width = active_menu
            .map_or(224., |menu| tray_menu_width(&menu.items, window, colors))
            .min((f32::from(viewport.width) - RIGHT_INSET as f32 * 2.).max(1.));
        let menu_height = active_menu.map_or(46., |menu| tray_menu_height(&menu.items));
        let menu_right = active_item.map(|_| RIGHT_INSET as f32);
        let menu_bounds = menu_right.map(|right| Bounds {
            origin: point(
                viewport.width - px(right + menu_width),
                px(TOP_BAR_HEIGHT as f32),
            ),
            size: Size::new(px(menu_width), px(menu_height.max(46.))),
        });

        let mut menu_hit_bounds = menu_bounds.into_iter().collect::<Vec<_>>();

        let mut menu_surface = active_menu.and_then(|_| menu_bounds);
        let mut menu_content: Vec<AnyElement> = Vec::new();
        if let (Some((_, owner)), Some(menu), Some(bounds), Some(right)) = (
            self.open_menu.as_ref(),
            active_menu,
            menu_bounds,
            menu_right,
        ) {
            let rows = self.render_menu_items(&menu.items, owner, menu_width, false, cx);
            menu_content.push(
                div()
                    .absolute()
                    .top(bounds.origin.y)
                    .right(px(right))
                    .w(px(menu_width))
                    .h(px(menu_height * menu_progress))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .py(px(8.))
                    .children(rows)
                    .into_any_element(),
            );

            if let Some(submenu) = self
                .active_submenu
                .and_then(|id| menu.items.iter().find(|item| item.id == id))
            {
                let submenu_height = tray_menu_height(&submenu.children);
                let submenu_right = right + menu_width - 2.;
                let submenu_width = tray_menu_width(&submenu.children, window, colors)
                    .min((f32::from(viewport.width) - submenu_right).max(1.));
                let submenu_rows =
                    self.render_menu_items(&submenu.children, owner, submenu_width, true, cx);
                let submenu_bounds = Bounds {
                    origin: point(
                        viewport.width - px(submenu_right + submenu_width),
                        bounds.origin.y,
                    ),
                    size: Size::new(px(submenu_width), px(submenu_height.max(46.))),
                };
                menu_hit_bounds.push(submenu_bounds);
                menu_content.push(
                    div()
                        .absolute()
                        .top(bounds.origin.y)
                        .right(px(submenu_right))
                        .w(px(submenu_width))
                        .h(px(submenu_height * menu_progress))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .py(px(8.))
                        .children(submenu_rows)
                        .into_any_element(),
                );
                menu_surface = Some(Bounds {
                    origin: submenu_bounds.origin,
                    size: Size::new(
                        bounds.right() - submenu_bounds.left(),
                        bounds.size.height.max(submenu_bounds.size.height),
                    ),
                });
            }
        }
        self.submenu_bounds = menu_hit_bounds.get(1).copied();
        if let Some(surface) = menu_surface.as_mut() {
            surface.size.height *= menu_progress;
        }

        let frame_input_regions = if active_item.is_some() {
            vec![Bounds {
                origin: point(px(0.), px(0.)),
                size: window.viewport_size(),
            }]
        } else {
            vec![top_bar_input[0], launcher_bounds, system_bounds]
        };
        if speech_bubbles.is_empty() {
            window.set_input_region(Some(&frame_input_regions));
        }

        let mut shell = div()
            .size_full()
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some((start, volume)) = this.audio_drag {
                    if !event.dragging() {
                        this.audio_drag = None;
                        cx.notify();
                        return;
                    }
                    let distance = f32::from(event.position.x) - start;
                    if distance.abs() > 3. {
                        this.audio_dragged = true;
                    }
                    if this.audio_dragged {
                        cx.global::<crate::audio::AudioState>()
                            .set_volume(crate::audio::drag_volume(volume, distance));
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.audio_drag.take().is_some() {
                        if !this.audio_dragged {
                            cx.global::<crate::audio::AudioState>().toggle_mute();
                        }
                        cx.notify();
                    }
                }),
            )
            .on_mouse_exit(cx.listener(|this, _, _, cx| {
                if this.audio_drag.take().is_some() {
                    cx.notify();
                }
            }))
            .font_family(power::FONT_FAMILY)
            .font_features(power::font_features())
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        let opening = crate::geometry::desktop_opening(bounds.size);
                        let inner_bounds =
                            Bounds::new(bounds.origin + opening.origin, opening.size);
                        crate::design::paint_desktop_frame(
                            window,
                            bounds,
                            inner_bounds,
                            px(crate::geometry::OPENING_RADIUS),
                            colors.background.rgb(),
                            colors.border.rgb(),
                        );
                        if let Some(menu_bounds) = menu_surface {
                            crate::design::paint_shell_extension(
                                window,
                                menu_bounds,
                                colors.background.rgb(),
                                colors.border.rgb(),
                                true,
                            );
                        }
                    },
                )
                .size_full(),
            )
            .child(top_bar)
            .child(crate::design::shell_clock_element(
                cx.global::<crate::clock::Clock>().label.clone(),
                clock_bounds,
                crate::geometry::TOP_BAR_CLOCK_PX,
                &colors,
            ))
            .child(
                div()
                    .id("system-menu-button")
                    .aria_label("Open power menu")
                    .absolute()
                    .left(system_bounds.origin.x)
                    .top(system_bounds.origin.y)
                    .size(px(SYSTEM_BUTTON_SIZE))
                    .text_color(colors.primary.rgb())
                    .text_size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child("\u{f0425}")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_tray_menu(cx);
                            cx.stop_propagation();
                            if let Some(display) = window.display(cx) {
                                if let Err(error) = crate::system_menu::toggle(display.id(), cx) {
                                    eprintln!("Dojo could not open power menu: {error:#}");
                                }
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(-1.))
                    .top(launcher_bounds.origin.y)
                    .w(px(LEFT_PANEL_WIDTH as f32))
                    .h(px(LAUNCHER_BUTTON_SIZE))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("open-launcher")
                            .aria_label(speech_label)
                            .size(px(LAUNCHER_BUTTON_SIZE))
                            .rounded(px(LAUNCHER_BUTTON_SIZE / 2.))
                            .text_color(colors.primary.rgb())
                            .text_size(px(32.))
                            .line_height(px(LAUNCHER_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if launcher_hovered {
                                colors.background_highlight.rgb().opacity(0.35)
                            } else {
                                colors.background.rgb()
                            })
                            .on_hover(cx.listener(|this, hovered, _, cx| {
                                if this.launcher_hovered != *hovered {
                                    this.launcher_hovered = *hovered;
                                    cx.notify();
                                }
                            }))
                            .child(
                                gpui::canvas(
                                    |_, _, _| (),
                                    move |bounds, (), window, cx| {
                                        let text: gpui::SharedString = speech_glyph.into();
                                        let run = gpui::TextRun {
                                            len: text.len(),
                                            font: window.text_style().font(),
                                            color: colors.primary.rgb().into(),
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        };
                                        let line = window.text_system().shape_line(
                                            text,
                                            px(speech_icon_size),
                                            &[run],
                                            None,
                                        );
                                        // Compensate for the circle's leftward offset without
                                        // shifting the icon's screen position.
                                        if let Err(error) = line.paint(
                                            point(bounds.origin.x + px(1.), bounds.origin.y),
                                            bounds.size.height,
                                            gpui::TextAlign::Center,
                                            Some(bounds.size.width),
                                            window,
                                            cx,
                                        ) {
                                            eprintln!(
                                                "Dojo could not paint launcher icon: {error}"
                                            );
                                        }
                                    },
                                )
                                .size_full(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.launcher_hovered = false;
                                    this.close_tray_menu(cx);
                                    window.dispatch_action(
                                        Box::new(crate::launcher::ToggleLauncher {
                                            display_id: window
                                                .display(cx)
                                                .map(|display| display.id()),
                                        }),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }),
                            ),
                    ),
            );
        if speech_phase != crate::speech::SpeechPhase::Settled {
            static ANIMATION_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let elapsed = ANIMATION_START
                .get_or_init(Instant::now)
                .elapsed()
                .as_secs_f32();
            let pulse = if reduced_motion || speech_phase == crate::speech::SpeechPhase::Sent {
                1.
            } else {
                0.45 + 0.35 * (elapsed * 3.).sin().abs()
            };
            shell = shell.child(
                div()
                    .absolute()
                    .left(px(LEFT_PANEL_WIDTH as f32 / 2. + 6.))
                    .bottom(px(BOTTOM_INSET as f32 + 13.))
                    .size(px(7.))
                    .rounded_full()
                    .border_1()
                    .border_color(colors.background.rgb())
                    .bg(colors.primary.rgb().opacity(pulse)),
            );
        }
        let blur_bounds = std::rc::Rc::new(std::cell::RefCell::new(Vec::<
            gpui::Bounds<gpui::Pixels>,
        >::new()));
        let bubble_width = |text: &str| {
            let text_width = text
                .lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0) as f32
                * 8.4
                + 80.;
            text_width
                .max(152.)
                .min(520.)
                .min((f32::from(viewport.width) - 48.).max(1.))
        };
        let bubble_height = |text: &str, is_response: bool| {
            let label = gpui::TextRun {
                len: if is_response { 4 } else { 3 },
                font: power::font(),
                color: colors.foreground.rgb().into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let label_width = window
                .text_system()
                .shape_line(
                    if is_response {
                        "Dojo".into()
                    } else {
                        "You".into()
                    },
                    px(10.),
                    &[label],
                    None,
                )
                .width;
            let run = gpui::TextRun {
                len: text.len(),
                font: power::font(),
                color: colors.foreground.rgb().into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_text(
                    text.to_owned().into(),
                    px(14.),
                    &[run],
                    Some(px(
                        (bubble_width(text) - 40. - f32::from(label_width)).max(1.)
                    )),
                    None,
                )
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| f32::from(line.size(px(21.)).height))
                        .sum::<f32>()
                        + 18.
                })
                .unwrap_or(39.)
        };
        let bubble_count = speech_bubbles.len();
        if bubble_count == 0 {
            window.set_background_blur_regions(&[], px(crate::design::SURFACE_RADIUS));
        }
        let mut stack_offset = 0.;
        for (id, text, is_response, opacity, rise, entry) in speech_bubbles {
            let bottom = BOTTOM_INSET as f32 + 18. - rise + stack_offset;
            stack_offset += (bubble_height(&text, is_response) + 10.) * entry;
            let width = bubble_width(&text);
            let left = (f32::from(viewport.width) - width) / 2.;
            let surface = if is_response {
                colors.background_highlight.rgb()
            } else {
                colors.primary.rgb()
            };
            let outline = if is_response {
                colors.border.rgb().opacity(0.2)
            } else {
                colors.primary.rgb().opacity(0.2)
            };
            let blur_bounds = blur_bounds.clone();
            let frame_input_regions = frame_input_regions.clone();
            let set_blur_region = move |children: Vec<gpui::Bounds<gpui::Pixels>>,
                                        window: &mut Window,
                                        _cx: &mut gpui::App| {
                if let Some(mut bounds) = children
                    .into_iter()
                    .reduce(|bounds, child| bounds.union(&child))
                {
                    bounds.origin.x = px(left);
                    bounds.origin.y -= px(8.);
                    bounds.size.width = px(width);
                    bounds.size.height += px(16.);
                    let mut combined = blur_bounds.borrow_mut();
                    combined.push(bounds);
                    if combined.len() == bubble_count {
                        let mut input_regions = frame_input_regions.clone();
                        input_regions.extend(combined.iter().copied());
                        window.set_input_region(Some(&input_regions));
                        window.set_background_blur_regions(&combined, px(22.));
                    }
                }
            };
            shell = shell.child(
                div()
                    .absolute()
                    .left(px(left))
                    .bottom(px(bottom))
                    .w(px(width))
                    .rounded(px(22.))
                    .px(px(14.))
                    .py(px(8.))
                    .bg(surface.opacity(0.2))
                    .border_1()
                    .border_color(outline)
                    .text_size(px(14.))
                    .line_height(px(21.))
                    .text_color(colors.foreground.rgb())
                    .opacity(opacity)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .on_children_prepainted(set_blur_region)
                    .id(("speech-bubble", id))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, _, _, cx| {
                            cx.update_global::<crate::speech::SpeechUi, _>(|speech, _| {
                                speech.dismiss(id)
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(if is_response {
                                colors.foreground_muted.rgb()
                            } else {
                                colors.primary.rgb()
                            })
                            .child(if is_response { "Dojo" } else { "You" }),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(text)),
            );
        }
        if let Some((menu_key, _)) = &self.open_menu {
            let menu_key = menu_key.clone();
            let hit_bounds = menu_hit_bounds.clone();
            for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
                let menu_key = menu_key.clone();
                let hit_bounds = hit_bounds.clone();
                shell = shell.on_mouse_down(
                    button,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        let inside_menu = hit_bounds
                            .iter()
                            .any(|bounds| bounds.contains(&event.position));
                        if !inside_menu
                            && this
                                .open_menu
                                .as_ref()
                                .is_some_and(|(open_key, _)| open_key == &menu_key)
                        {
                            this.close_tray_menu(cx);
                        }
                    }),
                );
            }
        }
        for child in menu_content {
            shell = shell.child(child);
        }
        shell
    }
}

fn audio_volume_label(
    reveal: f32,
    volume: Option<crate::audio::Volume>,
    colors: ThemeColors,
) -> gpui::Div {
    let text = volume.map_or("?".into(), |v| {
        if v.muted {
            "M".into()
        } else {
            format!("{}", v.percent)
        }
    });
    div()
        .w(px(28. * reveal))
        .h(px(22.))
        .flex_shrink_0()
        .overflow_hidden()
        .flex()
        .items_center()
        .opacity(((reveal - 0.85) / 0.15).clamp(0., 1.))
        .child(
            div()
                .w(px(28.))
                .h(px(14.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_right()
                .pr(px(6.))
                .text_size(px(12.))
                .line_height(px(14.))
                .text_color(colors.foreground_muted.rgb())
                .child(text),
        )
}

fn top_bar_icon(glyph: &str, em_extent: f32) -> gpui::Div {
    div()
        .size(px(TOP_BAR_BUTTON_WIDTH))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(TOP_BAR_ICON_SIZE / em_extent))
        .line_height(px(TOP_BAR_BUTTON_WIDTH))
        .child(glyph.to_owned())
}

fn fallback_tray_glyph(item: &TrayItem) -> String {
    item.icon_name
        .chars()
        .find(|character| character.is_alphanumeric())
        .map(|character| character.to_uppercase().to_string())
        .or_else(|| {
            item.title
                .chars()
                .find(|character| character.is_alphanumeric())
                .map(|character| character.to_uppercase().to_string())
        })
        .unwrap_or_else(|| "•".to_string())
}

impl Render for ReservationView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.set_input_region(Some(&[]));
        window.set_exclusive_zone(px(self.exclusive_zone));
        div().size_full()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn system_trigger_and_clock_use_output_geometry() {
        let viewport = gpui::size(gpui::px(1920.), gpui::px(1080.));
        let trigger = super::system_button_bounds(viewport);
        assert_eq!(
            trigger.origin,
            gpui::point(gpui::px(1897.), gpui::px(1057.))
        );
        assert_eq!(trigger.size, gpui::size(gpui::px(20.), gpui::px(20.)));
    }
    use super::*;

    #[tokio::test]
    async fn power_subscription_tracks_service_absence_stop_and_return() {
        use std::io::{BufRead, BufReader, Write};
        use std::process::{Command, Stdio};
        struct Bus(std::process::Child);
        impl Drop for Bus {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut config = tempfile::NamedTempFile::new().unwrap();
        config.write_all(b"<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context='default'><allow send_destination='*'/><allow receive_sender='*'/><allow own='*'/></policy></busconfig>").unwrap();
        let mut bus = Bus(Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.path().display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap());
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let connection = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let service = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut events = PowerEvents::subscribe(connection).await.unwrap();
        assert!(
            !events.available,
            "A connected bus is not an available UPower service"
        );
        let start = Instant::now();
        let mut schedule = HostRefreshSchedule::new(start);
        assert_eq!(schedule.due(start, true, events.available), (true, false));
        assert_eq!(schedule.power, start + Duration::from_secs(10));
        for available in [true, false, true] {
            if available {
                service
                    .request_name("org.freedesktop.UPower")
                    .await
                    .unwrap();
            } else {
                service
                    .release_name("org.freedesktop.UPower")
                    .await
                    .unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_secs(2), events.next())
                    .await
                    .unwrap()
            );
            assert_eq!(events.available, available);
            assert_eq!(schedule.due(start, true, events.available), (true, false));
            assert_eq!(
                schedule.power,
                start + Duration::from_secs(if available { 60 } else { 10 })
            );
        }
        // More than four queued signals must not backpressure a bus reply.
        // This uses the same draining seam as initial NameHasOwner setup.
        let query_connection = events._connection.clone();
        let burst_then_query = async {
            for _ in 0..32 {
                service
                    .emit_signal(
                        None::<&str>,
                        "/org/freedesktop/UPower/devices/battery_BAT0",
                        "org.freedesktop.DBus.Properties",
                        "PropertiesChanged",
                        &(
                            "org.freedesktop.UPower.Device",
                            std::collections::HashMap::<String, zbus::zvariant::OwnedValue>::new(),
                            Vec::<String>::new(),
                        ),
                    )
                    .await?;
            }
            let proxy = zbus::Proxy::new(
                &query_connection,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )
            .await?;
            proxy
                .call::<_, _, bool>("NameHasOwner", &("org.freedesktop.UPower",))
                .await
        };
        assert!(
            tokio::time::timeout(
                Duration::from_secs(2),
                events.while_draining(burst_then_query)
            )
            .await
            .unwrap()
            .unwrap()
        );
        // Initial subscription while the service owns its name also selects the slow fallback.
        let connection = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        assert!(PowerEvents::subscribe(connection).await.unwrap().available);
    }

    #[test]
    fn power_events_do_not_scan_processes_or_starve_their_deadline() {
        let start = Instant::now();
        let mut schedule = HostRefreshSchedule::new(start);
        for second in 1..10 {
            assert_eq!(
                schedule.due(start + Duration::from_secs(second), true, true),
                (true, false)
            );
            assert_eq!(schedule.next_deadline(), start + Duration::from_secs(10));
        }
        assert_eq!(
            schedule.due(start + Duration::from_secs(10), true, true),
            (true, true)
        );
        assert_eq!(
            schedule.due(start + Duration::from_secs(20), false, true),
            (false, true)
        );
    }

    #[test]
    fn power_polling_slows_only_with_a_healthy_subscription() {
        let start = Instant::now();
        let mut schedule = HostRefreshSchedule::new(start);
        assert_eq!(
            schedule.due(start + Duration::from_secs(10), false, true),
            (true, true)
        );
        for second in [20, 30, 40, 50, 60] {
            assert_eq!(
                schedule.due(start + Duration::from_secs(second), false, true),
                (false, true)
            );
        }
        assert_eq!(
            schedule.due(start + Duration::from_secs(70), false, true),
            (true, true)
        );
        // Disconnection triggers immediate reconciliation and restores the fast fallback.
        assert_eq!(
            schedule.due(start + Duration::from_secs(71), true, false),
            (true, false)
        );
        assert_eq!(
            schedule.due(start + Duration::from_secs(81), false, false),
            (true, true)
        );
    }

    #[test]
    fn vigiland_probe_revalidates_cached_process_and_recovers_after_exit() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("self")).unwrap();
        let process = root.path().join("123");
        std::fs::create_dir(&process).unwrap();
        std::fs::write(process.join("comm"), "vigiland\n").unwrap();
        let mut probe = VigilandProbe::default();
        assert!(probe.read_at(root.path()));
        assert_eq!(probe.process.as_ref(), Some(&process));
        assert!(probe.read_at(root.path()));
        // A reused PID must not leave the indicator active.
        std::fs::write(process.join("comm"), "other\n").unwrap();
        assert!(!probe.read_at(root.path()));
        assert!(probe.process.is_none());
        // Numeric PIDs only, and another instance can replace the cached one.
        let replacement = root.path().join("456");
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(replacement.join("comm"), "vigiland\n").unwrap();
        assert!(probe.read_at(root.path()));
        assert_eq!(probe.process.as_ref(), Some(&replacement));
        std::fs::remove_dir_all(replacement).unwrap();
        let unrelated = root.path().join("not-a-pid");
        std::fs::create_dir(&unrelated).unwrap();
        std::fs::write(unrelated.join("comm"), "vigiland\n").unwrap();
        assert!(!probe.read_at(root.path()));
    }

    #[gpui::test]
    fn audio_percentage_stays_on_one_line_while_revealing(cx: &mut gpui::TestAppContext) {
        use std::cell::RefCell;
        use std::rc::Rc;
        struct Probe {
            reveal: f32,
            bounds: Rc<RefCell<Vec<Bounds<gpui::Pixels>>>>,
        }
        impl Render for Probe {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let bounds = self.bounds.clone();
                div().w(px(80.)).h(px(22.)).flex().items_center().child(
                    audio_volume_label(
                        self.reveal,
                        Some(crate::audio::Volume {
                            percent: 100,
                            muted: false,
                        }),
                        ThemeColors::default(),
                    )
                    .on_children_prepainted(move |children, _, _| *bounds.borrow_mut() = children),
                )
            }
        }
        let bounds = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window(|_, _| Probe {
            reveal: 1.,
            bounds: bounds.clone(),
        });
        let mut full_bounds = None;
        for reveal in [1., 0.02, 0.2, 0.5, 0.8, 1., 0.8, 0.5, 0.2, 0.02] {
            window
                .update(cx, |view, _, cx| {
                    view.reveal = reveal;
                    cx.notify();
                })
                .unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                let _ = window.draw(cx);
            })
            .unwrap();
            let observed = bounds.borrow();
            assert_eq!(observed.len(), 1);
            let full = *full_bounds.get_or_insert(observed[0]);
            assert_eq!(
                observed[0].size, full.size,
                "Number layout changed at reveal {reveal}"
            );
            assert_eq!(
                observed[0].origin.y, full.origin.y,
                "Number baseline moved at reveal {reveal}"
            );
            assert!(
                observed[0].origin.y >= px(0.) && observed[0].bottom() <= px(22.),
                "Volume wrapped at reveal {reveal}: {:?}",
                observed[0]
            );
        }
    }

    #[test]
    fn launcher_button_has_an_input_target_inside_the_left_reserved_edge() {
        let viewport = Size::new(px(1645.), px(1028.));
        let bounds = launcher_button_bounds(viewport);
        assert!(bounds.left() >= px(0.));
        assert!(bounds.right() <= px(LEFT_PANEL_WIDTH as f32));
        assert_eq!(
            bounds.left() + bounds.size.width / 2.,
            px(LEFT_PANEL_WIDTH as f32 / 2. - 1.)
        );
        assert!(bounds.bottom() < viewport.height);
        assert!(bounds.contains(&point(px(32.), px(988.))));
        assert!(!bounds.contains(&point(px(100.), px(988.))));
    }

    #[test]
    fn hovering_a_submenu_leaf_keeps_its_parent_open() {
        assert_eq!(submenu_hover_target(Some(17), true, 18, false), Some(17));
        assert_eq!(submenu_hover_target(Some(17), false, 19, false), None);
        assert_eq!(submenu_hover_target(Some(17), false, 20, true), Some(20));
    }

    #[test]
    fn menu_aim_protects_diagonal_travel_toward_a_left_submenu() {
        let submenu = Bounds {
            origin: point(px(0.), px(0.)),
            size: Size::new(px(200.), px(300.)),
        };
        let previous = point(px(400.), px(150.));
        assert!(menu_aim_contains(
            previous,
            point(px(300.), px(100.)),
            submenu
        ));
        assert!(menu_aim_contains(
            previous,
            point(px(300.), px(200.)),
            submenu
        ));
        assert!(!menu_aim_contains(
            previous,
            point(px(450.), px(150.)),
            submenu
        ));
        assert!(!menu_aim_contains(
            previous,
            point(px(300.), px(350.)),
            submenu
        ));
    }

    #[test]
    fn top_bar_accepts_clicks_at_the_configured_viewports_right_edge() {
        // A 2880-wide output can initially report 1440 logical pixels at integer
        // scale 2, then configure a 1645-wide viewport at fractional scale 1.75.
        let viewport = Size::new(px(1645.), px(1028.));
        let region = top_bar_input_region(viewport);
        assert!(region.contains(&point(px(1600.), px(12.))));
        assert_eq!(region.size.width, viewport.width);
        assert!(!region.contains(&point(px(1600.), px(100.))));
    }

    #[test]
    fn menu_height_accounts_for_compact_separators() {
        let entries = vec![
            TrayMenuItem {
                id: 1,
                label: "Open".into(),
                enabled: true,
                visible: true,
                separator: false,
                checked: false,
                has_submenu: false,
                children: Vec::new(),
            },
            TrayMenuItem {
                id: 2,
                label: String::new(),
                enabled: true,
                visible: true,
                separator: true,
                checked: false,
                has_submenu: false,
                children: Vec::new(),
            },
            TrayMenuItem {
                id: 3,
                label: "Quit".into(),
                enabled: true,
                visible: true,
                separator: false,
                checked: false,
                has_submenu: false,
                children: Vec::new(),
            },
        ];

        assert_eq!(tray_menu_height(&entries), 16. + 30. + 9. + 30.);
        let mut multiline = entries;
        multiline[0].label = "host.example.ts.net\naccount@example.com".into();
        assert_eq!(tray_menu_row_height(&multiline[0]), 48.);
        assert_eq!(tray_menu_height(&multiline), 16. + 48. + 9. + 30.);
    }

    #[test]
    fn primary_click_activates_items_that_are_not_menus() {
        let item = TrayItem {
            key: "1Password_status_icon_1".into(),
            service: ":1.702".into(),
            object_path: "/StatusNotifierItem".into(),
            title: String::new(),
            status: "Active".into(),
            icon_name: String::new(),
            icon_pixmaps: Vec::new(),
            item_is_menu: false,
            menu_path: Some("/com/canonical/dbusmenu".into()),
        };

        assert_eq!(primary_tray_action(&item), PrimaryTrayAction::Activate);
    }
}
