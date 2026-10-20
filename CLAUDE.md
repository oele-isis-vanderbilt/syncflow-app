# SyncFlow App

A client app for sharing media (screen, camera, microphone) into sessions of a single SyncFlow project. It's a Cargo workspace (root `Cargo.toml`) built from two crates:

- **`livekit-gstreamer`:** platform-specific device discovery, and one composite GStreamer pipeline (using the Rust bindings) that drives every configured device on one clock, records each to its own file and streams selected outputs to LiveKit through identified appsinks. Design and scenarios: `composite-stream-plan.md`.
- **`syncflow-publisher`:** the Tauri app (SvelteKit frontend) that uses `livekit-gstreamer` to build the client for the SyncFlow project. It provides session listening (via RabbitMQ), along with other features that let end users connect their devices to SyncFlow sessions to record locally and/or stream.

Works in two modes:

- **Offline mode:** a local recorder that records multimedia files from connected devices. `local_recording_only` currently uploads to S3 when `s3_credentials.json` exists and some output has streaming on.
- **Listen mode:** the configured devices wait for a SyncFlow session to start and join it. Every output is recorded, and each can also stream to LiveKit (you can record a session without streaming). Device failures are isolated and reported to the UI and to the room on the `ROOM_EVENTS` data topic. When the session ends, they exit and upload the recorded media to a pre-configured S3 bucket. RabbitMQ powers session listening (start/stop events for the project's sessions).

## Terms
- **SyncFlow**: a platform for collecting multimodal learning-analytics data in real time, built on LiveKit (WebRTC). This app talks to it through the `client` and `shared` crates from the SyncFlow repository (imported as `syncflow_client` and `syncflow_shared`).
- **Session**: a SyncFlow session in the configured project, backed by one LiveKit room. The room name is the session name.

## Package structure

```
.
├── Cargo.toml                 # workspace: livekit-gstreamer, syncflow-publisher/src-tauri
├── Cargo.lock                 # single lockfile for both crates
├── .cargo/config.toml         # per-target rustflags (static CRT on Windows, -ObjC on Apple), PKG_CONFIG_PATH
├── .github/workflows/         # macos.yml, windows.yml (reusable builds), release.yml (nightly on push to main)
├── scripts/gstreamer/         # fetch-macos.sh, fetch-windows.ps1: vendor the pinned GStreamer
├── third_party/gstreamer/     # vendored GStreamer (gitignored, created by the fetch scripts)
├── livekit-gstreamer/
│   ├── src/
│   │   ├── runtime.rs         # points GStreamer at the runtime the process loaded (bundled or vendored)
│   │   ├── devices/           # per-OS device discovery (linux.rs, mac.rs, win.rs) behind one API in mod.rs
│   │   ├── composite/         # the pipeline: spec (validation), builder (graph layers), bus (error
│   │   │                      #   isolation), stream (CompositeStream), manifest, ids/tags/naming
│   │   ├── media_device.rs    # device lookup, capabilities, format checks, per-OS screen source
│   │   ├── publish_options.rs # Video/Audio/ScreenPublishOptions (the format a device is asked for)
│   │   └── lk_participant.rs  # publishing a composite stream's appsinks and data messages to LiveKit
│   └── examples/              # get_devices, stream_composite, lk_publish_composite, lk_listen_data;
│                              #   devices come from .env (examples/helper/composite_env.rs)
└── syncflow-publisher/
    ├── src/                   # SvelteKit frontend
    │   ├── routes/            # / (recording setup + mode), /register, /stream (sessions, local recording)
    │   └── lib/               # components/, store.svelte.ts (shared runes state), compositeConfig.ts
    │                          #   (setup draft and checks), publishOptions.ts, notifications.ts
    ├── docs/design-rules.md   # full UI design guide
    ├── scripts/check-design.mjs # design-token checker (`pnpm check:design`)
    └── src-tauri/src/
        ├── lib.rs             # Tauri setup, AppState, all #[tauri::command]s and the invoke handler
        ├── register.rs        # device registration/deregistration with SyncFlow
        ├── session_listener.rs# RabbitMQ listener for session start/stop
        ├── syncflow_publisher.rs # per-session record/stream flow, output dirs, S3 upload trigger
        ├── pipeline_events.rs # pipeline events -> UI notifications and ROOM_EVENTS messages
        ├── s3_uploader.rs     # per-file parallel upload with retries, upload-state.json (rusoto)
        ├── devices.rs, models.rs, errors.rs, utils.rs
```

Where changes go: device discovery, pipeline construction, encoding, and LiveKit publishing belong in `livekit-gstreamer`. Mode logic, RabbitMQ listening, S3 upload, and UI belong in `syncflow-publisher`. Keep `livekit-gstreamer` free of SyncFlow-specific logic.

Frontend to Rust: the frontend calls Rust only through `invoke(...)` on commands registered in `lib.rs` `generate_handler!`. Rust pushes updates with `emit` on the `session-update` and `publication-notification` events. A new command must be added to `generate_handler!`, or `invoke` fails at runtime.

Platform-specific code: OS differences in device handling go in `livekit-gstreamer/src/devices/{linux,mac,win}.rs`, behind `#[cfg(target_os)]`. Keep the three files exposing the same functions as `devices/mod.rs` re-exports.

### App data on disk
Everything lives in `~/.syncflow-publisher/`:

- `credentials.json`: SyncFlow API key/secret and RabbitMQ password (plaintext).
- `s3_credentials.json`: S3 access/secret key (plaintext).
- `registration.json`, `recording_mode.json`, `composite_config.json` (the recording setup).
- `recordings/<session-slug>_<session-id>/<join>/`: one folder per session, one numbered folder per join (`1`, `2`, … for join, exit, join). Each holds the recordings, `manifest.json` and, once an upload starts, `upload-state.json` (per-file upload status, used to retry). Offline sessions use `local-<nanos>` as the session id and always have one join.

Rules:
- Never log, print, or emit the contents of the credential files, and never commit sample files with real values.
- Don't delete recordings in code without a confirmed successful upload (every file `done` in `upload-state.json`). These are study data.
- Don't change the S3 key format without asking; downstream analysis depends on it. Each file goes to `<prefix>/<join>/<file>`, with `manifest.json` uploaded last, only after every other file, so its presence marks a complete join. Session mode: `<project>-<id>/<session>/local-recordings/<participant>/<join>/`. Offline: `recordings/<session-id>-<session>-local-recorder/<join>/`. The local folder layout isn't read downstream.

## Stack and commands

Run Rust commands from the repo root (workspace):
```sh
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test --workspace   # composite pipeline tests run on live test sources (test-sources feature)
```

### livekit-gstreamer
Uses tokio and the LiveKit Rust SDK to provide streaming/recording capabilities as tasks. GStreamer comes from the vendored copy (below) on macOS and Windows, and from a system install (with development files) on Linux.

### Packaging GStreamer
GStreamer 1.28 (gstreamer-rs 0.24, feature `v1_28`) is vendored, not installed. Rules for this area: `.claude/rules/packaging.md`.

- `scripts/gstreamer/fetch-macos.sh` / `fetch-windows.ps1` download the pinned official binaries (checksum-verified) into `third_party/gstreamer/`. `third_party/gstreamer/sdk` points at the platform's runtime root: a symlink on macOS, a directory junction on Windows. `sdk/lib/pkgconfig-cargo` is what `.cargo/config.toml` exposes to cargo.
- macOS: Tauri copies `GStreamer.framework` into `Contents/Frameworks` (`tauri.conf.json`), and `build.rs` adds the rpaths.
- Windows: the fetch script stages `third_party/gstreamer/windows/bundle/`, which holds the DLLs, `gst-plugin-scanner.exe` and the MSVC runtime at the top, with plugins in `lib\gstreamer-1.0`. `tauri.windows.conf.json` installs it next to the exe (NSIS, per-user).
- `livekit_gstreamer::initialize_gstreamer()` runs `runtime::configure()` before `gst::init` so plugins load only from the runtime that was actually loaded. Use it instead of `gstreamer::init()`.
- Windows dev setup: run `.\scripts\gstreamer\fetch-windows.ps1`, then for `cargo test` and the examples put the DLLs on PATH: `$env:PATH = "$PWD\third_party\gstreamer\sdk\bin;$env:PATH"`. `pnpm tauri dev` needs no PATH change because Tauri copies the bundle next to the dev exe.
- CI: `release.yml` runs on every push to main. It calls `macos.yml` and `windows.yml` and replaces the `nightly` pre-release with the DMGs and the setup.exe. The builds are unsigned (ad-hoc on macOS).

Examples: `cargo run -p livekit-gstreamer --example get_devices` lists device ids and formats; put the ones to use in `.env` (quote values with spaces) for `stream_composite`. The LiveKit examples also need `LIVEKIT_URL`, `LIVEKIT_API_KEY` and `LIVEKIT_API_SECRET` there.

Tests use `test:` device ids (`test:video`, `test:audio:<hz>`, `test:tones:<hz>:…`, `test:zero-ts:<hz>`, `test:missing`) that map to live test sources; they are on in unit tests and behind the `test-sources` feature elsewhere.

### syncflow-publisher
SvelteKit 3 + Svelte 5 (runes, ≥5.57), Vite 8, TypeScript 6, `@sveltejs/adapter-static` 4, Tailwind CSS 4.3 + `flowbite-svelte` 1.x for UI, with Tauri v2. Requires Node ≥22.17. Use runes only; don't use legacy `$:` or stores syntax in new code.

Version pins to respect:
- TypeScript stays on 6.x: SvelteKit 3 peers on `^6`. Don't move to TS 7 until it supports it.
- flowbite-svelte stays on 1.x with the `flowbite` 3.x plugin (1.x depends on it). 2.0 switches to Flowbite 4's semantic theme tokens and will require rewriting the design rules. Upgrade only when asked.

SvelteKit 3 specifics:
- All SvelteKit config lives in the `sveltekit({...})` plugin in `vite.config.js` (adapter, preprocess). There is no `svelte.config.js`; don't recreate it.
- `tsconfig.json` extends `$app/tsconfig`.
- Import app code through `#lib/...` (Node subpath import in `package.json` `imports`) with file extensions, e.g. `#lib/store.svelte.js`, `#lib/components/types.js`. `$lib` no longer exists.
- Use `$app/state` (not `$app/stores`, which is removed) and `$app/env` (not `$app/environment`). Hook types such as `ClientInit` come from `@sveltejs/kit/hooks`.
- `goto` rejects URLs that don't match a route; use `window.location.href` for external URLs.

UI rules: summary in `.claude/rules/design-system.md`, full guide in `syncflow-publisher/docs/design-rules.md`. Design tokens are defined only in `syncflow-publisher/src/app.css`. Fonts and assets are bundled locally (the app must work offline); don't load them from a CDN.

Depends on `syncflow_client` / `syncflow_shared` from the SyncFlow repository as git dependencies on branch `main`. The commit in `Cargo.lock` is the effective pin; it's updated manually. Don't run `cargo update` on them or change their source unless asked, and don't modify them from this repo.

Depends on `livekit-gstreamer` via workspace dependencies in Cargo (`livekit-gstreamer = { path = "livekit-gstreamer" }`). Changes to its public API must be updated in `syncflow-publisher` in the same change.

Package manager is **pnpm** (v11). Do not use npm or yarn or commit another lockfile. pnpm settings (such as `allowBuilds` for packages with build scripts) go in `syncflow-publisher/pnpm-workspace.yaml`, not the `pnpm` field of `package.json`, which pnpm 11 ignores.

Vite config is `vite.config.js` (Tauri dev server on port 1420).

Commands (run from `./syncflow-publisher`):
```sh
pnpm tauri dev     # starts the SyncFlow App in local development mode, watches for local changes
pnpm tauri build   # builds installers for the current platform (fetch GStreamer first)
pnpm check         # typecheck
pnpm check:design  # design-token checker (also runs as a PostToolUse hook)
pnpm format        # Prettier formatting
```
