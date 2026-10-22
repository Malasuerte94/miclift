# Security

Do not include personal audio, credentials, certificate files or private system data in bug reports. Diagnostics contain device names/IDs and performance counters, never microphone samples.

For a vulnerability that could execute code or expose user data, use GitHub's private vulnerability reporting if enabled. Do not post exploit details in a public issue before a fix is available.

Official release archives contain no third-party audio drivers. The optional licensed bootstrap accepts only the pinned official package and a valid expected-publisher Authenticode signature before requesting elevation. Runtime audio processing needs no administrator privileges.
