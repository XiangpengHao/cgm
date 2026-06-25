//! Small presentation helpers: friendly status text, relative time, and the
//! Tailwind colour classes for glucose moods. Pure functions, no Dioxus.

use cgm_core::glucose::trend_arrow;
use cgm_core::ranges::Severity;

/// How a value should feel to a non-expert user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Neutral,
    Good,
    Warn,
    Bad,
}

/// The one mapping from a glucose [`Severity`] to a UI colour. Thresholds live
/// only in `cgm_core::ranges`; this just translates the severity to a colour.
impl From<Severity> for Mood {
    fn from(s: Severity) -> Mood {
        match s {
            Severity::Good => Mood::Good,
            Severity::Warn => Mood::Warn,
            Severity::Bad => Mood::Bad,
        }
    }
}

impl Mood {
    /// Strong text colour (the big number, headline).
    pub fn text(self) -> &'static str {
        match self {
            Mood::Neutral => "text-slate-400 dark:text-slate-500",
            Mood::Good => "text-emerald-600 dark:text-emerald-400",
            Mood::Warn => "text-amber-600 dark:text-amber-400",
            Mood::Bad => "text-rose-600 dark:text-rose-400",
        }
    }

    /// Soft chip background.
    pub fn chip(self) -> &'static str {
        match self {
            Mood::Neutral => "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300",
            Mood::Good => "bg-emerald-100 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300",
            Mood::Warn => "bg-amber-100 text-amber-700 dark:bg-amber-950 dark:text-amber-300",
            Mood::Bad => "bg-rose-100 text-rose-700 dark:bg-rose-950 dark:text-rose-300",
        }
    }
}

/// "just now", "3 min ago", "1 h 5 min ago" for a past instant.
pub fn relative_time(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    if secs < 45 {
        return "just now".into();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins} min ago");
    }
    let hours = mins / 60;
    let rem = mins % 60;
    if hours < 24 {
        if rem == 0 {
            format!("{hours} h ago")
        } else {
            format!("{hours} h {rem} min ago")
        }
    } else {
        format!("{} d ago", hours / 24)
    }
}

/// "+2/min ↗" trend label for a mg/dL-per-minute slope.
pub fn trend_label(trend: i8) -> String {
    format!("{}{}/min {}", if trend >= 0 { "+" } else { "" }, trend, trend_arrow(trend))
}

/// Quick-pick event types for the add-event popover: (emoji, label).
pub const EVENT_TYPES: [(&str, &str); 6] = [
    ("☕", "coffee"),
    ("🍎", "snack"),
    ("🍽️", "meal"),
    ("🥤", "drink"),
    ("🏃", "exercise"),
    ("💉", "insulin"),
];

/// Default emoji for a known event label.
pub fn icon_for(label: &str) -> String {
    EVENT_TYPES
        .iter()
        .find(|(_, l)| *l == label)
        .map(|(icon, _)| (*icon).to_string())
        .unwrap_or_else(|| "📝".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moods() {
        // Boundaries 100/140 mg/dL: ≤100 in range, 100–140 elevated, >140 high.
        let mood = |mgdl| Mood::from(cgm_core::ranges::classify(mgdl).severity());
        assert_eq!(mood(90), Mood::Good);
        assert_eq!(mood(100), Mood::Good);
        assert_eq!(mood(120), Mood::Warn);
        assert_eq!(mood(160), Mood::Bad);
    }

    #[test]
    fn relative() {
        assert_eq!(relative_time(10_000, 10_000), "just now");
        assert_eq!(relative_time(60_000 * 3, 0), "3 min ago");
        assert_eq!(relative_time(60_000 * 65, 0), "1 h 5 min ago");
    }
}
