# The AiDEX X / GX-01S BLE protocol

A complete, worked explanation of how the **MicroTech AiDEX X (model GX-01S)** continuous
glucose monitor talks over Bluetooth Low Energy — reverse-engineered from the official app's
decompiled native library (`libblecomm`) and the Android UI layer, and **validated byte-for-byte
against a real device**. (All keys/serials below are fabricated examples — substitute your own.)

> Educational / interoperability documentation. This is not affiliated with or endorsed by
> MicroTech, and nothing here is a medical device or medical advice.

---

## 1. The big picture

The transmitter encrypts everything at the application layer. Before you can ask for a glucose
value you must (a) prove you know the device's serial number, and (b) agree on a per-connection
session key. Everything is derived from the **serial number printed on the device** — it is the
shared secret. Once you hold the session key, you exchange small encrypted command/response
packets ("DevComm2").

Two phases:
- **Pairing** — done **once** per transmitter. You prove knowledge of the serial; the device hands
  back a permanent 16-byte **pair key**. Save it.
- **Reconnect** — done **every** connection. The device gives you an encrypted blob; you decrypt it
  with the pair key to recover a fresh **session key**.

---

## 2. BLE GATT layout

The device advertises with the name `AiDEX X-<serial>` and service UUID `0x181F`. Three services:

| Service | Purpose |
|---|---|
| `0x180A` Device Information | Manufacturer "Microtech Medical", Model "GX-01S", Serial, SW rev |
| `0x181F` (Bluetooth SIG CGM Service UUID) | both standard CGM chars **and** the proprietary ones |
| `1D14D6EE-FD63-4FA1-BFA4-8F47B42119F0` | Silicon Labs OTA (firmware update) — ignore |

Characteristics inside `0x181F`:

| UUID | Properties | Role |
|---|---|---|
| `F001` | write, notify | **pairing** (used while unpaired) |
| `F002` | read, write-without-response, notify | **DevComm2** data channel (the workhorse) |
| `F003` | notify | **reconnect** notify (used once paired) |
| `F005` | read | small state byte |
| `2AA7` `2AA8` `2AA9` `2AAA` `2AAB` `2A52` `2AAC` | — | standard SIG CGM chars (see §8) |

The private/pairing characteristic is computed as `((paired & 1) << 1) | 0xF001`, i.e. **`F001`
while unpaired, `F003` once paired**.

---

## 3. Serial → key material

Each serial character maps to a number (base-36): `0–9 → 0–9`, `A–Z → 10–35`. So example serial
`EXAMPLE001` → `14,33,10,22,25,21,14,0,0,1` (operating on the raw serial bytes; no padding).

Two independent transforms, each MD5'd:

- **Key seed** = `MD5( byte(c) = (n·13 + 61) mod 256 for each serial char )` — written to `F001` during
  pairing. Local only; never used to encrypt traffic.
- **IV** = `MD5( byte(c) = (n·17 + 19) mod 256 for each serial char )` — fixed per serial, used as the
  AES IV for **all** crypto.

Worked example (serial `EXAMPLE001`): **IV = `8ADF6CA259158051F483161F11CC1D6F`**.

---

## 4. Pairing (once per transmitter)

1. Subscribe to `F001` (and `F002`).
2. Write the **key seed** (§3) to `F001`.
3. The device replies on `F001` with a **device-issued 16-byte pair key**.

The pair key is **not** a function of the serial — the device generates it and returns it, so you
**must** pair live once and persist it. (Internally the app also sends the cloud user id as the
pairing payload via opcode `0x07`; the simplified flow above is enough to obtain the key.)

Worked (fabricated) example pair key: `0123456789ABCDEF0123456789ABCDEF`. Keep your real pair key
private — together with the (publicly-advertised) serial it grants full read access to your device.

---

## 5. Reconnect handshake (every connection)

1. Subscribe to `F003` (and `F002`).
2. **Read `F002`** → a **17-byte** encrypted blob (the "challenge").
3. Decrypt the blob with **AES-128-CFB128**, key = the **pair key**, iv = the serial **IV** (§3).
4. The first **16** plaintext bytes are the **session key**; byte **16** is a **CRC-8/MAXIM** over those
   16 bytes — verify it matches. If it does, you derived the key correctly.
5. Use the session key (with the same IV) for all subsequent DevComm2 packets.

The session key changes every connection. Worked example (fabricated, self-consistent — reproducible
with the example serial/pair key above):

```
challenge  = 2E390C058040DF1996C4AEA2CB16274ED2     (read from F002, 17 bytes)
pair key   = 0123456789ABCDEF0123456789ABCDEF
iv         = 8ADF6CA259158051F483161F11CC1D6F        (= MD5 transform of serial EXAMPLE001)
decrypt    -> 00112233445566778899AABBCCDDEEFF D7    (16-byte session key + CRC8 0xD7)
crc8maxim(00112233...EEFF) = D7  ✓
```

---

## 6. DevComm2 packet framing

Every command and response is one AES-128-CFB128 block of:

```
plaintext = [ op (1 byte) ] [ payload (N bytes) ] [ CRC16-CCITT over the preceding (1+N) bytes, little-endian ]
ciphertext = AES-128-CFB128( plaintext, key = session key, iv = serial IV )
```

- **AES mode:** CFB with **128-bit (full-block) feedback** (matches CommonCrypto `kCCModeCFB`), not CFB-8.
  The IV is reloaded at the start of every packet (no chaining across packets).
- **CRC16-CCITT:** poly `0x1021`, init `0xFFFF`, no reflection, no final XOR; stored little-endian.
- **CRC8 (session-key tag):** MAXIM/Dallas, poly reflected `0x8C`, init `0x00`.
- No app-level fragmentation: each packet is one GATT write to `F002`; responses arrive as `F002`
  notifications (and the response echoes the request opcode).

Worked example: "read current glucose" is opcode `0x11`, empty payload → plaintext `11` + its
CRC16-CCITT, AES-CFB128-encrypted to a 3-byte ciphertext; the reply decrypts to op `0x11` followed
by a `result` byte and the broadcast body (§9).

---

## 7. Opcodes

On-wire opcode byte for `BleController::send(0xFF, op, 0xFF, payload, len)`:

| Op | Name | Payload | Response |
|---|---|---|---|
| `0x10` | deviceInfo | — | hardware/type/edition/life + model string |
| `0x11` | **broadcast / current glucose** | — | `[result][broadcast body]` (§9) |
| `0x20` | **newSensor (activate)** ⚠ | 9-byte datetime (§10) | ack `01` — **IRREVERSIBLE** |
| `0x21` | startTime | — | `[result][9-byte datetime]` = sensor start |
| `0x22` | historyRange | — | `[result][minIdx u16]…[maxIdx u16]` |
| `0x23` | histories | `idx u16 LE` | `[result][startIdx u16][glucose words…]` (§9) |
| `0x24` | rawHistories | `idx u16 LE` | raw electrode currents |
| `0x25` | calibration | 4 bytes (2×u16 LE) | enter a fingerstick reference |
| `0x34` | setAutoUpdate | 1 byte | enable device push |
| `0x35` | setDynamicAdv | 1 byte | dynamic advertising |

Async pushes (after `setAutoUpdate`): `0xFE01` full history, `0xFE02` calibration, etc.

---

## 8. Standard SIG CGM characteristics (read-only diagnostics)

Because `0x181F` is the Bluetooth SIG CGM service UUID, the device also exposes standard chars you
can read without the handshake. Most useful is **`2AA9` CGM Status**:
`[timeOffset u16][status][cal/temp][warning][E2E-CRC]`. `status` bit0 = **Session Stopped**. On a
fresh, un-started transmitter you'll see status `0x01` (stopped) — that's why glucose reads empty
until you start a session (§10). `2AAA` is the session start time, `2AAB` the run time, `2A52` the
Record Access Control Point, `2AAC` the CGM Specific Ops Control Point.

---

## 9. Decoding glucose

### Broadcast body (advertisement, and the `0x11` response after its `result` byte)

```
[0..1] timeOffset   u16 LE  — minutes since sensor start
[2]    status       bitfield (bit0 + cal/temp bit0 ⇒ "new/used sensor"; bits1-5 ⇒ malfunctions)
[3]    calTemp      bitfield (bit0 lifecycle, bit1 calibration gate)
[4]    trend        i8 signed — mg/dL per minute
[5..]  history slots, 3 bytes each: { glucose word u16 LE, quality u8 }
[N-2..N-1] calIndex u16 LE
```

For each glucose word:
- `0xFFFF` ⇒ **no reading** (skip).
- `glucose_mgdl = word & 0x03FF`  (0–1023).
- `recStatus   = (word >> 10) & 0x03`.
- **`mmol/L = mg/dL / 18`** (the app divides the stored mg/dL integer by exactly 18.0).
- `0x3FF` (1023) is the **saturated/warmup-invalid** marker, not a real value.

### History block (`0x23` response)

`[result=01][startIdx u16 LE][glucose words u16 LE …]`. The words are consecutive **per-minute**
records starting at `startIdx`; **index == sensor-minute**, so a record's timestamp is
`sensorStart + index·minutes` (sensorStart from `0x21`). `historyRange` (`0x22`) gives the available
`[min..max]` index window; `max` equals the current minute offset. Calling `0x23` with index `k`
returns records from `k` to the newest, so to **backfill** you fetch only the missing indices.

### Validity

A reading is a real, trustworthy glucose value only when **all** hold (mirrors the app's
`TransmitterModel`): `timeOffset ≥ 60` (warmup done), `status` clean, `recStatus == 0`,
`0 < mg/dL < 1023`, and no malfunction bits. Otherwise it's warmup/flagged.

---

## 10. Lifecycle: starting a sensor (⚠ irreversible)

A new transmitter advertises "new/used sensor" (status `0x01`, cal/temp bit0 set) and produces no
glucose. To start a session you send **`newSensor` (`0x20`)** with a 9-byte datetime payload:

```
[0..1] year   u16 LE
[2]    month (1-12)   [3] day   [4] hour   [5] minute   [6] second
[7]    timeZone  i8 — quarter-hours from UTC (e.g. UTC-6 = -24)
[8]    dstOffset i8 — quarter-hours of DST (e.g. +1h = 4)
```

This **commits the single-use sensor and begins a ~60-minute warmup** — it cannot be undone. After
the ack, `getStartTime` returns your datetime, the advertisement flips to `WARMING-UP` (status
`0x00`), `timeOffset` climbs 0→60, and once `timeOffset ≥ 60` valid glucose appears (and only then,
and only if the sensor is in tissue).

---

## 11. Reference implementations in this repo

- `cgm.py` — Python CLI (`bleak`): `pair` (§4), `scan`, `read`, `monitor`, `info`, `history`
  (time-range backfill), `start-sensor` (guarded).
- `index.html` — Web Bluetooth app (Chrome/Edge): connect, handshake, sync history, localStorage
  persistence, ECharts chart, event logging. Read-only (cannot activate a sensor).

Both implement §3–§9 identically; the crypto was verified against the device's own captured
challenge→session-key vectors.
