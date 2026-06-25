//! Collapsible "Advanced & diagnostics" — everything an expert needs to debug
//! and nothing a casual user has to look at. Hidden behind a disclosure.

use crate::components::RangesCard;
use crate::platform::SharedPlatform;
use crate::state::AppState;
use cgm_core::datetime::{format_hm, format_local};
use cgm_core::glucose::record_valid;
use cgm_core::stats::{age_text, AgeUrgency};
use dioxus::prelude::*;

#[component]
pub fn Diagnostics() -> Element {
    let mut state = use_context::<AppState>();
    let platform = use_context::<SharedPlatform>();
    let offset = platform.clock().local_offset_minutes();

    let open = *state.show_diagnostics.read();
    let live = state.live.read().clone();
    let data = state.data.read();

    let count = data.records.len();
    let session = data
        .sensor_start_ms()
        .map(|ms| format_local(ms, offset))
        .unwrap_or_else(|| "—".into());

    let age_min = live
        .time_offset
        .map(|t| t as i64)
        .or_else(|| data.latest_index().map(|i| i as i64));
    let (age_str, urgency) = age_text(age_min);
    let age_color = match urgency {
        AgeUrgency::Expired => "text-rose-600 dark:text-rose-400",
        AgeUrgency::NearExpiry => "text-amber-600 dark:text-amber-400",
        AgeUrgency::Normal => "text-slate-700 dark:text-slate-300",
    };

    let validity = match data.latest() {
        Some((idx, mgdl, rs)) => {
            let status_ok = live.status_byte.is_none_or(|s| s & 0x3f == 0);
            if record_valid(mgdl, rs, idx as i32) && status_ok {
                ("valid", "text-emerald-600 dark:text-emerald-400")
            } else if mgdl >= 0x3ff {
                ("saturated / warmup", "text-amber-600 dark:text-amber-400")
            } else {
                ("warming / invalid", "text-amber-600 dark:text-amber-400")
            }
        }
        None => ("—", "text-slate-500"),
    };
    let quality = live
        .quality
        .map(|q| q.to_string())
        .unwrap_or_else(|| "—".into());

    let logs = state.logs.read();
    let row = "flex justify-between gap-4 py-1 text-sm";
    let key_cls = "text-slate-500 dark:text-slate-400";
    rsx! {
        div { class: "rounded-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800",
            button {
                class: "w-full flex items-center justify-between px-5 py-3 text-sm font-medium text-slate-700 dark:text-slate-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-sky-500 rounded-2xl",
                "aria-expanded": if open { "true" } else { "false" },
                onclick: move |_| {
                    let v = !*state.show_diagnostics.read();
                    state.show_diagnostics.set(v);
                },
                span { "Advanced & diagnostics" }
                span { class: "text-slate-500 dark:text-slate-400", if open { "▲" } else { "▼" } }
            }
            if open {
                div { class: "px-5 pb-5 space-y-4 border-t border-slate-200 dark:border-slate-800 pt-4",
                    div { class: "text-xs text-slate-500 dark:text-slate-400", "{platform.label()}" }

                    div {
                        div { class: row, span { class: key_cls, "Readings stored" } b { "{count}" } }
                        div { class: row, span { class: key_cls, "Sensor session" } b { "{session}" } }
                        div { class: row,
                            span { class: key_cls, "Sensor age" }
                            b { class: "{age_color}", "{age_str}" }
                        }
                        div { class: row,
                            span { class: key_cls, "Latest validity" }
                            b { class: "{validity.1}", "{validity.0}" }
                        }
                        div { class: row, span { class: key_cls, "Signal quality" } b { "{quality}" } }
                    }

                    RangesCard {}

                    // Raw diagnostic log.
                    div {
                        div { class: "text-xs text-slate-500 dark:text-slate-400 mb-1", "Log" }
                        div { class: "max-h-40 overflow-auto rounded-lg bg-slate-50 dark:bg-slate-950 border border-slate-200 dark:border-slate-800 p-2 font-mono text-xs text-slate-600 dark:text-slate-400 space-y-0.5",
                            if logs.is_empty() {
                                div { class: "text-slate-400", "No log entries yet." }
                            }
                            for (i , line) in logs.iter().enumerate() {
                                div { key: "{i}", "[{format_hm(line.t_ms, offset)}] {line.msg}" }
                            }
                        }
                    }

                    p { class: "text-xs text-slate-400 dark:text-slate-500 pt-1",
                        "Backup, import, and clear-data live in Settings ▸ Data & backup."
                    }
                }
            }
        }
    }
}
