//! Browser file I/O: download a blob, pick a text file, and the Apple Health
//! export (web hands off a JSON file to the Shortcuts recipe).

use cgm_core::engine::LocalFuture;
use cgm_ui::platform::Files;
use futures::channel::oneshot;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Blob, BlobPropertyBag, FileReader, HtmlAnchorElement, HtmlInputElement, Url};

pub struct WebFiles;

fn trigger_download(filename: &str, mime: &str, contents: &str) -> Option<()> {
    let window = web_sys::window()?;
    let document = window.document()?;
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(contents));
    let opts = BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = Blob::new_with_str_sequence_and_options(&parts, &opts).ok()?;
    let url = Url::create_object_url_with_blob(&blob).ok()?;
    let a: HtmlAnchorElement = document.create_element("a").ok()?.dyn_into().ok()?;
    a.set_href(&url);
    a.set_download(filename);
    a.click();
    let _ = Url::revoke_object_url(&url);
    Some(())
}

impl Files for WebFiles {
    fn download(&self, filename: &str, mime: &str, contents: &str) {
        trigger_download(filename, mime, contents);
    }

    fn pick_text(&self) -> LocalFuture<'static, Option<String>> {
        Box::pin(async move {
            let (tx, rx) = oneshot::channel::<Option<String>>();
            let tx = Rc::new(RefCell::new(Some(tx)));
            let document = web_sys::window().and_then(|w| w.document())?;
            let Ok(input) = document
                .create_element("input")
                .and_then(|e| e.dyn_into::<HtmlInputElement>().map_err(Into::into))
            else {
                return None;
            };
            input.set_type("file");
            input.set_accept(".json,application/json");

            let onchange = Closure::once_into_js(move |ev: web_sys::Event| {
                let file = ev
                    .target()
                    .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
                    .and_then(|i| i.files())
                    .and_then(|fl| fl.get(0));
                let Some(file) = file else {
                    if let Some(tx) = tx.borrow_mut().take() {
                        let _ = tx.send(None);
                    }
                    return;
                };
                let Ok(reader) = FileReader::new() else {
                    if let Some(tx) = tx.borrow_mut().take() {
                        let _ = tx.send(None);
                    }
                    return;
                };
                let reader2 = reader.clone();
                let onload = Closure::once_into_js(move |_e: web_sys::Event| {
                    let text = reader2.result().ok().and_then(|v| v.as_string());
                    if let Some(tx) = tx.borrow_mut().take() {
                        let _ = tx.send(text);
                    }
                });
                reader.set_onloadend(Some(onload.unchecked_ref()));
                let _ = reader.read_as_text(&file);
            });
            input.set_onchange(Some(onchange.unchecked_ref()));
            input.click();

            rx.await.ok().flatten()
        })
    }

    fn export_health(
        &self,
        json: String,
        samples: usize,
    ) -> LocalFuture<'static, Result<String, String>> {
        Box::pin(async move {
            trigger_download("glucose-health.json", "application/json", &json);
            Ok(format!(
                "downloaded {samples} readings — import via the Shortcuts recipe"
            ))
        })
    }

    fn health_is_native(&self) -> bool {
        false
    }
}
