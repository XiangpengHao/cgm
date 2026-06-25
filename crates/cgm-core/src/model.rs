//! The persisted data model. Field names and shapes match the existing web
//! app's localStorage / export format **exactly**, so data round-trips between
//! the old `index.html` and these apps.

use crate::datetime::rfc3339_to_epoch_ms;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Glucose display unit.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    #[default]
    Mgdl,
    Mmol,
}

impl Unit {
    pub fn is_mgdl(self) -> bool {
        matches!(self, Unit::Mgdl)
    }
    pub fn label(self) -> &'static str {
        if self.is_mgdl() { "mg/dL" } else { "mmol/L" }
    }
    pub fn toggled(self) -> Unit {
        if self.is_mgdl() { Unit::Mmol } else { Unit::Mgdl }
    }
}

/// UI theme.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
    pub fn toggled(self) -> Theme {
        match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }
}

/// User settings (`cgm.settings`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Settings {
    #[serde(default)]
    pub unit: Unit,
    #[serde(default)]
    pub theme: Theme,
}

/// A registered transmitter. `pairkey` is the 32-hex device-issued pair key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Device {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub pairkey: String,
}

impl Device {
    /// Display name, falling back to serial then a generic label.
    pub fn display_name(&self) -> &str {
        if !self.name.is_empty() {
            &self.name
        } else if !self.serial.is_empty() {
            &self.serial
        } else {
            "sensor"
        }
    }

    pub fn is_paired(&self) -> bool {
        is_pair_key(&self.pairkey)
    }
}

/// Whether a string is a valid 32-hex-character pair key.
pub fn is_pair_key(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The device registry (`cgm.devices`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct DeviceRegistry {
    #[serde(rename = "activeId")]
    pub active_id: Option<String>,
    #[serde(default)]
    pub list: Vec<Device>,
}

impl DeviceRegistry {
    pub fn active(&self) -> Option<&Device> {
        let id = self.active_id.as_deref()?;
        self.list.iter().find(|d| d.id == id)
    }

    pub fn find(&self, id: &str) -> Option<&Device> {
        self.list.iter().find(|d| d.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Device> {
        self.list.iter_mut().find(|d| d.id == id)
    }

    pub fn find_by_serial(&self, serial: &str) -> Option<&Device> {
        self.list.iter().find(|d| d.serial == serial)
    }
}

/// A logged event marker (coffee, meal, …) at a wall-clock instant.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Epoch milliseconds.
    pub t: i64,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// One stored glucose record: `(mgdl, rec_status)`, serialized as a 2-element
/// array to match the web app (`[mgdl, recStatus]`).
pub type Record = (u16, u8);

/// Per-device data (`cgm.data.<id>`). `records` is keyed by sensor-minute index.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct DeviceData {
    #[serde(rename = "sensorStartIso")]
    pub sensor_start_iso: Option<String>,
    #[serde(default)]
    pub records: BTreeMap<u32, Record>,
    #[serde(default)]
    pub events: Vec<Event>,
}

impl DeviceData {
    /// Sensor start as epoch milliseconds, if known and parseable.
    pub fn sensor_start_ms(&self) -> Option<i64> {
        self.sensor_start_iso
            .as_deref()
            .and_then(rfc3339_to_epoch_ms)
    }

    /// Epoch milliseconds for a record index (`sensor_start + index minutes`).
    pub fn record_time_ms(&self, index: u32) -> Option<i64> {
        Some(self.sensor_start_ms()? + index as i64 * 60_000)
    }

    /// The highest stored index, if any.
    pub fn latest_index(&self) -> Option<u32> {
        self.records.keys().next_back().copied()
    }

    /// The most recent record `(index, mgdl, rec_status)`.
    pub fn latest(&self) -> Option<(u32, u16, u8)> {
        self.records
            .iter()
            .next_back()
            .map(|(&i, &(g, s))| (i, g, s))
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_serialize_lowercase() {
        let s = Settings {
            unit: Unit::Mmol,
            theme: Theme::Dark,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"unit":"mmol","theme":"dark"}"#);
    }

    #[test]
    fn device_data_uses_string_keys_and_array_records() {
        let mut d = DeviceData::default();
        d.sensor_start_iso = Some("2026-06-25T00:00:00.000Z".into());
        d.records.insert(60, (100, 0));
        d.records.insert(61, (102, 0));
        d.events.push(Event {
            t: 1_782_758_245_123,
            label: "coffee".into(),
            icon: Some("☕".into()),
        });
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""60":[100,0]"#), "{json}");
        assert!(json.contains(r#""sensorStartIso":"2026-06-25T00:00:00.000Z""#));
        assert!(json.contains(r#""label":"coffee""#));
        // round-trip
        let back: DeviceData = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn pair_key_validation() {
        assert!(is_pair_key("0123456789ABCDEF0123456789ABCDEF"));
        assert!(!is_pair_key("0123"));
        assert!(!is_pair_key("0123456789ABCDEF0123456789ABCDEZ"));
    }

    #[test]
    fn latest_helpers() {
        let mut d = DeviceData::default();
        d.records.insert(5, (90, 0));
        d.records.insert(10, (110, 0));
        d.records.insert(7, (95, 0));
        assert_eq!(d.latest_index(), Some(10));
        assert_eq!(d.latest(), Some((10, 110, 0)));
    }
}
