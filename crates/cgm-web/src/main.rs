//! Web entry point. Builds the [`WebPlatform`], provides it via context, and
//! mounts the shared [`cgm_ui::App`].

mod ble;
mod files;
mod platform;
mod storage;

use cgm_ui::platform::SharedPlatform;
use dioxus::prelude::*;
use platform::WebPlatform;
use std::rc::Rc;

const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

fn main() {
    dioxus::launch(Root);
}

#[component]
fn Root() -> Element {
    use_context_provider(|| -> SharedPlatform { Rc::new(WebPlatform::new()) });
    rsx! {
        document::Meta {
            name: "viewport",
            content: "width=device-width, initial-scale=1, viewport-fit=cover",
        }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Title { "Glucose — AiDEX X / GX-01S" }
        cgm_ui::App {}
    }
}
