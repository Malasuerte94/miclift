# Changelog

## 0.2.0-alpha.3

- Save the active sound profile or undo all unsaved sound changes from the icons beside Manage on Overview, Equalizer and Processing.
- Compare microphone input and processed output on one six-second RMS graph. The traces share a dBFS scale and show the measured gain for each 50 ms interval, including quiet signals.
- Keep the percentage and amplification multiplier synchronized with the active gain when editing a saved profile.

## 0.2.0-alpha.2

- Add named profiles capturing microphone boost, five-band EQ and every processing setting.
- Switch profiles live from the audio pages, save edits or create a new profile, select a startup default and delete unused profiles.
- Protect unsaved edits when switching and confirm profile deletion.
- Add an EQ preset dropdown with Flat, Voice, Warm, Podcast and Bright. Applying an EQ preset preserves boost, high-pass, gate, compressor and limiter settings.
- Refresh buttons, keyboard focus and active navigation; add brief hover, press and page entrance animations.
- Respect Windows animation preferences and add a Reduce interface motion setting. No looping graphics or new tray timers.
- Migrate alpha 0.1 settings, preserving current sound, per-microphone settings, routing and Windows startup preferences. A migrated My setup profile captures the current sound; a startup default is opt-in.
- Isolate installer smoke checks from an existing user installation and keep the saved profile library outside the audio supervisor.
- Merge dependency PRs #1–#5: update the pinned GitHub Actions and winreg 0.56. Adapt the startup registry restoration test to winreg's owned `RegValue<'static>` API.

Initial builds remain unsigned. Physical device compatibility and the two-hour soak validation limitations documented in docs/VALIDATION.md still apply.
