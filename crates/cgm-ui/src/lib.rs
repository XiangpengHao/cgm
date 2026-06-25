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
            // Respect iOS safe areas (no effect on web, where the insets are 0).
            style: "padding-top: env(safe-area-inset-top); padding-left: env(safe-area-inset-left); padding-right: env(safe-area-inset-right);",
            Header {}
            main { class: "max-w-5xl mx-auto p-4 space-y-4",
                ErrorBanner {}
                div { class: "grid grid-cols-1 lg:grid-cols-[340px_1fr] gap-4 items-start",
                    div { class: "space-y-4",
                        HeroCard {}
                        TimeInRangeBar {}
                    }
                    div { class: "{ui::CARD} p-4 min-w-0 overflow-hidden space-y-3",
                        div { class: "flex items-center justify-between gap-2",
                            h2 { class: "text-sm font-medium text-slate-500 dark:text-slate-400", "Glucose over time" }
                            ChartWindowControl {}
                        }
                        GlucoseChart {}
                    }
                }
                Diagnostics {}
                footer {
                    class: "pt-2 text-center text-xs text-slate-500 dark:text-slate-400",
                    style: "padding-bottom: calc(2rem + env(safe-area-inset-bottom));",
                    "Experimental, uncalibrated, reverse-engineered software — "
                    b { "not a medical device" }
                    " and not for treatment decisions. Confirm anything that matters with a fingerstick and your clinician."
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
