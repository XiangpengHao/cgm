//! Settings sheet — the home for everything that isn't the live glance: units,
//! the (read-only) target range, display, data/backup, sensors, and about.
//! Replaces the flat toolbar so the main screen stays glanceable.

use crate::actions::use_actions;
use crate::platform::SharedPlatform;
use crate::state::{AppState, ConfirmKind, Dialog};
use crate::ui::{BTN_GHOST, BTN_GHOST_SM, FOCUS};
use cgm_core::model::{Theme, Unit};
use cgm_core::ranges;
use cgm_core::stats::format_value;
use dioxus::prelude::*;

#[component]
pub fn SettingsSheet() -> Element {
    let mut state = use_context::<AppState>();
    let platform = use_context::<SharedPlatform>();
    let actions = use_actions();

    if !*state.show_settings.read() {
        return rsx! {};
    }

    let unit = state.settings.read().unit;
    let theme = state.settings.read().theme;
    let close = move |_| state.show_settings.set(false);

    // Read-only zone boundaries, from the single source of truth, in the user's unit.
    let edge_lo = format_value(ranges::NORMAL_MAX_MGDL, unit);
    let edge_hi = format_value(ranges::ELEVATED_MAX_MGDL, unit);
    let u = unit.label();
    let in_range = format!("≤ {edge_lo} {u}");
    let elevated = format!("{edge_lo}–{edge_hi} {u}");
    let high = format!("> {edge_hi} {u}");

    let seg = |on: bool| {
        if on {
            "h-11 sm:h-8 px-3 text-sm font-semibold bg-sky-600 text-white"
        } else {
            "h-11 sm:h-8 px-3 text-sm font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800"
        }
    };

    rsx! {
        div {
            // Bottom sheet on mobile, centered dialog at sm+.
            class: "fixed inset-0 z-40 bg-black/50 flex items-end sm:items-center justify-center sm:p-10",
            role: "dialog",
            "aria-modal": "true",
            onclick: close,
            div {
                class: "w-full sm:max-w-md h-[92vh] sm:h-auto sm:max-h-[85vh] rounded-t-2xl sm:rounded-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 shadow-xl overflow-y-auto space-y-5 p-5",
                style: "padding-bottom: calc(1.25rem + env(safe-area-inset-bottom));",
                tabindex: "-1",
                autofocus: true,
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |e| if e.key() == Key::Escape { state.show_settings.set(false) },

                div { class: "flex items-center justify-between",
                    h2 { class: "text-base font-semibold text-slate-900 dark:text-slate-100", "Settings" }
                    button { class: "{BTN_GHOST_SM}", onclick: close, "Close" }
                }

                // ── Units ─────────────────────────────────────────────────────
                Section { title: "Units",
                    div { class: "inline-flex rounded-lg border border-slate-300 dark:border-slate-700 overflow-hidden",
                        role: "group", "aria-label": "Glucose unit",
                        button {
                            class: "{seg(unit == Unit::Mgdl)} {FOCUS}",
                            onclick: move |_| if unit != Unit::Mgdl { actions.toggle_unit() },
                            "mg/dL"
                        }
                        button {
                            class: "{seg(unit == Unit::Mmol)} {FOCUS}",
                            onclick: move |_| if unit != Unit::Mmol { actions.toggle_unit() },
                            "mmol/L"
                        }
                    }
                }

                // ── Target range ──────────────────────────────────────────────
                Section { title: "Target range",
                    div { class: "space-y-1 text-sm",
                        Row { k: "🟢 In range", v: in_range.clone() }
                        Row { k: "🟡 Elevated", v: elevated.clone() }
                        Row { k: "🔴 High", v: high.clone() }
                    }
                    p { class: "mt-2 text-xs text-slate-500 dark:text-slate-400",
                        "Boundaries 5.6 / 7.8 mmol/L (100 / 140 mg/dL). The one definition used for the chart bands, time-in-range, and status colours."
                    }
                }

                // ── Display ───────────────────────────────────────────────────
                Section { title: "Display",
                    div { class: "flex items-center justify-between",
                        span { class: "text-sm text-slate-700 dark:text-slate-200", "Theme" }
                        div { class: "inline-flex rounded-lg border border-slate-300 dark:border-slate-700 overflow-hidden",
                            role: "group", "aria-label": "Theme",
                            button {
                                class: "{seg(theme == Theme::Light)} {FOCUS}",
                                onclick: move |_| if theme != Theme::Light { actions.toggle_theme() },
                                "🌞 Light"
                            }
                            button {
                                class: "{seg(theme == Theme::Dark)} {FOCUS}",
                                onclick: move |_| if theme != Theme::Dark { actions.toggle_theme() },
                                "🌙 Dark"
                            }
                        }
                    }
                }

                // ── Data & backup ─────────────────────────────────────────────
                Section { title: "Data & backup",
                    div { class: "grid grid-cols-2 gap-2",
                        button { class: "{BTN_GHOST}", onclick: move |_| actions.export_backup(), "Export backup" }
                        button { class: "{BTN_GHOST}", onclick: move |_| actions.import_backup(), "Import backup" }
                        button {
                            class: "{BTN_GHOST}",
                            onclick: move |_| {
                                state.show_settings.set(false);
                                state.show_health.set(true);
                            },
                            "Apple Health"
                        }
                        button {
                            class: "col-span-2 h-11 sm:h-9 px-3 rounded-lg border border-rose-300 dark:border-rose-800 text-sm font-medium text-rose-600 dark:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rose-500",
                            onclick: move |_| state.dialog.set(Some(Dialog::Confirm {
                                message: "Clear all stored readings and events for this sensor?".into(),
                                confirm_label: "Clear data".into(),
                                kind: ConfirmKind::ClearData,
                            })),
                            "Clear data"
                        }
                    }
                }

                // ── Sensors ───────────────────────────────────────────────────
                Section { title: "Sensors",
                    button {
                        class: "{BTN_GHOST}",
                        onclick: move |_| {
                            state.show_settings.set(false);
                            state.show_devices.set(true);
                        },
                        "Manage sensors"
                    }
                }

                // ── About ─────────────────────────────────────────────────────
                Section { title: "About",
                    div { class: "space-y-1 text-sm",
                        Row { k: "Version", v: env!("CARGO_PKG_VERSION").to_string() }
                        Row { k: "Platform", v: platform.label() }
                    }
                    p { class: "mt-2 text-xs text-slate-500 dark:text-slate-400",
                        "Experimental, uncalibrated, reverse-engineered — not a medical device and not for treatment decisions."
                    }
                }
            }
        }
    }
}

#[component]
fn Section(title: String, children: Element) -> Element {
    rsx! {
        section {
            h3 { class: "text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400 mb-2", "{title}" }
            {children}
        }
    }
}

#[component]
fn Row(k: String, v: String) -> Element {
    rsx! {
        div { class: "flex justify-between gap-4 py-0.5",
            span { class: "text-slate-500 dark:text-slate-400", "{k}" }
            span { class: "text-slate-800 dark:text-slate-100 text-right", "{v}" }
        }
    }
}
