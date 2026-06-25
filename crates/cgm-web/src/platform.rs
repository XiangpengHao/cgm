//! The web `Platform`: wires localStorage, the browser clock, Web Bluetooth,
//! and file/health export into the single handle the UI consumes.

use crate::ble::WebBle;
use crate::files::WebFiles;
use crate::storage::LocalStorage;
use cgm_core::engine::LocalFuture;
use cgm_core::protocol::NewSensorTime;
use cgm_core::store::Storage;
use cgm_ui::platform::{Ble, Clock, Files, Platform};
use std::rc::Rc;

pub struct WebPlatform {
    storage: Rc<dyn Storage>,
    clock: WebClock,
    ble: WebBle,
    files: WebFiles,
}

impl WebPlatform {
    pub fn new() -> Self {
        WebPlatform {
            storage: Rc::new(LocalStorage),
            clock: WebClock,
            ble: WebBle::default(),
            files: WebFiles,
        }
    }
}

impl Default for WebPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for WebPlatform {
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
        web_sys::window()
            .and_then(|w| w.crypto().ok())
            .map(|c| c.random_uuid())
            .unwrap_or_else(|| format!("d{}", js_sys::Date::now() as u64))
    }

    fn label(&self) -> String {
        "Web Bluetooth · Chrome/Edge (desktop or Android), over https/localhost".into()
    }
}

struct WebClock;

impl Clock for WebClock {
    fn now_ms(&self) -> i64 {
        js_sys::Date::now() as i64
    }

    fn local_offset_minutes(&self) -> i32 {
        // getTimezoneOffset is minutes *behind* UTC; negate for an east-positive offset.
        -(js_sys::Date::new_0().get_timezone_offset() as i32)
    }

    fn naive_local_to_epoch_ms(&self, y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        let date = js_sys::Date::new_with_year_month_day_hr_min_sec(
            y as u32,
            mo as i32 - 1,
            d as i32,
            h as i32,
            mi as i32,
            s as i32,
        );
        date.get_time() as i64
    }

    fn new_sensor_time(&self) -> NewSensorTime {
        let now = js_sys::Date::new_0();
        let y = now.get_full_year();
        let jan = js_sys::Date::new_with_year_month_day(y, 0, 1).get_timezone_offset();
        let jul = js_sys::Date::new_with_year_month_day(y, 6, 1).get_timezone_offset();
        let std_off = jan.max(jul);
        let tz_q = (-std_off / 15.0).round() as i8;
        let dst_q = ((std_off - now.get_timezone_offset()) / 15.0).round() as i8;
        NewSensorTime {
            year: y as u16,
            month: now.get_month() as u8 + 1,
            day: now.get_date() as u8,
            hour: now.get_hours() as u8,
            min: now.get_minutes() as u8,
            sec: now.get_seconds() as u8,
            tz_quarter_hours: tz_q,
            dst_quarter_hours: dst_q,
        }
    }

    fn sleep(&self, ms: u32) -> LocalFuture<'static, ()> {
        Box::pin(async move {
            gloo_timers::future::TimeoutFuture::new(ms).await;
        })
    }
}
