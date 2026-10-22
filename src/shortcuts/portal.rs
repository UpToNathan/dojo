//! `org.freedesktop.impl.portal.GlobalShortcuts` version 2, as xdg-desktop-portal's frontend
//! calls it. Protocol reference:
//! <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.GlobalShortcuts.html>
//!
//! The frontend picks a backend by `.portal` file and routing config; see the install notes in
//! `docs/global-shortcuts.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use tokio::sync::oneshot;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, ObjectServer, interface};

use super::compositor::{Compositor, KeyEvent};
use super::registry::{Choice, Pending, Registry, Requested, View};
use super::store::Store;

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.dojo";
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.impl.portal.GlobalShortcuts";

/// Response codes of the portal Request protocol.
const SUCCESS: u32 = 0;
const CANCELLED: u32 = 1;
const FAILED: u32 = 2;

type Results = HashMap<String, OwnedValue>;
type ShortcutList = Vec<(String, Results)>;

/// What the user is asked to decide.
#[derive(Clone, Debug)]
pub struct ChoiceRequest {
    pub app_id: String,
    pub parent_window: String,
    /// The user opened the panel from the app's configure button, not from a first bind.
    pub reconfigure: bool,
    pub shortcuts: Vec<Pending>,
}

/// Decides which keys a bind request gets. The shell shows a panel; tests and headless use
/// [`PreferredTriggers`].
#[async_trait]
pub trait Approver: Send + Sync + 'static {
    /// `None` means the user cancelled. Dropping the future withdraws the question.
    async fn choose(&self, request: ChoiceRequest) -> Option<Vec<Choice>>;
}

/// Grants each shortcut the key its app suggested, when the suggestion is a usable trigger.
pub struct PreferredTriggers;

#[async_trait]
impl Approver for PreferredTriggers {
    async fn choose(&self, request: ChoiceRequest) -> Option<Vec<Choice>> {
        Some(
            request
                .shortcuts
                .into_iter()
                .map(|pending| Choice {
                    trigger: pending.current.or(pending.preferred),
                    id: pending.id,
                })
                .collect(),
        )
    }
}

pub enum BindOutcome {
    Bound(Vec<View>),
    Cancelled,
    Failed(String),
}

/// The portal's behaviour, independent of D-Bus.
pub struct Service {
    registry: Mutex<Registry>,
    store: Option<Store>,
    compositor: Arc<dyn Compositor>,
    approver: Arc<dyn Approver>,
    /// Grabs reach the compositor in the order they were planned.
    sync_lock: tokio::sync::Mutex<()>,
    runtime: tokio::runtime::Handle,
}

impl Service {
    pub fn new(
        store: Option<Store>,
        compositor: Arc<dyn Compositor>,
        approver: Arc<dyn Approver>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let assignments = match &store {
            Some(store) => {
                let (assignments, problem) = store.load();
                if let Some(problem) = problem {
                    eprintln!("Dojo shortcuts: {problem:#}");
                }
                assignments
            }
            None => Default::default(),
        };
        Self {
            registry: Mutex::new(Registry::new(assignments)),
            store,
            compositor,
            approver,
            sync_lock: tokio::sync::Mutex::new(()),
            runtime,
        }
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry.lock().expect("shortcut registry poisoned")
    }

    pub fn create_session(&self, handle: &str, app_id: &str) -> anyhow::Result<()> {
        self.registry().create_session(handle, app_id)?;
        Ok(())
    }

    pub async fn close_session(&self, handle: &str) {
        if self.registry().close_session(handle) {
            if let Err(error) = self.sync().await {
                eprintln!("Dojo shortcuts: releasing keys failed: {error:#}");
            }
        }
    }

    pub async fn bind(
        &self,
        handle: &str,
        parent_window: &str,
        requested: Vec<Requested>,
        mut cancelled: oneshot::Receiver<()>,
    ) -> BindOutcome {
        let (app_id, pending) = {
            let registry = self.registry();
            let app_id = match registry.app_id(handle) {
                Ok(app_id) => app_id.to_owned(),
                Err(error) => return BindOutcome::Failed(error.to_string()),
            };
            match registry.plan_bind(handle, &requested) {
                Ok(pending) => (app_id, pending),
                Err(error) => return BindOutcome::Failed(error.to_string()),
            }
        };
        let choices = if pending.is_empty() {
            Vec::new()
        } else {
            let request = ChoiceRequest {
                app_id,
                parent_window: parent_window.to_owned(),
                reconfigure: false,
                shortcuts: pending,
            };
            // A closed channel means nobody can cancel, not that someone did.
            let withdrawn = async {
                if (&mut cancelled).await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            let answer = tokio::select! {
                answer = self.approver.choose(request) => answer,
                () = withdrawn => None,
            };
            match answer {
                Some(choices) => choices,
                None => return BindOutcome::Cancelled,
            }
        };
        let changed = match self.registry().commit_bind(handle, &requested, &choices) {
            Ok(changed) => changed,
            Err(error) => return BindOutcome::Failed(error.to_string()),
        };
        if changed {
            self.persist();
        }
        if let Err(error) = self.sync().await {
            return BindOutcome::Failed(format!("{error:#}"));
        }
        match self.registry().list(handle) {
            Ok(views) => BindOutcome::Bound(views),
            Err(error) => BindOutcome::Failed(error.to_string()),
        }
    }

    pub fn list(&self, handle: &str) -> anyhow::Result<Vec<View>> {
        Ok(self.registry().list(handle)?)
    }

    /// Show the panel for a session's shortcuts. Returns the shortcuts as they now stand when the
    /// user changed something, `None` when they cancelled or changed nothing.
    pub async fn configure(
        &self,
        handle: &str,
        parent_window: &str,
    ) -> anyhow::Result<Option<Vec<View>>> {
        let (app_id, pending) = {
            let registry = self.registry();
            (
                registry.app_id(handle)?.to_owned(),
                registry.plan_configure(handle)?,
            )
        };
        if pending.is_empty() {
            return Ok(None);
        }
        let request = ChoiceRequest {
            app_id,
            parent_window: parent_window.to_owned(),
            reconfigure: true,
            shortcuts: pending,
        };
        let Some(choices) = self.approver.choose(request).await else {
            return Ok(None);
        };
        let changed = self.registry().apply_choices(handle, &choices)?;
        if !changed {
            return Ok(None);
        }
        self.persist();
        self.sync().await?;
        Ok(Some(self.registry().list(handle)?))
    }

    /// The session and shortcut a key event belongs to.
    pub fn resolve(&self, event: &KeyEvent) -> Option<(String, String)> {
        self.registry()
            .resolve(&event.grab_id)
            .map(|(handle, id)| (handle.to_owned(), id.to_owned()))
    }

    fn persist(&self) {
        let Some(store) = &self.store else {
            return;
        };
        let assignments = self.registry().assignments().clone();
        if let Err(error) = store.save(&assignments) {
            eprintln!("Dojo shortcuts: saving choices failed: {error:#}");
        }
    }

    /// Give the compositor the current set of keys and record what it refused.
    async fn sync(&self) -> anyhow::Result<()> {
        let _order = self.sync_lock.lock().await;
        let grabs = self.registry().plan_grabs();
        let compositor = self.compositor.clone();
        // Handlers run on zbus's executor, so anything needing tokio's timer or blocking pool is
        // spawned onto our runtime and awaited through its join handle.
        let handle = self.runtime.clone();
        let refused = self
            .runtime
            .spawn(async move {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    handle.spawn_blocking(move || compositor.set_grabs(&grabs)),
                )
                .await
            })
            .await
            .context("shortcut sync stopped")?
            .context("Niri did not answer in time")?
            .context("shortcut sync stopped")??;
        for (grab_id, reason) in &refused {
            eprintln!("Dojo shortcuts: Niri refused {grab_id}: {reason}");
        }
        self.registry().apply_refusals(&refused);
        Ok(())
    }
}

fn string_option(options: &Results, key: &str) -> Option<String> {
    options
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| String::try_from(value).ok())
}

fn owned(text: &str) -> OwnedValue {
    Value::from(text)
        .try_to_owned()
        .expect("a string is never a file descriptor")
}

fn shortcut_list(views: Vec<View>) -> ShortcutList {
    views
        .into_iter()
        .map(|view| {
            let mut properties = Results::new();
            properties.insert("description".into(), owned(&view.description));
            properties.insert(
                "trigger_description".into(),
                owned(
                    &view
                        .trigger
                        .map(|trigger| trigger.description())
                        .unwrap_or_default(),
                ),
            );
            (view.id, properties)
        })
        .collect()
}

fn requested(shortcuts: ShortcutList) -> Vec<Requested> {
    shortcuts
        .into_iter()
        .map(|(id, options)| Requested {
            description: string_option(&options, "description").unwrap_or_default(),
            preferred_trigger: string_option(&options, "preferred_trigger")
                .filter(|trigger| !trigger.is_empty()),
            id,
        })
        .collect()
}

fn failure() -> (u32, Results) {
    (FAILED, Results::new())
}

/// Served at a call's `handle` while its dialog is open, so the frontend can withdraw the call.
struct RequestObject {
    cancel: Mutex<Option<oneshot::Sender<()>>>,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestObject {
    fn close(&self) {
        if let Some(cancel) = self.cancel.lock().expect("request lock poisoned").take() {
            let _ = cancel.send(());
        }
    }
}

struct SessionObject {
    service: Arc<Service>,
    handle: OwnedObjectPath,
}

#[interface(name = "org.freedesktop.impl.portal.Session")]
impl SessionObject {
    async fn close(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        self.service.close_session(self.handle.as_str()).await;
        let _ = Self::closed(&emitter).await;
        let _ = server.remove::<Self, _>(&self.handle).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

struct Backend {
    service: Arc<Service>,
}

#[interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl Backend {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    async fn create_session(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        _options: Results,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        if let Err(error) = self.service.create_session(session_handle.as_str(), &app_id) {
            eprintln!("Dojo shortcuts: CreateSession failed: {error:#}");
            return failure();
        }
        let session = SessionObject {
            service: self.service.clone(),
            handle: session_handle.clone(),
        };
        if let Err(error) = server.at(&session_handle, session).await {
            eprintln!("Dojo shortcuts: exporting the session failed: {error:#}");
            self.service.close_session(session_handle.as_str()).await;
            return failure();
        }
        (SUCCESS, Results::new())
    }

    async fn bind_shortcuts(
        &self,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        shortcuts: ShortcutList,
        parent_window: String,
        _options: Results,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let (cancel, cancelled) = oneshot::channel();
        let exported = server
            .at(
                &handle,
                RequestObject {
                    cancel: Mutex::new(Some(cancel)),
                },
            )
            .await
            .unwrap_or(false);
        let outcome = self
            .service
            .bind(
                session_handle.as_str(),
                &parent_window,
                requested(shortcuts),
                cancelled,
            )
            .await;
        if exported {
            let _ = server.remove::<RequestObject, _>(&handle).await;
        }
        match outcome {
            BindOutcome::Bound(views) => {
                let mut results = Results::new();
                results.insert(
                    "shortcuts".into(),
                    Value::new(shortcut_list(views))
                        .try_to_owned()
                        .expect("shortcut list holds no file descriptors"),
                );
                (SUCCESS, results)
            }
            BindOutcome::Cancelled => (CANCELLED, Results::new()),
            BindOutcome::Failed(reason) => {
                eprintln!("Dojo shortcuts: BindShortcuts failed: {reason}");
                failure()
            }
        }
    }

    async fn list_shortcuts(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
    ) -> (u32, Results) {
        match self.service.list(session_handle.as_str()) {
            Ok(views) => {
                let mut results = Results::new();
                results.insert(
                    "shortcuts".into(),
                    Value::new(shortcut_list(views))
                        .try_to_owned()
                        .expect("shortcut list holds no file descriptors"),
                );
                (SUCCESS, results)
            }
            Err(error) => {
                eprintln!("Dojo shortcuts: ListShortcuts failed: {error:#}");
                failure()
            }
        }
    }

    /// Opens the panel and returns at once: the frontend expects no reply content.
    async fn configure_shortcuts(
        &self,
        session_handle: OwnedObjectPath,
        parent_window: String,
        _options: Results,
        #[zbus(connection)] connection: &Connection,
    ) {
        let service = self.service.clone();
        let connection = connection.clone();
        service.runtime.clone().spawn(async move {
            match service.configure(session_handle.as_str(), &parent_window).await {
                Ok(Some(views)) => {
                    let _ = connection
                        .emit_signal(
                            None::<&str>,
                            OBJECT_PATH,
                            INTERFACE,
                            "ShortcutsChanged",
                            &(&session_handle, shortcut_list(views)),
                        )
                        .await;
                }
                Ok(None) => {}
                Err(error) => eprintln!("Dojo shortcuts: ConfigureShortcuts failed: {error:#}"),
            }
        });
    }
}

/// A running backend. Dropping it stops the service and releases every key.
pub struct Portal {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Portal {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Portal {
    /// Own the backend's bus name on the session bus (or `address`, for tests) and serve it.
    pub fn start(
        address: Option<String>,
        store: Option<Store>,
        compositor: Arc<dyn Compositor>,
        events: async_channel::Receiver<KeyEvent>,
        approver: Arc<dyn Approver>,
    ) -> anyhow::Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("dojo-shortcuts".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(anyhow::Error::new(error)));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let service = Arc::new(Service::new(
                        store,
                        compositor,
                        approver,
                        tokio::runtime::Handle::current(),
                    ));
                    let connection = match connect(address, service.clone()).await {
                        Ok(connection) => {
                            let _ = ready_tx.send(Ok(()));
                            connection
                        }
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    let forward = forward_events(connection.clone(), service.clone(), events);
                    tokio::select! {
                        _ = forward => {}
                        _ = stop_rx => {}
                    }
                    // Free the keys before the bus name goes.
                    service.registry().close_all();
                    let _ = service.sync().await;
                });
                runtime.shutdown_timeout(Duration::from_secs(1));
            })
            .context("starting the shortcut portal")?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                stop: Some(stop_tx),
                thread: Some(thread),
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => anyhow::bail!("the shortcut portal did not start in time"),
        }
    }
}

async fn connect(address: Option<String>, service: Arc<Service>) -> anyhow::Result<Connection> {
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address.as_str())?,
        None => zbus::connection::Builder::session()?,
    };
    builder
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Backend { service })?
        .build()
        .await
        .with_context(|| format!("owning {BUS_NAME}"))
}

/// Turn the compositor's key events into portal signals until Niri goes away.
async fn forward_events(
    connection: Connection,
    service: Arc<Service>,
    events: async_channel::Receiver<KeyEvent>,
) {
    while let Ok(event) = events.recv().await {
        let Some((handle, id)) = service.resolve(&event) else {
            continue;
        };
        let Ok(session) = ObjectPath::try_from(handle.as_str()) else {
            continue;
        };
        let member = if event.pressed {
            "Activated"
        } else {
            "Deactivated"
        };
        let _ = connection
            .emit_signal(
                None::<&str>,
                OBJECT_PATH,
                INTERFACE,
                member,
                &(session, id.as_str(), event.time_ms, Results::new()),
            )
            .await;
    }
    eprintln!("Dojo shortcuts: Niri closed its connection; global shortcuts have stopped");
    // Park: the portal stays up so apps see an error from sync, not a vanished service.
    std::future::pending::<()>().await;
}

#[cfg(test)]
#[path = "portal_tests.rs"]
mod tests;
