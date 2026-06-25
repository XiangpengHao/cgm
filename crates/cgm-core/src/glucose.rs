//! Decoding glucose from broadcast records, stored history, and advertisements,
//! plus the sensor-state and validity rules. See `PROTOCOL.md` §9–§10.

/// One decoded glucose reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub mgdl: u16,
    pub rec_status: u8,
    pub quality: u8,
    /// Sensor-minute index (minutes since sensor start).
    pub index: i32,
}

impl Reading {
    pub fn mmol(&self) -> f64 {
        self.mgdl as f64 / 18.0
    }
}

/// Lifecycle state derived from the broadcast status bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorState {
    /// Fresh transmitter that has never been started.
    NewOrUsed,
    /// Session ended.
    Expired,
    /// Started, still in the ~60-minute warmup.
    WarmingUp,
    /// Producing readings.
    Active,
}

impl SensorState {
    pub fn label(self) -> &'static str {
        match self {
            SensorState::NewOrUsed => "NEW/USED",
            SensorState::Expired => "EXPIRED",
            SensorState::WarmingUp => "WARMING-UP",
            SensorState::Active => "ACTIVE",
        }
    }
}

/// The decoded broadcast body (advertisement, or the `0x11` response body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broadcast {
    pub time_offset_min: i32,
    pub status: u8,
    pub cal_temp: u8,
    pub trend_mgdl_min: i8,
    pub cal_index: u16,
    pub readings: Vec<Reading>,
}

impl Broadcast {
    pub fn state(&self) -> SensorState {
        let s0 = self.status & 1 != 0;
        let c0 = self.cal_temp & 1 != 0;
        if s0 && c0 {
            SensorState::NewOrUsed
        } else if s0 && !c0 {
            SensorState::Expired
        } else if self.time_offset_min < 60 {
            SensorState::WarmingUp
        } else {
            SensorState::Active
        }
    }

    /// The most-recent reading.
    pub fn current(&self) -> Option<&Reading> {
        self.readings.first()
    }

    /// Whether the current reading is a real, trustworthy glucose value (mirrors
    /// the official app's `TransmitterModel`): warmup done, status clean, record
    /// status clean, in-range, no malfunction bits.
    pub fn current_is_valid(&self) -> bool {
        match self.current() {
            Some(r) => {
                self.time_offset_min >= 60
                    && (self.status & 0x3f) == 0
                    && r.rec_status == 0
                    && r.mgdl > 0
                    && r.mgdl < 0x3ff
            }
            None => false,
        }
    }
}

fn u16le(b: &[u8], o: usize) -> u16 {
    b[o] as u16 | ((b[o + 1] as u16) << 8)
}

/// Decode a broadcast body. Returns `None` if the length is implausible.
pub fn decode_broadcast(body: &[u8]) -> Option<Broadcast> {
    if body.len() < 10 || !(body.len() - 7).is_multiple_of(3) {
        return None;
    }
    let time_offset = u16le(body, 0) as i32;
    let mut readings = Vec::new();
    let n = (body.len() - 7) / 3;
    for i in 0..n {
        let off = 5 + i * 3;
        let word = u16le(body, off);
        if word == 0xffff {
            continue;
        }
        readings.push(Reading {
            mgdl: word & 0x3ff,
            rec_status: ((word >> 10) & 3) as u8,
            quality: body[off + 2],
            index: time_offset - i as i32,
        });
    }
    Some(Broadcast {
        time_offset_min: time_offset,
        status: body[2],
        cal_temp: body[3],
        trend_mgdl_min: body[4] as i8,
        cal_index: u16le(body, body.len() - 2),
        readings,
    })
}

/// Decode an AiDEX BLE advertisement manufacturer payload (company `0x0059`),
/// returning the broadcast plus the `native_paired` / `aes_init` flag bits.
pub fn decode_advertisement(mfr: &[u8]) -> Option<(Broadcast, Option<bool>, Option<bool>)> {
    if mfr.len() < 20 {
        return None;
    }
    let bc = decode_broadcast(&mfr[..16])?;
    let (paired, aes_init) = if mfr.len() > 20 {
        let flags = mfr[20];
        (Some(flags & 0x01 != 0), Some(flags & 0x02 != 0))
    } else {
        (None, None)
    };
    Some((bc, paired, aes_init))
}

/// Sensor start time, as the device's *local* wall-clock fields (`0x21`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub min: u8,
    pub sec: u8,
}

/// Decode the `getStartTime` (`0x21`) response.
pub fn decode_start_time(p: &[u8]) -> Option<StartTime> {
    if p.len() < 9 || p[0] != 1 {
        return None;
    }
    Some(StartTime {
        year: u16le(p, 1),
        month: p[3],
        day: p[4],
        hour: p[5],
        min: p[6],
        sec: p[7],
    })
}

/// Decode `getHistoryRange` (`0x22`) into `(min_idx, max_idx)`; `max` is the
/// current sensor minute.
pub fn decode_history_range(p: &[u8]) -> Option<(u16, u16)> {
    if p.len() < 5 || p[0] != 1 {
        return None;
    }
    Some((u16le(p, 1), u16le(p, p.len() - 2)))
}

/// A contiguous block of stored history words starting at `start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryBlock {
    pub start: u16,
    pub words: Vec<u16>,
}

/// Decode the `getHistories` (`0x23`) response.
pub fn decode_history_block(p: &[u8]) -> Option<HistoryBlock> {
    if p.len() < 3 || p[0] != 1 {
        return None;
    }
    let start = u16le(p, 1);
    let count = (p.len() - 3) / 2;
    let words = (0..count).map(|i| u16le(p, 3 + 2 * i)).collect();
    Some(HistoryBlock { start, words })
}

/// Split a raw history word into `(mgdl, rec_status)`.
pub fn split_word(word: u16) -> (u16, u8) {
    (word & 0x3ff, ((word >> 10) & 3) as u8)
}

/// Whether a stored reading is a real, trustworthy value: record status clean,
/// in physiological range, and past the warmup window (index ≥ 60).
pub fn record_valid(mgdl: u16, rec_status: u8, index: i32) -> bool {
    rec_status == 0 && mgdl > 0 && mgdl < 0x3ff && index >= 60
}

/// Trend arrow for a mg/dL-per-minute slope (matches the app's thresholds).
pub fn trend_arrow(mgdl_min: i8) -> &'static str {
    let t = mgdl_min as i32;
    if t < -30 {
        "↓↓"
    } else if t < -20 {
        "↓"
    } else if t < -10 {
        "↘"
    } else if t < 11 {
        "→"
    } else if t < 21 {
        "↗"
    } else if t < 31 {
        "↑"
    } else {
        "↑↑"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_a_synthetic_broadcast() {
        // time_offset=120, status=0, calTemp=0, trend=+2, one reading 100 mg/dL
        // quality 88, plus a 0xFFFF (no-reading) slot, cal_index=7.
        let mut body = vec![120, 0, 0, 0, 2];
        body.extend_from_slice(&[100, 0, 88]); // word=100 -> mgdl 100, rs 0
        body.extend_from_slice(&[0xff, 0xff, 0]); // skipped
        body.extend_from_slice(&[7, 0]); // cal_index
        let bc = decode_broadcast(&body).unwrap();
        assert_eq!(bc.time_offset_min, 120);
        assert_eq!(bc.trend_mgdl_min, 2);
        assert_eq!(bc.state(), SensorState::Active);
        assert_eq!(bc.readings.len(), 1);
        let r = bc.current().unwrap();
        assert_eq!(r.mgdl, 100);
        assert_eq!(r.index, 120);
        assert_eq!(r.quality, 88);
        assert!(bc.current_is_valid());
    }

    #[test]
    fn state_transitions() {
        let mk = |status, cal, t: i32| Broadcast {
            time_offset_min: t,
            status,
            cal_temp: cal,
            trend_mgdl_min: 0,
            cal_index: 0,
            readings: vec![],
        };
        assert_eq!(mk(1, 1, 0).state(), SensorState::NewOrUsed);
        assert_eq!(mk(1, 0, 0).state(), SensorState::Expired);
        assert_eq!(mk(0, 0, 30).state(), SensorState::WarmingUp);
        assert_eq!(mk(0, 0, 120).state(), SensorState::Active);
    }

    #[test]
    fn history_decoders() {
        // start time: result=1, year=2026, month..sec
        let st = decode_start_time(&[1, 0xEA, 0x07, 6, 25, 14, 3, 9, 0]).unwrap();
        assert_eq!(st.year, 2026);
        assert_eq!((st.month, st.day, st.hour, st.min, st.sec), (6, 25, 14, 3, 9));

        // range: result=1, min=10, max=130
        let (mn, mx) = decode_history_range(&[1, 10, 0, 130, 0]).unwrap();
        assert_eq!((mn, mx), (10, 130));

        // block: result=1, start=10, words 100, 0x3FF (saturated)
        let blk = decode_history_block(&[1, 10, 0, 100, 0, 0xff, 0x03]).unwrap();
        assert_eq!(blk.start, 10);
        assert_eq!(blk.words, vec![100, 0x3ff]);
        assert_eq!(split_word(0x3ff), (0x3ff, 0));
    }

    #[test]
    fn validity_rules() {
        assert!(record_valid(100, 0, 60));
        assert!(!record_valid(100, 0, 59)); // warmup
        assert!(!record_valid(0x3ff, 0, 200)); // saturated
        assert!(!record_valid(0, 0, 200)); // zero
        assert!(!record_valid(100, 1, 200)); // bad record status
    }

    #[test]
    fn arrows() {
        assert_eq!(trend_arrow(0), "→");
        assert_eq!(trend_arrow(-35), "↓↓");
        assert_eq!(trend_arrow(40), "↑↑");
        assert_eq!(trend_arrow(15), "↗");
    }
}
