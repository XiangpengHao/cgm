//! iOS file I/O + Apple Health. Backups land in the app's Documents directory
//! (visible in the Files app); Apple Health writes go straight to HealthKit.

use crate::health;
use cgm_core::engine::LocalFuture;
use cgm_core::health::HealthSample;
use cgm_ui::platform::Files;
use std::path::PathBuf;

const IMPORT_FILE: &str = "aidex-import.json";

fn documents_dir() -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join("Documents"))
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&base);
    base
}

pub struct IosFiles;

impl Files for IosFiles {
    fn download(&self, filename: &str, _mime: &str, contents: &str) {
        // Write into Documents so it appears in the Files app (requires the
        // file-sharing Info.plist keys — see README.md).
        let _ = std::fs::write(documents_dir().join(filename), contents);
    }

    fn pick_text(&self) -> LocalFuture<'static, Option<String>> {
        // Lightweight import: read a backup the user dropped into the app's
        // Documents folder as `aidex-import.json`.
        Box::pin(async move { std::fs::read_to_string(documents_dir().join(IMPORT_FILE)).ok() })
    }

    fn export_health(
        &self,
        json: String,
        _samples: usize,
    ) -> LocalFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let samples: Vec<HealthSample> =
                serde_json::from_str(&json).map_err(|e| format!("bad sample data: {e}"))?;
            let n = health::write_samples(samples).await?;
            Ok(format!("wrote {n} readings to Apple Health"))
        })
    }

    fn health_is_native(&self) -> bool {
        true
    }
}
