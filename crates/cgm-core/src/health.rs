//! Apple Health export: the valid, downsampled (1 per 5 min) reading list that
//! the Shortcuts recipe logs as Blood Glucose. The `samples` serialize to the
//! `glucose-health.json` the Shortcut reads.

use crate::datetime::epoch_ms_to_rfc3339;
use crate::glucose::record_valid;
use crate::model::DeviceData;
use serde::{Deserialize, Serialize};

/// One Blood Glucose sample for Apple Health.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HealthSample {
    /// RFC 3339 timestamp.
    pub time: String,
    pub glucose_mgdl: u16,
}

/// Build the Apple Health sample list: valid readings only, one per 5 minutes
/// (Health doesn't need 1-minute resolution, and Shortcuts logs one at a time).
pub fn health_samples(data: &DeviceData) -> Vec<HealthSample> {
    let Some(start) = data.sensor_start_ms() else {
        return Vec::new();
    };
    data.records
        .iter()
        .filter(|&(&i, &(mgdl, rs))| i % 5 == 0 && record_valid(mgdl, rs, i as i32))
        .map(|(&i, &(mgdl, _))| HealthSample {
            time: epoch_ms_to_rfc3339(start + i as i64 * 60_000),
            glucose_mgdl: mgdl,
        })
        .collect()
}

/// Serialize the Health samples to the JSON the Shortcut consumes.
pub fn health_json(data: &DeviceData) -> String {
    serde_json::to_string(&health_samples(data)).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_to_valid_every_5_min() {
        let mut d = DeviceData {
            sensor_start_iso: Some("2026-06-25T00:00:00.000Z".into()),
            ..Default::default()
        };
        d.records.insert(60, (100, 0)); // valid, %5==0 -> included
        d.records.insert(61, (101, 0)); // %5!=0 -> excluded
        d.records.insert(65, (0x3ff, 0)); // saturated -> excluded
        d.records.insert(70, (120, 0)); // included
        d.records.insert(30, (90, 0)); // warmup -> excluded
        let s = health_samples(&d);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].glucose_mgdl, 100);
        assert_eq!(s[0].time, "2026-06-25T01:00:00.000Z");
        assert_eq!(s[1].glucose_mgdl, 120);
    }

    #[test]
    fn no_start_no_samples() {
        let d = DeviceData::default();
        assert!(health_samples(&d).is_empty());
        assert_eq!(health_json(&d), "[]");
    }
}
