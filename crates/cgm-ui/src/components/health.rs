//! Apple Health export panel. On iOS this writes Blood Glucose to HealthKit
//! directly; on web it produces the JSON the built-in Shortcuts recipe logs.

use crate::actions::use_actions;
use crate::platform::SharedPlatform;
use crate::state::AppState;
use cgm_core::health::health_samples;
use dioxus::prelude::*;

#[component]
pub fn HealthModal() -> Element {
    let mut state = use_context::<AppState>();
    let platform = use_context::<SharedPlatform>();
    let actions = use_actions();

    if !*state.show_health.read() {
        return rsx! {};
    }

    let native = platform.files().health_is_native();
    let count = health_samples(&state.data.read()).len();

    let primary = "h-9 px-4 rounded-lg bg-sky-600 hover:bg-sky-500 text-white text-sm font-semibold disabled:opacity-50";
    let ghost = "h-8 px-3 rounded-lg border border-slate-200 dark:border-slate-700 text-sm font-medium text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800";

    rsx! {
        div {
            class: "fixed inset-0 z-40 bg-black/50 flex items-start justify-center overflow-auto p-4 sm:p-10",
            onclick: move |_| state.show_health.set(false),
            div {
                class: "w-full max-w-lg rounded-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 p-5 shadow-xl",
                onclick: move |e| e.stop_propagation(),
                div { class: "flex items-center justify-between",
                    h2 { class: "text-base font-semibold text-slate-900 dark:text-slate-100", "Export to Apple Health" }
                    button { class: "{ghost}", onclick: move |_| state.show_health.set(false), "close" }
                }

                if native {
                    div { class: "mt-3 space-y-3",
                        p { class: "text-sm text-slate-600 dark:text-slate-300",
                            "Write your valid readings (1 per 5 minutes) to the Health app as Blood Glucose."
                        }
                        button {
                            class: "{primary}",
                            disabled: count == 0,
                            onclick: move |_| actions.export_health(),
                            "Write {count} readings to Health"
                        }
                        p { class: "text-xs text-slate-400 dark:text-slate-500",
                            "You'll be asked to grant Health write access the first time."
                        }
                    }
                } else {
                    div { class: "mt-3 space-y-3",
                        p { class: "text-sm text-slate-600 dark:text-slate-300",
                            "HealthKit is iOS-only, but Apple's built-in "
                            b { "Shortcuts" }
                            " app can log Blood Glucose — nothing to install."
                        }
                        div { class: "flex items-center gap-3",
                            button {
                                class: "{primary}",
                                disabled: count == 0,
                                onclick: move |_| actions.export_health(),
                                "⬇ Download Health JSON"
                            }
                            span { class: "text-xs text-slate-400 dark:text-slate-500",
                                "{count} valid readings · 1 per 5 min"
                            }
                        }
                        ol { class: "list-decimal pl-5 text-sm text-slate-600 dark:text-slate-300 space-y-1.5",
                            li { "Get " code { "glucose-health.json" } " onto your iPhone (AirDrop, Files, or iCloud Drive)." }
                            li {
                                "In Shortcuts, build: " b { "Get File" } " → " b { "Get Dictionary from Input" }
                                " → " b { "Repeat with Each" } " → " b { "Get Dictionary Value" } " "
                                code { "glucose_mgdl" } " and " code { "time" }
                                " → " b { "Log Health Sample" } " (Blood Glucose · mg/dL · value · date)."
                            }
                            li { "Run it and allow Health access. Re-run after each export to add new readings." }
                        }
                        p { class: "text-xs text-slate-400 dark:text-slate-500",
                            "Uncalibrated CGM values — not for clinical use."
                        }
                    }
                }
            }
        }
    }
}
