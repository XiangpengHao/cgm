//! Web Bluetooth transport: pairing on `F001` and the authenticated `F002`
//! byte pipe the engine drives. All protocol logic lives in `cgm_core`; this is
//! purely the browser GATT plumbing.

use cgm_core::engine::{BleBackend, BleError, LocalFuture};
use cgm_ui::platform::Ble;
use futures::channel::{mpsc, oneshot};
use futures::future::{select, Either};
use futures::StreamExt;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    BluetoothDevice, BluetoothLeScanFilterInit, BluetoothRemoteGattCharacteristic,
    BluetoothRemoteGattServer, BluetoothRemoteGattService, RequestDeviceOptions,
};

const SERVICE: &str = "0000181f-0000-1000-8000-00805f9b34fb";
const F001: &str = "0000f001-0000-1000-8000-00805f9b34fb";
const F002: &str = "0000f002-0000-1000-8000-00805f9b34fb";
const F003: &str = "0000f003-0000-1000-8000-00805f9b34fb";

fn js_err(e: JsValue) -> String {
    e.as_string()
        .or_else(|| js_sys::Reflect::get(&e, &JsValue::from_str("message")).ok()?.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

fn bluetooth() -> Result<web_sys::Bluetooth, String> {
    let nav = web_sys::window().ok_or("no window")?.navigator();
    nav.bluetooth()
        .ok_or_else(|| "Web Bluetooth not available (use Chrome/Edge over https or localhost)".into())
}

/// Await any (possibly typed) JS promise, flattening to the resolved `JsValue`.
async fn jsf<P: JsCast>(p: P) -> Result<JsValue, String> {
    JsFuture::from(p.unchecked_into::<js_sys::Promise>())
        .await
        .map_err(js_err)
}

/// Pull the raw bytes out of a characteristic's current `DataView` value.
fn char_bytes(c: &BluetoothRemoteGattCharacteristic) -> Option<Vec<u8>> {
    let dv = c.value()?;
    Some(js_sys::Uint8Array::new(&dv.buffer()).to_vec())
}

async fn request_device(bt: &web_sys::Bluetooth) -> Result<BluetoothDevice, String> {
    let services = [js_sys::JsString::from(SERVICE)];
    let filter = BluetoothLeScanFilterInit::new();
    filter.set_services(&services);
    let filters = [filter];

    let opts = RequestDeviceOptions::new();
    opts.set_filters(&filters);
    let optional = [js_sys::JsString::from(SERVICE)];
    opts.set_optional_services(&optional);

    let dev = jsf(bt.request_device(&opts)).await?;
    dev.dyn_into::<BluetoothDevice>().map_err(js_err)
}

async fn service_of(device: &BluetoothDevice) -> Result<(BluetoothRemoteGattServer, BluetoothRemoteGattService), String> {
    let gatt = device.gatt().ok_or("device has no GATT server")?;
    let server: BluetoothRemoteGattServer = jsf(gatt.connect()).await?.dyn_into().map_err(js_err)?;
    let svc: BluetoothRemoteGattService = jsf(server.get_primary_service_with_str(SERVICE))
        .await?
        .dyn_into()
        .map_err(js_err)?;
    Ok((server, svc))
}

async fn characteristic(
    svc: &BluetoothRemoteGattService,
    uuid: &str,
) -> Result<BluetoothRemoteGattCharacteristic, String> {
    jsf(svc.get_characteristic_with_str(uuid))
        .await?
        .dyn_into()
        .map_err(js_err)
}

/// The web `Ble` service. Remembers the just-paired device so `connect()` can
/// reuse it without a second chooser prompt (mirrors the web app's `reuseConn`).
#[derive(Default)]
pub struct WebBle {
    last_device: RefCell<Option<BluetoothDevice>>,
}

impl Ble for WebBle {
    fn available(&self) -> bool {
        bluetooth().is_ok()
    }

    fn pair(&self, secret: [u8; 16]) -> LocalFuture<'static, Result<String, String>> {
        let last = self.last_device.clone();
        Box::pin(async move {
            let bt = bluetooth()?;
            let device = request_device(&bt).await?;
            *last.borrow_mut() = Some(device.clone());
            let (server, svc) = service_of(&device).await?;

            let f001 = characteristic(&svc, F001).await?;
            let (tx, rx) = oneshot::channel::<Vec<u8>>();
            let tx = Rc::new(RefCell::new(Some(tx)));
            let cb = {
                let tx = tx.clone();
                Closure::wrap(Box::new(move |ev: web_sys::Event| {
                    if let Some(c) = ev
                        .target()
                        .and_then(|t| t.dyn_into::<BluetoothRemoteGattCharacteristic>().ok())
                        && let Some(bytes) = char_bytes(&c)
                            && bytes.len() == 16
                                && let Some(tx) = tx.borrow_mut().take() {
                                    let _ = tx.send(bytes);
                                }
                }) as Box<dyn FnMut(web_sys::Event)>)
            };
            // Use addEventListener (not the oncharacteristicvaluechanged
            // property), which iOS WebBLE shims like Bluefy actually honor.
            jsf(f001.start_notifications()).await?;
            f001.add_event_listener_with_callback(
                "characteristicvaluechanged",
                cb.as_ref().unchecked_ref(),
            )
            .map_err(js_err)?;
            // Best-effort: subscribe to F002 too, as the real device expects.
            if let Ok(f002) = characteristic(&svc, F002).await {
                let _ = jsf(f002.start_notifications()).await;
            }
            gloo_timers::future::TimeoutFuture::new(300).await;

            let arr = js_sys::Uint8Array::from(&secret[..]);
            let p = f001.write_value_with_u8_array(&arr).map_err(js_err)?;
            jsf(p).await?;

            let timeout = gloo_timers::future::TimeoutFuture::new(8000);
            let key = match select(rx, timeout).await {
                Either::Left((Ok(bytes), _)) => bytes,
                _ => {
                    server.disconnect();
                    drop(cb);
                    return Err("no key returned — pairing only works on a fresh / unpaired transmitter".into());
                }
            };
            server.disconnect();
            drop(cb);
            Ok(key.iter().map(|b| format!("{b:02X}")).collect())
        })
    }

    fn connect(&self) -> LocalFuture<'static, Result<Box<dyn BleBackend>, String>> {
        let reuse = self.last_device.borrow_mut().take();
        Box::pin(async move {
            let device = match reuse {
                Some(d) => d,
                None => request_device(&bluetooth()?).await?,
            };
            let (server, svc) = service_of(&device).await?;

            // Subscribe to F003 (reconnect notify) best-effort.
            if let Ok(f003) = characteristic(&svc, F003).await {
                let _ = jsf(f003.start_notifications()).await;
            }
            let f002 = characteristic(&svc, F002).await?;

            let (tx, rx) = mpsc::unbounded::<Vec<u8>>();
            let cb = Closure::wrap(Box::new(move |ev: web_sys::Event| {
                if let Some(c) = ev
                    .target()
                    .and_then(|t| t.dyn_into::<BluetoothRemoteGattCharacteristic>().ok())
                    && let Some(bytes) = char_bytes(&c) {
                        let _ = tx.unbounded_send(bytes);
                    }
            }) as Box<dyn FnMut(web_sys::Event)>);
            // addEventListener, not the oncharacteristicvaluechanged property —
            // the latter is a no-op in iOS WebBLE shims (Bluefy), so the
            // notification callback would never fire and every command would
            // time out even though the handshake (a GATT read) succeeded.
            jsf(f002.start_notifications()).await?;
            f002.add_event_listener_with_callback(
                "characteristicvaluechanged",
                cb.as_ref().unchecked_ref(),
            )
            .map_err(js_err)?;

            Ok(Box::new(WebBackend {
                f002,
                rx,
                _cb: cb,
                server,
                _device: device,
            }) as Box<dyn BleBackend>)
        })
    }

    fn disconnect(&self) {
        if let Some(d) = self.last_device.borrow().as_ref()
            && let Some(g) = d.gatt() {
                g.disconnect();
            }
    }
}

/// An open GATT session. Owning the server + closure keeps the connection and
/// notification listener alive for the lifetime of the backend.
struct WebBackend {
    f002: BluetoothRemoteGattCharacteristic,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    _cb: Closure<dyn FnMut(web_sys::Event)>,
    server: BluetoothRemoteGattServer,
    _device: BluetoothDevice,
}

impl Drop for WebBackend {
    fn drop(&mut self) {
        // Dropping the session (e.g. on Disconnect) closes the GATT link.
        self.server.disconnect();
    }
}

impl BleBackend for WebBackend {
    fn read_value(&mut self) -> LocalFuture<'_, Result<Vec<u8>, BleError>> {
        Box::pin(async move {
            jsf(self.f002.read_value()).await.map_err(BleError::new)?;
            char_bytes(&self.f002).ok_or_else(|| BleError::new("empty F002 read"))
        })
    }

    fn write_command<'a>(&'a mut self, data: &'a [u8]) -> LocalFuture<'a, Result<(), BleError>> {
        Box::pin(async move {
            let arr = js_sys::Uint8Array::from(data);
            // Prefer write-without-response (what F002 advertises), but fall back
            // to write-with-response when the peripheral/shim doesn't offer it —
            // iOS WebBLE often lacks write-without-response. Mirrors index.html.
            let p = if self.f002.properties().write_without_response() {
                self.f002.write_value_without_response_with_u8_array(&arr)
            } else {
                self.f002.write_value_with_response_with_u8_array(&arr)
            }
            .map_err(|e| BleError::new(js_err(e)))?;
            jsf(p).await.map_err(BleError::new)?;
            Ok(())
        })
    }

    fn next_notification(&mut self, timeout_ms: u32) -> LocalFuture<'_, Option<Vec<u8>>> {
        Box::pin(async move {
            let timeout = gloo_timers::future::TimeoutFuture::new(timeout_ms);
            match select(self.rx.next(), timeout).await {
                Either::Left((v, _)) => v,
                Either::Right(_) => None,
            }
        })
    }
}
