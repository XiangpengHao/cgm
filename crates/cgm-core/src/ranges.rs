//! **The** single source of truth for glucose zoning.
//!
//! Every consumer surface — the chart background bands, time-in-range, the hero
//! mood/headline, and the live value color — derives from this module. No
//! glucose-level threshold number may live anywhere else in the app (the only
//! other home is the static *diagnostic* blood-test reference table in
//! `cgm-ui/src/components/ranges.rs`, a different concept).
//!
//! The two boundaries are **5.6 and 7.8 mmol/L (100 and 140 mg/dL)** — the
//! fasting / post-meal "normal" breakpoints. Below 5.6 is in range (green),
//! 5.6–7.8 elevated (amber), above 7.8 high (red).
//!
//! Storage is always mg/dL, so classification is unit-blind; only *display*
//! (chart axis / band labels) converts via [`crate::stats::convert`].

use crate::model::Unit;
use crate::stats::convert;

// ── The ONLY glucose-level threshold literals in the codebase ───────────────
/// Upper edge of the in-range / "normal" band — 5.6 mmol/L.
pub const NORMAL_MAX_MGDL: u16 = 100;
/// Upper edge of the elevated band — 7.8 mmol/L.
pub const ELEVATED_MAX_MGDL: u16 = 140;

/// A glucose reading's zone, low → high. `InRange` is the only "good" state.
/// This is the **only** glucose-level classifier in the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// `≤ 100 mg/dL` (`≤ 5.6 mmol/L`) — the green in-range band.
    InRange,
    /// `101 ..= 140 mg/dL` (`5.6 ..= 7.8 mmol/L`).
    Elevated,
    /// `> 140 mg/dL` (`> 7.8 mmol/L`).
    High,
}

/// THE classifier. Everything else calls this.
pub fn classify(mgdl: u16) -> Zone {
    if mgdl <= NORMAL_MAX_MGDL {
        Zone::InRange
    } else if mgdl <= ELEVATED_MAX_MGDL {
        Zone::Elevated
    } else {
        Zone::High
    }
}

/// Three-level severity for coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Good,
    Warn,
    Bad,
}

impl Zone {
    /// Plain-language headline — a non-color cue that is never dropped.
    pub fn label(self) -> &'static str {
        match self {
            Zone::InRange => "In range",
            Zone::Elevated => "Elevated",
            Zone::High => "High",
        }
    }

    pub fn severity(self) -> Severity {
        match self {
            Zone::InRange => Severity::Good,
            Zone::Elevated => Severity::Warn,
            Zone::High => Severity::Bad,
        }
    }

    pub fn is_in_range(self) -> bool {
        matches!(self, Zone::InRange)
    }
}

/// The two zone boundaries in mg/dL, low → high. Derived from the same
/// constants — there is no second copy of any number. Time-in-range, the chart
/// bands, and the chart y-domain anchors all consume these, so they cannot
/// disagree.
pub const BAND_EDGES_MGDL: [u16; 2] = [NORMAL_MAX_MGDL, ELEVATED_MAX_MGDL];

/// The two band edges converted to the display unit, for chart drawing.
pub fn band_edges(unit: Unit) -> [f64; 2] {
    BAND_EDGES_MGDL.map(|m| convert(m, unit))
}

#[cfg(test)]
mod consistency {
    use super::*;

    #[test]
    fn edges_match_classifier_boundaries() {
        // The chart's band edges ARE the classifier's zone boundaries.
        assert_eq!(BAND_EDGES_MGDL, [NORMAL_MAX_MGDL, ELEVATED_MAX_MGDL]);
        assert_eq!(classify(NORMAL_MAX_MGDL), Zone::InRange);
        assert_eq!(classify(NORMAL_MAX_MGDL + 1), Zone::Elevated);
        assert_eq!(classify(ELEVATED_MAX_MGDL), Zone::Elevated);
        assert_eq!(classify(ELEVATED_MAX_MGDL + 1), Zone::High);
    }

    #[test]
    fn severity_maps_one_to_one() {
        assert_eq!(classify(90).severity(), Severity::Good);
        assert_eq!(classify(120).severity(), Severity::Warn);
        assert_eq!(classify(160).severity(), Severity::Bad);
    }

    #[test]
    fn in_range_partitions_the_domain_at_the_edges() {
        for v in 0u16..=600 {
            assert_eq!(classify(v).is_in_range(), v <= NORMAL_MAX_MGDL);
        }
    }

    #[test]
    fn mmol_edges_are_the_same_constants_converted() {
        // No independent 5.6/7.8 literal — mg/dL and mmol agree exactly.
        let m = band_edges(Unit::Mmol);
        assert!((m[0] - convert(NORMAL_MAX_MGDL, Unit::Mmol)).abs() < 1e-9);
        assert!((m[1] - convert(ELEVATED_MAX_MGDL, Unit::Mmol)).abs() < 1e-9);
        // ...and they display as the familiar 5.6 / 7.8.
        assert_eq!(crate::stats::format_value(NORMAL_MAX_MGDL, Unit::Mmol), "5.6");
        assert_eq!(crate::stats::format_value(ELEVATED_MAX_MGDL, Unit::Mmol), "7.8");
    }
}
