//! Minimal, dependency-free UTC date math + RFC 3339 (the only timestamp format
//! the app stores). We avoid a date crate so the core stays portable to wasm
//! with no platform time dependency; *local-time* formatting is the platform's
//! job (it supplies the UTC offset).

/// Civil (Y, M, D) -> days since the Unix epoch. Howard Hinnant's algorithm.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Days since the Unix epoch -> civil (Y, M, D).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// A broken-down UTC timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    pub ms: u32,
}

/// Convert a broken-down UTC time to epoch milliseconds.
pub fn civil_to_epoch_ms(c: Civil) -> i64 {
    let days = days_from_civil(c.year, c.month as i64, c.day as i64);
    let secs = days * 86400 + c.hour as i64 * 3600 + c.min as i64 * 60 + c.sec as i64;
    secs * 1000 + c.ms as i64
}

/// Convert epoch milliseconds to a broken-down UTC time.
pub fn epoch_ms_to_civil(ms: i64) -> Civil {
    let mut secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000) as u32;
    let days = secs.div_euclid(86400);
    secs = secs.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    Civil {
        year,
        month,
        day,
        hour: (secs / 3600) as u32,
        min: (secs % 3600 / 60) as u32,
        sec: (secs % 60) as u32,
        ms: millis,
    }
}

/// Format epoch milliseconds as a UTC RFC 3339 string, `YYYY-MM-DDTHH:MM:SS.sssZ`
/// (the same shape JavaScript's `Date.toISOString()` produces).
pub fn epoch_ms_to_rfc3339(ms: i64) -> String {
    let c = epoch_ms_to_civil(ms);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        c.year, c.month, c.day, c.hour, c.min, c.sec, c.ms
    )
}

/// Parse an RFC 3339 timestamp to epoch milliseconds. Accepts a trailing `Z` or
/// a numeric `±HH:MM` offset, and optional fractional seconds (any precision).
pub fn rfc3339_to_epoch_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let min = num(14, 16)?;
    let sec = num(17, 19)?;

    let mut ms = 0i64;
    let mut i = 19;
    if b.get(19) == Some(&b'.') {
        let start = 20;
        let mut end = start;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
        // Use the first 3 fractional digits as milliseconds.
        let frac = &s[start..end];
        let take = frac.len().min(3);
        if take > 0 {
            let mut v: i64 = frac[..take].parse().ok()?;
            for _ in take..3 {
                v *= 10;
            }
            ms = v;
        }
        i = end;
    }

    // Timezone offset.
    let mut offset_min = 0i64;
    match b.get(i) {
        Some(b'Z') | Some(b'z') | None => {}
        Some(&sign @ (b'+' | b'-')) => {
            let oh: i64 = s.get(i + 1..i + 3)?.parse().ok()?;
            let om: i64 = s.get(i + 4..i + 6)?.parse().ok()?;
            offset_min = oh * 60 + om;
            if sign == b'-' {
                offset_min = -offset_min;
            }
        }
        _ => return None,
    }

    let utc = civil_to_epoch_ms(Civil {
        year,
        month: month as u32,
        day: day as u32,
        hour: hour as u32,
        min: min as u32,
        sec: sec as u32,
        ms: ms as u32,
    });
    Some(utc - offset_min * 60_000)
}

/// `HH:MM` for an epoch-ms instant at a given UTC offset (minutes, east-positive).
pub fn format_hm(ms: i64, offset_min: i32) -> String {
    let c = epoch_ms_to_civil(ms + offset_min as i64 * 60_000);
    format!("{:02}:{:02}", c.hour, c.min)
}

/// A friendly local date-time, e.g. `2026-06-25 14:03`, at the given offset.
pub fn format_local(ms: i64, offset_min: i32) -> String {
    let c = epoch_ms_to_civil(ms + offset_min as i64 * 60_000);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.min
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_epoch() {
        // 2026-06-29T18:37:25.123Z
        let ms = 1_782_758_245_123;
        let c = epoch_ms_to_civil(ms);
        assert_eq!(c.year, 2026);
        assert_eq!(c.month, 6);
        assert_eq!(c.day, 29);
        assert_eq!((c.hour, c.min, c.sec, c.ms), (18, 37, 25, 123));
        assert_eq!(civil_to_epoch_ms(c), ms);
    }

    #[test]
    fn rfc3339_format_and_parse() {
        let ms = 1_700_000_000_000; // 2023-11-14T22:13:20.000Z
        let s = epoch_ms_to_rfc3339(ms);
        assert_eq!(s, "2023-11-14T22:13:20.000Z");
        assert_eq!(rfc3339_to_epoch_ms(&s), Some(ms));
    }

    #[test]
    fn parse_with_offset_and_fraction() {
        // Same instant expressed in UTC and in +02:00.
        let z = rfc3339_to_epoch_ms("2024-01-02T03:04:05.678Z").unwrap();
        let off = rfc3339_to_epoch_ms("2024-01-02T05:04:05.678+02:00").unwrap();
        assert_eq!(z, off);
        // Fractional seconds truncated/padded to ms.
        assert_eq!(
            rfc3339_to_epoch_ms("2024-01-02T03:04:05Z"),
            Some(z - 678)
        );
    }

    #[test]
    fn epoch_zero() {
        assert_eq!(epoch_ms_to_rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_to_epoch_ms("1970-01-01T00:00:00Z"), Some(0));
    }

    #[test]
    fn local_formatting() {
        let ms = 1_700_000_000_000; // 22:13 UTC
        assert_eq!(format_hm(ms, 0), "22:13");
        assert_eq!(format_hm(ms, -300), "17:13"); // UTC-5
        assert_eq!(format_hm(ms, 60), "23:13"); // UTC+1
    }
}
