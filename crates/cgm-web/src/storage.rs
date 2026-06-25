//! `localStorage`-backed [`Storage`].

use cgm_core::store::Storage;

pub struct LocalStorage;

fn store() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

impl Storage for LocalStorage {
    fn get(&self, key: &str) -> Option<String> {
        store()?.get_item(key).ok().flatten()
    }

    fn set(&self, key: &str, value: &str) {
        if let Some(s) = store() {
            let _ = s.set_item(key, value);
        }
    }

    fn remove(&self, key: &str) {
        if let Some(s) = store() {
            let _ = s.remove_item(key);
        }
    }
}
