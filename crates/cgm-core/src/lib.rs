//! # cgm-core
//!
//! The platform-agnostic heart of the AiDEX X / GX-01S glucose app. It owns
//! everything that is *not* UI or device I/O:
//!
//! * [`crypto`] — AES-CFB128, MD5, CRC, and the serial→key derivations.
//! * [`protocol`] — DevComm2 framing and the reconnect handshake.
//! * [`glucose`] — decoding broadcast/history records and the validity rules.
//! * [`model`] — the persisted data model (byte-compatible with the web app).
//! * [`store`] — a `Storage` KV abstraction + a `Repository` for load/save,
//!   migration, and the full export/import backup.
//! * [`stats`] — unit conversion, glucose zones, sensor age, time-in-range.
//! * [`health`] — the Apple Health sample feed.
//! * [`engine`] — a `BleBackend` byte pipe and the connect/handshake/sync logic.
//! * [`datetime`] — dependency-free UTC date math + RFC 3339.
//!
//! Web and iOS each implement only a thin `Storage` and `BleBackend`; all the
//! domain logic is shared and tested here.

pub mod crypto;
pub mod datetime;
pub mod engine;
pub mod glucose;
pub mod health;
pub mod model;
pub mod protocol;
pub mod ranges;
pub mod stats;
pub mod store;

// Convenience re-exports for the common surface.
pub use engine::{BleBackend, Connection, EngineError};
pub use glucose::{Broadcast, Reading, SensorState};
pub use model::{Device, DeviceData, DeviceRegistry, Event, Settings, Theme, Unit};
pub use ranges::{Severity, Zone, classify};
pub use store::{Repository, Storage};
