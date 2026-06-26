//! Platform-service traits the UI depends on. The web backend provides one
//! concrete `Platform`; the UI never touches `web-sys` or any device API
//! directly. Async methods return boxed *local* futures so the trait objects
//! stay object-safe on the single-threaded web target.

use cgm_core::engine::{BleBackend, LocalFuture};
use cgm_core::protocol::NewSensorTime;
use cgm_core::store::Storage;
use std::rc::Rc;

/// Wall-clock and timezone services — the core deliberately has no clock.
pub trait Clock {
    /// Current time in epoch milliseconds.
    fn now_ms(&self) -> i64;

    /// Current local UTC offset in minutes (east-positive), for display.
    fn local_offset_minutes(&self) -> i32;

    /// Interpret device-local wall-clock fields as an instant (epoch ms). Used
    /// to turn the sensor's reported start time into the stored ISO string the
    /// same way the web app's `new Date(y,mo,d,…)` did.
    fn naive_local_to_epoch_ms(&self, y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64;

    /// The `NewSensor` datetime payload fields for "now" (local time + tz/DST).
    fn new_sensor_time(&self) -> NewSensorTime;

    /// Sleep for `ms` milliseconds (drives the live-poll timer).
    fn sleep(&self, ms: u32) -> LocalFuture<'static, ()>;
}

/// BLE lifecycle: pairing and opening an authenticated transport. The platform
/// owns the device chooser and any reuse of a just-paired device.
pub trait Ble {
    /// Whether Web Bluetooth is usable at all.
    fn available(&self) -> bool;

    /// First-time pairing on a *fresh* transmitter: write `secret` to `F001` and
    /// return the device-issued 32-hex pair key. Shows the device chooser.
    fn pair(&self, secret: [u8; 16]) -> LocalFuture<'static, Result<String, String>>;

    /// Open a GATT connection (chooser if needed) to a device advertising the
    /// CGM service, subscribe to `F002`/`F003`, and return a backend ready for
    /// the reconnect handshake.
    fn connect(&self) -> LocalFuture<'static, Result<Box<dyn BleBackend>, String>>;

    /// Tear down any open connection.
    fn disconnect(&self);
}

/// File import/export and Apple Health.
pub trait Files {
    /// Save `contents` as a file (browser download).
    fn download(&self, filename: &str, mime: &str, contents: &str);

    /// Pick a file and return its UTF-8 contents, or `None` if cancelled.
    fn pick_text(&self) -> LocalFuture<'static, Option<String>>;

    /// Export Apple Health samples by downloading the JSON the built-in
    /// Shortcuts recipe logs as Blood Glucose. Returns a human-readable result.
    fn export_health(&self, json: String, samples: usize)
    -> LocalFuture<'static, Result<String, String>>;
}

/// The umbrella a backend implements and provides to the UI via context.
pub trait Platform {
    fn storage(&self) -> Rc<dyn Storage>;
    fn clock(&self) -> &dyn Clock;
    fn ble(&self) -> &dyn Ble;
    fn files(&self) -> &dyn Files;

    /// Mint a unique device id (UUID v4 or equivalent).
    fn new_id(&self) -> String;

    /// One-line capabilities note for the diagnostics panel, e.g.
    /// "Web Bluetooth (Chrome/Edge)".
    fn label(&self) -> String;
}

/// Convenient handle stored in Dioxus context.
pub type SharedPlatform = Rc<dyn Platform>;
