//! CoreBluetooth transport for iOS: scan → connect → discover → the `F002` byte
//! pipe the engine drives, plus first-time pairing on `F001`. All protocol/crypto
//! stays in `cgm_core`; this module is the CoreBluetooth delegate plumbing.
//!
//! The `CBCentralManager` runs on the **main** dispatch queue so its delegate
//! callbacks share the Dioxus UI thread — that lets the shared state be a plain
//! `Rc<RefCell<…>>` with no locking. Callbacks forward bytes/events to the async
//! engine through `futures` channels.
//!
//! Build note: requires the iOS SDK and `NSBluetoothAlwaysUsageDescription` in
//! Info.plist (see README.md). Untested off-device.

use cgm_core::engine::{BleBackend, BleError, LocalFuture};
use cgm_ui::platform::Ble;
use futures::channel::{mpsc, oneshot};
use futures::future::{select, Either};
use futures::StreamExt;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{define_class, msg_send, DefinedClass};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicWriteType,
    CBManagerState, CBPeripheral, CBPeripheralDelegate, CBService, CBUUID,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSNumber, NSString};
use std::cell::RefCell;
use std::rc::Rc;

const SERVICE: &str = "181F";
const F001: &str = "F001";
const F002: &str = "F002";
const F003: &str = "F003";

#[derive(PartialEq)]
enum Mode {
    Idle,
    Connect,
    Pair,
}

/// Shared delegate state. Lives on the main thread (see module note).
#[derive(Default)]
struct Shared {
    mode: Mode,
    central: Option<Retained<CBCentralManager>>,
    peripheral: Option<Retained<CBPeripheral>>,
    f001: Option<Retained<CBCharacteristic>>,
    f002: Option<Retained<CBCharacteristic>>,
    // event sinks (taken when fulfilled)
    powered_on: Option<oneshot::Sender<()>>,
    connected: Option<oneshot::Sender<Result<(), String>>>,
    read_value: Option<oneshot::Sender<Vec<u8>>>,
    pair_key: Option<oneshot::Sender<Vec<u8>>>,
    notify_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Idle
    }
}

fn cbuuid(s: &str) -> Retained<CBUUID> {
    let ns = NSString::from_str(s);
    unsafe { CBUUID::UUIDWithString(&ns) }
}

fn data_bytes(c: &CBCharacteristic) -> Option<Vec<u8>> {
    let value: Option<Retained<NSData>> = unsafe { c.value() };
    value.map(|d| d.to_vec())
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "CgmBleDelegate"]
    #[ivars = Rc<RefCell<Shared>>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBCentralManagerDelegate for Delegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            let state = unsafe { central.state() };
            if state == CBManagerState::PoweredOn {
                if let Some(tx) = self.ivars().borrow_mut().powered_on.take() {
                    let _ = tx.send(());
                }
            }
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn did_discover(
            &self,
            central: &CBCentralManager,
            peripheral: &CBPeripheral,
            _adv: &NSDictionary<NSString, AnyObject>,
            _rssi: &NSNumber,
        ) {
            // First matching peripheral wins: stop scanning and connect.
            unsafe { central.stopScan() };
            let p = peripheral.retain();
            self.ivars().borrow_mut().peripheral = Some(p.clone());
            unsafe { central.connectPeripheral_options(&p, None) };
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            unsafe {
                peripheral.setDelegate(Some(self.as_ref()));
                let services = NSArray::from_retained_slice(&[cbuuid(SERVICE)]);
                peripheral.discoverServices(Some(&services));
            }
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail(&self, _c: &CBCentralManager, _p: &CBPeripheral, error: *mut NSError) {
            self.fail_connect(error_string(error, "failed to connect"));
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(&self, _c: &CBCentralManager, _p: &CBPeripheral, _error: *mut NSError) {
            // Closing the notify channel makes the engine's next_notification end.
            self.ivars().borrow_mut().notify_tx = None;
        }
    }

    unsafe impl CBPeripheralDelegate for Delegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(&self, peripheral: &CBPeripheral, error: *mut NSError) {
            if !error.is_null() {
                self.fail_connect(error_string(error, "service discovery failed"));
                return;
            }
            let services: Option<Retained<NSArray<CBService>>> = unsafe { peripheral.services() };
            if let Some(services) = services {
                for svc in services.iter() {
                    unsafe { peripheral.discoverCharacteristics_forService(None, &svc) };
                }
            }
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_chars(
            &self,
            peripheral: &CBPeripheral,
            service: &CBService,
            error: *mut NSError,
        ) {
            if !error.is_null() {
                self.fail_connect(error_string(error, "characteristic discovery failed"));
                return;
            }
            let chars: Option<Retained<NSArray<CBCharacteristic>>> =
                unsafe { service.characteristics() };
            let Some(chars) = chars else { return };

            let mode = self.ivars().borrow().mode == Mode::Pair;
            for ch in chars.iter() {
                let uuid = unsafe { ch.UUID() };
                let id = unsafe { uuid.UUIDString() }.to_string().to_uppercase();
                let id = id.trim_start_matches("0000").to_string();
                if id.starts_with(F002) {
                    self.ivars().borrow_mut().f002 = Some(ch.retain());
                    unsafe { peripheral.setNotifyValue_forCharacteristic(true, &ch) };
                } else if id.starts_with(F003) {
                    unsafe { peripheral.setNotifyValue_forCharacteristic(true, &ch) };
                } else if id.starts_with(F001) {
                    self.ivars().borrow_mut().f001 = Some(ch.retain());
                    unsafe { peripheral.setNotifyValue_forCharacteristic(true, &ch) };
                }
            }
            // Connection (or pairing) is ready once F002 (and for pairing F001) exist.
            let ready = {
                let s = self.ivars().borrow();
                s.f002.is_some() && (!mode || s.f001.is_some())
            };
            if ready {
                if let Some(tx) = self.ivars().borrow_mut().connected.take() {
                    let _ = tx.send(Ok(()));
                }
            }
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_update_value(
            &self,
            _peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            _error: *mut NSError,
        ) {
            let Some(bytes) = data_bytes(characteristic) else { return };
            let uuid = unsafe { characteristic.UUID() };
            let id = unsafe { uuid.UUIDString() }.to_string().to_uppercase();

            let mut s = self.ivars().borrow_mut();
            if id.contains(F001) {
                if bytes.len() == 16 {
                    if let Some(tx) = s.pair_key.take() {
                        let _ = tx.send(bytes);
                    }
                }
                return;
            }
            // F002: a pending read takes priority; otherwise it's a notification.
            if let Some(tx) = s.read_value.take() {
                let _ = tx.send(bytes);
            } else if let Some(tx) = &s.notify_tx {
                let _ = tx.unbounded_send(bytes);
            }
        }
    }
);

impl Delegate {
    fn fail_connect(&self, msg: String) {
        if let Some(tx) = self.ivars().borrow_mut().connected.take() {
            let _ = tx.send(Err(msg));
        }
    }
}

fn error_string(error: *mut NSError, fallback: &str) -> String {
    unsafe { error.as_ref() }
        .map(|e| e.localizedDescription().to_string())
        .unwrap_or_else(|| fallback.to_string())
}

/// Create a central manager on the main queue with our delegate.
fn make_central(shared: Rc<RefCell<Shared>>) -> (Retained<Delegate>, Retained<CBCentralManager>) {
    let delegate = Delegate::alloc().set_ivars(shared.clone());
    let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
    // queue = nil ⇒ the main dispatch queue.
    let central: Retained<CBCentralManager> = unsafe {
        let proto = Retained::cast_unchecked::<objc2::runtime::ProtocolObject<dyn CBCentralManagerDelegate>>(
            delegate.clone(),
        );
        CBCentralManager::initWithDelegate_queue(CBCentralManager::alloc(), Some(&proto), None)
    };
    shared.borrow_mut().central = Some(central.clone());
    (delegate, central)
}

async fn await_powered(shared: &Rc<RefCell<Shared>>, central: &CBCentralManager) -> Result<(), String> {
    if unsafe { central.state() } == CBManagerState::PoweredOn {
        return Ok(());
    }
    let (tx, rx) = oneshot::channel();
    shared.borrow_mut().powered_on = Some(tx);
    rx.await.map_err(|_| "bluetooth never powered on".to_string())
}

fn start_scan(central: &CBCentralManager) {
    let services = NSArray::from_retained_slice(&[cbuuid(SERVICE)]);
    unsafe { central.scanForPeripheralsWithServices_options(Some(&services), None) };
}

/// The iOS `Ble` service.
#[derive(Default)]
pub struct CoreBluetoothBle;

impl Ble for CoreBluetoothBle {
    fn available(&self) -> bool {
        true // permission is requested at scan time
    }

    fn pair(&self, secret: [u8; 16]) -> LocalFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let shared = Rc::new(RefCell::new(Shared {
                mode: Mode::Pair,
                ..Default::default()
            }));
            let (_delegate, central) = make_central(shared.clone());
            await_powered(&shared, &central).await?;

            let (ctx, crx) = oneshot::channel();
            shared.borrow_mut().connected = Some(ctx);
            start_scan(&central);
            crx.await.map_err(|_| "scan cancelled".to_string())??;

            // Subscribe for the key, then write the SN-derived secret to F001.
            let (ktx, krx) = oneshot::channel::<Vec<u8>>();
            shared.borrow_mut().pair_key = Some(ktx);
            {
                let s = shared.borrow();
                let (Some(p), Some(f001)) = (&s.peripheral, &s.f001) else {
                    return Err("F001 not found (not a pairable transmitter?)".into());
                };
                let data = NSData::with_bytes(&secret);
                unsafe {
                    p.writeValue_forCharacteristic_type(
                        &data,
                        f001,
                        CBCharacteristicWriteType::WithResponse,
                    )
                };
            }
            let key = krx.await.map_err(|_| {
                "no key returned — pairing only works on a fresh / unpaired transmitter".to_string()
            })?;
            if let Some(p) = shared.borrow().peripheral.clone() {
                unsafe { central.cancelPeripheralConnection(&p) };
            }
            Ok(key.iter().map(|b| format!("{b:02X}")).collect())
        })
    }

    fn connect(&self) -> LocalFuture<'static, Result<Box<dyn BleBackend>, String>> {
        Box::pin(async move {
            let shared = Rc::new(RefCell::new(Shared {
                mode: Mode::Connect,
                ..Default::default()
            }));
            let (delegate, central) = make_central(shared.clone());
            await_powered(&shared, &central).await?;

            let (ctx, crx) = oneshot::channel();
            shared.borrow_mut().connected = Some(ctx);
            start_scan(&central);
            crx.await.map_err(|_| "scan cancelled".to_string())??;

            let (tx, rx) = mpsc::unbounded::<Vec<u8>>();
            shared.borrow_mut().notify_tx = Some(tx);

            let peripheral = shared
                .borrow()
                .peripheral
                .clone()
                .ok_or("no peripheral")?;
            let f002 = shared.borrow().f002.clone().ok_or("F002 not found")?;

            Ok(Box::new(IosBackend {
                shared,
                central,
                peripheral,
                f002,
                rx,
                _delegate: delegate,
            }) as Box<dyn BleBackend>)
        })
    }

    fn disconnect(&self) {
        // Handled by dropping the backend (which cancels the connection).
    }
}

struct IosBackend {
    shared: Rc<RefCell<Shared>>,
    central: Retained<CBCentralManager>,
    peripheral: Retained<CBPeripheral>,
    f002: Retained<CBCharacteristic>,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    _delegate: Retained<Delegate>,
}

impl Drop for IosBackend {
    fn drop(&mut self) {
        unsafe { self.central.cancelPeripheralConnection(&self.peripheral) };
    }
}

impl BleBackend for IosBackend {
    fn read_value(&mut self) -> LocalFuture<'_, Result<Vec<u8>, BleError>> {
        Box::pin(async move {
            let (tx, rx) = oneshot::channel::<Vec<u8>>();
            self.shared.borrow_mut().read_value = Some(tx);
            unsafe { self.peripheral.readValueForCharacteristic(&self.f002) };
            rx.await.map_err(|_| BleError::new("F002 read cancelled"))
        })
    }

    fn write_command<'a>(&'a mut self, data: &'a [u8]) -> LocalFuture<'a, Result<(), BleError>> {
        Box::pin(async move {
            let nsdata = NSData::with_bytes(data);
            unsafe {
                self.peripheral.writeValue_forCharacteristic_type(
                    &nsdata,
                    &self.f002,
                    CBCharacteristicWriteType::WithoutResponse,
                )
            };
            Ok(())
        })
    }

    fn next_notification(&mut self, timeout_ms: u32) -> LocalFuture<'_, Option<Vec<u8>>> {
        Box::pin(async move {
            let timeout = crate::platform::sleep_future(timeout_ms);
            match select(self.rx.next(), timeout).await {
                Either::Left((v, _)) => v,
                Either::Right(_) => None,
            }
        })
    }
}
