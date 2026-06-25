# AiDEX X / GX-01S — open BLE reader

Reverse-engineered, interoperable tools to read a **MicroTech AiDEX X (model GX-01S)** continuous
glucose monitor over Bluetooth Low Energy — a browser app, a Python CLI, and a macOS probe. The
full wire protocol is documented in **[PROTOCOL.md](PROTOCOL.md)**.

> Not affiliated with MicroTech. This is experimental, uncalibrated, reverse-engineered software —
> **not a medical device and not for treatment decisions.** Confirm anything that matters with a
> fingerstick meter and your clinician.

## What's here

| Tool | Platform | Use |
|---|---|---|
| **`index.html`** | Chrome/Edge (desktop, Android) — Web Bluetooth | click-to-connect web app: live reading, history sync, chart, localStorage. **Read-only.** |
| **`cgm.py`** | any (Python + `bleak`) | CLI: `scan` / `read` / `monitor` / `history` / `info` / `start-sensor` |
| **`CGMProbe.swift`** | macOS (CoreBluetooth) | low-level probe; also does first-time pairing |

## Getting your device's pair key (one-time)

Everything is derived from the device **serial** (printed on it / in its BLE name), plus a 16-byte
**pair key** the device hands out **once** during pairing (it is *not* derivable from the serial).
Pair once to obtain it — e.g. on macOS:

```sh
# build CGMProbe (see below), then:
./CGMProbe --seconds 30 --target <SERIAL> --aidex --serial <SERIAL>
# -> prints  AIDEX_PAIR_SUCCESS key=<32 hex chars>   — save this; it's your pair key
```

Keep the pair key **private**: together with the (publicly-advertised) serial it grants full read
access to your transmitter. Never commit it.

## Web app

```sh
# locally:
python3 -m http.server 8000        # then open Chrome at http://localhost:8000/
```
Open it, type your **serial** and **pair key** into the fields (saved in your browser's
localStorage, never in the page source), and click **Connect**. It handshakes, backfills only the
missing history, charts it with a target-range band, and persists across refresh. Unit toggle
(mg/dL ↔ mmol/L), light/dark theme, and CSV export. **Chrome/Edge on desktop or Android only** —
Safari/Firefox/iOS don't support Web Bluetooth. Libraries (crypto-js, echarts) are vendored under
`vendor/`, so it runs fully offline with no CDN.

### Deploy to GitHub Pages

Push to GitHub and set **Settings → Pages → Source = GitHub Actions**. The included workflow
(`.github/workflows/deploy.yml`) publishes `index.html` + `vendor/` on every push to `main`. The
page ships **no secrets** — each user enters their own serial/key.

## Python CLI

```sh
python3 -m pip install --user bleak pycryptodome
export AIDEX_SERIAL=<your serial>            # or pass --serial / --key each time
export AIDEX_PAIR_KEY=<your 32-hex pair key>

python3 cgm.py scan                          # passive: glucose from the advertisement
python3 cgm.py read [--json]                 # connect + handshake + current value
python3 cgm.py monitor --interval 60         # passive reading loop
python3 cgm.py info                          # device info + sensor start time
python3 cgm.py history --last 2h --format csv > glucose.csv   # backfill stored history
python3 cgm.py history --since "21:00" --until "21:30"
python3 cgm.py start-sensor --yes            # IRREVERSIBLE; refuses without --yes, and refuses a
                                             # non-fresh sensor without --force
```

## macOS probe (`CGMProbe.swift`)

```sh
swiftc CGMProbe.swift -o CGMProbe \
  -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk \
  -module-cache-path .build/module-cache -framework CoreBluetooth \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist
codesign --force --sign - --identifier local.cgm.probe CGMProbe
./CGMProbe --seconds 30 --target <SERIAL> --aidex-key <PAIRKEY> --serial <SERIAL> --device-info --start-time
```

## Protocol, in one paragraph

Service `181F`; characteristic `F002` carries encrypted **DevComm2** packets. From the serial you
derive an **IV** = `MD5((base36(c)·17+19) mod 256)`. Pair once to get the device's 16-byte **pair
key**. Each connection: read a 17-byte challenge from `F002`, AES-128-**CFB128**-decrypt it
(pair key + IV) to a per-session key (last byte = CRC-8/MAXIM check). Then every command is
`[op][payload][CRC16-CCITT LE]` encrypted with the session key — e.g. `0x11` returns the broadcast
record, `0x22`/`0x23` read stored per-minute history. Glucose = `word & 0x3FF` mg/dL
(`/18` → mmol/L); `0xFFFF` = no reading. **Activating a sensor (`0x20`) is irreversible.** Full
details, byte maps, and a reproducible worked example: **[PROTOCOL.md](PROTOCOL.md)**.

## Glucose reference ranges (general guidance, not medical advice)

| zone | mg/dL | mmol/L |
|---|---|---|
| urgent low | < 54 | < 3.0 |
| low | 54–69 | 3.0–3.8 |
| in range (target) | 70–180 | 3.9–10.0 |
| high | 181–250 | 10.1–13.9 |
| very high | > 250 | > 13.9 |

A brand-new sensor is uncalibrated and least accurate in its first hours — cross-check with a
fingerstick before trusting a value.
