# Changelog

## 0.2.0-alpha.1

- Add named profiles capturing microphone boost, five-band EQ and every processing setting.
- Switch profiles live from the audio pages, save edits or create a new profile, select a startup default and delete unused profiles.
- Protect unsaved edits when switching and confirm profile deletion.
- Add an EQ preset dropdown with Flat, Voice, Warm, Podcast and Bright. Applying an EQ preset preserves boost, high-pass, gate, compressor and limiter settings.
- Refresh buttons, keyboard focus and active navigation; add brief hover, press and page entrance animations.
- Respect Windows animation preferences and add a Reduce interface motion setting. No looping graphics or new tray timers.
- Migrate alpha 0.1 settings, preserving current sound, per-microphone settings, routing and Windows startup preferences. A migrated My setup profile captures the current sound; a startup default is opt-in.
- Isolate installer smoke checks from an existing user installation and keep the saved profile library outside the audio supervisor.

Initial builds remain unsigned. Physical device compatibility and the two-hour soak validation limitations documented in docs/VALIDATION.md still apply.
