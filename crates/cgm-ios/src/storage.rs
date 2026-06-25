//! File-backed [`Storage`] in the app's Documents directory. Mirrors
//! `localStorage` semantics (a flat string key/value map) so the same
//! `cgm.*` keys, migration, and export/import all work unchanged.

use cgm_core::store::Storage;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

const FILE: &str = "glucose-store.json";

fn store_path() -> PathBuf {
    // On iOS the sandbox HOME is the app container; Documents persists + is
    // backed up. Fall back to a temp dir if unavailable.
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join("Documents"))
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&base);
    base.join(FILE)
}

pub struct FileStorage {
    map: RefCell<HashMap<String, String>>,
    path: PathBuf,
}

impl FileStorage {
    pub fn load() -> Self {
        let path = store_path();
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok())
            .unwrap_or_default();
        FileStorage {
            map: RefCell::new(map),
            path,
        }
    }

    fn flush(&self) {
        if let Ok(json) = serde_json::to_string(&*self.map.borrow()) {
            let _ = std::fs::write(&self.path, json);
        }
    }
}

impl Storage for FileStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.map.borrow().get(key).cloned()
    }

    fn set(&self, key: &str, value: &str) {
        self.map.borrow_mut().insert(key.to_string(), value.to_string());
        self.flush();
    }

    fn remove(&self, key: &str) {
        self.map.borrow_mut().remove(key);
        self.flush();
    }
}
