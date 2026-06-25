//! User actions and the device-session orchestration. Components call these
//! from event handlers; the heavy lifting (handshake, backfill, polling) lives
//! in `cgm_core::engine`, so this module is just glue between the engine, the
//! reactive state, and the platform services.
//!
//! Signals require a `&mut` handle to write, so each mutating method aliases a
//! local `let mut app = self.app;` — `AppState` is `Copy`, and the copy points
//! at the same reactive slots.

use crate::platform::Clock;
use crate::state::{ActiveSession, AppState, ConnStatus, Live, POLL_INTERVAL_MS};
use cgm_core::datetime::epoch_ms_to_rfc3339;
use cgm_core::engine::{self, BleBackend, Connection};
use cgm_core::glucose::StartTime;
use cgm_core::model::{is_pair_key, DeviceData, Event};
use cgm_core::protocol::new_sensor_payload;
use cgm_core::store::Repository;
use dioxus::prelude::*;

/// A `Copy` handle to every action. Wraps the (also `Copy`) [`AppState`], so it
/// threads freely into multiple event handlers without manual clones.
#[derive(Clone, Copy)]
pub struct Actions {
    app: AppState,
}

/// Read the action handle from context.
pub fn use_actions() -> Actions {
    Actions {
        app: use_context::<AppState>(),
    }
}

fn parse_pairkey(s: &str) -> Option<[u8; 16]> {
    if !is_pair_key(s) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn start_iso(clock: &dyn Clock, st: StartTime) -> String {
    let ms = clock.naive_local_to_epoch_ms(
        st.year as i32,
        st.month as u32,
        st.day as u32,
        st.hour as u32,
        st.min as u32,
        st.sec as u32,
    );
    epoch_ms_to_rfc3339(ms)
}

impl Actions {
    fn log(&self, msg: impl Into<String>) {
        let now = self.app.platform().clock().now_ms();
        self.app.log(now, msg);
    }

    fn persist(&self) {
        self.app.persist();
    }

    /// Show a transient confirmation banner, auto-dismissed after a few seconds.
    pub fn show_toast(&self, msg: impl Into<String>) {
        let mut app = self.app;
        let id = app.toast_gen.read().wrapping_add(1);
        app.toast_gen.set(id);
        app.toast.set(Some(msg.into()));
        let this = *self;
        spawn(async move {
            this.app.platform().clock().sleep(3500).await;
            // Only clear if no newer toast replaced this one.
            if *this.app.toast_gen.read() == id {
                let mut toast = this.app.toast;
                toast.set(None);
            }
        });
    }

    // ── connection ────────────────────────────────────────────────────────────

    /// Connect, handshake, then sync. `activate` triggers the gated sensor-start
    /// after the handshake (used by the add-sensor wizard).
    pub fn connect(&self, activate: bool) {
        let this = *self;
        spawn(async move { this.connect_inner(activate).await });
    }

    async fn connect_inner(&self, activate: bool) {
        let mut app = self.app;
        let platform = app.platform();
        if !platform.ble().available() {
            self.log("Bluetooth unavailable — use Chrome/Edge over https or localhost.");
            return;
        }
        let Some(dev) = app.registry.read().active().cloned() else {
            self.log("No sensor selected — add one first.");
            return;
        };
        let Some(pair_key) = parse_pairkey(&dev.pairkey) else {
            self.log("This sensor isn't paired yet — pair it first.");
            return;
        };

        app.status.set(ConnStatus::Requesting);
        let mut backend = match platform.ble().connect().await {
            Ok(b) => b,
            Err(e) => {
                app.status.set(ConnStatus::Error(e.clone()));
                self.log(format!("connect failed: {e}"));
                return;
            }
        };

        app.status.set(ConnStatus::Connecting);
        let conn = match engine::handshake(&mut *backend, &dev.serial, &pair_key).await {
            Ok(c) => c,
            Err(e) => {
                app.status.set(ConnStatus::Error(e.to_string()));
                self.log(format!("handshake failed: {e}"));
                return;
            }
        };
        self.log("handshake ok — session established");
        app.status.set(ConnStatus::Connected);

        let generation = app.poll_gen.read().wrapping_add(1);
        app.poll_gen.set(generation);
        app.session()
            .set(ActiveSession::new(backend, conn, generation));

        if activate {
            self.activate_inner().await;
        }
        self.sync_inner().await;
        self.start_poll_loop(generation);
    }

    /// Manually re-sync stored history.
    pub fn sync(&self) {
        let this = *self;
        spawn(async move { this.sync_inner().await });
    }

    async fn sync_inner(&self) {
        let this = *self;
        self.app
            .session()
            .with(|mut s| async move {
                let ActiveSession { backend, conn, .. } = &mut s;
                this.do_sync(backend.as_mut(), conn).await;
                (Some(s), ())
            })
            .await;
    }

    async fn do_sync(&self, backend: &mut dyn BleBackend, conn: &Connection) {
        let mut app = self.app;
        app.status.set(ConnStatus::Syncing);
        let st = engine::start_time(backend, conn).await;
        let rng = engine::history_range(backend, conn).await;
        let (Some(st), Some((min, max))) = (st, rng) else {
            self.log("sync: could not read start time / range");
            app.status.set(ConnStatus::Connected);
            return;
        };
        let iso = start_iso(app.platform().clock(), st);

        let cur_iso = app.data.read().sensor_start_iso.clone();
        let new_session = cur_iso.as_deref() != Some(iso.as_str());
        let mut records = app.data.read().records.clone();
        if new_session {
            records.clear();
            app.chart_view.set(None); // resume following latest on a fresh session
            self.log("new sensor session detected — resetting stored data");
        }

        let added = engine::backfill(backend, conn, &mut records, min, max).await;
        {
            let mut d = app.data.write();
            d.sensor_start_iso = Some(iso);
            d.records = records;
            if new_session {
                d.events.clear();
            }
        }
        self.persist();
        self.poll_once(backend, conn).await;
        let total = app.data.read().records.len();
        self.log(format!("sync complete — {total} readings stored ({added} new)"));
        app.status.set(ConnStatus::Connected);
    }

    /// One broadcast poll: refresh live telemetry and merge the newest readings.
    /// Returns whether the device responded.
    async fn poll_once(&self, backend: &mut dyn BleBackend, conn: &Connection) -> bool {
        let mut app = self.app;
        let Some(bc) = engine::broadcast(backend, conn).await else {
            return false;
        };
        app.live.set(Live {
            trend: Some(bc.trend_mgdl_min),
            state: Some(bc.state()),
            time_offset: Some(bc.time_offset_min),
            quality: bc.current().map(|r| r.quality),
            status_byte: Some(bc.status),
        });
        let mut records = app.data.read().records.clone();
        engine::merge_broadcast(&mut records, &bc);
        app.data.write().records = records;
        self.persist();
        true
    }

    fn start_poll_loop(&self, generation: u64) {
        let this = *self;
        spawn(async move {
            let mut failures = 0;
            loop {
                this.app.platform().clock().sleep(POLL_INTERVAL_MS).await;
                if this.app.session().generation() != Some(generation) {
                    break; // disconnected or superseded
                }
                let responded = this
                    .app
                    .session()
                    .with(|mut s| async move {
                        let ActiveSession { backend, conn, .. } = &mut s;
                        let ok = this.poll_once(backend.as_mut(), conn).await;
                        (Some(s), ok)
                    })
                    .await
                    .unwrap_or(false);
                if responded {
                    failures = 0;
                } else {
                    failures += 1;
                    if failures >= 2 {
                        this.log("connection lost — disconnecting");
                        this.disconnect();
                        break;
                    }
                }
            }
        });
    }

    /// Irreversible sensor activation (gated in the engine to NEW/USED only).
    async fn activate_inner(&self) {
        let this = *self;
        self.app
            .session()
            .with(|mut s| async move {
                let ActiveSession { backend, conn, .. } = &mut s;
                let payload = new_sensor_payload(this.app.platform().clock().new_sensor_time());
                match engine::activate_sensor(backend.as_mut(), conn, &payload).await {
                    Ok(()) => this.log("✓ new sensor started — ~60-min warmup begins"),
                    Err(e) => this.log(format!("not starting: {e}")),
                }
                (Some(s), ())
            })
            .await;
    }

    pub fn disconnect(&self) {
        let mut app = self.app;
        // Bump the generation so the poll loop self-cancels, drop the session.
        let g = app.poll_gen.read().wrapping_add(1);
        app.poll_gen.set(g);
        app.platform().ble().disconnect();
        app.session().clear();
        app.live.set(Live::default());
        app.status.set(ConnStatus::Disconnected);
        self.log("disconnected");
    }

    // ── pairing (driven by the add-sensor wizard) ──────────────────────────────

    /// Pair a fresh transmitter and return its 32-hex pair key.
    pub async fn pair(&self, serial: String) -> Result<String, String> {
        let secret = cgm_core::crypto::derive_pair_secret(&serial)
            .ok_or_else(|| "invalid serial number".to_string())?;
        self.app.platform().ble().pair(secret).await
    }

    // ── device registry ─────────────────────────────────────────────────────────

    /// Add or update a device by serial, set it active, and load its data.
    /// Returns the device id.
    pub fn upsert_device(&self, name: String, serial: String, pairkey: String) -> String {
        let mut app = self.app;
        let fresh_id = app.platform().new_id();
        let id = {
            let mut reg = app.registry.write();
            if let Some(existing) = reg.find_by_serial(&serial).map(|d| d.id.clone()) {
                let dev = reg.find_mut(&existing).unwrap();
                if !name.is_empty() {
                    dev.name = name;
                }
                if !pairkey.is_empty() {
                    dev.pairkey = pairkey;
                }
                reg.active_id = Some(existing.clone());
                existing
            } else {
                let id = fresh_id;
                reg.list.push(cgm_core::model::Device {
                    id: id.clone(),
                    name: if name.is_empty() { serial.clone() } else { name },
                    serial,
                    pairkey,
                });
                reg.active_id = Some(id.clone());
                id
            }
        };
        self.reload_active_data();
        self.persist();
        id
    }

    pub fn set_active_device(&self, id: String) {
        let mut app = self.app;
        if app.session().is_active() {
            self.disconnect();
        }
        app.registry.write().active_id = Some(id);
        self.reload_active_data();
        self.persist();
    }

    pub fn rename_device(&self, id: &str, name: String) {
        let mut app = self.app;
        if let Some(d) = app.registry.write().find_mut(id) {
            d.name = if name.trim().is_empty() {
                d.serial.clone()
            } else {
                name
            };
        }
        self.persist();
    }

    pub fn delete_device(&self, id: &str) {
        let mut app = self.app;
        let repo = Repository::new(app.platform().storage());
        repo.remove_data(id);
        {
            let mut reg = app.registry.write();
            reg.list.retain(|d| d.id != id);
            if reg.active_id.as_deref() == Some(id) {
                reg.active_id = reg.list.first().map(|d| d.id.clone());
            }
        }
        self.reload_active_data();
        self.persist();
    }

    fn reload_active_data(&self) {
        let mut app = self.app;
        let repo = Repository::new(app.platform().storage());
        let id = app.registry.read().active_id.clone();
        let data = repo.load_data(id.as_deref());
        app.data.set(data);
        // A new dataset invalidates any panned window — resume following latest.
        app.chart_view.set(None);
    }

    // ── settings ──────────────────────────────────────────────────────────────────

    pub fn toggle_unit(&self) {
        let mut app = self.app;
        let u = app.settings.read().unit.toggled();
        app.settings.write().unit = u;
        self.persist();
    }

    pub fn toggle_theme(&self) {
        let mut app = self.app;
        let t = app.settings.read().theme.toggled();
        app.settings.write().theme = t;
        self.persist();
    }

    // ── events ──────────────────────────────────────────────────────────────────

    pub fn add_event(&self, t_ms: i64, label: String, icon: Option<String>) {
        let mut app = self.app;
        app.data.write().events.push(Event {
            t: t_ms,
            label: label.clone(),
            icon,
        });
        self.persist();
        self.log(format!("logged {label}"));
    }

    pub fn remove_event_near(&self, t_ms: i64) {
        let mut app = self.app;
        let idx = {
            let d = app.data.read();
            d.events
                .iter()
                .enumerate()
                .filter(|(_, e)| (e.t - t_ms).abs() < 15 * 60_000)
                .min_by_key(|(_, e)| (e.t - t_ms).abs())
                .map(|(i, _)| i)
        };
        if let Some(i) = idx {
            app.data.write().events.remove(i);
            self.persist();
        }
    }

    pub fn clear_data(&self) {
        let mut app = self.app;
        app.data.set(DeviceData::default());
        app.chart_view.set(None);
        self.persist();
        self.log("data cleared");
        self.show_toast("Data cleared");
    }

    // ── backup + health ─────────────────────────────────────────────────────────

    pub fn export_backup(&self) {
        let app = self.app;
        let platform = app.platform();
        let repo = Repository::new(platform.storage());
        let now = platform.clock().now_ms();
        let exported_at = epoch_ms_to_rfc3339(now);
        let json = repo.export_json(
            &app.settings.read(),
            &app.registry.read(),
            exported_at.clone(),
        );
        let filename = format!("aidex-backup-{}.json", &exported_at[..10]);
        platform
            .files()
            .download(&filename, "application/json", &json);
        let n = app.registry.read().list.len();
        self.log(format!("exported {n} sensor(s)"));
        self.show_toast(format!("Exported {n} sensor(s)"));
    }

    pub fn import_backup(&self) {
        let this = *self;
        spawn(async move {
            let mut app = this.app;
            let platform = app.platform();
            let Some(text) = platform.files().pick_text().await else {
                return;
            };
            let repo = Repository::new(platform.storage());
            let mut settings = *app.settings.read();
            let mut registry = app.registry.read().clone();
            match repo.import_json(&text, &mut settings, &mut registry) {
                Ok(summary) => {
                    app.settings.set(settings);
                    app.registry.set(registry);
                    this.reload_active_data();
                    this.persist();
                    this.log(format!("imported {} sensor(s)", summary.sensors));
                    this.show_toast(format!("Imported {} sensor(s)", summary.sensors));
                }
                Err(e) => {
                    this.log(format!("import failed: {e}"));
                    this.show_toast(format!("Import failed: {e}"));
                }
            }
        });
    }

    pub fn export_health(&self) {
        let app = self.app;
        let json = cgm_core::health::health_json(&app.data.read());
        let samples = cgm_core::health::health_samples(&app.data.read()).len();
        let this = *self;
        spawn(async move {
            match this
                .app
                .platform()
                .files()
                .export_health(json, samples)
                .await
            {
                Ok(msg) => {
                    this.log(msg.clone());
                    this.show_toast(msg);
                }
                Err(e) => {
                    this.log(format!("health export failed: {e}"));
                    this.show_toast(format!("Health export failed: {e}"));
                }
            }
        });
    }
}
