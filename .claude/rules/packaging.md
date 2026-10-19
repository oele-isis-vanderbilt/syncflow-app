---
paths:
  - "scripts/**"
  - ".github/**"
  - "livekit-gstreamer/src/runtime.rs"
  - "**/build.rs"
  - "syncflow-publisher/src-tauri/tauri*.conf.json"
  - ".cargo/config.toml"
---

# Packaging GStreamer

Overview in CLAUDE.md ("Packaging GStreamer").

- Keep `fetch-macos.sh` and `fetch-windows.ps1` on the same GStreamer version, and stay within what gstreamer-rs supports (0.24 means up to 1.28). Bump the version and every checksum together. Take checksums from the `.sha256sum` files next to the official downloads, never from a local hash of an unverified download.
- Never commit anything under `third_party/gstreamer/`. The fetch scripts recreate it, and CI caches it keyed on the script's hash.
- The fetch scripts must leave nothing installed on the machine. On Windows that means no uninstall entry and no `GSTREAMER_1_0_ROOT_*` variable: the script installs into a scratch folder, copies the files out and uninstalls.
- Windows bundle layout: DLLs, `gst-plugin-scanner.exe`, `gspawn-win64-helper*.exe` and the MSVC runtime sit next to the app exe, with plugins in `lib\gstreamer-1.0` and GIO modules in `lib\gio\modules`. `runtime.rs` detects this layout. Change the script, `tauri.windows.conf.json` and `runtime.rs` together.
- Every change to what ships (layout, plugins, required elements) needs a matching assertion in the workflow's "Check bundle" step.
- `macos.yml` and `windows.yml` are reusable (`workflow_call`) and keep the same step order and artifact naming (`syncflow-publisher-<sha7>-<arch>...`). Only `release.yml` runs on push and publishes `nightly`.
- Builds are unsigned (ad-hoc on macOS, none on Windows). Don't add signing secrets or notarization unless asked.
