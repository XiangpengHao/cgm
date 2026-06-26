//! Sensor management modal: the device list (use / rename / remove) and the
//! guided add-sensor wizard (name → pair → start → connect). Rename/remove use
//! the in-app dialog so they work on iOS too.

use crate::actions::use_actions;
use crate::state::{AppState, ConfirmKind, ConnStatus, Dialog, PromptKind};
use crate::ui::{BTN_GHOST, BTN_GHOST_SM, BTN_PRIMARY, INPUT};
use cgm_core::model::is_pair_key;
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum View {
    Home,
    Wizard,
}

#[derive(Clone, PartialEq, Default)]
struct Wizard {
    step: u8,
    name: String,
    serial: String,
    key: String,
    manual_key: String,
    activate_checked: bool,
    pairing: bool,
    error: Option<String>,
}

#[component]
pub fn DevicesModal() -> Element {
    let mut state = use_context::<AppState>();
    let mut view = use_signal(|| View::Home);
    let mut wiz = use_signal(Wizard::default);

    if !*state.show_devices.read() {
        return rsx! {};
    }

    rsx! {
        div {
            // Bottom sheet on mobile, centered dialog at sm+.
            class: "fixed inset-0 z-40 bg-black/50 flex items-end sm:items-center justify-center sm:p-10",
            role: "dialog",
            "aria-modal": "true",
            // Don't discard an in-flight wizard (typed SN, pairing) on a stray
            // backdrop tap; the wizard has its own Cancel/Close.
            onclick: move |_| if *view.read() != View::Wizard {
                state.show_devices.set(false);
                view.set(View::Home);
                wiz.set(Wizard::default());
            },
            div {
                class: "w-full sm:max-w-md max-h-[92vh] sm:max-h-[85vh] rounded-t-2xl sm:rounded-2xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 p-5 shadow-xl overflow-y-auto",
                style: "padding-bottom: calc(1.25rem + env(safe-area-inset-bottom));",
                tabindex: "-1",
                autofocus: true,
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |e| if e.key() == Key::Escape && *view.read() != View::Wizard {
                    state.show_devices.set(false);
                    view.set(View::Home);
                    wiz.set(Wizard::default());
                },
                match *view.read() {
                    View::Home => rsx! { HomeView { view, wiz } },
                    View::Wizard => rsx! { WizardView { view, wiz } },
                }
            }
        }
    }
}

#[component]
fn HomeView(view: Signal<View>, wiz: Signal<Wizard>) -> Element {
    let (mut view, mut wiz) = (view, wiz);
    let mut state = use_context::<AppState>();
    let actions = use_actions();
    let registry = state.registry.read();
    let active = registry.active_id.clone();

    rsx! {
        div { class: "flex items-center justify-between",
            h2 { class: "text-base font-semibold text-slate-900 dark:text-slate-100", "Sensors" }
            button {
                class: "{BTN_GHOST_SM}",
                onclick: move |_| {
                    state.show_devices.set(false);
                    view.set(View::Home);
                },
                "Close"
            }
        }
        div { class: "mt-3 space-y-2",
            if registry.list.is_empty() {
                div { class: "text-sm text-slate-500 dark:text-slate-400", "No sensors yet — add one to begin." }
            }
            for dev in registry.list.iter() {
                {
                    let id = dev.id.clone();
                    let name = dev.display_name().to_string();
                    let is_active = active.as_deref() == Some(dev.id.as_str());
                    let paired = dev.is_paired();
                    rsx! {
                        div {
                            key: "{dev.id}",
                            class: "flex items-center gap-2 rounded-xl border border-slate-200 dark:border-slate-800 p-2.5",
                            span {
                                class: if is_active { "h-2 w-2 rounded-full bg-emerald-500" } else { "h-2 w-2 rounded-full bg-slate-300 dark:bg-slate-600" },
                                title: if is_active { "Active sensor" } else { "Inactive" },
                            }
                            div { class: "flex-1 min-w-0",
                                div { class: "font-medium text-sm text-slate-800 dark:text-slate-100 truncate", "{dev.display_name()}" }
                                div { class: "text-xs text-slate-500 dark:text-slate-400",
                                    "SN {dev.serial}" if !paired { " · not paired" }
                                }
                            }
                            button {
                                class: "{BTN_GHOST_SM}",
                                disabled: is_active,
                                "aria-label": "Use this sensor",
                                onclick: {
                                    let id = id.clone();
                                    move |_| actions.set_active_device(id.clone())
                                },
                                if is_active { "active" } else { "use" }
                            }
                            button {
                                class: "{BTN_GHOST_SM}",
                                "aria-label": "Rename sensor",
                                onclick: {
                                    let id = id.clone();
                                    let name = name.clone();
                                    move |_| state.dialog.set(Some(Dialog::Prompt {
                                        message: "Name for this sensor".into(),
                                        value: name.clone(),
                                        kind: PromptKind::RenameDevice(id.clone()),
                                    }))
                                },
                                "rename"
                            }
                            button {
                                class: "{BTN_GHOST_SM}",
                                "aria-label": "Remove sensor",
                                onclick: {
                                    let id = id.clone();
                                    move |_| state.dialog.set(Some(Dialog::Confirm {
                                        message: "Remove this sensor and its stored readings?".into(),
                                        confirm_label: "Remove".into(),
                                        kind: ConfirmKind::DeleteDevice(id.clone()),
                                    }))
                                },
                                "✕"
                            }
                        }
                    }
                }
            }
        }
        button {
            class: "{BTN_PRIMARY} mt-4 w-full",
            onclick: move |_| {
                wiz.set(Wizard::default());
                view.set(View::Wizard);
            },
            "＋ Add a sensor"
        }
    }
}

#[component]
fn WizardView(view: Signal<View>, wiz: Signal<Wizard>) -> Element {
    let (mut view, mut wiz) = (view, wiz);
    let mut state = use_context::<AppState>();
    let actions = use_actions();
    let step = wiz.read().step;
    let status = state.status.read().clone();

    let cancel = move |_| view.set(View::Home);

    rsx! {
        div { class: "flex items-center justify-between",
            h2 { class: "text-base font-semibold text-slate-900 dark:text-slate-100",
                match step {
                    0 => "Add a sensor",
                    1 => "Pair",
                    2 => "Start the sensor",
                    _ => "Connect",
                }
            }
            button { class: "{BTN_GHOST_SM}", onclick: cancel, "Cancel" }
        }
        if step < 3 {
            div { class: "text-xs text-slate-500 dark:text-slate-400 mt-1", "Step {step + 1} of 3" }
        }

        if step == 0 {
            div { class: "mt-3 space-y-3",
                p { class: "text-sm text-slate-600 dark:text-slate-300",
                    "About a minute. We'll "
                    b { "pair" } ", " b { "start" } " the sensor, then " b { "connect" } " — explained as we go."
                }
                div {
                    label { class: "text-xs text-slate-500 dark:text-slate-400", r#for: "wiz-name", "Name (optional)" }
                    input {
                        id: "wiz-name",
                        class: "{INPUT} mt-1",
                        placeholder: "e.g. Left arm",
                        value: "{wiz.read().name}",
                        oninput: move |e| wiz.write().name = e.value(),
                    }
                }
                div {
                    label { class: "text-xs text-slate-500 dark:text-slate-400", r#for: "wiz-sn", "Serial number (SN)" }
                    input {
                        id: "wiz-sn",
                        class: "{INPUT} mt-1",
                        placeholder: "printed on the transmitter",
                        value: "{wiz.read().serial}",
                        oninput: move |e| wiz.write().serial = e.value(),
                    }
                    p { class: "text-xs text-slate-400 dark:text-slate-500 mt-1",
                        "~10 letters/numbers, on the transmitter and its box."
                    }
                }
                if let Some(err) = wiz.read().error.clone() {
                    div { class: "text-sm text-rose-600 dark:text-rose-400", "{err}" }
                }
                div { class: "flex justify-end",
                    button {
                        class: "{BTN_PRIMARY}",
                        onclick: move |_| {
                            let serial = wiz.read().serial.trim().to_string();
                            if serial.is_empty() {
                                wiz.write().error = Some("Enter the serial number (SN).".into());
                            } else {
                                wiz.write().serial = serial;
                                wiz.write().step = 1;
                                wiz.write().error = None;
                            }
                        },
                        "Next →"
                    }
                }
            }
        } else if step == 1 {
            div { class: "mt-3 space-y-3",
                p { class: "text-sm text-slate-600 dark:text-slate-300",
                    "Pairing makes a "
                    b { "private key" }
                    " so only this device can read your sensor — it happens once. Keep the transmitter nearby and powered on."
                }
                if !wiz.read().key.is_empty() {
                    div { class: "text-sm text-emerald-600 dark:text-emerald-400 font-medium break-all",
                        "✓ Paired — key saved."
                    }
                }
                if let Some(err) = wiz.read().error.clone() {
                    div { class: "text-sm text-rose-600 dark:text-rose-400", "Couldn't pair: {err}" }
                }
                div { class: "flex items-center gap-2",
                    button { class: "{BTN_GHOST}", onclick: move |_| wiz.write().step = 0, "← back" }
                    span { class: "flex-1" }
                    if wiz.read().key.is_empty() {
                        button {
                            class: "{BTN_PRIMARY}",
                            disabled: wiz.read().pairing,
                            onclick: move |_| {
                                let serial = wiz.read().serial.clone();
                                wiz.write().pairing = true;
                                wiz.write().error = None;
                                spawn(async move {
                                    match actions.pair(serial).await {
                                        Ok(key) => {
                                            let name = wiz.read().name.clone();
                                            let serial = wiz.read().serial.clone();
                                            actions.upsert_device(name, serial, key.clone());
                                            let mut w = wiz.write();
                                            w.key = key;
                                            w.pairing = false;
                                        }
                                        Err(e) => {
                                            let mut w = wiz.write();
                                            w.error = Some(e);
                                            w.pairing = false;
                                        }
                                    }
                                });
                            },
                            if wiz.read().pairing { "pairing…" } else { "Pair now" }
                        }
                    } else {
                        button { class: "{BTN_PRIMARY}", onclick: move |_| wiz.write().step = 2, "Next →" }
                    }
                }
                // Inline manual-key entry (works on every platform, no native prompt).
                if wiz.read().key.is_empty() {
                    details { class: "text-xs",
                        summary { class: "cursor-pointer text-sky-600 dark:text-sky-400", "Already have a key? Enter it manually" }
                        div { class: "mt-2 flex gap-1.5",
                            input {
                                class: "{INPUT} h-9 font-mono",
                                placeholder: "32 hex characters",
                                value: "{wiz.read().manual_key}",
                                "aria-label": "Pair key",
                                oninput: move |e| wiz.write().manual_key = e.value(),
                            }
                            button {
                                class: "{BTN_GHOST}",
                                onclick: move |_| {
                                    let key = wiz.read().manual_key.trim().replace(char::is_whitespace, "").to_uppercase();
                                    if is_pair_key(&key) {
                                        let name = wiz.read().name.clone();
                                        let serial = wiz.read().serial.clone();
                                        actions.upsert_device(name, serial, key.clone());
                                        let mut w = wiz.write();
                                        w.key = key;
                                        w.error = None;
                                    } else {
                                        wiz.write().error = Some("That doesn't look like a 32-hex key.".into());
                                    }
                                },
                                "Use"
                            }
                        }
                    }
                }
            }
        } else if step == 2 {
            div { class: "mt-3 space-y-3",
                p { class: "text-sm text-slate-600 dark:text-slate-300",
                    "Starting begins a "
                    b { "~1-hour warmup" }
                    ", then ~15 days of readings. "
                    b { class: "text-rose-600 dark:text-rose-400", "A sensor can be started only once" }
                    " — do this only when it's applied to your body and the transmitter is clipped on."
                }
                label { class: "flex items-center gap-2 text-sm text-slate-700 dark:text-slate-200",
                    input {
                        r#type: "checkbox",
                        checked: wiz.read().activate_checked,
                        onchange: move |e| wiz.write().activate_checked = e.checked(),
                    }
                    "The sensor is applied to my body"
                }
                div { class: "flex flex-wrap items-center gap-2",
                    button { class: "{BTN_GHOST}", onclick: move |_| wiz.write().step = 1, "← back" }
                    span { class: "flex-1" }
                    button {
                        class: "{BTN_GHOST}",
                        onclick: move |_| {
                            wiz.write().step = 3;
                            actions.connect(false);
                        },
                        "Already started — skip"
                    }
                    button {
                        class: "{BTN_PRIMARY}",
                        disabled: !wiz.read().activate_checked,
                        onclick: move |_| {
                            wiz.write().step = 3;
                            actions.connect(true);
                        },
                        "Start the sensor"
                    }
                }
            }
        } else {
            div { class: "mt-3 space-y-3",
                match status {
                    ConnStatus::Error(e) => rsx! {
                        p { class: "text-sm text-rose-600 dark:text-rose-400",
                            "Connection problem: {e}. Close this and tap Connect to retry."
                        }
                    },
                    ConnStatus::Connected | ConnStatus::Syncing => rsx! {
                        p { class: "text-sm text-emerald-600 dark:text-emerald-400",
                            "✓ Connected — your readings are loading."
                        }
                    },
                    _ => rsx! {
                        p { class: "text-sm text-slate-600 dark:text-slate-300",
                            "Connecting and loading your readings… you can close this; the status is shown at the top."
                        }
                    },
                }
                div { class: "flex justify-end",
                    button {
                        class: "{BTN_PRIMARY}",
                        onclick: move |_| {
                            state.show_devices.set(false);
                            view.set(View::Home);
                            wiz.set(Wizard::default());
                        },
                        "Done"
                    }
                }
            }
        }
    }
}
