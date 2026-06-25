//! The iOS `Platform`: wires file storage, the system clock/timezone,
//! CoreBluetooth, and Apple Health into the handle the shared UI consumes.

use crate::ble::CoreBluetoothBle;
use crate::files::IosFiles;
use crate::storage::FileStorage;
use cgm_core::datetime::epoch_ms_to_civil;
use cgm_core::engine::LocalFuture;
use cgm_core::protocol::NewSensorTime;
use cgm_core::store::Storage;
use cgm_ui::platform::{Ble, Clock, Files, Platform};
use objc2_foundation::{NSDate, NSTimeZone, NSUUID};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A runtime-agnostic sleep (a background thread fulfils a oneshot), so it works
/// regardless of the async executor Dioxus mobile uses.
pub fn sleep_future(ms: u32) -> LocalFuture<'static, ()> {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
        let _ = tx.send(());
    });
    Box::pin(async move {
        let _ = rx.await;
    })
}

pub struct IosPlatform {
    storage: Rc<dyn Storage>,
    clock: IosClock,
    ble: CoreBluetoothBle,
    files: IosFiles,
}

impl IosPlatform {
    pub fn new() -> Self {
        IosPlatform {
            storage: Rc::new(FileStorage::load()),
            clock: IosClock,
            ble: CoreBluetoothBle,
            files: IosFiles,
        }
    }
}

impl Default for IosPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for IosPlatform {
    fn storage(&self) -> Rc<dyn Storage> {
        self.storage.clone()
    }
    fn clock(&self) -> &dyn Clock {
        &self.clock
    }
    fn ble(&self) -> &dyn Ble {
        &self.ble
    }
    fn files(&self) -> &dyn Files {
        &self.files
    }

    fn new_id(&self) -> String {
        unsafe { NSUUID::UUID().UUIDString() }.to_string()
    }

    fn label(&self) -> String {
        "iOS · CoreBluetooth + Apple Health".into()
    }
}

struct IosClock;

fn now_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Clock for IosClock {
    fn now_ms(&self) -> i64 {
        now_ms_now()
    }

    fn local_offset_minutes(&self) -> i32 {
        unsafe {
            let tz = NSTimeZone::localTimeZone();
            let date = NSDate::now();
            (tz.secondsFromGMTForDate(&date) / 60) as i32
        }
    }

    fn naive_local_to_epoch_ms(&self, y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        // Interpret the fields as UTC then subtract the local offset. (DST edge
        // cases use the current offset, which is fine for a sensor-start stamp.)
        let utc = cgm_core::datetime::civil_to_epoch_ms(cgm_core::datetime::Civil {
            year: y as i64,
            month: mo,
            day: d,
            hour: h,
            min: mi,
            sec: s,
            ms: 0,
        });
        utc - self.local_offset_minutes() as i64 * 60_000
    }

    fn new_sensor_time(&self) -> NewSensorTime {
        unsafe {
            let tz = NSTimeZone::localTimeZone();
            let date = NSDate::now();
            let total = tz.secondsFromGMTForDate(&date) as f64;
            let dst = tz.daylightSavingTimeOffsetForDate(&date);
            let std = total - dst;
            let local = epoch_ms_to_civil(now_ms_now() + (total as i64) * 1000);
            NewSensorTime {
                year: local.year as u16,
                month: local.month as u8,
                day: local.day as u8,
                hour: local.hour as u8,
                min: local.min as u8,
                sec: local.sec as u8,
                tz_quarter_hours: (std / 900.0).round() as i8,
                dst_quarter_hours: (dst / 900.0).round() as i8,
            }
        }
    }

    fn sleep(&self, ms: u32) -> LocalFuture<'static, ()> {
        sleep_future(ms)
    }
}
