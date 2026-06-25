//! iOS entry point. Builds the [`IosPlatform`] (CoreBluetooth + HealthKit +
//! file storage), provides it via context, and mounts the shared
//! [`cgm_ui::App`] — the same UI the web build renders.

mod ble;
mod files;
mod health;
mod platform;
mod storage;

use cgm_ui::platform::SharedPlatform;
use dioxus::prelude::*;
use platform::IosPlatform;
use std::rc::Rc;

const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

fn main() {
    dioxus::launch(Root);
}

#[component]
fn Root() -> Element {
    use_context_provider(|| -> SharedPlatform { Rc::new(IosPlatform::new()) });
    rsx! {
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        cgm_ui::App {}
    }
}
