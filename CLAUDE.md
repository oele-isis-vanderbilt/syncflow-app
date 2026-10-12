# SyncFlow App

A client app for sharing media (screen, camera, microphone) into sessions of a single SyncFlow project. It's a Cargo workspace (root `Cargo.toml`) built from two crates:

- **`livekit-gstreamer`:** platform-specific device discovery, and GStreamer pipelines (using the Rust bindings) that can record locally and stream to SyncFlow, using tee elements and appsinks that publish to LiveKit.
- **`syncflow-publisher`:** the Tauri app (SvelteKit frontend) that uses `livekit-gstreamer` to build the client for the SyncFlow project. It provides session listening (via RabbitMQ), along with other features that let end users connect their devices to SyncFlow sessions to record locally and/or stream.

Works in two modes:

- **Offline mode:** a local recorder that records multimedia files from connected devices. `local_recording_only` currently uploads to S3 when `s3_credentials.json` exists and a device has `enable_streaming` set.
- **Listen mode:** multiple devices, each with codecs in a pre-configured pipeline, wait for a SyncFlow session to start and join it. Each can stream to LiveKit, record, or both (you can record a session without streaming). When the session ends, they exit and upload the recorded media to a pre-configured S3 bucket. RabbitMQ powers session listening (start/stop events for the project's sessions).

## Terms
- **SyncFlow**: a platform for collecting multimodal learning-analytics data in real time, built on LiveKit (WebRTC). This app talks to it through the `client` and `shared` crates from the SyncFlow repository (imported as `syncflow_client` and `syncflow_shared`).
- **Session**: a SyncFlow session in the configured project, backed by one LiveKit room. The room name is the session name.

## Package structure

```
.
├── Cargo.toml                 # workspace: livekit-gstreamer, syncflow-publisher/src-tauri
├── Cargo.lock                 # single lockfile for both crates
├── .cargo/config.toml         # per-target rustflags (static CRT on Windows, -ObjC on Apple)
├── livekit-gstreamer/
│   ├── src/
│   │   ├── devices/           # per-OS device discovery (linux.rs, mac.rs, win.rs) behind one API in mod.rs
│   │   ├── media_device.rs    # device/caps model and pipeline element selection (largest file)
│   │   ├── media_stream.rs    # single-device pipeline: tee -> local file and/or appsink -> LiveKit
│   │   ├── av_mix_stream.rs   # combined audio+video stream
│   │   └── lk_participant.rs  # publishing tracks to a LiveKit room
│   └── examples/              # runnable examples; most need LiveKit/SyncFlow creds in .env
└── syncflow-publisher/
    ├── src/                   # SvelteKit frontend
    │   ├── routes/            # / (device + mode setup), /register, /stream (sessions, local recording)
    │   └── lib/               # components/ and store.svelte.ts (shared runes state)
    ├── docs/design-rules.md   # full UI design guide
    ├── scripts/check-design.mjs # design-token checker (`pnpm check:design`)
    └── src-tauri/src/
        ├── lib.rs             # Tauri setup, AppState, all #[tauri::command]s and the invoke handler
        ├── register.rs        # device registration/deregistration with SyncFlow
        ├── session_listener.rs# RabbitMQ listener for session start/stop
        ├── syncflow_publisher.rs # per-session record/stream flow, output dirs, S3 upload trigger
        ├── s3_uploader.rs     # zip + multipart upload (rusoto)
        ├── devices.rs, models.rs, errors.rs, utils.rs
```

Where changes go: device discovery, pipeline construction, encoding, and LiveKit publishing belong in `livekit-gstreamer`. Mode logic, RabbitMQ listening, S3 upload, and UI belong in `syncflow-publisher`. Keep `livekit-gstreamer` free of SyncFlow-specific logic.

Frontend to Rust: the frontend calls Rust only through `invoke(...)` on commands registered in `lib.rs` `generate_handler!`. Rust pushes updates with `emit` on the `session-update` and `publication-notification` events. A new command must be added to `generate_handler!`, or `invoke` fails at runtime.

Platform-specific code: OS differences in device handling go in `livekit-gstreamer/src/devices/{linux,mac,win}.rs`, behind `#[cfg(target_os)]`. Keep the three files exposing the same functions as `devices/mod.rs` re-exports.

### App data on disk
Everything lives in `~/.syncflow-publisher/`:

- `credentials.json`: SyncFlow API key/secret and RabbitMQ password (plaintext).
- `s3_credentials.json`: S3 access/secret key (plaintext).
- `registration.json`, `recording_mode.json`, streaming config.
- `recordings/<session_id>-<session_name>-<participant>/`: recordings, zipped next to the folder before upload. Offline sessions use `local-<nanos>` as the session id.

Rules:
- Never log, print, or emit the contents of the credential files, and never commit sample files with real values.
- Don't delete recordings or zips in code without a confirmed successful upload. These are study data.
- Don't change the recordings folder naming or the S3 key format (`<project>-<id>/<session>/local-recordings/<participant>.zip`) without asking. Downstream analysis depends on them.

## Stack and commands

Run Rust commands from the repo root (workspace):
```sh
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test --workspace   # very few tests yet; examples are the real checks
```

### livekit-gstreamer
Assumes GStreamer and its development files are already installed on the system. Packaging GStreamer is planned; don't try to bundle or vendor it unless asked. Required: GStreamer Full Installation. Uses tokio and the LiveKit Rust SDK to provide streaming/recording capabilities as tasks.

Examples: `cargo run -p livekit-gstreamer --example get_devices`. Most other examples need LiveKit/SyncFlow credentials in a `.env` (loaded with `dotenvy`).

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
pnpm tauri build   # builds installers for the current platform
pnpm check         # typecheck
pnpm check:design  # design-token checker (also runs as a PostToolUse hook)
pnpm format        # Prettier formatting
```
