//! Time-in-range bar — the single most useful CGM metric, shown as a simple
//! three-segment bar so anyone can read "how green was my day".

use crate::state::AppState;
use cgm_core::stats::time_in_range;
use dioxus::prelude::*;

#[component]
pub fn TimeInRangeBar() -> Element {
    let state = use_context::<AppState>();
    let data = state.data.read();

    let Some(tir) = time_in_range(&data) else {
        return rsx! {
            div { class: "flex items-center justify-between",
                span { class: "text-sm text-slate-500 dark:text-slate-400", "Time in range (24 h)" }
                span { class: "text-sm text-slate-400 dark:text-slate-500", "Not enough data yet" }
            }
        };
    };

    rsx! {
        div {
            div { class: "flex items-center justify-between",
                span { class: "text-sm text-slate-500 dark:text-slate-400", "Time in range (24 h)" }
                span { class: "text-sm font-semibold text-emerald-600 dark:text-emerald-400",
                    "{tir.in_pct}% in range"
                }
            }
            // Segments left→right match the chart bands: green in-range, amber
            // elevated, red high.
            div { class: "mt-3 flex h-3 w-full overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800",
                div { class: "bg-emerald-500", style: "width: {tir.in_pct}%" }
                div { class: "bg-amber-400", style: "width: {tir.elevated_pct}%" }
                div { class: "bg-rose-400", style: "width: {tir.high_pct}%" }
            }
            div { class: "mt-2 flex justify-between text-xs text-slate-500 dark:text-slate-400",
                span { class: "text-emerald-600 dark:text-emerald-400", "In range {tir.in_pct}%" }
                span { class: "text-amber-600 dark:text-amber-400", "Elevated {tir.elevated_pct}%" }
                span { class: "text-rose-600 dark:text-rose-400", "High {tir.high_pct}%" }
            }
        }
    }
}
