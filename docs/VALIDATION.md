# Validation

Initial validation on 22 October 2026. This is an **alpha**, not a claim of universal device compatibility or measured acoustic end-to-end latency.

Reference machine: Windows 11 Pro build 26300, AMD Ryzen 9 9950X3D (32 logical processors), HyperX Cloud III S Wireless microphone and dongle output, installed VB-CABLE. Microphone, cable render and headphone periods negotiated to 10 ms. VB-CABLE render format was 48 kHz; its capture format was 44.1 kHz.

## Completed locally

| Check | Result |
| --- | --- |
| Formatting, Clippy with all features | Passed |
| Automated tests | 13 passed: atomic settings-file persistence/preservation, unity and 5x gain, limiter, EQ response, high-pass, gate, compressor, smoothing, profiles, atomic parameter snapshots, no hot-path allocation, rate conversion, anti-alias rejection, bounded queues and simulated clock drift |
| Native UI callback self-test | Passed: boost, EQ row updates, processing, presets, monitor volume, per-input selection, preferences, show/hide and stopped meter timer in tray; uses in-memory settings |
| Opt-in Windows tests | 4 passed locally: live gain/mute without stream restart, true stop/release, recovery from a failed endpoint start, missing monitor fallback, and startup registration read-back with original registry value restored |
| Visual inspection | All five pages rendered and inspected; RGBA preview encoding corrected, and overview/EQ layout clipping repaired |
| Installer | Clean install, upgrade, installed executable self-test and uninstall passed; no driver installed during these checks |
| Static CRT | Windows x64 build statically links the CRT rather than requiring a VC++ redistributable installer |
| DSP-only benchmark | All basic effects enabled, 128-frame blocks at 48 kHz: p99 upper bound 4 µs/block; maximum observed 43.6 µs in this run |
| Headless engine + Live Listen, 30 seconds | 0 cable underruns, 0 monitor underruns, 0 overrun frames, 0 sampled seconds stopped; max private memory 2.17 MiB; total CPU 0.0146% (0.469% of one logical core); DSP p99 upper bound 16 µs |
| Complete app after hiding to tray, 59 measured seconds | Max private memory **8.09 MiB**, total CPU **0.0157%**, DSP p99 upper bound **32 µs**; 0 cable underruns, 0 dropped frames, 0 session errors, 1 session started, no sampled stopped time. This run overlapped local build/packaging work. |
| VB-CABLE probe | 8.45 ms endpoint-clock offset, correlation 0.99986; excludes microphone capture, MicLift DSP/queues and physical headphone transport |

The CPU percentages depend strongly on this high-core-count machine. Headless figures do not include Slint UI initialization and must not be presented as the complete application's footprint. Diagnostic sample collection itself has bounded memory overhead. The first WASAPI packet can report a startup discontinuity; it is counted separately from subsequent queue underruns.

The previous Microboost repository was left unchanged. Its existing E2E harness was built and run against its supplied fixture; all three quality comparisons passed. That harness labels captured data using the render sample rate despite the current 44.1/48 kHz endpoint mismatch, and does not provide a valid comparable live microphone latency measurement. **No percentage speedup over Microboost is claimed.**

## Remaining release-validation matrix

- Two-hour complete-app tray soak report, including session restart/error counters. Short tray measurement is complete; the two-hour result is not yet certified.
- Physical AUX/USB alternatives and Bluetooth, beyond the available wireless dongle headset.
- Actual unplug/replug and Windows sleep/resume on each supported device class. Recovery paths are implemented; physical testing remains necessary.
- Windows 10 22H2 and a second Windows 11 machine with a clean user profile.
- Actual sign-in startup, including Task Manager disabling the startup entry. Registry registration/read-back/restoration passed locally; sign-in itself requires a fresh session.
- Optional licensed driver bootstrap: UAC cancellation, installation requiring reboot and post-reboot endpoint detection. The default public build does not enable automatic driver integration.
- Authenticode signing with a real supplied certificate. Initial artifacts are unsigned.

## Reproduce

Use release builds. Run one producer at a time into the virtual cable. Close unrelated audio tools when measuring and record device mix formats. Use `miclift-diagnostics --help` for headless tests. For the complete native app:

```powershell
.\target\release\miclift.exe --benchmark-seconds 7200 --report C:\absolute\path\tray-soak.json
```

This renders the overview once, hides to tray, runs audio with temporary default settings, and exits after the requested duration. A separate local helper timer samples resources only during this benchmark mode; normal hidden operation has no meter timer. Reports contain counters, not audio samples.

To validate actual acoustic latency, use a shared physical reference/loopback measurement with appropriate hardware. Do not add up UI counters and label the result as measured mouth-to-ear delay.
