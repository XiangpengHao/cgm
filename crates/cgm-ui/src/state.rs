//! Reactive application state (Dioxus signals) plus the live BLE session handle.
//!
//! `AppState` is `Copy` (every field is a `Signal`), so it is provided once via
//! context and freely captured into event handlers and spawned tasks.

use crate::platform::SharedPlatform;
use cgm_core::engine::{BleBackend, Connection};
use cgm_core::glucose::SensorState;
use cgm_core::model::{DeviceData, DeviceRegistry, Settings};
use cgm_core::store::Repository;
use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Default visible chart window.
pub const DEFAULT_WINDOW_HOURS: u32 = 12;
/// Live-poll cadence (matches the device's per-minute records).
pub const POLL_INTERVAL_MS: u32 = 60_000;
/// Maximum diagnostic log lines retained.
pub const MAX_LOG_LINES: usize = 200;

/// Live telemetry from the most recent broadcast poll.
#[derive(Clone, PartialEq, Default)]
pub struct Live {
    pub trend: Option<i8>,
    pub state: Option<SensorState>,
    pub time_offset: Option<i32>,
    pub quality: Option<u8>,
    pub status_byte: Option<u8>,
}

/// Connection lifecycle.
#[derive(Clone, PartialEq)]
pub enum ConnStatus {
    Disconnected,
    Requesting,
    Connecting,
    Connected,
    Syncing,
    Error(String),
}

impl ConnStatus {
    /// Short human label.
    pub fn label(&self) -> String {
        match self {
            ConnStatus::Disconnected => "disconnected".into(),
            ConnStatus::Requesting => "requesting device…".into(),
            ConnStatus::Connecting => "connecting…".into(),
            ConnStatus::Connected => "connected".into(),
            ConnStatus::Syncing => "syncing…".into(),
            ConnStatus::Error(e) => format!("error: {e}"),
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, ConnStatus::Connected | ConnStatus::Syncing)
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            ConnStatus::Requesting | ConnStatus::Connecting | ConnStatus::Syncing
        )
    }
}

/// One diagnostic log line.
#[derive(Clone, PartialEq)]
pub struct LogLine {
    pub t_ms: i64,
    pub msg: String,
}

/// Draft state for the add-event popover, anchored at a chart instant.
#[derive(Clone, PartialEq)]
pub struct EventDraft {
    /// Instant the event would be logged at (epoch ms).
    pub t_ms: i64,
    /// Screen position for the popover (viewport px), used when `anchored`.
    pub x: f64,
    pub y: f64,
    /// `true` when anchored to a chart tap; `false` opens the popover centered
    /// (e.g. from the "Log event" button — the touch-friendly path).
    pub anchored: bool,
}

/// A modal confirm/prompt request, rendered in-app rather than via native
/// `window.confirm`/`prompt` (unreliable in some mobile browsers), so dialogs
/// look and behave consistently.
#[derive(Clone, PartialEq)]
pub enum Dialog {
    Confirm {
        message: String,
        confirm_label: String,
        kind: ConfirmKind,
    },
    Prompt {
        message: String,
        value: String,
        kind: PromptKind,
    },
}

#[derive(Clone, PartialEq)]
pub enum ConfirmKind {
    DeleteDevice(String),
    ClearData,
}

#[derive(Clone, PartialEq)]
pub enum PromptKind {
    RenameDevice(String),
}

/// The live, authenticated BLE session — owned outside the signal graph so its
/// `&mut` can cross await points. Operations *take* it out for their duration
/// (see [`SessionCell::with`]); concurrent attempts see `None` and bail, which
/// gives the same serialized behaviour as the web app.
pub struct ActiveSession {
    pub backend: Box<dyn BleBackend>,
    pub conn: Connection,
    /// Tied to [`AppState::poll_gen`] so a stale poll loop self-cancels.
    pub generation: u64,
}

impl ActiveSession {
    pub fn new(backend: Box<dyn BleBackend>, conn: Connection, generation: u64) -> Self {
        ActiveSession {
            backend,
            conn,
            generation,
        }
    }
}

/// Shared, interior-mutable holder for the live session.
#[derive(Clone, Default)]
pub struct SessionCell(Rc<RefCell<Option<ActiveSession>>>);

impl SessionCell {
    pub fn set(&self, session: ActiveSession) {
        *self.0.borrow_mut() = Some(session);
    }

    pub fn clear(&self) {
        *self.0.borrow_mut() = None;
    }

    pub fn is_active(&self) -> bool {
        self.0.borrow().is_some()
    }

    pub fn generation(&self) -> Option<u64> {
        self.0.borrow().as_ref().map(|s| s.generation)
    }

    /// Take the session, run `f` with exclusive access across awaits, then put
    /// it back. Returns `None` if no session is present (or it was taken by a
    /// concurrent operation). The closure returns the (possibly updated)
    /// session — yielding `None` drops it, i.e. disconnects.
    pub async fn with<F, Fut, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(ActiveSession) -> Fut,
        Fut: std::future::Future<Output = (Option<ActiveSession>, T)>,
    {
        let session = self.0.borrow_mut().take()?;
        let (session, out) = f(session).await;
        *self.0.borrow_mut() = session;
        Some(out)
    }
}

/// All reactive state for the app. `Copy`, so it threads freely through event
/// handlers and tasks. The platform handle and live session live here too (as
/// signals) so the whole bundle stays `Copy`.
#[derive(Clone, Copy)]
pub struct AppState {
    pub settings: Signal<Settings>,
    pub registry: Signal<DeviceRegistry>,
    pub data: Signal<DeviceData>,
    pub live: Signal<Live>,
    pub status: Signal<ConnStatus>,
    pub logs: Signal<Vec<LogLine>>,
    pub window_hours: Signal<u32>,
    pub poll_gen: Signal<u64>,
    pub chart_width: Signal<f64>,
    /// Explicit visible window `[start, end]` ms when the user has panned;
    /// `None` means follow the latest reading at `window_hours`.
    pub chart_view: Signal<Option<(i64, i64)>>,
    pub event_draft: Signal<Option<EventDraft>>,
    pub dialog: Signal<Option<Dialog>>,
    /// Transient confirmation banner text (auto-dismissed).
    pub toast: Signal<Option<String>>,
    /// Monotonic id so an auto-dismiss timer only clears its own toast.
    pub toast_gen: Signal<u64>,
    pub show_devices: Signal<bool>,
    pub show_health: Signal<bool>,
    pub show_settings: Signal<bool>,
    pub show_diagnostics: Signal<bool>,
    platform: Signal<SharedPlatform>,
    session: Signal<SessionCell>,
}

impl AppState {
    /// Build the initial state from persisted storage. Must be called inside a
    /// component scope (it allocates signals).
    pub fn load(platform: SharedPlatform) -> Self {
        let repo = Repository::new(platform.storage());
        let settings = repo.load_settings();
        let registry = repo.load_registry(|| platform.new_id());
        let data = repo.load_data(registry.active_id.as_deref());

        AppState {
            settings: Signal::new(settings),
            registry: Signal::new(registry),
            data: Signal::new(data),
            live: Signal::new(Live::default()),
            status: Signal::new(ConnStatus::Disconnected),
            logs: Signal::new(Vec::new()),
            window_hours: Signal::new(DEFAULT_WINDOW_HOURS),
            poll_gen: Signal::new(0),
            chart_width: Signal::new(960.0),
            chart_view: Signal::new(None),
            event_draft: Signal::new(None),
            dialog: Signal::new(None),
            toast: Signal::new(None),
            toast_gen: Signal::new(0),
            show_devices: Signal::new(false),
            show_health: Signal::new(false),
            show_settings: Signal::new(false),
            show_diagnostics: Signal::new(false),
            platform: Signal::new(platform),
            session: Signal::new(SessionCell::default()),
        }
    }

    /// The platform services handle.
    pub fn platform(&self) -> SharedPlatform {
        self.platform.read().clone()
    }

    /// The shared live-session holder.
    pub fn session(&self) -> SessionCell {
        self.session.read().clone()
    }

    /// Append a diagnostic log line (newest first, capped).
    pub fn log(self, t_ms: i64, msg: impl Into<String>) {
        let mut logs = self.logs;
        let mut g = logs.write();
        g.insert(0, LogLine { t_ms, msg: msg.into() });
        g.truncate(MAX_LOG_LINES);
    }

    /// Persist settings, registry, and the active device's data.
    pub fn persist(&self) {
        let repo = Repository::new(self.platform().storage());
        repo.save_settings(&self.settings.read());
        repo.save_registry(&self.registry.read());
        if let Some(id) = self.registry.read().active_id.clone() {
            repo.save_data(&id, &self.data.read());
        }
    }
}
