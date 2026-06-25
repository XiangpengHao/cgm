# AiDEX X / GX-01S — open BLE reader

Reverse-engineered, interoperable tools to read a **MicroTech AiDEX X (model GX-01S)** continuous
glucose monitor over Bluetooth Low Energy — a browser app and a Python CLI. The full wire protocol
is documented in **[PROTOCOL.md](PROTOCOL.md)**.

> Not affiliated with MicroTech. This is experimental, uncalibrated, reverse-engineered software —
> **not a medical device and not for treatment decisions.** Confirm anything that matters with a
> fingerstick meter and your clinician.

## What's here

| Tool | Platform | Use |
|---|---|---|
| **`index.html`** | Chrome/Edge (desktop, Android) — Web Bluetooth | click-to-connect web app: live reading, history sync, chart, localStorage. **Read-only.** |
| **`cgm.py`** | any (Python + `bleak`) | CLI: `pair` / `scan` / `read` / `monitor` / `history` / `info` / `start-sensor` |

## Getting your device's pair key (one-time)

Everything is derived from the device **serial** (printed on it / in its BLE name), plus a 16-byte
**pair key** the device hands out **once** during pairing (it is *not* derivable from the serial).
Pair once to obtain it — e.g. on macOS:

```sh
python3 cgm.py pair --serial <SERIAL>
# -> prints  AIDEX_PAIR_SUCCESS serial=<SERIAL> key=<32 hex chars>   — save this; it's your pair key
```
(Pairing only works on a fresh / unpaired transmitter.)

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
(mg/dL ↔ mmol/L), light/dark theme, CSV export, and double-click-to-log events (coffee/meal/etc.).
**Chrome/Edge on desktop or Android only** — Safari/Firefox/iOS don't support Web Bluetooth.
Libraries (crypto-js, echarts) load from the jsDelivr CDN, pinned with Subresource Integrity hashes.

### Deploy to GitHub Pages

Push to GitHub and set **Settings → Pages → Source = GitHub Actions**. The included workflow
(`.github/workflows/deploy.yml`) publishes `index.html` on every push to `main`. The page ships
**no secrets** — each user enters their own serial/key.

## Python CLI

```sh
python3 -m pip install --user bleak pycryptodome
export AIDEX_SERIAL=<your serial>            # or pass --serial / --key each time
export AIDEX_PAIR_KEY=<your 32-hex pair key>

python3 cgm.py pair --serial <SERIAL>        # one-time: obtain the pair key (fresh transmitter)
python3 cgm.py scan                          # passive: glucose from the advertisement
python3 cgm.py read [--json]                 # connect + handshake + current value
python3 cgm.py monitor --interval 60         # passive reading loop
python3 cgm.py info                          # device info + sensor start time
python3 cgm.py history --last 2h --format csv > glucose.csv   # backfill stored history
python3 cgm.py history --since "21:00" --until "21:30"
python3 cgm.py start-sensor --yes            # IRREVERSIBLE; refuses without --yes, and refuses a
                                             # non-fresh sensor without --force
```

## Export to Apple Health (no iOS app)

HealthKit is iOS-only, so something on the iPhone must write the data — but you don't have to build
an app: Apple's built-in **Shortcuts** can log Blood Glucose to Health.

1. Export a **downsampled, valid-only** JSON (Health doesn't need 1-minute resolution, and Shortcuts
   logs samples one at a time):
   ```sh
   python3 cgm.py history --last 1d --valid-only --every 5 --format json > glucose.json
   ```
2. Get `glucose.json` onto the iPhone — AirDrop, or save to iCloud Drive / Files.
3. Build a Shortcut once:
   - **Get File** → pick `glucose.json`  (or **Get Contents of URL** if you host it)
   - **Get Dictionary from** the file → a list
   - **Repeat with Each** item in the list:
     - **Get Dictionary Value** `glucose_mgdl`  → the value
     - **Get Dictionary Value** `time` → **Get Dates from Input** → the date
     - **Log Health Sample** → type **Blood Glucose**, unit **mg/dL**, Value = the value, Date = the date
4. Run it and grant Health write permission once; re-run after each export to add new readings.

An **experimental** prebuilt shortcut (`aidex-to-health.shortcut`) is included — import it with
*Settings ▸ Shortcuts ▸ Advanced ▸ Allow Untrusted Shortcuts* enabled. It's hand-authored and
untested (the Health action's type/unit enums are undocumented), so if it won't import or logs
nothing, build it from the steps above — that path is reliable.

Keep the count modest (use `--last` / `--every`). These are **uncalibrated** CGM values — don't treat
them as clinical. (If you also use the official AiDEX app, it may already sync to Apple Health.)

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

**CGM zones** (color the live value and the chart's target band):

| zone | mg/dL | mmol/L |
|---|---|---|
| low | < 70 | < 3.9 |
| in range (target) | 70–180 | 3.9–10.0 |
| high | > 180 | > 10.0 |
| urgent | < 54 or > 250 | < 3.0 or > 13.9 |

**Diagnostic thresholds — Mayo Clinic / ADA** (for blood tests under fasting / glucose-tolerance conditions, *not* arbitrary CGM readings):

| test | normal | prediabetes | diabetes |
|---|---|---|---|
| Fasting (8 h+) | < 100 (< 5.6) | 100–125 (5.6–6.9) | ≥ 126 (≥ 7.0) |
| 2 h after 75 g (OGTT) | < 140 (< 7.8) | 140–199 (7.8–11.0) | ≥ 200 (≥ 11.1) |

mg/dL (mmol/L). Sources: [Mayo Clinic — blood sugar testing](https://www.mayoclinic.org/diseases-conditions/diabetes/in-depth/blood-sugar/art-20046628), [Mayo Clinic — diabetes diagnosis](https://www.mayoclinic.org/diseases-conditions/diabetes/diagnosis-treatment/drc-20371451).

A brand-new sensor is uncalibrated and least accurate in its first hours, and a CGM reads
interstitial fluid (timing/meal-dependent) — cross-check with a fingerstick before trusting a value.
