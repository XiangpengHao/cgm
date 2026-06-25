//! DevComm2 packet framing and the reconnect handshake.
//!
//! Every command/response is one AES-128-CFB128 block of
//! `[op][payload][CRC16-CCITT LE]`, keyed by the per-connection session key with
//! the serial-derived IV. See `PROTOCOL.md` §5–§7.

use crate::crypto::{cfb128, crc16_ccitt, crc8_maxim};

/// On-wire opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    DeviceInfo = 0x10,
    /// Current glucose / broadcast record.
    Broadcast = 0x11,
    /// Activate a fresh sensor. **Irreversible.**
    NewSensor = 0x20,
    /// Sensor start time.
    StartTime = 0x21,
    /// Available `[minIdx..maxIdx]` history window.
    HistoryRange = 0x22,
    /// Stored per-minute history starting at an index.
    Histories = 0x23,
}

impl Op {
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Read-only opcodes that the app may issue freely. The single
    /// state-changing opcode (`NewSensor`) is deliberately excluded and must go
    /// through the gated activation path.
    pub const READ_ONLY: [Op; 5] = [
        Op::DeviceInfo,
        Op::Broadcast,
        Op::StartTime,
        Op::HistoryRange,
        Op::Histories,
    ];

    pub fn is_read_only(self) -> bool {
        Self::READ_ONLY.contains(&self)
    }
}

/// A decoded DevComm2 frame: an opcode and its payload (CRC already verified).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub op: u8,
    pub payload: Vec<u8>,
}

/// Errors decoding the reconnect handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeError {
    /// The challenge read from `F002` was not 17 bytes.
    BadChallengeLength(usize),
    /// CRC-8 over the session key did not match — wrong serial or pair key.
    SessionKeyCrc,
}

impl core::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HandshakeError::BadChallengeLength(n) => {
                write!(f, "expected a 17-byte challenge, got {n}")
            }
            HandshakeError::SessionKeyCrc => {
                write!(f, "session-key CRC mismatch (wrong serial or pair key?)")
            }
        }
    }
}

/// Decrypt the 17-byte reconnect challenge into the 16-byte session key,
/// verifying the trailing CRC-8/MAXIM check byte.
pub fn session_key_from_challenge(
    challenge: &[u8],
    pair_key: &[u8; 16],
    iv: &[u8; 16],
) -> Result<[u8; 16], HandshakeError> {
    if challenge.len() != 17 {
        return Err(HandshakeError::BadChallengeLength(challenge.len()));
    }
    let plain = cfb128(pair_key, iv, challenge, true);
    let mut key = [0u8; 16];
    key.copy_from_slice(&plain[..16]);
    if crc8_maxim(&key) != plain[16] {
        return Err(HandshakeError::SessionKeyCrc);
    }
    Ok(key)
}

/// Encode a command: `[op][payload][CRC16 LE]`, AES-CFB128 encrypted.
pub fn encode_command(op: u8, payload: &[u8], session_key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + payload.len() + 2);
    frame.push(op);
    frame.extend_from_slice(payload);
    let crc = crc16_ccitt(&frame);
    frame.push((crc & 0xff) as u8);
    frame.push((crc >> 8) as u8);
    cfb128(session_key, iv, &frame, false)
}

/// Decode a response frame, returning `None` if it is too short or the CRC fails.
pub fn decode_frame(data: &[u8], session_key: &[u8; 16], iv: &[u8; 16]) -> Option<Frame> {
    if data.len() < 3 {
        return None;
    }
    let plain = cfb128(session_key, iv, data, true);
    let n = plain.len();
    let got = plain[n - 2] as u16 | ((plain[n - 1] as u16) << 8);
    if got != crc16_ccitt(&plain[..n - 2]) {
        return None;
    }
    Some(Frame {
        op: plain[0],
        payload: plain[1..n - 2].to_vec(),
    })
}

/// Wall-clock fields for the `NewSensor` (`0x20`) datetime payload. The caller
/// supplies *local* time plus the timezone/DST offsets in quarter-hours (the
/// platform knows the local zone; the core does not).
#[derive(Debug, Clone, Copy)]
pub struct NewSensorTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub min: u8,
    pub sec: u8,
    /// Standard UTC offset in quarter-hours (e.g. UTC-6 → -24).
    pub tz_quarter_hours: i8,
    /// DST offset in quarter-hours (e.g. +1h → 4).
    pub dst_quarter_hours: i8,
}

/// Build the 9-byte `NewSensor` payload (year u16 LE, month, day, hour, min,
/// sec, tz i8, dst i8).
pub fn new_sensor_payload(t: NewSensorTime) -> [u8; 9] {
    [
        (t.year & 0xff) as u8,
        (t.year >> 8) as u8,
        t.month,
        t.day,
        t.hour,
        t.min,
        t.sec,
        t.tz_quarter_hours as u8,
        t.dst_quarter_hours as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::derive_iv;

    #[test]
    fn command_round_trips_through_decode() {
        let iv = derive_iv("EXAMPLE001").unwrap();
        let sk = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
            0xEE, 0xFF,
        ];
        let packet = encode_command(Op::Histories.code(), &[0x10, 0x00], &sk, &iv);
        let frame = decode_frame(&packet, &sk, &iv).expect("decodes");
        assert_eq!(frame.op, 0x23);
        assert_eq!(frame.payload, vec![0x10, 0x00]);
    }

    #[test]
    fn corrupt_frame_rejected() {
        let iv = derive_iv("EXAMPLE001").unwrap();
        let sk = [1u8; 16];
        let mut packet = encode_command(Op::Broadcast.code(), &[], &sk, &iv);
        packet[0] ^= 0xff;
        assert!(decode_frame(&packet, &sk, &iv).is_none());
    }

    #[test]
    fn handshake_bad_length() {
        let iv = [0u8; 16];
        let key = [0u8; 16];
        assert_eq!(
            session_key_from_challenge(&[0u8; 16], &key, &iv),
            Err(HandshakeError::BadChallengeLength(16))
        );
    }

    #[test]
    fn new_sensor_payload_layout() {
        let p = new_sensor_payload(NewSensorTime {
            year: 2026,
            month: 6,
            day: 25,
            hour: 14,
            min: 3,
            sec: 9,
            tz_quarter_hours: -24,
            dst_quarter_hours: 4,
        });
        assert_eq!(p[0], (2026u16 & 0xff) as u8);
        assert_eq!(p[1], (2026u16 >> 8) as u8);
        assert_eq!(&p[2..7], &[6, 25, 14, 3, 9]);
        assert_eq!(p[7] as i8, -24);
        assert_eq!(p[8] as i8, 4);
    }

    #[test]
    fn only_reads_are_allowlisted() {
        assert!(Op::Broadcast.is_read_only());
        assert!(Op::Histories.is_read_only());
        assert!(!Op::NewSensor.is_read_only());
    }
}
