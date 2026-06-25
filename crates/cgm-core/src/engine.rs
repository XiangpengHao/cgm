//! The protocol engine: a thin, platform-agnostic [`BleBackend`] byte pipe plus
//! the connect → handshake → read/sync orchestration on top of it. All crypto
//! and decoding lives in the core, so the web (Web Bluetooth) and iOS
//! (CoreBluetooth) backends only have to move raw bytes.
//!
//! [`BleBackend`] is object-safe (its async methods return boxed local futures)
//! so the UI can hold a `Box<dyn BleBackend>` without making every component
//! generic over the transport.

use crate::crypto::derive_iv;
use crate::glucose::{
    Broadcast, HistoryBlock, SensorState, StartTime, decode_broadcast, decode_history_block,
    decode_history_range, decode_start_time, split_word,
};
use crate::model::Record;
use crate::protocol::{
    Frame, HandshakeError, Op, decode_frame, encode_command, session_key_from_challenge,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

/// A future that need not be `Send` (web/iOS run single-threaded).
pub type LocalFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// A transport error, carried as a human-readable message.
#[derive(Debug, Clone)]
pub struct BleError(pub String);

impl BleError {
    pub fn new(msg: impl Into<String>) -> Self {
        BleError(msg.into())
    }
}

impl core::fmt::Display for BleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A raw BLE byte pipe for the DevComm2 `F002` channel. The contract matches
/// what both Web Bluetooth and CoreBluetooth expose:
///
/// * [`read_value`](BleBackend::read_value) — a GATT *read* of `F002` (the
///   17-byte reconnect challenge).
/// * [`write_command`](BleBackend::write_command) — a GATT *write* of an
///   encrypted command packet to `F002`.
/// * [`next_notification`](BleBackend::next_notification) — the next `F002`
///   notification, or `None` on timeout.
pub trait BleBackend {
    fn read_value(&mut self) -> LocalFuture<'_, Result<Vec<u8>, BleError>>;
    fn write_command<'a>(&'a mut self, data: &'a [u8]) -> LocalFuture<'a, Result<(), BleError>>;
    fn next_notification(&mut self, timeout_ms: u32) -> LocalFuture<'_, Option<Vec<u8>>>;
}

/// An established session: the serial-derived IV and the per-connection key.
#[derive(Debug, Clone, Copy)]
pub struct Connection {
    pub iv: [u8; 16],
    pub session_key: [u8; 16],
}

/// Engine-level errors.
#[derive(Debug)]
pub enum EngineError {
    /// Serial number is not valid base-36.
    BadSerial,
    /// The reconnect handshake failed.
    Handshake(HandshakeError),
    /// The underlying transport errored.
    Backend(BleError),
}

impl core::fmt::Display for EngineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EngineError::BadSerial => write!(f, "invalid serial number"),
            EngineError::Handshake(e) => write!(f, "{e}"),
            EngineError::Backend(e) => write!(f, "{e}"),
        }
    }
}

/// Default per-command response timeout (the device answers promptly).
pub const COMMAND_TIMEOUT_MS: u32 = 5000;

/// Read the reconnect challenge from `F002` and derive the session key.
pub async fn handshake(
    backend: &mut dyn BleBackend,
    serial: &str,
    pair_key: &[u8; 16],
) -> Result<Connection, EngineError> {
    let iv = derive_iv(serial).ok_or(EngineError::BadSerial)?;
    let challenge = backend.read_value().await.map_err(EngineError::Backend)?;
    let session_key =
        session_key_from_challenge(&challenge, pair_key, &iv).map_err(EngineError::Handshake)?;
    Ok(Connection { iv, session_key })
}

/// Issue one command and wait for the matching response, ignoring stray frames.
/// Only the read-only opcodes are reachable here; the single state-changing
/// opcode goes through [`activate_sensor`].
pub async fn command(
    backend: &mut dyn BleBackend,
    conn: &Connection,
    op: Op,
    payload: &[u8],
) -> Option<Frame> {
    debug_assert!(op.is_read_only(), "use activate_sensor for state changes");
    raw_command(backend, conn, op.code(), payload).await
}

async fn raw_command(
    backend: &mut dyn BleBackend,
    conn: &Connection,
    op: u8,
    payload: &[u8],
) -> Option<Frame> {
    let packet = encode_command(op, payload, &conn.session_key, &conn.iv);
    backend.write_command(&packet).await.ok()?;
    // The device echoes the request opcode; skip a few unrelated frames.
    for _ in 0..8 {
        let raw = backend.next_notification(COMMAND_TIMEOUT_MS).await?;
        if let Some(frame) = decode_frame(&raw, &conn.session_key, &conn.iv)
            && frame.op == op
        {
            return Some(frame);
        }
    }
    None
}

fn result_body(frame: &Frame) -> Option<&[u8]> {
    // Most responses are `[result=01][body...]`.
    match frame.payload.split_first() {
        Some((1, body)) => Some(body),
        _ => None,
    }
}

/// Read the current broadcast / glucose record (`0x11`).
pub async fn broadcast(backend: &mut dyn BleBackend, conn: &Connection) -> Option<Broadcast> {
    let frame = command(backend, conn, Op::Broadcast, &[]).await?;
    decode_broadcast(result_body(&frame)?)
}

/// Read the sensor start time (`0x21`).
pub async fn start_time(backend: &mut dyn BleBackend, conn: &Connection) -> Option<StartTime> {
    let frame = command(backend, conn, Op::StartTime, &[]).await?;
    decode_start_time(&frame.payload)
}

/// Read the available `[min, max]` history index window (`0x22`).
pub async fn history_range(backend: &mut dyn BleBackend, conn: &Connection) -> Option<(u16, u16)> {
    let frame = command(backend, conn, Op::HistoryRange, &[]).await?;
    decode_history_range(&frame.payload)
}

/// Read one stored history block starting at `index` (`0x23`).
pub async fn history_block(
    backend: &mut dyn BleBackend,
    conn: &Connection,
    index: u16,
) -> Option<HistoryBlock> {
    let frame = command(backend, conn, Op::Histories, &index.to_le_bytes()).await?;
    decode_history_block(&frame.payload)
}

/// Backfill the missing records in `[min, max]` into `records`, fetching only
/// the gaps. Returns how many records were added. Mirrors the web app's
/// incremental sync.
pub async fn backfill(
    backend: &mut dyn BleBackend,
    conn: &Connection,
    records: &mut BTreeMap<u32, Record>,
    min: u16,
    max: u16,
) -> usize {
    let mut missing: Vec<u32> = (min as u32..=max as u32)
        .filter(|i| !records.contains_key(i))
        .collect();
    let mut added = 0;
    let mut guard = 0;
    while !missing.is_empty() && guard < 200 {
        guard += 1;
        let start_i = missing[0];
        let Some(block) = history_block(backend, conn, start_i as u16).await else {
            break;
        };
        if block.words.is_empty() {
            break;
        }
        for (i, &w) in block.words.iter().enumerate() {
            let idx = block.start as u32 + i as u32;
            if idx >= min as u32 && idx <= max as u32 && records.insert(idx, split_word(w)).is_none()
            {
                added += 1;
            }
        }
        let next = block.start as u32 + block.words.len() as u32;
        missing.retain(|&i| i >= next);
        if next <= start_i {
            break;
        }
    }
    added
}

/// Outcome of attempting to activate a sensor.
#[derive(Debug)]
pub enum ActivateError {
    /// The sensor is not a fresh NEW/USED unit — refusing to clobber a running
    /// session. Carries the current state.
    NotFresh(SensorState),
    /// Could not read the current state to confirm it is safe.
    NoState,
    /// The device did not acknowledge the activation.
    NoAck,
}

impl core::fmt::Display for ActivateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ActivateError::NotFresh(s) => write!(
                f,
                "sensor is {} — only a fresh NEW/USED sensor can be started",
                s.label()
            ),
            ActivateError::NoState => write!(f, "could not read sensor state"),
            ActivateError::NoAck => write!(f, "device did not acknowledge activation"),
        }
    }
}

/// Activate a fresh sensor (`0x20`). **Irreversible.** This is the only
/// state-changing opcode and is gated: it first confirms the sensor reports
/// `NEW/USED`, then sends the 9-byte datetime payload (built by the platform
/// from local time). Mirrors the web app's confirmation-gated `tryActivate`.
pub async fn activate_sensor(
    backend: &mut dyn BleBackend,
    conn: &Connection,
    payload: &[u8; 9],
) -> Result<(), ActivateError> {
    let bc = broadcast(backend, conn).await.ok_or(ActivateError::NoState)?;
    if bc.state() != SensorState::NewOrUsed {
        return Err(ActivateError::NotFresh(bc.state()));
    }
    let frame = raw_command(backend, conn, Op::NewSensor.code(), payload)
        .await
        .ok_or(ActivateError::NoAck)?;
    if frame.payload.first() == Some(&1) {
        Ok(())
    } else {
        Err(ActivateError::NoAck)
    }
}

/// Merge a broadcast's readings into the record store (used by the live poll).
pub fn merge_broadcast(records: &mut BTreeMap<u32, Record>, bc: &Broadcast) {
    for r in &bc.readings {
        if r.index > 0 {
            records.insert(r.index as u32, (r.mgdl, r.rec_status));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{cfb128, crc8_maxim};
    use std::collections::VecDeque;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    /// Poll a future to completion; the mock backend never yields `Pending`.
    fn block_on<F: Future>(mut fut: F) -> F::Output {
        fn vt() -> RawWaker {
            RawWaker::new(core::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(|_| vt(), |_| {}, |_| {}, |_| {});
        let waker = unsafe { Waker::from_raw(vt()) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    /// A scripted device: serves a fixed challenge, then answers each command
    /// with a canned `[result=1][body]` (history blocks computed from glucose).
    struct MockDevice {
        iv: [u8; 16],
        sk: [u8; 16],
        notifications: VecDeque<Vec<u8>>,
        glucose: Vec<u16>,
        min: u16,
        max: u16,
    }

    impl MockDevice {
        fn new(serial: &str, sk: [u8; 16], glucose: Vec<u16>) -> Self {
            let iv = derive_iv(serial).unwrap();
            let max = (glucose.len() as u16).saturating_sub(1);
            MockDevice {
                iv,
                sk,
                notifications: VecDeque::new(),
                glucose,
                min: 0,
                max,
            }
        }
        fn challenge(&self) -> Vec<u8> {
            let mut plain = self.sk.to_vec();
            plain.push(crc8_maxim(&self.sk));
            cfb128(&self.sk, &self.iv, &plain, false)
        }
        fn reply(&self, op: u8, payload: &[u8]) -> Vec<u8> {
            let body: Vec<u8> = match op {
                x if x == Op::StartTime.code() => vec![1, 0xEA, 0x07, 6, 25, 0, 0, 0, 0],
                x if x == Op::HistoryRange.code() => {
                    let mut b = vec![1];
                    b.extend_from_slice(&self.min.to_le_bytes());
                    b.extend_from_slice(&self.max.to_le_bytes());
                    b
                }
                x if x == Op::Histories.code() => {
                    let start = u16::from_le_bytes([payload[0], payload[1]]);
                    let mut b = vec![1];
                    b.extend_from_slice(&start.to_le_bytes());
                    for w in &self.glucose[start as usize..] {
                        b.extend_from_slice(&w.to_le_bytes());
                    }
                    b
                }
                _ => vec![1],
            };
            encode_command(op, &body, &self.sk, &self.iv)
        }
    }

    impl BleBackend for MockDevice {
        fn read_value(&mut self) -> LocalFuture<'_, Result<Vec<u8>, BleError>> {
            let c = self.challenge();
            Box::pin(async move { Ok(c) })
        }
        fn write_command<'a>(
            &'a mut self,
            data: &'a [u8],
        ) -> LocalFuture<'a, Result<(), BleError>> {
            Box::pin(async move {
                let frame =
                    decode_frame(data, &self.sk, &self.iv).ok_or(BleError::new("bad frame"))?;
                let reply = self.reply(frame.op, &frame.payload);
                self.notifications.push_back(reply);
                Ok(())
            })
        }
        fn next_notification(&mut self, _timeout_ms: u32) -> LocalFuture<'_, Option<Vec<u8>>> {
            let n = self.notifications.pop_front();
            Box::pin(async move { n })
        }
    }

    #[test]
    fn handshake_then_backfill() {
        let sk = [7u8; 16];
        let glucose: Vec<u16> = (0..130u16).map(|i| 80 + i).collect();
        let mut dev = MockDevice::new("EXAMPLE001", sk, glucose);

        let conn = block_on(handshake(&mut dev, "EXAMPLE001", &sk)).unwrap();
        let (min, max) = block_on(history_range(&mut dev, &conn)).unwrap();
        assert_eq!((min, max), (0, 129));

        let mut records: BTreeMap<u32, Record> = BTreeMap::new();
        let added = block_on(backfill(&mut dev, &conn, &mut records, min, max));
        assert_eq!(added, 130);
        assert_eq!(records.len(), 130);
        assert_eq!(records[&100], (180, 0));

        let st = block_on(start_time(&mut dev, &conn)).unwrap();
        assert_eq!((st.year, st.month, st.day), (2026, 6, 25));
    }

    #[test]
    fn backfill_only_fetches_gaps() {
        let sk = [3u8; 16];
        let glucose: Vec<u16> = (0..50u16).map(|i| 90 + i).collect();
        let mut dev = MockDevice::new("EXAMPLE001", sk, glucose);
        let conn = block_on(handshake(&mut dev, "EXAMPLE001", &sk)).unwrap();
        let mut records: BTreeMap<u32, Record> = BTreeMap::new();
        for i in 0..49u32 {
            records.insert(i, (1, 0));
        }
        let added = block_on(backfill(&mut dev, &conn, &mut records, 0, 49));
        assert_eq!(added, 1);
        assert_eq!(records[&49], (90 + 49, 0));
        assert_eq!(records[&0], (1, 0));
    }
}
