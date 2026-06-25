# AiDEX X / GX-01S — open glucose reader

Reverse-engineered, interoperable apps to read a **MicroTech AiDEX X (model GX-01S)** continuous
glucose monitor over Bluetooth Low Energy. Built in **Rust + [Dioxus](https://dioxuslabs.com/)**: a
shared, tested core drives both a **web app** (pure-WASM, runs in your browser) and a native **iOS
app** (reads the sensor directly and writes to Apple Health). The full wire protocol is documented
in **[PROTOCOL.md](PROTOCOL.md)**.

> Not affiliated with MicroTech. Experimental, uncalibrated, reverse-engineered software —
> **not a medical device and not for treatment decisions.** Confirm anything that matters with a
> fingerstick meter and your clinician.

## Design

One pure-Rust core, one shared UI, two thin platform backends — each backend implements only the
device/browser specifics (a BLE byte pipe, key/value storage, a clock, file/health export):

```
crates/
├── cgm-core   pure domain logic — crypto, DevComm2 protocol, glucose decoding,
│              the data model + persistence format, stats, and the BLE engine.
│              No platform dependencies. 36 unit tests (protocol vectors, store
│              back-compat, backfill against a mock device).
├── cgm-ui     shared Dioxus components + reactive state. Talks to the device,
│              storage, and health only through `Platform` traits, so the exact
│              same UI renders on web and iOS. Tailwind styling, light/dark.
├── cgm-web    WASM build: Web Bluetooth, localStorage, browser download/import.
└── cgm-ios    iOS app: CoreBluetooth, HealthKit, file storage (macOS-only build).
```

**Backwards compatible:** the model, the `cgm.settings` / `cgm.devices` / `cgm.data.<id>`
localStorage keys, the legacy single-device migration, and the Export/Import JSON backup are
byte-for-byte the same as the previous web app — your existing data and backups load unchanged.

## What you can do

- **Add a sensor** with a guided wizard: name → **pair** over BLE (one-time, gets a private key) →
  optionally **start** the sensor → **connect**.
- **Live reading** with trend, a plain-language status ("In range" / "Below range" …), and a
  glucose chart with the target band, event markers, and a selectable window (3h / 6h / 12h / 24h /
  All).
- **Log events** — double-click the chart to drop a coffee / meal / insulin / custom marker.
- **Time-in-range** (24 h) at a glance.
- **Multiple sensors**, unit toggle (mg/dL ↔ mmol/L), light/dark theme.
- **Export / Import** a full JSON backup (all sensors + data + settings) to move between
  browsers/devices.
- **Apple Health** — the iOS app writes Blood Glucose directly; the web app exports a JSON the
  built-in Shortcuts app can log.
- An **Advanced & diagnostics** panel (collapsed by default) with sensor age vs the 15-day life,
  validity, signal quality, reference ranges, and a raw log — there for experts, out of the way for
  everyone else.

## Develop

Everything is pinned by the Nix flake (`dioxus-cli`, `wasm-bindgen`, `tailwindcss`, Rust nightly +
the wasm target). With [direnv](https://direnv.net/) the shell loads automatically; otherwise:

```sh
nix develop            # enter the dev shell

# core + shared UI (portable, run on any host)
cargo test             # 38 tests
cargo clippy

# web app
tailwindcss -i crates/cgm-web/tailwind.css -o crates/cgm-web/assets/tailwind.css
dx serve --package cgm-web        # dev server with hot reload
dx build --release --package cgm-web   # production bundle -> target/dx/cgm-web/release/web/public
```

Open the dev server in **Chrome or Edge** (desktop or Android) — Web Bluetooth isn't available in
Safari/Firefox/iOS, which is exactly why the iOS app exists.

The **iOS app** builds only on macOS with the Apple SDK — see
**[crates/cgm-ios/README.md](crates/cgm-ios/README.md)** (`dx serve --platform ios`, HealthKit
capability, Bluetooth/Health usage strings).

### Deploy to GitHub Pages

Push to `main` and set **Settings → Pages → Source = GitHub Actions**. The workflow
(`.github/workflows/deploy.yml`) builds the WASM bundle in the flake's pinned toolchain and
publishes it. The page ships **no secrets** — each user pairs their own transmitter, and the
serial/pair-key live only in their browser's localStorage.

## Getting your device's pair key (one-time)

Everything is derived from the device **serial** (printed on it / in its BLE name), plus a 16-byte
**pair key** the device hands out **once** during pairing (it is *not* derivable from the serial).
The apps do this for you — **Add a sensor → enter the SN → Pair** fetches and saves the key over
Bluetooth. Pairing only works on a fresh / unpaired transmitter. Keep the pair key **private**:
together with the (publicly-advertised) serial it grants full read access to your transmitter.

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

**CGM target band** (shaded green on the chart, and the time-in-range metric): **70–180 mg/dL
(3.9–10.0 mmol/L)**; below 70 is low, above 180 is high; urgent below 54 or above 250.

**Diagnostic thresholds — Mayo Clinic / ADA** (for blood tests under fasting / glucose-tolerance
conditions, *not* arbitrary CGM readings), shown in the diagnostics panel:

| test | normal | prediabetes | diabetes |
|---|---|---|---|
| Fasting (8 h+) | < 100 (< 5.6) | 100–125 (5.6–6.9) | ≥ 126 (≥ 7.0) |
| 2 h after 75 g (OGTT) | < 140 (< 7.8) | 140–199 (7.8–11.0) | ≥ 200 (≥ 11.1) |

mg/dL (mmol/L). Sources: [Mayo Clinic — blood sugar testing](https://www.mayoclinic.org/diseases-conditions/diabetes/in-depth/blood-sugar/art-20046628), [Mayo Clinic — diabetes diagnosis](https://www.mayoclinic.org/diseases-conditions/diabetes/diagnosis-treatment/drc-20371451).

A brand-new sensor is uncalibrated and least accurate in its first hours, and a CGM reads
interstitial fluid (timing/meal-dependent) — cross-check with a fingerstick before trusting a value.
