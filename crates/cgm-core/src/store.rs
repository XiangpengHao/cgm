//! Persistence: a tiny string key/value `Storage` abstraction (localStorage on
//! web) plus a `Repository` that reads and writes
//! the registry, per-device data and settings, and handles the full
//! export/import backup. Everything here is byte-compatible with the existing
//! web app, including the one-time migration from the old single-device layout.

use crate::model::{Device, DeviceData, DeviceRegistry, Settings, Theme, Unit};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const KEY_SETTINGS: &str = "cgm.settings";
pub const KEY_DEVICES: &str = "cgm.devices";
pub const KEY_LEGACY_DATA: &str = "cgm.data";

/// Per-device data key: `cgm.data.<id>`.
pub fn data_key(id: &str) -> String {
    format!("cgm.data.{id}")
}

/// A minimal string key/value store. The implementations are platform-specific;
/// the contract matches the browser `localStorage` API.
pub trait Storage {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str);
    fn remove(&self, key: &str);
}

// Blanket impls so the UI can hold an `Rc<dyn Storage>` / `Box<dyn Storage>`
// (single-threaded web) and still build a `Repository` over it.
impl<S: Storage + ?Sized> Storage for std::rc::Rc<S> {
    fn get(&self, key: &str) -> Option<String> {
        (**self).get(key)
    }
    fn set(&self, key: &str, value: &str) {
        (**self).set(key, value);
    }
    fn remove(&self, key: &str) {
        (**self).remove(key);
    }
}

impl<S: Storage + ?Sized> Storage for Box<S> {
    fn get(&self, key: &str) -> Option<String> {
        (**self).get(key)
    }
    fn set(&self, key: &str, value: &str) {
        (**self).set(key, value);
    }
    fn remove(&self, key: &str) {
        (**self).remove(key);
    }
}

/// One backup file: every sensor + its data + settings, as a single JSON blob
/// for moving between browsers/devices. Schema matches the web app's `exportAll`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Backup {
    pub app: String,
    pub version: u32,
    #[serde(rename = "exportedAt")]
    pub exported_at: String,
    pub settings: Settings,
    pub devices: DeviceRegistry,
    pub data: BTreeMap<String, DeviceData>,
}

pub const BACKUP_APP: &str = "aidex-cgm";
pub const BACKUP_VERSION: u32 = 1;

/// Outcome of importing a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    pub sensors: usize,
}

/// Reads and writes the app's persisted state via a [`Storage`] backend.
pub struct Repository<S: Storage> {
    storage: S,
}

impl<S: Storage> Repository<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    // ── settings ────────────────────────────────────────────────────────────

    /// Load settings, tolerating missing/garbage values (defaults: mg/dL, light)
    /// exactly like the web app.
    pub fn load_settings(&self) -> Settings {
        let v: Option<Value> = self
            .storage
            .get(KEY_SETTINGS)
            .and_then(|s| serde_json::from_str(&s).ok());
        let unit = match v.as_ref().and_then(|v| v.get("unit")).and_then(Value::as_str) {
            Some("mmol") => Unit::Mmol,
            _ => Unit::Mgdl,
        };
        let theme = match v
            .as_ref()
            .and_then(|v| v.get("theme"))
            .and_then(Value::as_str)
        {
            Some("dark") => Theme::Dark,
            _ => Theme::Light,
        };
        Settings { unit, theme }
    }

    pub fn save_settings(&self, settings: &Settings) {
        if let Ok(s) = serde_json::to_string(settings) {
            self.storage.set(KEY_SETTINGS, &s);
        }
    }

    // ── device registry ───────────────────────────────────────────────────────

    /// Load the device registry, performing the one-time migration from the old
    /// single-device layout (serial/pairkey in `cgm.settings`, data in
    /// `cgm.data`) when no registry exists yet. `new_id` mints a fresh device id.
    pub fn load_registry(&self, new_id: impl FnOnce() -> String) -> DeviceRegistry {
        // A valid registry must be an object with a `list` array.
        let parsed: Option<DeviceRegistry> = self
            .storage
            .get(KEY_DEVICES)
            .and_then(|s| serde_json::from_str(&s).ok());
        if let Some(reg) = parsed {
            return reg;
        }

        let mut reg = DeviceRegistry::default();
        // Migrate a legacy single device whose serial/pairkey lived in settings.
        if let Some(legacy) = self
            .storage
            .get(KEY_SETTINGS)
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        {
            let serial = legacy.get("serial").and_then(Value::as_str).unwrap_or("");
            let pairkey = legacy.get("pairkey").and_then(Value::as_str).unwrap_or("");
            if !serial.is_empty() || !pairkey.is_empty() {
                let id = new_id();
                reg.list.push(Device {
                    id: id.clone(),
                    name: if serial.is_empty() {
                        "Device 1".into()
                    } else {
                        serial.into()
                    },
                    serial: serial.into(),
                    pairkey: pairkey.into(),
                });
                reg.active_id = Some(id.clone());
                if let Some(old) = self.storage.get(KEY_LEGACY_DATA) {
                    self.storage.set(&data_key(&id), &old);
                }
            }
        }
        self.save_registry(&reg);
        reg
    }

    pub fn save_registry(&self, reg: &DeviceRegistry) {
        if let Ok(s) = serde_json::to_string(reg) {
            self.storage.set(KEY_DEVICES, &s);
        }
    }

    // ── per-device data ───────────────────────────────────────────────────────

    /// Load a device's data, defaulting to an empty dataset on missing/garbage.
    pub fn load_data(&self, id: Option<&str>) -> DeviceData {
        id.and_then(|id| self.storage.get(&data_key(id)))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_data(&self, id: &str, data: &DeviceData) {
        if let Ok(s) = serde_json::to_string(data) {
            self.storage.set(&data_key(id), &s);
        }
    }

    pub fn remove_data(&self, id: &str) {
        self.storage.remove(&data_key(id));
    }

    // ── export / import ───────────────────────────────────────────────────────

    /// Build a full backup of all sensors, their data, and settings.
    pub fn build_backup(
        &self,
        settings: &Settings,
        registry: &DeviceRegistry,
        exported_at: String,
    ) -> Backup {
        let mut data = BTreeMap::new();
        for d in &registry.list {
            if let Some(raw) = self.storage.get(&data_key(&d.id))
                && let Ok(dd) = serde_json::from_str::<DeviceData>(&raw) {
                    data.insert(d.id.clone(), dd);
                }
        }
        Backup {
            app: BACKUP_APP.into(),
            version: BACKUP_VERSION,
            exported_at,
            settings: *settings,
            devices: registry.clone(),
            data,
        }
    }

    /// Pretty-printed backup JSON (2-space indent, as the web app emits).
    pub fn export_json(
        &self,
        settings: &Settings,
        registry: &DeviceRegistry,
        exported_at: String,
    ) -> String {
        serde_json::to_string_pretty(&self.build_backup(settings, registry, exported_at))
            .unwrap_or_default()
    }

    /// Import a backup, merging into the current settings/registry (existing
    /// sensors kept; matching ids overwritten), and persisting everything.
    /// Returns the number of sensors imported, or an error message.
    pub fn import_json(
        &self,
        json: &str,
        settings: &mut Settings,
        registry: &mut DeviceRegistry,
    ) -> Result<ImportSummary, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;
        let list = v
            .get("devices")
            .and_then(|d| d.get("list"))
            .and_then(Value::as_array)
            .ok_or("not an AiDEX backup file")?;

        if let Some(s) = v.get("settings") {
            if let Some(u) = s.get("unit").and_then(Value::as_str) {
                if u == "mgdl" {
                    settings.unit = Unit::Mgdl;
                } else if u == "mmol" {
                    settings.unit = Unit::Mmol;
                }
            }
            if let Some(t) = s.get("theme").and_then(Value::as_str) {
                if t == "light" {
                    settings.theme = Theme::Light;
                } else if t == "dark" {
                    settings.theme = Theme::Dark;
                }
            }
        }

        let data_map = v.get("data");
        let mut count = 0;
        for item in list {
            let Ok(device) = serde_json::from_value::<Device>(item.clone()) else {
                continue;
            };
            if device.id.is_empty() {
                continue;
            }
            count += 1;
            // Per-device data, if present.
            if let Some(dd) = data_map.and_then(|m| m.get(&device.id)) {
                self.storage.set(&data_key(&device.id), &dd.to_string());
            }
            match registry.list.iter().position(|d| d.id == device.id) {
                Some(i) => registry.list[i] = device,
                None => registry.list.push(device),
            }
        }

        // Activate the backup's active device if it now exists; otherwise keep a
        // valid active id.
        if let Some(active) = v
            .get("devices")
            .and_then(|d| d.get("activeId"))
            .and_then(Value::as_str)
            && registry.list.iter().any(|d| d.id == active) {
                registry.active_id = Some(active.to_string());
            }
        if registry.active_id.is_none() {
            registry.active_id = registry.list.first().map(|d| d.id.clone());
        }

        self.save_settings(settings);
        self.save_registry(registry);
        Ok(ImportSummary { sensors: count })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// In-memory storage for tests, mirroring localStorage.
    #[derive(Default)]
    struct MemStore(RefCell<HashMap<String, String>>);
    impl Storage for MemStore {
        fn get(&self, key: &str) -> Option<String> {
            self.0.borrow().get(key).cloned()
        }
        fn set(&self, key: &str, value: &str) {
            self.0.borrow_mut().insert(key.into(), value.into());
        }
        fn remove(&self, key: &str) {
            self.0.borrow_mut().remove(key);
        }
    }

    #[test]
    fn reads_existing_web_app_localstorage() {
        // A snapshot of what the current index.html writes.
        let mem = MemStore::default();
        mem.set(KEY_SETTINGS, r#"{"unit":"mmol","theme":"dark"}"#);
        mem.set(
            KEY_DEVICES,
            r#"{"activeId":"abc","list":[{"id":"abc","name":"Left arm","serial":"EXAMPLE001","pairkey":"0123456789ABCDEF0123456789ABCDEF"}]}"#,
        );
        mem.set(
            &data_key("abc"),
            r#"{"sensorStartIso":"2026-06-25T00:00:00.000Z","records":{"60":[100,0],"61":[102,0]},"events":[{"t":1782758245123,"label":"coffee","icon":"☕"}]}"#,
        );
        let repo = Repository::new(mem);

        let settings = repo.load_settings();
        assert_eq!(settings.unit, Unit::Mmol);
        assert_eq!(settings.theme, Theme::Dark);

        let reg = repo.load_registry(|| "new".into());
        assert_eq!(reg.active_id.as_deref(), Some("abc"));
        assert_eq!(reg.list.len(), 1);
        assert_eq!(reg.active().unwrap().serial, "EXAMPLE001");

        let data = repo.load_data(Some("abc"));
        assert_eq!(data.records.get(&60), Some(&(100, 0)));
        assert_eq!(data.events.len(), 1);
        assert_eq!(data.sensor_start_ms(), Some(1_782_345_600_000));
    }

    #[test]
    fn migrates_legacy_single_device() {
        let mem = MemStore::default();
        // Old layout: serial/pairkey in settings, data in cgm.data.
        mem.set(
            KEY_SETTINGS,
            r#"{"unit":"mgdl","theme":"light","serial":"EXAMPLE001","pairkey":"0123456789ABCDEF0123456789ABCDEF"}"#,
        );
        mem.set(
            KEY_LEGACY_DATA,
            r#"{"sensorStartIso":"2026-06-25T00:00:00.000Z","records":{"60":[120,0]}}"#,
        );
        let repo = Repository::new(mem);
        let reg = repo.load_registry(|| "MIGRATED".into());
        assert_eq!(reg.list.len(), 1);
        let dev = &reg.list[0];
        assert_eq!(dev.id, "MIGRATED");
        assert_eq!(dev.serial, "EXAMPLE001");
        assert_eq!(reg.active_id.as_deref(), Some("MIGRATED"));
        // Legacy data copied under the new id.
        let data = repo.load_data(Some("MIGRATED"));
        assert_eq!(data.records.get(&60), Some(&(120, 0)));
        // Registry persisted.
        assert!(repo.storage().get(KEY_DEVICES).is_some());
    }

    #[test]
    fn export_import_round_trips() {
        let mem = MemStore::default();
        mem.set(
            KEY_DEVICES,
            r#"{"activeId":"abc","list":[{"id":"abc","name":"Left arm","serial":"EXAMPLE001","pairkey":"0123456789ABCDEF0123456789ABCDEF"}]}"#,
        );
        mem.set(
            &data_key("abc"),
            r#"{"sensorStartIso":"2026-06-25T00:00:00.000Z","records":{"60":[100,0]},"events":[]}"#,
        );
        let repo = Repository::new(mem);
        let settings = Settings {
            unit: Unit::Mmol,
            theme: Theme::Dark,
        };
        let reg = repo.load_registry(|| "x".into());
        let json = repo.export_json(&settings, &reg, "2026-06-25T12:00:00.000Z".into());
        assert!(json.contains("\"app\": \"aidex-cgm\""));
        assert!(json.contains("\"version\": 1"));

        // Import into a fresh repo.
        let mem2 = MemStore::default();
        let repo2 = Repository::new(mem2);
        let mut s2 = Settings::default();
        let mut reg2 = repo2.load_registry(|| "y".into());
        let summary = repo2.import_json(&json, &mut s2, &mut reg2).unwrap();
        assert_eq!(summary.sensors, 1);
        assert_eq!(s2.unit, Unit::Mmol);
        assert_eq!(reg2.active_id.as_deref(), Some("abc"));
        let data = repo2.load_data(Some("abc"));
        assert_eq!(data.records.get(&60), Some(&(100, 0)));
    }

    #[test]
    fn import_rejects_non_backup() {
        let repo = Repository::new(MemStore::default());
        let mut s = Settings::default();
        let mut reg = DeviceRegistry::default();
        assert!(
            repo.import_json("{\"foo\":1}", &mut s, &mut reg)
                .is_err()
        );
    }
}
