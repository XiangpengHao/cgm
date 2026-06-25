//! The headline card: the big current value, what it means in plain language,
//! the trend, and how fresh it is. Built for a glance, not for analysis.

use crate::format::{Mood, relative_time, trend_label};
use crate::platform::SharedPlatform;
use crate::state::AppState;
use cgm_core::glucose::{SensorState, record_valid, trend_arrow};
use cgm_core::ranges::classify;
use cgm_core::stats::format_value;
use dioxus::prelude::*;

#[component]
pub fn HeroCard() -> Element {
    let state = use_context::<AppState>();
    let platform = use_context::<SharedPlatform>();
    let now = platform.clock().now_ms();

    let unit = state.settings.read().unit;
    let live = state.live.read().clone();
    let data = state.data.read();

    let Some((idx, mgdl, rs)) = data.latest() else {
        let has_device = !state.registry.read().list.is_empty();
        let (headline, sub) = if !has_device {
            ("No sensor yet", "Add a sensor to begin — we'll guide you through pairing.")
        } else {
            ("No readings yet", "Connect your sensor to sync and chart your glucose.")
        };
        return rsx! {
            div { class: "rounded-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 p-6",
                h2 { class: "text-sm text-slate-500 dark:text-slate-400 font-normal", "Current glucose" }
                div { class: "mt-2 text-2xl font-semibold text-slate-400 dark:text-slate-500", "{headline}" }
                p { class: "mt-1 text-sm text-slate-500 dark:text-slate-400", "{sub}" }
            }
        };
    };

    let status_ok = live.status_byte.is_none_or(|s| s & 0x3f == 0);
    let valid = record_valid(mgdl, rs, idx as i32) && status_ok;
    // All glucose-level classification flows through the single source of truth.
    let (headline, mood) = if valid {
        let zone = classify(mgdl);
        (zone.label(), Mood::from(zone.severity()))
    } else if mgdl >= 0x3ff {
        ("Sensor warming / saturated", Mood::Neutral)
    } else {
        ("Warming up", Mood::Neutral)
    };

    let value = format_value(mgdl, unit);
    let arrow = if valid { live.trend.map(trend_arrow) } else { None };
    let updated = data
        .record_time_ms(idx)
        .map(|t| relative_time(now, t))
        .unwrap_or_else(|| "—".into());

    let state_label = live
        .state
        .map(|s| match s {
            SensorState::NewOrUsed => "Not started",
            SensorState::Expired => "Expired",
            SensorState::WarmingUp => "Warming up",
            SensorState::Active => "Active",
        })
        .unwrap_or(if idx >= 60 { "Active" } else { "Warming up" });

    rsx! {
        div { class: "rounded-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 p-6",
            div { class: "flex items-center justify-between",
                h2 { class: "text-sm text-slate-500 dark:text-slate-400 font-normal", "Current glucose" }
                span { class: "text-xs px-2 py-0.5 rounded-full bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300",
                    "{state_label}"
                }
            }
            // Announce zone changes (e.g. crossing into "Urgent low") to screen readers.
            div { "aria-live": "polite",
                div { class: "mt-1 flex items-baseline gap-2",
                    span { class: "text-6xl font-bold tracking-tight {mood.text()}", "{value}" }
                    span { class: "text-lg font-semibold text-slate-400 dark:text-slate-500", "{unit.label()}" }
                    if let Some(arrow) = arrow {
                        span { class: "text-4xl font-semibold {mood.text()}", "aria-hidden": "true", "{arrow}" }
                    }
                }
                div { class: "mt-3 flex flex-wrap items-center gap-2",
                    span { class: "text-sm font-semibold px-2.5 py-1 rounded-full {mood.chip()}", "{headline}" }
                    if let Some(trend) = live.trend {
                        span { class: "text-sm px-2.5 py-1 rounded-full bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300",
                            "{trend_label(trend)}"
                        }
                    }
                }
            }
            p { class: "mt-3 text-sm text-slate-500 dark:text-slate-400", "Updated {updated}" }
        }
    }
}
