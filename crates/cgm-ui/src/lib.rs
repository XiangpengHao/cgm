//! # cgm-ui
//!
//! The shared Dioxus UI for the glucose app. It depends only on [`cgm_core`] and
//! `dioxus`; everything device- or browser-specific is reached through the
//! [`platform`] traits, so this exact UI renders on both the web (WASM) and iOS
//! (Dioxus mobile) targets.
//!
//! A backend builds a `Platform`, provides it via context, and renders [`App`].

// The `rsx!` macro expands text/attribute interpolations like `"{x}"` to
// `format!`, and conditional class attributes to `if/else` blocks — both trip
// these style lints in macro-generated code, not in ours.
#![allow(clippy::useless_format, clippy::suspicious_else_formatting)]

pub mod actions;
pub mod components;
pub mod format;
pub mod platform;
pub mod state;
pub mod ui;

use components::{
    ChartWindowControl, DevicesModal, Diagnostics, DialogModal, ErrorBanner, EventPopover,
    GlucoseChart, HealthModal, Header, HeroCard, SettingsSheet, TimeInRangeBar, Toast,
};
use dioxus::prelude::*;
use platform::SharedPlatform;
use state::AppState;

pub use platform::{Ble, Clock, Files, Platform};

/// Root component. Expects a [`SharedPlatform`] already provided in context.
#[component]
pub fn App() -> Element {
    let platform = use_context::<SharedPlatform>();
    let state = use_context_provider(|| AppState::load(platform.clone()));

    let dark = state.settings.read().theme == cgm_core::model::Theme::Dark;
    let root_class = if dark { "dark" } else { "" };

    rsx! {
        div {
            class: "{root_class} min-h-screen bg-slate-50 dark:bg-slate-950 text-slate-900 dark:text-slate-100",
            // Respect iOS side safe areas; the sticky header owns the top inset
            // (Dynamic Island) so it stays put when scrolled.
            style: "padding-left: env(safe-area-inset-left); padding-right: env(safe-area-inset-right);",
            Header {}
            main { class: "max-w-5xl mx-auto p-3 sm:p-4 space-y-3 sm:space-y-4",
                ErrorBanner {}
                div { class: "grid grid-cols-1 lg:grid-cols-[340px_1fr] gap-3 sm:gap-4 items-start",
                    // One glance card: the current value and time-in-range together.
                    div { class: "{ui::CARD} p-4 sm:p-6",
                        HeroCard {}
                        div { class: "mt-3 sm:mt-4 pt-3 sm:pt-4 border-t border-slate-200 dark:border-slate-800",
                            TimeInRangeBar {}
                        }
                    }
                    div { class: "{ui::CARD} p-4 sm:p-6 min-w-0 overflow-hidden space-y-2 sm:space-y-3",
                        div { class: "flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2",
                            h2 { class: "sr-only sm:not-sr-only text-sm font-medium text-slate-500 dark:text-slate-400", "Glucose over time" }
                            ChartWindowControl {}
                        }
                        GlucoseChart {}
                    }
                }
                Diagnostics {}
                footer {
                    class: "pt-2 text-center text-xs text-slate-500 dark:text-slate-400",
                    style: "padding-bottom: calc(1.5rem + env(safe-area-inset-bottom));",
                    span { class: "sm:hidden", "Not a medical device — confirm with a fingerstick." }
                    span { class: "hidden sm:inline",
                        "Experimental, uncalibrated, reverse-engineered software — "
                        b { "not a medical device" }
                        " and not for treatment decisions. Confirm anything that matters with a fingerstick and your clinician."
                    }
                }
            }
            DevicesModal {}
            HealthModal {}
            SettingsSheet {}
            EventPopover {}
            DialogModal {}
            Toast {}
        }
    }
}
