//! Top bar: identity, connection state, the device picker, and the one primary
//! action a user needs right now (Add sensor → Connect → Disconnect).

use crate::actions::use_actions;
use crate::state::{AppState, ConnStatus};
use crate::ui::{BTN_GHOST, BTN_PRIMARY, FOCUS};
use dioxus::prelude::*;

#[component]
pub fn Header() -> Element {
    let mut state = use_context::<AppState>();
    let actions = use_actions();

    let status = state.status.read().clone();
    let registry = state.registry.read();
    let has_devices = !registry.list.is_empty();
    let connected = status.is_connected();
    let busy = status.is_busy();

    let dot = if matches!(status, ConnStatus::Error(_)) {
        "bg-rose-500"
    } else if connected {
        "bg-emerald-500"
    } else if busy {
        "bg-amber-500 animate-pulse"
    } else {
        "bg-slate-400 dark:bg-slate-500"
    };

    rsx! {
        header { class: "flex flex-wrap items-center gap-3 px-4 py-3 border-b border-slate-200 dark:border-slate-800",
            span { class: "h-2.5 w-2.5 rounded-full flex-none {dot}", "aria-hidden": "true" }
            h1 { class: "text-base font-semibold text-slate-900 dark:text-slate-100", "Glucose" }
            span { class: "text-sm text-slate-500 dark:text-slate-400 truncate max-w-[40vw]", role: "status", "{status.label()}" }
            span { class: "flex-1" }

            if has_devices {
                select {
                    class: "h-9 max-w-44 rounded-lg border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 text-sm px-2 text-slate-700 dark:text-slate-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500",
                    "aria-label": "Active sensor",
                    value: "{registry.active_id.clone().unwrap_or_default()}",
                    onchange: move |e| actions.set_active_device(e.value()),
                    for dev in registry.list.iter() {
                        option { key: "{dev.id}", value: "{dev.id}", "{dev.display_name()}" }
                    }
                }
            }

            if !has_devices {
                button {
                    class: "{BTN_PRIMARY}",
                    onclick: move |_| state.show_devices.set(true),
                    "Add a sensor"
                }
            } else if connected {
                button {
                    class: "{BTN_GHOST}",
                    disabled: busy,
                    onclick: move |_| actions.sync(),
                    if busy { "Syncing…" } else { "Sync" }
                }
                button {
                    class: "{BTN_GHOST}",
                    onclick: move |_| actions.disconnect(),
                    "Disconnect"
                }
            } else {
                button {
                    class: "{BTN_PRIMARY}",
                    disabled: busy,
                    onclick: move |_| actions.connect(false),
                    if busy { "Connecting…" } else { "Connect" }
                }
            }

            button {
                class: "h-9 w-9 inline-flex items-center justify-center rounded-lg border border-slate-300 dark:border-slate-700 text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 {FOCUS}",
                "aria-label": "Settings",
                onclick: move |_| state.show_settings.set(true),
                "⚙"
            }
        }
    }
}
