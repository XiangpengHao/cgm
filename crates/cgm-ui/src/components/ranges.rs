//! Reference diagnostic ranges (Mayo Clinic / ADA). Expert context, shown in the
//! diagnostics panel — not in a beginner's face.

use crate::state::AppState;
use dioxus::prelude::*;

#[component]
pub fn RangesCard() -> Element {
    let state = use_context::<AppState>();
    let mgdl = state.settings.read().unit.is_mgdl();

    // [test, normal, prediabetes, diabetes]
    let rows: [[&str; 4]; 2] = if mgdl {
        [
            ["Fasting (8 h+)", "< 100", "100–125", "≥ 126"],
            ["2 h post-meal", "< 140", "140–199", "≥ 200"],
        ]
    } else {
        [
            ["Fasting (8 h+)", "< 5.6", "5.6–6.9", "≥ 7.0"],
            ["2 h post-meal", "< 7.8", "7.8–11.0", "≥ 11.1"],
        ]
    };
    let unit_label = if mgdl { "mg/dL" } else { "mmol/L" };

    rsx! {
        div {
            div { class: "text-xs text-slate-500 dark:text-slate-400 mb-2",
                "Diagnostic reference ({unit_label}) — blood tests, not arbitrary CGM readings"
            }
            table { class: "w-full text-xs border-collapse",
                thead {
                    tr { class: "text-slate-500 dark:text-slate-400",
                        th { class: "text-left font-medium py-1", "Test" }
                        th { class: "text-emerald-600 dark:text-emerald-400 font-medium", "Normal" }
                        th { class: "text-amber-600 dark:text-amber-400 font-medium", "Pre" }
                        th { class: "text-rose-600 dark:text-rose-400 font-medium", "Diabetes" }
                    }
                }
                tbody {
                    for row in rows.iter() {
                        tr { class: "border-t border-slate-200 dark:border-slate-800",
                            td { class: "text-left py-1 text-slate-700 dark:text-slate-300", "{row[0]}" }
                            td { class: "text-center text-slate-600 dark:text-slate-400", "{row[1]}" }
                            td { class: "text-center text-slate-600 dark:text-slate-400", "{row[2]}" }
                            td { class: "text-center text-slate-600 dark:text-slate-400", "{row[3]}" }
                        }
                    }
                }
            }
        }
    }
}
