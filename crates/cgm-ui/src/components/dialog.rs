//! In-app confirm/prompt dialog + a transient toast. These replace
//! `window.confirm`/`window.prompt` (unreliable in some mobile browsers) and
//! the silent diagnostics-only feedback, so destructive actions, renames, and
//! import/export confirmations behave consistently.

use crate::actions::{use_actions, Actions};
use crate::state::{AppState, ConfirmKind, ConnStatus, Dialog, PromptKind};
use crate::ui::{BTN_DANGER, BTN_GHOST, BTN_GHOST_SM, BTN_PRIMARY, CARD, INPUT};
use dioxus::prelude::*;

fn run_confirm(kind: &ConfirmKind, actions: Actions) {
    match kind {
        ConfirmKind::DeleteDevice(id) => actions.delete_device(id),
        ConfirmKind::ClearData => actions.clear_data(),
    }
}

fn run_prompt(kind: &PromptKind, value: String, actions: Actions) {
    match kind {
        PromptKind::RenameDevice(id) => actions.rename_device(id, value),
    }
}

#[component]
pub fn DialogModal() -> Element {
    let mut state = use_context::<AppState>();
    let actions = use_actions();

    let Some(dialog) = state.dialog.read().clone() else {
        return rsx! {};
    };

    rsx! {
        div {
            class: "fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4",
            role: "dialog",
            "aria-modal": "true",
            onclick: move |_| state.dialog.set(None),
            div {
                class: "{CARD} w-full max-w-sm p-5 shadow-xl",
                tabindex: "-1",
                autofocus: true,
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |e| if e.key() == Key::Escape { state.dialog.set(None) },

                match dialog {
                    Dialog::Confirm { message, confirm_label, kind } => rsx! {
                        p { class: "text-sm text-slate-700 dark:text-slate-200", "{message}" }
                        div { class: "mt-4 flex justify-end gap-2",
                            button { class: "{BTN_GHOST}", onclick: move |_| state.dialog.set(None), "Cancel" }
                            button {
                                class: "{BTN_DANGER}",
                                onclick: move |_| {
                                    run_confirm(&kind, actions);
                                    state.dialog.set(None);
                                },
                                "{confirm_label}"
                            }
                        }
                    },
                    // A child component so the editable text lives in a local
                    // signal seeded once — a controlled input echoed from the
                    // enum each keystroke collapses the caret to the end.
                    Dialog::Prompt { message, value, kind } => rsx! {
                        PromptBody { message, initial: value, kind }
                    },
                }
            }
        }
    }
}

#[component]
fn PromptBody(message: String, initial: String, kind: PromptKind) -> Element {
    let mut state = use_context::<AppState>();
    let actions = use_actions();
    let mut draft = use_signal(|| initial.clone());

    rsx! {
        label { class: "text-sm text-slate-700 dark:text-slate-200", "{message}" }
        input {
            class: "{INPUT} mt-2",
            value: "{draft}",
            autofocus: true,
            "aria-label": "{message}",
            oninput: move |e| draft.set(e.value()),
            onkeydown: {
                let kind = kind.clone();
                move |e| if e.key() == Key::Enter {
                    run_prompt(&kind, draft.read().trim().to_string(), actions);
                    state.dialog.set(None);
                }
            },
        }
        div { class: "mt-4 flex justify-end gap-2",
            button { class: "{BTN_GHOST}", onclick: move |_| state.dialog.set(None), "Cancel" }
            button {
                class: "{BTN_PRIMARY}",
                onclick: move |_| {
                    run_prompt(&kind, draft.read().trim().to_string(), actions);
                    state.dialog.set(None);
                },
                "Save"
            }
        }
    }
}

/// A prominent, dismissible banner for connection errors, with a Reconnect CTA
/// (errors otherwise only reach the collapsed diagnostics log).
#[component]
pub fn ErrorBanner() -> Element {
    let mut state = use_context::<AppState>();
    let actions = use_actions();
    let ConnStatus::Error(msg) = state.status.read().clone() else {
        return rsx! {};
    };
    rsx! {
        div {
            class: "rounded-xl border border-rose-300 dark:border-rose-800 bg-rose-50 dark:bg-rose-950 px-4 py-3 flex flex-wrap items-center gap-3",
            role: "alert",
            span { class: "flex-1 min-w-0 text-sm text-rose-700 dark:text-rose-300", "Connection problem: {msg}" }
            button { class: "{BTN_GHOST_SM}", onclick: move |_| actions.connect(false), "Reconnect" }
            button {
                class: "{BTN_GHOST_SM}",
                onclick: move |_| state.status.set(ConnStatus::Disconnected),
                "Dismiss"
            }
        }
    }
}

#[component]
pub fn Toast() -> Element {
    let state = use_context::<AppState>();
    let Some(msg) = state.toast.read().clone() else {
        return rsx! {};
    };
    rsx! {
        div {
            class: "fixed bottom-5 left-1/2 -translate-x-1/2 z-50 rounded-lg bg-slate-900 dark:bg-slate-100 text-white dark:text-slate-900 px-4 py-2 text-sm font-medium shadow-lg max-w-[90vw]",
            role: "status",
            "aria-live": "polite",
            "{msg}"
        }
    }
}
