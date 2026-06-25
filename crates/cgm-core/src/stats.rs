//! Derived metrics for display: unit conversion, glucose zones, sensor age, and
//! time-in-range. Pure functions over the data model — easy to test, identical
//! across platforms.

use crate::glucose::record_valid;
use crate::model::{DeviceData, Unit};
use crate::ranges::{Zone, classify};

/// AiDEX X sensor wear life: 15 days, in minutes.
pub const SENSOR_LIFE_MIN: i64 = 15 * 1440;

/// Convert a stored mg/dL value into the display unit.
pub fn convert(mgdl: u16, unit: Unit) -> f64 {
    if unit.is_mgdl() {
        mgdl as f64
    } else {
        mgdl as f64 / 18.0
    }
}

/// Format a stored mg/dL value for display: integer mg/dL, or one decimal mmol/L.
pub fn format_value(mgdl: u16, unit: Unit) -> String {
    if unit.is_mgdl() {
        format!("{}", (mgdl as f64).round() as i64)
    } else {
        format!("{:.1}", mgdl as f64 / 18.0)
    }
}

/// How urgently the sensor age should be flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeUrgency {
    Normal,
    /// Within ~2 days of the 15-day limit.
    NearExpiry,
    Expired,
}

/// Human-readable sensor age vs the 15-day life, e.g. `2d 4h / 15 d · ~13 d left`.
pub fn age_text(min: Option<i64>) -> (String, AgeUrgency) {
    let Some(min) = min else {
        return ("—".into(), AgeUrgency::Normal);
    };
    let d = min / 1440;
    let h = (min % 1440) / 60;
    let age = if min < 60 {
        format!("{min} min")
    } else if d > 0 {
        format!("{d}d {h}h")
    } else {
        format!("{h}h {}m", min % 60)
    };
    let left = SENSOR_LIFE_MIN - min;
    if left <= 0 {
        return (format!("{age} · EXPIRED (>15 d)"), AgeUrgency::Expired);
    }
    let urgency = if left < 2 * 1440 {
        AgeUrgency::NearExpiry
    } else {
        AgeUrgency::Normal
    };
    let days_left = (left + 1439) / 1440;
    (format!("{age} / 15 d · ~{days_left} d left"), urgency)
}

/// Time spent in each glucose zone over the last 24 h of valid readings, as
/// percentages. The three buckets are exactly the [`Zone`]s, so this can never
/// disagree with the chart bands or the hero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeInRange {
    /// In range (≤ 5.6 mmol/L), green.
    pub in_pct: u32,
    /// Elevated (5.6–7.8 mmol/L), amber.
    pub elevated_pct: u32,
    /// High (> 7.8 mmol/L), red.
    pub high_pct: u32,
    pub total: u32,
}

/// Compute time-in-range over the most recent 24 hours of valid records.
pub fn time_in_range(data: &DeviceData) -> Option<TimeInRange> {
    let max = data.latest_index()? as i64;
    data.sensor_start_iso.as_ref()?;
    let cutoff = max - 24 * 60;
    let (mut in_r, mut elev, mut hi, mut tot) = (0u32, 0u32, 0u32, 0u32);
    for (&i, &(mgdl, rs)) in &data.records {
        let idx = i as i64;
        if idx < cutoff {
            continue;
        }
        if !record_valid(mgdl, rs, i as i32) {
            continue;
        }
        tot += 1;
        match classify(mgdl) {
            Zone::InRange => in_r += 1,
            Zone::Elevated => elev += 1,
            Zone::High => hi += 1,
        }
    }
    if tot == 0 {
        return None;
    }
    let pct = |x: u32| ((x as f64 / tot as f64) * 100.0).round() as u32;
    Some(TimeInRange {
        in_pct: pct(in_r),
        elevated_pct: pct(elev),
        high_pct: pct(hi),
        total: tot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_and_format() {
        assert_eq!(convert(180, Unit::Mgdl), 180.0);
        assert!((convert(180, Unit::Mmol) - 10.0).abs() < 1e-9);
        assert_eq!(format_value(100, Unit::Mgdl), "100");
        assert_eq!(format_value(180, Unit::Mmol), "10.0");
    }

    #[test]
    fn age_formatting() {
        assert_eq!(age_text(None).0, "—");
        // Matches the web app: the "/ 15 d" suffix shows even early on.
        assert_eq!(age_text(Some(30)).0, "30 min / 15 d · ~15 d left");
        let (txt, urg) = age_text(Some(2 * 1440 + 4 * 60));
        assert_eq!(txt, "2d 4h / 15 d · ~13 d left");
        assert_eq!(urg, AgeUrgency::Normal);
        assert_eq!(age_text(Some(SENSOR_LIFE_MIN + 10)).1, AgeUrgency::Expired);
        assert_eq!(
            age_text(Some(SENSOR_LIFE_MIN - 1440)).1,
            AgeUrgency::NearExpiry
        );
    }

    #[test]
    fn tir_over_recent_valid() {
        let mut d = DeviceData {
            sensor_start_iso: Some("2026-06-25T00:00:00.000Z".into()),
            ..Default::default()
        };
        // 4 valid readings within 24h of the latest (index 300): 2 in-range
        // (≤100), 1 elevated (100–140), 1 high (>140).
        d.records.insert(300, (120, 0)); // elevated
        d.records.insert(299, (60, 0)); // in range
        d.records.insert(298, (200, 0)); // high
        d.records.insert(297, (90, 0)); // in range
        d.records.insert(50, (100, 0)); // warmup -> invalid, ignored
        let tir = time_in_range(&d).unwrap();
        assert_eq!(tir.total, 4);
        assert_eq!(tir.in_pct, 50);
        assert_eq!(tir.elevated_pct, 25);
        assert_eq!(tir.high_pct, 25);
    }
}
