//! The add-event popover. Opens either anchored at a desktop double-click, or
//! centered (the touch-friendly "Log event" button path). Preserves the quick
//! "log a coffee/meal/insulin" interaction.

use crate::actions::use_actions;
use crate::format::EVENT_TYPES;
use crate::platform::SharedPlatform;
use crate::state::AppState;
use crate::ui::{BTN_PRIMARY, INPUT};
use cgm_core::datetime::format_hm;
use dioxus::prelude::*;

#[component]
pub fn EventPopover() -> Element {
    let mut state = use_context::<AppState>();
    let platform = use_context::<SharedPlatform>();
    let actions = use_actions();
    let mut custom = use_signal(String::new);

    let Some(draft) = state.event_draft.read().clone() else {
        return rsx! {};
    };
    let offset = platform.clock().local_offset_minutes();
    let when = format_hm(draft.t_ms, offset);
    let t_ms = draft.t_ms;

    let near_label = {
        let d = state.data.read();
        d.events
            .iter()
            .filter(|e| (e.t - t_ms).abs() < 15 * 60_000)
            .min_by_key(|e| (e.t - t_ms).abs())
            .map(|e| e.label.clone())
    };

    // Anchored (desktop dbl-click) is clamped into the viewport with CSS min/max
    // so it never goes off-screen; centered (button) uses a flex backdrop.
    let pos_style = if draft.anchored {
        format!(
            "left: max(0.5rem, min({}px, calc(100vw - 16rem))); top: max(0.5rem, min({}px, calc(100vh - 22rem)));",
            draft.x,
            draft.y + 8.0
        )
    } else {
        String::new()
    };
    let backdrop_class = if draft.anchored {
        "fixed inset-0 z-40"
    } else {
        "fixed inset-0 z-40 bg-black/40 flex items-center justify-center p-4"
    };
    let card_class = if draft.anchored {
        "fixed z-50 w-64 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-3 shadow-xl"
    } else {
        "relative z-50 w-64 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-3 shadow-xl"
    };

    rsx! {
        div { class: backdrop_class, onclick: move |_| state.event_draft.set(None),
            div {
                class: card_class,
                style: "{pos_style}",
                role: "dialog",
                "aria-modal": "true",
                tabindex: "-1",
                autofocus: true,
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |e| if e.key() == Key::Escape { state.event_draft.set(None) },

                div { class: "text-xs text-slate-500 dark:text-slate-400 mb-2", "Log event at " b { "{when}" } }
                div { class: "grid grid-cols-2 gap-1.5",
                    for (icon , label) in EVENT_TYPES.iter() {
                        button {
                            key: "{label}",
                            class: "h-9 px-2 rounded-lg border border-slate-200 dark:border-slate-700 text-xs text-slate-700 dark:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-800 active:bg-slate-200 dark:active:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500",
                            onclick: move |_| {
                                actions.add_event(t_ms, (*label).to_string(), Some((*icon).to_string()));
                                state.event_draft.set(None);
                            },
                            "{icon} {label}"
                        }
                    }
                }
                div { class: "mt-2 flex gap-1.5",
                    input {
                        r#type: "text",
                        class: "{INPUT} h-9 text-xs",
                        placeholder: "custom…",
                        value: "{custom}",
                        "aria-label": "Custom event name",
                        oninput: move |e| custom.set(e.value()),
                        onkeydown: move |e| if e.key() == Key::Enter { let v = custom.read().trim().to_string(); if !v.is_empty() { actions.add_event(t_ms, v, None); custom.set(String::new()); state.event_draft.set(None); } },
                    }
                    button {
                        class: "{BTN_PRIMARY} h-9 px-3 text-xs",
                        onclick: move |_| { let v = custom.read().trim().to_string(); if !v.is_empty() { actions.add_event(t_ms, v, None); custom.set(String::new()); state.event_draft.set(None); } },
                        "Add"
                    }
                }
                if let Some(label) = near_label {
                    button {
                        class: "mt-2 w-full h-9 rounded-lg border border-rose-200 dark:border-rose-800 text-xs text-rose-600 dark:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rose-500",
                        onclick: move |_| {
                            actions.remove_event_near(t_ms);
                            state.event_draft.set(None);
                        },
                        "✕ remove “{label}”"
                    }
                }
            }
        }
    }
}
