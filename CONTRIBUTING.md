# Contributing

Use Windows x64, the pinned Rust toolchain, and the MSVC C++ build tools. Keep audio correctness, bounded latency and low idle resource usage ahead of additional effects or graphics.

## Architecture

- `audio`: COM/WASAPI discovery, minimum-period negotiation with shared-mode fallback, capture/render threads, recovery supervisor. COM objects remain on their creating thread. An audio failure stops the whole session before retrying the saved devices.
- `dsp`, `resample`, `shared`: bounded audio processing, streaming ASRC, nonblocking parameter snapshots and atomic telemetry. No mutexes, heap allocations, file/network access, logging, or UI calls in healthy audio loops. Configure and allocate before starting streams.
- `config`, `platform`, application controller: versioned local profiles, debounced atomic persistence, native singleton signaling, HKCU startup registration, user-interface lifetime and tray.
- `ui`: Slint components with static EQ response and ≤30 Hz meters while visible.

Run `cargo fmt --all`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and `cargo test --locked --all-targets` before opening a PR. Changes to audio require relevant DSP/ASRC tests and real-device diagnostics. CI runners do not validate microphone timing or Bluetooth behavior.

On a Windows test PC with a microphone and VB-CABLE, close other audio producers and run `cargo test --locked --test windows_devices -- --ignored --test-threads=1`. These opt-in tests use the microphone, verify stream lifetime and recovery, and temporarily check/restore the current user's MicLift startup registry value. They do not reboot the PC, change device defaults, or install a driver.

## Releases

1. Update the package version, lockfile, UI version labels, and validation/release notes together.
2. Run CI and the hardware validation checklist. Record the source revision and device formats with results.
3. Push a matching `vX.Y.Z` tag. The release workflow validates the version, tests, builds, packages, runs installer smoke checks, and publishes artifacts. Tags such as `v0.2.0-beta.1` become prereleases.
4. Optional repository secrets `MICLIFT_CERTIFICATE_BASE64` and `MICLIFT_CERTIFICATE_PASSWORD` enable Authenticode signing. The certificate is loaded ephemerally; it must never be committed. The workflow can publish unsigned artifacts and states signing status in BUILD-INFO.
5. Keep repository variable `VBCABLE_INTEGRATION_APPROVED` unset unless integration rights have been obtained. When set to `true`, the release build enables the gated driver bootstrap. The driver archive remains hosted by VB-Audio and its SHA-256 and publisher signature are verified. Changes to the pinned package require explicit verification.

Only the publish job has `contents: write`. External actions are pinned to full commits. Dependency changes must regenerate the lockfile and third-party notices (`tools/generate-notices.ps1`). Dependency metadata reflects all resolved Windows dependencies, including build tools, so notices intentionally over-include rather than omit licenses.

New UI effects must stop when hidden. Never describe a configured buffer size or DSP processing time as physical end-to-end latency.
