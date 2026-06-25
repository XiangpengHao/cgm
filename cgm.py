#!/usr/bin/env python3
"""
cgm.py — read a MicroTech AiDEX X / GX-01S continuous glucose monitor over BLE.

Pure-Python port of the validated AiDEX protocol (see README.md):
  * BLE service 181F; F002 = encrypted DevComm2 data channel; F003 = reconnect notify.
  * IV       = MD5( (base36(snChar)*17 + 19) mod 256 )            (fixed per serial)
  * Pair key = 16 bytes issued by the device during pairing       (not derivable from SN)
  * Reconnect: read F002 -> 17-byte blob -> AES-128-CFB128 decrypt(pairKey, IV)
               -> first 16 bytes = session key, byte 16 = CRC8/MAXIM(sessionKey)
  * Command  : plaintext = [op][payload][CRC16-CCITT LE], AES-128-CFB128(sessionKey, IV)
  * Glucose  : history word (u16 LE); mg/dL = word & 0x3FF; mmol/L = mg/dL / 18; 0xFFFF = none.

NOT a medical device. Do not use for treatment decisions.

Examples
  ./cgm.py scan                      # passive: decode the broadcast glucose from advertisements
  ./cgm.py scan --watch 60           # ...repeat every 60 s
  ./cgm.py read                      # connect + handshake + read current glucose
  ./cgm.py read --json
  ./cgm.py monitor --interval 60     # log one reading per minute
  ./cgm.py info                      # device info + sensor start time
  ./cgm.py start-sensor --yes        # IRREVERSIBLE: start a new sensor session
"""
from __future__ import annotations

import argparse
import asyncio
import hashlib
import os
import re
import struct
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Optional

from Crypto.Cipher import AES
from bleak import BleakClient, BleakScanner

# ── defaults for the test unit (override on the CLI) ─────────────────────────
# No device secrets in source. Provide via --serial/--key, or env AIDEX_SERIAL / AIDEX_PAIR_KEY.
DEFAULT_SERIAL = os.environ.get("AIDEX_SERIAL", "")
DEFAULT_PAIR_KEY = os.environ.get("AIDEX_PAIR_KEY", "")
AIDEX_COMPANY_ID = 0x0059

def char_uuid(short: str) -> str:
    return f"0000{short.lower()}-0000-1000-8000-00805f9b34fb"

F001 = char_uuid("F001")   # pairing (write / notify) — used on an unpaired transmitter
F002 = char_uuid("F002")   # DevComm2 data (read / write-without-response / notify)
F003 = char_uuid("F003")   # reconnect notify

# opcodes (on-wire byte for BleController::send)
OP_DEVICE_INFO = 0x10
OP_BROADCAST = 0x11
OP_NEW_SENSOR = 0x20
OP_START_TIME = 0x21
OP_HISTORY_RANGE = 0x22
OP_HISTORIES = 0x23


# ── crypto / framing ─────────────────────────────────────────────────────────
def base36(ch: str) -> int:
    o = ord(ch)
    if 48 <= o <= 57:  return o - 48
    if 65 <= o <= 90:  return o - 55
    if 97 <= o <= 122: return o - 87
    raise ValueError(f"non-base36 serial character: {ch!r}")


def _md5_transform(serial: str, mul: int, add: int) -> bytes:
    return hashlib.md5(bytes((base36(c) * mul + add) & 0xFF for c in serial)).digest()


def derive_iv(serial: str) -> bytes:
    """Serial -> AES IV (fixed per device)."""
    return _md5_transform(serial, 17, 19)


def derive_pair_secret(serial: str) -> bytes:
    """Serial -> the value written to F001 during first-time pairing."""
    return _md5_transform(serial, 13, 61)


def crc16_ccitt(data: bytes, init: int = 0xFFFF) -> int:
    crc = init
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def crc8_maxim(data: bytes, init: int = 0) -> int:
    crc = init
    for b in data:
        crc ^= b
        for _ in range(8):
            crc = (crc >> 1) ^ 0x8C if crc & 1 else crc >> 1
    return crc & 0xFF


def _aes_cfb(key: bytes, iv: bytes):
    # CFB-128 (full-block feedback), matching CommonCrypto kCCModeCFB.
    return AES.new(key, AES.MODE_CFB, iv=iv, segment_size=128)


def session_key_from_challenge(challenge: bytes, pair_key: bytes, iv: bytes) -> bytes:
    """Decrypt the 17-byte reconnect blob -> 16-byte session key (CRC8-verified)."""
    if len(challenge) != 17:
        raise ValueError(f"expected 17-byte challenge, got {len(challenge)}")
    plain = _aes_cfb(pair_key, iv).decrypt(challenge)
    skey, check = plain[:16], plain[16]
    if crc8_maxim(skey) != check:
        raise ValueError("session-key CRC8 mismatch (wrong serial or pair key?)")
    return skey


def devcomm2_encode(op: int, payload: bytes, key: bytes, iv: bytes) -> bytes:
    frame = bytes([op]) + payload
    frame += struct.pack("<H", crc16_ccitt(frame))
    return _aes_cfb(key, iv).encrypt(frame)


def devcomm2_decode(data: bytes, key: bytes, iv: bytes) -> Optional[tuple[int, bytes]]:
    if len(data) < 3:
        return None
    plain = _aes_cfb(key, iv).decrypt(data)
    got = struct.unpack_from("<H", plain, len(plain) - 2)[0]
    if got != crc16_ccitt(plain[:-2]):
        return None
    return plain[0], plain[1:-2]


# ── broadcast / glucose decoding ─────────────────────────────────────────────
@dataclass
class Reading:
    glucose_mgdl: int
    record_status: int
    quality: int
    time_offset_min: int          # per-record minutes since sensor start

    @property
    def mmol(self) -> float:
        return self.glucose_mgdl / 18.0


@dataclass
class Broadcast:
    time_offset_min: int
    status: int
    cal_temp: int
    trend_mgdl_min: int           # signed
    cal_index: int
    readings: list[Reading] = field(default_factory=list)

    @property
    def state(self) -> str:
        s0, c0 = self.status & 1, self.cal_temp & 1
        if s0 and c0:        return "NEW/USED-SENSOR (needs start)"
        if s0 and not c0:    return "SENSOR-EXPIRED"
        if self.time_offset_min < 60: return "WARMING-UP"
        return "ACTIVE"

    @property
    def malfunctions(self) -> list[str]:
        names = {1: "malfunc1", 2: "sensorMalfunction", 3: "malfunc8",
                 4: "malfunc16", 5: "generalFault"}
        return [n for bit, n in names.items() if self.status & (1 << bit)]

    def is_valid(self, r: Reading) -> bool:
        return (self.time_offset_min >= 60 and (self.status & 0x3F) == 0
                and r.record_status == 0 and 0 < r.glucose_mgdl < 0x3FF
                and not self.malfunctions)

    @property
    def current(self) -> Optional[Reading]:
        return self.readings[0] if self.readings else None


def decode_broadcast_body(body: bytes) -> Optional[Broadcast]:
    if len(body) < 10 or (len(body) - 7) % 3 != 0:
        return None
    time_offset = struct.unpack_from("<H", body, 0)[0]
    bc = Broadcast(
        time_offset_min=time_offset,
        status=body[2], cal_temp=body[3],
        trend_mgdl_min=struct.unpack_from("<b", body, 4)[0],
        cal_index=struct.unpack_from("<H", body, len(body) - 2)[0],
    )
    for i in range((len(body) - 7) // 3):
        off = 5 + i * 3
        word = struct.unpack_from("<H", body, off)[0]
        if word == 0xFFFF:
            continue
        bc.readings.append(Reading(
            glucose_mgdl=word & 0x03FF,
            record_status=(word >> 10) & 0x03,
            quality=body[off + 2],
            time_offset_min=(time_offset - i) & 0xFFFF,
        ))
    return bc


def decode_advertisement(mfr: bytes) -> Optional[tuple[Broadcast, Optional[bool], Optional[bool]]]:
    """`mfr` = manufacturer payload for company 0x0059 (bytes after the company id)."""
    if len(mfr) < 20:
        return None
    bc = decode_broadcast_body(mfr[:16])
    if bc is None:
        return None
    flags = mfr[20] if len(mfr) > 20 else None
    native_paired = bool(flags & 0x01) if flags is not None else None
    aes_init = bool(flags & 0x02) if flags is not None else None
    return bc, native_paired, aes_init


def decode_start_time(payload: bytes) -> Optional[datetime]:
    """getStartTime (0x21) response: [result][year u16 LE][mon][day][hr][min][sec][tz][dst]."""
    if len(payload) < 9 or payload[0] != 1:
        return None
    year = struct.unpack_from("<H", payload, 1)[0]
    mon, day, hr, mi, se = payload[3], payload[4], payload[5], payload[6], payload[7]
    try:
        return datetime(year, mon, day, hr, mi, se)   # device stores local time
    except ValueError:
        return None


def decode_history_range(payload: bytes) -> Optional[tuple[int, int]]:
    """getHistoryRange (0x22) response: [result][minIdx u16]...[maxIdx u16]; maxIdx == sensor minute."""
    if len(payload) < 5 or payload[0] != 1:
        return None
    min_idx = struct.unpack_from("<H", payload, 1)[0]
    max_idx = struct.unpack_from("<H", payload, len(payload) - 2)[0]
    return min_idx, max_idx


def decode_history_block(payload: bytes) -> Optional[tuple[int, list[int]]]:
    """getHistories (0x23) response: [result][startIdx u16][glucose words u16 LE...]."""
    if len(payload) < 3 or payload[0] != 1:
        return None
    start = struct.unpack_from("<H", payload, 1)[0]
    words = [struct.unpack_from("<H", payload, 3 + 2 * i)[0] for i in range((len(payload) - 3) // 2)]
    return start, words


@dataclass
class HistoryRecord:
    index: int                       # sensor minute since start
    timestamp: Optional[datetime]
    glucose_mgdl: int
    record_status: int

    @property
    def mmol(self) -> float:
        return self.glucose_mgdl / 18.0

    @property
    def valid(self) -> bool:
        return self.record_status == 0 and 0 < self.glucose_mgdl < 0x3FF


def parse_duration_minutes(s: str) -> int:
    s = s.strip().lower()
    mult = {"m": 1, "h": 60, "d": 1440}
    if s and s[-1] in mult:
        return int(float(s[:-1]) * mult[s[-1]])
    return int(float(s))                          # bare number = minutes


def parse_when(s: str) -> datetime:
    try:
        return datetime.fromisoformat(s)
    except ValueError:
        t = datetime.strptime(s, "%H:%M")         # HH:MM -> today
        return datetime.now().replace(hour=t.hour, minute=t.minute, second=0, microsecond=0)


def new_sensor_payload(now: Optional[datetime] = None) -> bytes:
    now = (now or datetime.now()).astimezone()
    off = now.utcoffset()
    dst = now.dst() or (now - now)        # timedelta(0) if naive of dst
    raw = off - dst
    tz_q = int(raw.total_seconds() // 900)
    dst_q = int(dst.total_seconds() // 900)
    return struct.pack("<HBBBBBbb", now.year, now.month, now.day,
                       now.hour, now.minute, now.second, tz_q, dst_q)


# ── pretty printing ──────────────────────────────────────────────────────────
TREND_ARROWS = [(-30, "↓↓"), (-20, "↓"), (-10, "↘"), (11, "→"), (21, "↗"), (31, "↑")]

def trend_arrow(mgdl_min: int) -> str:
    for thr, arr in TREND_ARROWS:
        if mgdl_min < thr:
            return arr
    return "↑↑"


def format_reading(bc: Broadcast, *, source: str) -> str:
    r = bc.current
    if r is None:
        return f"[{source}] {bc.state}  t+{bc.time_offset_min}min  glucose: none"
    valid = bc.is_valid(r)
    tag = "VALID" if valid else "not-valid"
    note = " (saturated/warmup)" if r.glucose_mgdl >= 0x3FF else (
           "" if valid else " (provisional/warmup)")
    return (f"[{source}] glucose: {r.glucose_mgdl} mg/dL "
            f"({r.mmol:.1f} mmol/L){note}  "
            f"trend: {bc.trend_mgdl_min:+d} mg/dL/min {trend_arrow(bc.trend_mgdl_min)}  "
            f"state: {bc.state}  quality: {r.quality}  "
            f"t+{bc.time_offset_min}/60min  [{tag}]")


def reading_dict(bc: Broadcast, *, source: str) -> dict:
    r = bc.current
    return {
        "source": source,
        "time_utc": datetime.utcnow().isoformat() + "Z",
        "state": bc.state,
        "time_offset_min": bc.time_offset_min,
        "trend_mgdl_min": bc.trend_mgdl_min,
        "malfunctions": bc.malfunctions,
        "glucose_mgdl": r.glucose_mgdl if r else None,
        "glucose_mmol": round(r.mmol, 2) if r else None,
        "record_status": r.record_status if r else None,
        "quality": r.quality if r else None,
        "valid": bool(r and bc.is_valid(r)),
    }


# ── BLE: device discovery ────────────────────────────────────────────────────
async def find_device(timeout: float, address: Optional[str], serial: Optional[str]):
    """Return (BLEDevice, AdvertisementData) for the target, or (None, None)."""
    found = await BleakScanner.discover(timeout=timeout, return_adv=True)
    serial_l = (serial or "").lower()
    best = None
    for dev, adv in found.values():
        if address and dev.address.lower() == address.lower():
            return dev, adv
        name = (adv.local_name or dev.name or "").lower()
        if serial_l and serial_l in name:
            return dev, adv
        if AIDEX_COMPANY_ID in adv.manufacturer_data and decode_advertisement(
                adv.manufacturer_data[AIDEX_COMPANY_ID]):
            best = best or (dev, adv)   # fall back to any decodable AiDEX
    return best if best else (None, None)


# ── BLE: authenticated session (connect + handshake + commands) ──────────────
class Session:
    def __init__(self, client: BleakClient, pair_key: bytes, iv: bytes):
        self.client = client
        self.pair_key = pair_key
        self.iv = iv
        self.session_key: Optional[bytes] = None
        self._queue: asyncio.Queue = asyncio.Queue()

    def _on_notify(self, _sender, data: bytearray):
        if self.session_key is None:
            return
        msg = devcomm2_decode(bytes(data), self.session_key, self.iv)
        if msg is not None:
            self._queue.put_nowait(msg)

    async def handshake(self):
        await self.client.start_notify(F002, self._on_notify)
        try:
            await self.client.start_notify(F003, lambda *_: None)
        except Exception:
            pass
        challenge = bytes(await self.client.read_gatt_char(F002))
        self.session_key = session_key_from_challenge(challenge, self.pair_key, self.iv)

    async def command(self, op: int, payload: bytes = b"", timeout: float = 4.0
                      ) -> Optional[tuple[int, bytes]]:
        assert self.session_key is not None, "handshake() first"
        while not self._queue.empty():
            self._queue.get_nowait()
        packet = devcomm2_encode(op, payload, self.session_key, self.iv)
        await self.client.write_gatt_char(F002, packet, response=False)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                cmd, resp = await asyncio.wait_for(self._queue.get(), timeout=timeout)
            except asyncio.TimeoutError:
                return None
            if cmd == op:
                return cmd, resp
        return None


async def with_session(args, fn):
    if not args.serial or not args.key:
        print("set --serial and --key (or env AIDEX_SERIAL / AIDEX_PAIR_KEY)", file=sys.stderr)
        return 2
    if not re.fullmatch(r"[0-9a-fA-F]{32}", args.key):
        print("--key must be 32 hex characters (the device-issued pair key)", file=sys.stderr)
        return 2
    dev, _ = await find_device(args.timeout, args.address, args.serial)
    if dev is None:
        print("device not found (try --address <macOS-UUID> or move closer)", file=sys.stderr)
        return 2
    pair_key = bytes.fromhex(args.key)
    iv = derive_iv(args.serial)
    async with BleakClient(dev) as client:
        sess = Session(client, pair_key, iv)
        await sess.handshake()
        return await fn(sess)


# ── subcommands ──────────────────────────────────────────────────────────────
async def cmd_scan(args) -> int:
    async def once() -> Optional[Broadcast]:
        dev, adv = await find_device(args.timeout, args.address, args.serial)
        if dev is None or AIDEX_COMPANY_ID not in (adv.manufacturer_data if adv else {}):
            print("no AiDEX advertisement seen", file=sys.stderr)
            return None
        decoded = decode_advertisement(adv.manufacturer_data[AIDEX_COMPANY_ID])
        if not decoded:
            print("advertisement did not decode", file=sys.stderr)
            return None
        bc, _, _ = decoded
        if args.json:
            import json
            print(json.dumps(reading_dict(bc, source="adv")))
        else:
            print(f"{datetime.now():%H:%M:%S}  {format_reading(bc, source='adv')}")
        return bc

    if not args.watch:
        return 0 if await once() else 1
    while True:
        await once()
        await asyncio.sleep(args.watch)


async def cmd_read(args) -> int:
    async def fn(sess: Session) -> int:
        msg = await sess.command(OP_BROADCAST)
        if not msg or not msg[1] or msg[1][0] != 1:
            print("no broadcast response", file=sys.stderr)
            return 1
        bc = decode_broadcast_body(msg[1][1:])
        if bc is None:
            print("broadcast did not decode", file=sys.stderr)
            return 1
        if args.json:
            import json
            print(json.dumps(reading_dict(bc, source="cmd")))
        else:
            print(format_reading(bc, source="cmd"))
        return 0
    return await with_session(args, fn)


async def cmd_monitor(args) -> int:
    print(f"# AiDEX monitor (every {args.interval}s). Ctrl-C to stop.")
    n = 0
    while args.count == 0 or n < args.count:
        n += 1
        try:
            dev, adv = await find_device(args.timeout, args.address, args.serial)
            if dev and adv and AIDEX_COMPANY_ID in adv.manufacturer_data:
                decoded = decode_advertisement(adv.manufacturer_data[AIDEX_COMPANY_ID])
                if decoded:
                    print(f"{datetime.now():%H:%M:%S}  {format_reading(decoded[0], source='adv')}",
                          flush=True)
                else:
                    print(f"{datetime.now():%H:%M:%S}  (advertisement undecoded)", flush=True)
            else:
                print(f"{datetime.now():%H:%M:%S}  (no broadcast)", flush=True)
        except Exception as e:  # keep the loop alive across transient BLE errors
            print(f"{datetime.now():%H:%M:%S}  (error: {e})", flush=True)
        if args.count and n >= args.count:
            break
        await asyncio.sleep(args.interval)
    return 0


async def cmd_info(args) -> int:
    async def fn(sess: Session) -> int:
        di = await sess.command(OP_DEVICE_INFO)
        st = await sess.command(OP_START_TIME)
        print(f"device-info: {di[1].hex() if di else '-'}")
        print(f"start-time : {st[1].hex() if st else '-'}")
        br = await sess.command(OP_BROADCAST)
        if br and br[1] and br[1][0] == 1:
            bc = decode_broadcast_body(br[1][1:])
            if bc:
                print(format_reading(bc, source="cmd"))
        return 0
    return await with_session(args, fn)


def _resolve_window(args, min_idx: int, max_idx: int,
                    start_dt: Optional[datetime]) -> tuple[int, int]:
    """Map the CLI time-range options onto a [from_idx, to_idx] of sensor-minute indices."""
    frm, to = min_idx, max_idx
    if args.from_index is not None:
        frm = args.from_index
    if args.to_index is not None:
        to = args.to_index
    if args.last is not None:
        to = max_idx
        frm = max(min_idx, max_idx - parse_duration_minutes(args.last) + 1)
    if (args.since or args.until):
        if start_dt is None:
            raise SystemExit("--since/--until need the sensor start time (unavailable)")
        idx_of = lambda t: round((t - start_dt).total_seconds() / 60.0)
        if args.since:
            frm = idx_of(parse_when(args.since))
        if args.until:
            to = idx_of(parse_when(args.until))
    return max(frm, min_idx), min(to, max_idx)


def _emit_history(records: list[HistoryRecord], fmt: str) -> None:
    if fmt == "json":
        import json
        print(json.dumps([{
            "index": r.index,
            "time": r.timestamp.isoformat() if r.timestamp else None,
            "glucose_mgdl": r.glucose_mgdl, "glucose_mmol": round(r.mmol, 2),
            "record_status": r.record_status, "valid": r.valid,
        } for r in records]))
    elif fmt == "csv":
        print("index,time,glucose_mgdl,glucose_mmol,record_status,valid")
        for r in records:
            ts = r.timestamp.strftime("%Y-%m-%d %H:%M:%S") if r.timestamp else ""
            print(f"{r.index},{ts},{r.glucose_mgdl},{r.mmol:.1f},{r.record_status},{int(r.valid)}")
    else:  # table
        print(f"{'idx':>4}  {'time':<19}  {'mg/dL':>5}  {'mmol/L':>6}  valid")
        for r in records:
            ts = r.timestamp.strftime("%Y-%m-%d %H:%M:%S") if r.timestamp else "-"
            print(f"{r.index:>4}  {ts:<19}  {r.glucose_mgdl:>5}  {r.mmol:>6.1f}  "
                  f"{'yes' if r.valid else 'no'}")


async def cmd_history(args) -> int:
    async def fn(sess: Session) -> int:
        st = await sess.command(OP_START_TIME)
        start_dt = decode_start_time(st[1]) if st and st[1] else None
        rng = await sess.command(OP_HISTORY_RANGE)
        rng_dec = decode_history_range(rng[1]) if rng and rng[1] else None
        if not rng_dec:
            print("could not read history range", file=sys.stderr)
            return 1
        min_idx, max_idx = rng_dec
        from_idx, to_idx = _resolve_window(args, min_idx, max_idx, start_dt)
        print(f"# stored range: index {min_idx}..{max_idx}"
              + (f" (sensor start {start_dt:%Y-%m-%d %H:%M:%S})" if start_dt else "")
              + f"; requesting {from_idx}..{to_idx}", file=sys.stderr)
        if from_idx > to_idx:
            print("empty selection", file=sys.stderr)
            return 1

        records: list[HistoryRecord] = []
        idx = from_idx
        while idx <= to_idx:
            resp = await sess.command(OP_HISTORIES, struct.pack("<H", idx))
            block = decode_history_block(resp[1]) if resp and resp[1] else None
            if not block or not block[1]:
                break
            start, words = block
            for i, w in enumerate(words):
                rec_idx = start + i
                if rec_idx < from_idx:
                    continue
                if rec_idx > to_idx:
                    break
                records.append(HistoryRecord(
                    index=rec_idx,
                    timestamp=(start_dt + timedelta(minutes=rec_idx)) if start_dt else None,
                    glucose_mgdl=w & 0x03FF, record_status=(w >> 10) & 0x03))
            nxt = start + len(words)
            if nxt <= idx:        # no forward progress -> stop (avoid loop)
                break
            idx = nxt

        if args.valid_only:
            records = [r for r in records if r.valid]
        if args.every and args.every > 1:          # downsample: one reading per N minutes
            records = [r for r in records if r.index % args.every == 0]
        _emit_history(records, args.format)
        return 0
    return await with_session(args, fn)


async def cmd_pair(args) -> int:
    """First-time pairing: write the SN-derived secret to F001; the device returns its 16-byte
    pair key (only works on a fresh/unpaired transmitter)."""
    if not args.serial:
        print("set --serial <device serial>", file=sys.stderr)
        return 2
    secret = derive_pair_secret(args.serial)
    dev, _ = await find_device(args.timeout, args.address, args.serial)
    if dev is None:
        print("device not found (try --address <macOS-UUID> or move closer)", file=sys.stderr)
        return 2
    holder, got = {}, asyncio.Event()
    def on_f001(_c, payload):
        b = bytes(payload)
        if len(b) == 16 and "key" not in holder:
            holder["key"] = b
            got.set()
    async with BleakClient(dev) as client:
        await client.start_notify(F001, on_f001)
        try:
            await client.start_notify(F002, lambda *a: None)
        except Exception:
            pass
        await asyncio.sleep(0.3)
        await client.write_gatt_char(F001, secret, response=True)
        try:
            await asyncio.wait_for(got.wait(), timeout=8.0)
        except asyncio.TimeoutError:
            print("no pair key returned — this works only on a fresh/unpaired transmitter.", file=sys.stderr)
            return 1
        key = holder["key"].hex().upper()
        print(f"AIDEX_PAIR_SUCCESS serial={args.serial} key={key}")
        print("Save this 32-hex pair key (use as --key, env AIDEX_PAIR_KEY, or in the web app).")
        return 0


async def cmd_start_sensor(args) -> int:
    if not args.yes:
        print("Refusing: start-sensor is IRREVERSIBLE (commits the single-use sensor and\n"
              "begins a ~60-min warmup). Re-run with --yes once a fresh sensor is applied.",
              file=sys.stderr)
        return 2
    async def fn(sess: Session) -> int:
        # pre-check: refuse to clobber a running/warming session unless --force
        br = await sess.command(OP_BROADCAST)
        bc = decode_broadcast_body(br[1][1:]) if br and br[1] and br[1][0] == 1 else None
        state = bc.state if bc else "unknown"
        print(f"current sensor state: {state}")
        if bc and not state.startswith("NEW/USED") and not args.force:
            print(f"Refusing: sensor is '{state}', not a fresh NEW/USED sensor — starting now would "
                  f"reset a running session. Re-run with --force to override.", file=sys.stderr)
            return 2
        payload = new_sensor_payload()
        print(f"newSensor payload (now): {payload.hex()}")
        ack = await sess.command(OP_NEW_SENSOR, payload)
        print(f"newSensor ack: {ack[1].hex() if ack else 'NONE'}")
        st = await sess.command(OP_START_TIME)
        print(f"start-time now: {st[1].hex() if st else '-'}")
        return 0 if ack and ack[1] == b"\x01" else 1
    return await with_session(args, fn)


# ── CLI ──────────────────────────────────────────────────────────────────────
def build_parser() -> argparse.ArgumentParser:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--serial", default=DEFAULT_SERIAL, help="device serial (default: %(default)s)")
    common.add_argument("--key", default=DEFAULT_PAIR_KEY,
                        help="device-issued 16-byte pair key, hex (default: this unit's)")
    common.add_argument("--address", help="macOS BLE UUID to target directly (skips name scan)")
    common.add_argument("--timeout", type=float, default=8.0,
                        help="scan/connect timeout s (default: %(default)s)")

    p = argparse.ArgumentParser(
        prog="cgm.py", parents=[common],
        description="Read a MicroTech AiDEX X / GX-01S CGM over BLE.",
        epilog="Common options (--serial/--key/--address/--timeout) work before or after the command. "
               "Not a medical device; do not use for treatment decisions.")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("scan", parents=[common],
                       help="passive: decode glucose from the advertisement (no connect)")
    s.add_argument("--watch", type=float, default=0, metavar="SEC", help="repeat every SEC seconds")
    s.add_argument("--json", action="store_true")
    s.set_defaults(func=cmd_scan)

    s = sub.add_parser("read", parents=[common], help="connect + handshake + read current glucose")
    s.add_argument("--json", action="store_true")
    s.set_defaults(func=cmd_read)

    s = sub.add_parser("monitor", parents=[common], help="passive reading loop (one line per interval)")
    s.add_argument("--interval", type=float, default=60, metavar="SEC")
    s.add_argument("--count", type=int, default=0, help="number of reads (0 = forever)")
    s.set_defaults(func=cmd_monitor)

    s = sub.add_parser("info", parents=[common], help="device info + sensor start time")
    s.set_defaults(func=cmd_info)

    s = sub.add_parser("pair", parents=[common],
                       help="first-time pairing: obtain the device-issued pair key (fresh transmitter)")
    s.set_defaults(func=cmd_pair)

    s = sub.add_parser("history", parents=[common],
                       help="read stored on-device history (optionally a time range)")
    g = s.add_mutually_exclusive_group()
    g.add_argument("--last", metavar="DUR", help="most recent window, e.g. 30m / 2h / 1d / 90 (min)")
    g.add_argument("--since", metavar="WHEN", help="start time, ISO 'YYYY-MM-DD HH:MM' or 'HH:MM'")
    s.add_argument("--until", metavar="WHEN", help="end time (with --since)")
    s.add_argument("--from-index", type=int, dest="from_index", help="raw start sensor-minute index")
    s.add_argument("--to-index", type=int, dest="to_index", help="raw end sensor-minute index")
    s.add_argument("--format", choices=["table", "csv", "json"], default="table")
    s.add_argument("--valid-only", action="store_true", help="omit warmup/invalid records")
    s.add_argument("--every", type=int, metavar="MIN", help="downsample to one reading per MIN minutes")
    s.set_defaults(func=cmd_history, last=None, since=None, until=None,
                   from_index=None, to_index=None, every=None)

    s = sub.add_parser("start-sensor", parents=[common],
                       help="IRREVERSIBLE: start a new sensor session")
    s.add_argument("--yes", action="store_true", help="confirm you really want to start the sensor")
    s.add_argument("--force", action="store_true", help="override the not-a-fresh-sensor refusal")
    s.set_defaults(func=cmd_start_sensor)

    return p


def main() -> int:
    args = build_parser().parse_args()
    try:
        return asyncio.run(args.func(args)) or 0
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
