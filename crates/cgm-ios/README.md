# cgm-ios — the iOS app

A native iOS app that reads the **AiDEX X / GX-01S** sensor directly over
CoreBluetooth and writes Blood Glucose to **Apple Health**. It renders the exact
same `cgm-ui` Dioxus components as the web build — only the platform services
(BLE, Health, storage, clock) differ.

```
┌─────────────┐     ┌──────────────────────────┐
│  cgm-core   │◄────┤ protocol · crypto · model │  (shared, pure Rust, tested)
└─────────────┘     └──────────────────────────┘
       ▲
┌─────────────┐     ┌──────────────────────────┐
│   cgm-ui    │◄────┤ Dioxus components + state │  (shared)
└─────────────┘     └──────────────────────────┘
       ▲
┌─────────────┐     ┌──────────────────────────────────────────────┐
│   cgm-ios   │◄────┤ CoreBluetooth · HealthKit · file store · clock │
└─────────────┘     └──────────────────────────────────────────────┘
```

## Why it isn't in the workspace

This crate links the Apple SDK (CoreBluetooth + HealthKit via `objc2`) and only
builds on **macOS with the iOS toolchain**. It is therefore `exclude`d from the
root Cargo workspace so the portable crates (`cgm-core`, `cgm-ui`, `cgm-web`)
build and test on Linux/CI. It still path-depends on `../cgm-core` and
`../cgm-ui`, so it always tracks the shared code.

## Build (macOS)

```sh
# One-time: the Dioxus CLI and the iOS Rust targets.
cargo install dioxus-cli            # provides `dx`
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

# Tailwind (the shared UI's classes); regenerate when components change.
tailwindcss -i tailwind.css -o assets/tailwind.css

# Run on a simulator or a connected device.
dx serve --platform ios
```

`dx` produces an Xcode-style app bundle. If your `dx` version doesn't merge
`ios/Info.plist` / `ios/cgm.entitlements` automatically, open the generated
project in Xcode and:

1. Add the **HealthKit** capability to the target (this also needs HealthKit
   enabled on the App ID in your Apple Developer account).
2. Ensure the Info.plist contains the keys in `ios/Info.plist`
   (`NSBluetoothAlwaysUsageDescription`, `NSHealthUpdateUsageDescription`,
   `NSHealthShareUsageDescription`).
3. Set a development team for signing.

CoreBluetooth and HealthKit do **not** work in the simulator's sensor sense —
test BLE on a real device near a powered AiDEX transmitter.

## What each module does

| File | Role |
|---|---|
| `src/ble.rs` | `CBCentralManager` + delegate; scans for service `181F`, connects, and exposes the `F002` byte pipe (`BleBackend`) plus `F001` pairing. The manager runs on the **main** dispatch queue so its callbacks share the UI thread and the shared state needs no locking. |
| `src/health.rs` | Writes valid, downsampled readings as `HKQuantitySample` Blood Glucose values (mg/dL), requesting write authorization first. |
| `src/storage.rs` | `localStorage`-equivalent: a flat `cgm.*` key/value JSON file in the app's Documents directory — so the same migration and export/import logic works unchanged. |
| `src/files.rs` | Backups land in Documents (visible in Files); `export_health` goes straight to HealthKit (`health_is_native() == true`). Import reads `Documents/aidex-import.json`. |
| `src/platform.rs` | `IosPlatform` wiring + clock/timezone via `NSTimeZone`, and a runtime-agnostic `sleep`. |

## Status / honesty

The shared core and UI are fully tested and the web build is verified end to
end. The Apple-specific modules here are written against the documented
CoreBluetooth/HealthKit selectors and `objc2` bindings but are **only buildable
on macOS** — they have not been compiled in the Linux CI environment, so expect
to fix minor `objc2` binding-name drift on first `dx serve`.

Not a medical device. Uncalibrated, reverse-engineered — not for treatment
decisions.
