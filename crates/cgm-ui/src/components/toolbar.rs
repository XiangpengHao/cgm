//! The chart's time-window control — the only *live* control on the glance
//! surface. Everything else (units, theme, data, sensors) lives in Settings.

use crate::state::AppState;
use crate::ui::FOCUS;
use dioxus::prelude::*;

const WINDOWS: [(u32, &str); 5] = [(3, "3h"), (6, "6h"), (12, "12h"), (24, "24h"), (0, "All")];

#[component]
pub fn ChartWindowControl() -> Element {
    let mut state = use_context::<AppState>();
    let window = *state.window_hours.read();
    // A button is "active" only when following that window (not while panned).
    let panned = state.chart_view.read().is_some();

    rsx! {
        div {
            // Full-width fat targets on mobile; compact inline group at sm+.
            class: "grid grid-cols-5 w-full sm:inline-flex sm:w-auto rounded-lg border border-slate-300 dark:border-slate-700 overflow-hidden",
            role: "group",
            "aria-label": "Chart time window",
            for (hours , label) in WINDOWS.iter() {
                button {
                    key: "{label}",
                    class: if window == *hours && !panned {
                        "h-11 sm:h-8 px-3 text-sm font-semibold bg-sky-600 text-white {FOCUS}"
                    } else {
                        "h-11 sm:h-8 px-3 text-sm font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 {FOCUS}"
                    },
                    onclick: {
                        let hours = *hours;
                        move |_| {
                            state.window_hours.set(hours);
                            state.chart_view.set(None); // resume following the latest
                        }
                    },
                    "{label}"
                }
            }
        }
    }
}
