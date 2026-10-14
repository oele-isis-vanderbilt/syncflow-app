# Composite pipeline refactor: architecture plan

## 1. Context

`livekit-gstreamer` records each device in its own GStreamer pipeline. The exception is `AvMixStream`: one camera plus 1–2 mics, downmixed to mono. This design has three problems:

1. **Appsinks can't be identified.** Elements, appsinks and pipelines are named `random_string(..)`, and track names ignore the `track_name` argument.
2. **Errors are swallowed.** `run_bus_loop` (`media_device.rs:227`) writes `.error.json` only when recording, then returns `Ok`. The app, the UI and the room never hear about it.
3. **Sync has to be fixed afterwards.** Each pipeline has its own latency and running time, so files from different devices need costly alignment and stitching after the fact.

Separately, the file and folder names can't be read by people. A real example: `<uuid>-Lecture 3-Host-1a2b3c4d(grp)/video-FaceTime_HD-0x14200005ac-Xy83kdP0q9ZtR2mA-2026-10-14-11-38-02.mp4`.

**Target:** one `CompositeStream` built on **one** `gst::Pipeline`, with one clock, one base time and one negotiated latency. It has:

| Part | File branch | Stream branch (optional, per output) |
|---|---|---|
| (a) AV mix: camera + 1–2 mics | `av-mix.mp4`: H.264 + AAC. With 2 mics the audio is **stereo, L = mic-1 and R = mic-2**; with 1 mic it is mono. Each mix mic also gets its own `mic-N_<dev>.m4a`. | Video track. **Mono** audio track: L+R downmix, then `audiornnoise`. |
| (b) Screenshare | `screen-N_<dev>.mp4` | Screenshare track |
| (c) Additional cameras | `camera-N_<dev>.mp4` each | Camera track each |
| (d) Additional mics | `mic-N_<dev>.m4a` each, at the device's configured channel count | Mono mic track each, through `audiornnoise` |

**Errors:**
- They reach the app UI in both modes.
- In session mode they are also published to the room on topic `ROOM_EVENTS`, carrying the participant identity, the session and the details of the failed device.

**Decided with the user:**
- **Naming:** the folder and file names become readable. The **S3 key format is unchanged** (`<project>-<id>/<session>/local-recordings/<participant>.zip`).
- **Topic:** `ROOM_EVENTS`.
- **Failures:** a failure is **isolated to the failed device**. Its files finalize, and everything else keeps recording.
- **Mix mics:** each one also writes its own `.m4a`.

**Verified on GStreamer 1.28.8**, both the bundled `third_party/gstreamer/macos/GStreamer.framework` and Homebrew:
- `audiornnoise` (from `rsaudiofx`) is present. Its caps are F32LE at 48000 Hz, interleaved, with any number of channels. It has one property, `voice-activity-threshold`.
- `audiomixer` and `mp4mux` are `GstAggregator` subclasses. They have `latency`, `start-time-selection` and `ignore-inactive-pads`, and per-pad `volume` and `mute`.
- `interleave` is a plain `GstElement`, so it has no live timeout handling.
- `mp4mux` has `fragment-duration`, `fragment-mode=first-moov-then-finalise` and `moov-recovery-file`.
- `audioconvert` has `mix-matrix`.
- `osxaudiosrc` has `provide-clock` and `slave-method`.
- `tee` has `allow-not-linked`.

---

## 2. Pipeline topology

Each box below is a group of elements. §4 describes every element in turn.

```
                                   ┌──────────────────────── FILE: av-mix.mp4 ───────────────────────┐
CAMERA-1 CHAIN ─▶ tee[camera-1] ─┬─▶ queue ▶ videorate ▶ h264 enc ▶ h264parse ─▶┐                    │
                                 │                                    mp4mux(av-mix) ▶ filesink      │
                                 └─▶ STREAM av-mix-video: queue(leaky) ▶ videoscale ▶ caps ▶ appsink  │
                                                                                 ▲                    │
MIC-1 CHAIN ─▶ tee[mic-1] ─┬─▶ FILE mic-1.m4a: queue ▶ aac ▶ mp4mux ▶ filesink    │                    │
                           └─▶ queue ▶ audioconvert(matrix→L) ─┐                  │                    │
MIC-2 CHAIN ─▶ tee[mic-2] ─┬─▶ FILE mic-2.m4a  …               ├▶ audiomixer(LR) ▶ tee[mix-lr] ─┬▶ queue ▶ aac ┘
                           └─▶ queue ▶ audioconvert(matrix→R) ─┘                               └▶ STREAM av-mix-audio:
                                                                    queue(leaky) ▶ audioconvert(→mono F32 48k) ▶ audiornnoise ▶ audioconvert(S16) ▶ appsink
SCREEN-1 CHAIN ─▶ tee ─┬▶ FILE screen-1.mp4: queue ▶ videorate ▶ h264 enc ▶ h264parse ▶ mp4mux ▶ filesink
                       └▶ STREAM screen-1: queue(leaky) ▶ videoscale ▶ caps ▶ appsink
CAMERA-N CHAIN ─▶ tee ─┬▶ FILE camera-N.mp4 (as above)       └▶ STREAM camera-N (as above)
MIC-N CHAIN    ─▶ tee ─┬▶ FILE mic-N.m4a                      └▶ STREAM mic-N: queue(leaky) ▶ audiornnoise chain ▶ appsink

SHARED MULTICHANNEL DEVICE (e.g. UMC1820 feeding mic-1 = ch1, mic-2 = ch2, mic-3 = ch3):
DEVICE CHAIN dev0 ─▶ tee[dev0] ─┬▶ ROLE mic-1: queue ▶ audioconvert(matrix: pick ch1) ▶ … ▶ tee[mic-1]
                                ├▶ ROLE mic-2: queue ▶ audioconvert(matrix: pick ch2) ▶ … ▶ tee[mic-2]
                                └▶ ROLE mic-3: queue ▶ audioconvert(matrix: pick ch3) ▶ … ▶ tee[mic-3]
```

**Layers, from source to sink:**
1. **Device chains:** one per *physical* device.
2. **Role chains:** one per role (camera-N, mic-N, screen-N). Each ends in a role tee.
3. **Mix:** only for the AV mix.
4. **Output branches:** files.
5. **Sink branches:** streams.

Every link points downstream only. A failed device can therefore be cut off at the device chain, and the effect spreads to exactly the branches it feeds (§6).

---

## 3. Library types (`livekit-gstreamer/src/composite/`)

### ids.rs: identity

```rust
pub enum Role { Camera(u8), Mic(u8), Screen(u8) }            // "camera-1", "mic-3", "screen-1"
pub enum OutputId { AvMix, Camera(u8), Mic(u8), Screen(u8) } // one per file: "av-mix", "camera-2", "mic-1"
pub enum SinkId { AvMixVideo, AvMixAudio, Camera(u8), Mic(u8), Screen(u8) } // one per appsink/track
pub struct DeviceKey(pub u8);                                // "dev0": one per physical device
pub struct DeviceRef { pub key: DeviceKey, pub device_id: String, pub display_name: String,
                       pub kind: DeviceKind /* Camera|Mic|Screen */, pub codec: String }
pub struct RoleRef { pub role: Role, pub device: DeviceKey, pub selected_channel: Option<u8> }
```

- **Numbering is global per kind.** The AV mix takes camera-1, mic-1 and mic-2. Extras continue from camera-2, mic-3 and so on. The screen is screen-1.
- **`DeviceKey`** is assigned in spec order to each distinct `device_id`.

### spec.rs: what to build

```rust
pub struct CompositeStreamSpec {
    pub av_mix: Option<AvMixSpec>,             // camera: VideoPublishOptions, mics: Vec<AudioPublishOptions> (1..=2), stream: bool
    pub screen: Option<Output<ScreenPublishOptions>>,
    pub cameras: Vec<Output<VideoPublishOptions>>,
    pub mics: Vec<Output<AudioPublishOptions>>,
    pub output_dir: Option<PathBuf>,           // None → no file branches (stream-only; library use)
    pub stream_audio: StreamAudioOptions,      // noise_suppression: bool (default true), vad_threshold: f32 (default 0.0 = off)
    pub encoders: EncoderOptions,              // h264: Auto|X264|Hardware, video_bits_per_pixel (0.12), video_kbps override, aac_kbps_per_channel
    pub start_policy: StartPolicy,             // BestEffort (default) | AllOrNothing
    pub manifest_context: Option<serde_json::Value>, // caller context written under `context` in manifest.json
}
pub struct Output<T> { pub options: T, pub stream: bool }
```

**`validate() -> Result<ResolvedSpec, SpecError>`** assigns roles, `DeviceKey`s, `OutputId`s and `SinkId`s. It rejects:
- an empty spec;
- more than 2 mix mics;
- the same `(device_id, selected_channel)` used twice;
- `stream: false` with no `output_dir`;
- a `selected_channel` greater than `channels`;
- conflicting formats for roles that share one device (codec, rate, channels).

Format support (`supports_video/audio/screen_share`, `media_device.rs:1315-1380`) needs real devices, so it is checked in `start()` instead. `validate()` reads display names once from `get_devices_info()` and delegates to the pure `validate_with_names()`, which the tests use.

The **`ResolvedSpec`** is the whole graph plan as plain data: devices, roles, outputs, sinks, and the edges between them. The builder, the bus attribution logic and the manifest all read from it, and it is unit-testable without GStreamer.

### tags.rs: element naming

Every element is named `<scope>:<purpose>`, and `ElementTag::parse(name)` maps it back to its scope.

| Scope prefix | Meaning | Examples |
|---|---|---|
| `dev0` | Device chain | `dev0:src`, `dev0:caps`, `dev0:tee` |
| `mic-1`, `camera-2`, `screen-1` | Role chain | `mic-1:pick`, `mic-1:resample`, `mic-1:tee` |
| `mix.lr` | AV mix | `mix.lr:to-l`, `mix.lr:mixer`, `mix.lr:tee` |
| `file.av-mix`, `file.mic-3` | Output branch | `file.av-mix:mux`, `file.mic-3:aac`, `file.mic-3:sink` |
| `sink.av-mix-audio`, `sink.camera-2` | Stream branch | `sink.av-mix-audio:rnnoise`, `sink.av-mix-audio:appsink` |

The bus walks `msg.src()` up to the first tagged element. The scope plus the `ResolvedSpec` then gives the device, roles, outputs and sinks affected.

### Other modules

| Module | Contents |
|---|---|
| `naming.rs` | `device_slug("FaceTime HD Camera") = "FaceTime-HD-Camera"`: ASCII letters, digits and `-`, at most 32 characters. `output_file_name(OutputId, &DeviceRef, selected_channel)` gives `av-mix.mp4`, `mic-3_UMC1820-ch3.m4a`, `camera-2_Logitech-C920.mp4`, `screen-1_Display-1.mp4`. The role prefix makes names unique, and there are no random strings or timestamps. |
| `manifest.rs` | `RecordingManifest` (§8). |
| `events.rs` | `CompositeEvent`, `ErrorInfo`, `FailurePhase`, `StopReason` (§6). |
| `builder.rs` | One function per layer (§4). Each returns a handles struct holding the elements and pads that isolation needs. |
| `bus.rs` | The bus task, attribution and isolation (§6). |
| `stream.rs` | `CompositeStream` (§5). |

---

## 4. Every element: role, settings, relationships

### 4.1 Device chain (one per physical device; scope `devK`)

| Element | Settings | Why / relationship |
|---|---|---|
| **Source** | Camera: `gstreamer::Device::create_element`, as today (`get_video_element`, `media_device.rs:1856`). Mic: `get_audio_element` (`:1865`). Screen: per-OS `get_screen_element` (`:1778` win `dx9screencapsrc`, `:1802` mac `avfvideosrc capture-screen=true` + crop, `:1831` linux `ximagesrc`). Audio sources get `provide-clock=false` (§5.2). | Live source. Its errors are the main ones to isolate. Today's `unwrap()` on a missing device becomes a `StartError` that names the device. |
| `capsfilter` (`devK:caps`) | Requested caps. Video: codec, w, h, fps. Audio: `audio/x-raw, channels=N, rate=R`, with **no** `channel-mask`. | Pins negotiation to the options the user chose. No mask is set, so a multichannel interface passes through whether it reports positioned or unpositioned channels: `audioconvert` can't convert unpositioned channels to positioned ones (a test with an unpositioned 4-channel source failed with `not-negotiated` when a mask was forced), and the role chain picks channels with `mix-matrix`, which works for either. |
| Video decode | `jpegdec` (MJPEG), `h264parse ! avdec_h264` (H.264), or nothing (raw). Then `videoconvert ! capsfilter(I420, w, h, PAR 1/1)`. | Moved from `video_xraw/xh264/image_jpeg_pipeline` (`:1411-1700`). Raw I420 feeds both the encoder and the stream scaler. |
| Audio normalize | `audioconvert ! audioresample ! capsfilter(F32LE, 48000, channels=N)` | 48 kHz F32 is what `audiornnoise` needs and what LiveKit's `NativeAudioSource` uses. Doing the conversion once per device avoids converting again for each role. |
| `tee` (`devK:tee`) | `allow-not-linked=true` | Fans a device out to its roles. Shared multichannel devices fan out to several roles; an ordinary device has exactly one. `allow-not-linked` keeps the tee from failing while branches are being removed. |

### 4.2 Role chain (scope `mic-1`, `camera-2`, …)

| Element | Settings | Why / relationship |
|---|---|---|
| `queue` (`role:q`) | `max-size-time=1s`, others 0, not leaky | Puts each role on its own thread, so one slow role can't stall its siblings on a shared device. |
| Mic: `audioconvert` (`role:pick`) | `mix-matrix` = 1×N row selecting `selected_channel`. With no channel selected, it averages the N channels to mono for mix roles, and keeps N for extra-mic files. | **Replaces `deinterleave`** (`:1052`). Its matrix is static, so there are no `pad-added` dynamic pads, the graph is known before PLAYING, and isolation stays simple. |
| Mic: `audiorate` (`role:rate`) | `tolerance=40ms`, `skip-to-first=true` | Fills short gaps and drops overlaps, so each mic's sample count matches wall time. Its `add`/`drop` counters go into the manifest as a sync-quality signal per mic. |
| `tee` (`role:tee`) | `allow-not-linked=true` | Fans the role out to its file branch, the L/R mix (mix mics only) and the stream branch. |
| Video and screen | No extra elements. The device tee is the role tee. | A camera has one role. |

### 4.3 L/R mix (only when the AV mix has 2 mics; scope `mix.lr`)

| Element | Settings | Why / relationship |
|---|---|---|
| `queue` per input | `max-size-time=1s` | Decouples the role tee from the mixer. |
| `audioconvert` (`mix.lr:to-l`, `mix.lr:to-r`) | `mix-matrix` mono→stereo: `[[1.0],[0.0]]` for L, `[[0.0],[1.0]]` for R. Followed by a caps filter: F32LE, 48 kHz, 2 channels, mask L\|R. | Puts mic-1 only on L and mic-2 only on R. Since each output channel has exactly one non-zero input, the "mix" is a placement, not a sum. |
| `audiomixer` (`mix.lr:mixer`) | `latency=40ms`, `start-time-selection=first`, `ignore-inactive-pads=true`, pad `volume=1.0`, `output-buffer-duration=10ms` | **Chosen over `interleave`** because it is an aggregator. It is live-aware, times out waiting for a stalled input, and keeps producing when one pad gets EOS. A failed mic therefore leaves a silent channel and the mix keeps going (§6). Pad volume is 1.0 because nothing is summed. |
| `tee` (`mix.lr:tee`) | `allow-not-linked=true` | Feeds the AAC branch of `av-mix.mp4` and the mono stream branch. |
| **1 mic** | No mixer. mic-1's role tee feeds `av-mix.mp4` as mono AAC and the stream branch. | A mixer would add nothing. |

### 4.4 File branches (scope `file.<output>`; only when `output_dir` is set)

| Element | Settings | Why / relationship |
|---|---|---|
| `queue` (`file:q`) | **Not leaky**, `max-size-time=3s`, `max-size-buffers=0`, `max-size-bytes=0` | Files must not drop data. The 3 s buffer absorbs encoder spikes. Only stream branches are leaky, so streaming can never hold files back. Today's video file queue is leaky; it changes to this. |
| `videorate` (video) | `max-rate=<fps>`, `drop-only` implied | Caps the frame rate without duplicating frames. MP4 handles a variable frame rate, so audio/video sync rests on timestamps, not on frame counts. |
| H.264 encoder | `EncoderOptions::Auto` uses `vtenc_h264_hw` on macOS and `mfh264enc` on Windows when available, falling back to `x264enc speed-preset=ultrafast bitrate=<kbps> key-int-max=<fps>`. The bitrate scales with each file's size: `width × height × fps × 0.12` bits per pixel (8.7 Mbps at 1552×1552@30, 3.3 Mbps at 1280×720@30), or a fixed `video_kbps` override. VideoToolbox runs with `rate-control=cbr`: in its default average-bitrate mode it delivered about a third of the target on a real camera (1.0 of 3 Mbps at 1552×1552), visibly softening the video. | Two or three 1080p x264 encodes plus a screen are heavy on CPU, so hardware encoding is preferred. `key-int-max = fps` gives 1 s keyframes that line up with 1 s MP4 fragments, keeping cut-off files seekable. Today's encoder is x264, so the fallback matches. Properties are set only where the chosen encoder has them: `bitrate`, then a 1 s keyframe interval through `key-int-max` (x264), `max-keyframe-interval` (vtenc, plus `realtime=true` and `allow-frame-reordering=false`) or `gop-size` (mfh264enc). |
| `h264parse` | `config-interval=-1` | Delivers stream-format avc, aligned to access units, for `mp4mux`. |
| `audioconvert` (+ caps for > 2 channels) | Extra mics with more than 2 channels: identity `mix-matrix` plus a caps filter with every channel bit set | AAC needs positioned channels, and interface channels may arrive unpositioned. With a mix matrix, `audioconvert` can assign positions. |
| `avenc_aac` | `bitrate = 128 kbps × channels` | Same codec as today. Stereo L/R gets 256 kbps. Takes interleaved F32LE, 1–16 channels. |
| `aacparse` |  | Framing for `mp4mux`. |
| `mp4mux` (`file:mux`) | `fragment-duration=1000`, `fragment-mode=first-moov-then-finalise`, `latency=40ms`, `start-time-selection=first`. Today's `faststart=true` is dropped; it doesn't work with fragments. | **Crash safety:** while recording, the file is a playable fragmented MP4. A clean EOS finalizes it into a normal MP4. A `kill -9` leaves a readable file. |
| `filesink` (`file:sink`) | `location=<output_dir>/<output_file_name>`, `sync=false`, `async=false` | Writes as soon as data arrives. Its sink-pad EOS probe is the "output finalized" signal (§6). |

The `av-mix` mux has two request pads: `video_0` from tee[camera-1], and `audio_0` from tee[mix-lr] (or tee[mic-1] when there is 1 mic).

### 4.5 Stream branches (scope `sink.<sink-id>`; only when `stream: true`)

| Element | Settings | Why / relationship |
|---|---|---|
| `queue` (`sink:q`) | `leaky=downstream`, `max-size-buffers=2` (video) or `max-size-time=200ms` (audio) | Network or LiveKit slowness never backs up into the tee, so files stay safe. |
| Video: `videoscale ! videoconvert ! capsfilter(I420, 640x480)` |  | Matches what `video_track_task` expects today: tightly packed I420 at 640x480 (`lk_participant.rs:462`). |
| Audio: `audioconvert` (to mono), `capsfilter(F32LE, 48k, 1ch)` | Any multichannel input (the L/R mix, or an extra interface mic) is downmixed by averaging with an explicit `mix-matrix` | **Streaming sees only mono.** The explicit matrix is required: `audioconvert`'s automatic downmix fails on unpositioned interface channels, and `audiornnoise` then panics on a buffer without caps. |
| `audiornnoise` (`sink:rnnoise`) | `voice-activity-threshold = stream_audio.vad_threshold`, default 0.0, meaning denoise only and no gating | One instance per streamed audio track, and only on the stream branch, so files keep the raw audio. If the element factory is missing, the branch is built without it and a `Warning` event is emitted. |
| `audioconvert ! capsfilter(S16LE, 48k, 1ch)` |  | `audio_track_task` (`:504`) reads the data as `&[i16]`. |
| `appsink` (`sink:appsink`) | Named `sink.<sink-id>:appsink`. `sync=false`, `async=false`, `emit-signals=true`. Video: `drop=true max-buffers=1`. Audio: `drop=false max-buffers=8`. | Based on `broadcast_appsink` (`:1876`), with a deterministic name. Audio isn't dropped at the appsink, because the leaky queue upstream already bounds it and dropped audio sounds like clicks. `SinkInfo` carries the caps, which drive `NativeAudioSource` and `NativeVideoSource`. |

---

## 5. Lifecycle: `CompositeStream`

```rust
impl CompositeStream {
    pub fn new(spec: CompositeStreamSpec) -> Self;
    pub async fn start(&mut self) -> Result<StartReport, CompositeError>; // StartReport { failed_at_start: Vec<(DeviceRef, ErrorInfo)> }
    pub async fn stop(&mut self) -> Result<RecordingManifest, CompositeError>;
    pub fn sinks(&self) -> Vec<SinkInfo>;            // SinkInfo { id, roles, kind: Video{w,h} | Audio{rate, channels} }
    pub fn subscribe(&self, id: &SinkId) -> Option<SinkSubscription>; // frames + close receivers
    pub fn events(&self) -> broadcast::Receiver<CompositeEvent>;
    pub fn devices(&self) -> Vec<DeviceRef>;
    pub fn live_outputs(&self) -> Vec<OutputId>;
}
```

### 5.1 Start

1. `validate()` → `ResolvedSpec`. A failure here returns `CompositeError::Spec`.
2. **Build**, adding each layer in order: device chains, role chains, mix, then file and stream branches.
   - **A device that can't be created** (missing, or permission denied):
     - Under `BestEffort`, everything that depends only on it is dropped from the plan, and it is recorded in `failed_at_start`. Anything it added to the pipeline is removed. An AV mix missing one mic keeps its L/R layout with that side silent, so the manifest's channel map stays true. An AV mix missing its camera records audio only.
     - Under `AllOrNothing`, `start()` fails.
3. **Set clock and state.** `pipeline.use_clock(SystemClock)` (§5.2), then PLAYING. Live pipelines return `NO_PREROLL`.
   - Wait up to 10 s for `AsyncDone` or the first buffer on every live branch.
   - **A device that errors during this window** is handled like a runtime failure (§6), with `phase=Start`.
4. Record `base_time` and the wall clock at that moment (`SystemTime::now()`) in the manifest. Write `manifest.json` with status `Recording` (§8).
5. Spawn the bus task (§6).

### 5.2 Clock and sync

- **The pipeline clock is `SystemClock`, monotonic, set explicitly with `use_clock`.** It is not left to whichever audio source happens to provide one.
- **Why:** if the clock came from a mic and that mic were unplugged, GStreamer would post `CLOCK_LOST` and the whole pipeline would have to re-select a clock, interrupting every recording. That breaks failure isolation.
- **Audio sources are re-timestamped on arrival** (`retimestamp_on_pipeline_clock` in `builder.rs`): each buffer gets `pipeline clock now − base time − buffer duration`, and `audiorate` smooths the arrival jitter into continuous samples. `slave-method` does not work here: `osxaudiosrc` (and likely `wasapi2src`) supplies its own ring-buffer timestamps from the device clock, which counts from zero, and GStreamer skips slaving when a source supplies timestamps. Against the system clock's base time they all clamp to 0, and `audiorate` then dropped everything after the first ~70 ms (found on a MacBook with two USB mics; covered by the `test:zero-ts` regression test).
- **Every branch shares one running time.** A timestamp t in `av-mix.mp4`, `camera-2.mp4` and `mic-3.m4a` refers to the same instant, so no post-hoc stitching is needed within a machine.
- **Across machines,** the manifest stores `wallclock_at_base_ns`, the NTP-disciplined system time. This allows coarse (millisecond-level) alignment with other participants. A `GstNetClientClock` or PTP clock is a possible future step, not in scope.

### 5.3 Stop (cancel, room `Disconnected`, user Stop)

1. Send `pipeline.send_event(Eos)`. Every source pushes EOS, which flows through every live branch.
2. Each filesink EOS probe emits `OutputFinalized`. Wait until all live outputs report it, or 10 s.
3. Set NULL. Finalize `manifest.json` with status `Completed`, or `CompletedWithErrors`.
4. **On timeout:** set NULL anyway. Fragmented MP4s stay readable. The manifest marks the unfinished outputs as `Partial`.
5. `Drop` runs the same steps with a 5 s budget, so a dropped stream still finalizes its files.

---

## 6. Failure handling: detection, attribution, isolation, events

```rust
pub enum CompositeEvent {
    DeviceFailed { device: DeviceRef, roles: Vec<Role>, outputs_affected: Vec<OutputId>, sinks_closed: Vec<SinkId>,
                   phase: FailurePhase /* Start | Running */, error: ErrorInfo, recording_continues: bool },
    OutputFailed { output: OutputId, error: ErrorInfo, recording_continues: bool },
    SinkFailed   { sink: SinkId, error: ErrorInfo },               // stream branch only; files unaffected
    OutputFinalized { output: OutputId, path: PathBuf },
    Warning { scope: Option<String>, message: String },            // e.g. audiornnoise missing, audiorate drops
    Stopped { reason: StopReason /* Requested | AllOutputsDone | Fatal(ErrorInfo) | IsolationTimeout */ },
}
pub struct ErrorInfo { pub element: String, pub domain: String, pub code: i32, pub message: String, pub debug: Option<String> }
```

**Attribution.** The bus task reads each `MessageView::Error`. `ElementTag::parse` on `msg.src()` (walking up through parents) gives the scope:

| Scope | Unit isolated |
|---|---|
| `devK` | The device: all its roles, their outputs and their sinks |
| `mic-N`, `camera-N` | The role. Same as the device for single-role devices. |
| `mix.lr` | The `av-mix` audio, treated as an output failure |
| `file.X` | Output X |
| `sink.X` | Sink X, stream only |
| Untagged (the pipeline or the clock) | Fatal: stop everything |

**Device isolation**, run on the bus task with a 5 s budget:
1. Add a blocking probe on `devK:src`'s src pad.
2. Send EOS into the next element's sink pad. EOS flows down every path the device feeds, through the device tee, the role tees and the mixer, and along each path:
   - **File branch:** the mux finalizes or ends the stream. Detected by the `OutputFinalized` probe.
   - **Mixer input:** the `audiomixer` pad goes EOS, and the mixer keeps going with that channel silent.
   - **`av-mix` mux video pad:** that pad goes EOS, and the mux keeps writing audio.
   - **Stream branch:** the appsink receives EOS, the close channel fires, and the track is unpublished.
3. Set the device's source chain to NULL, unlink it and remove it from the bin.
4. Emit `DeviceFailed`. `recording_continues` is true when `live_outputs()` is still non-empty.

**Output isolation** (an encoder, mux or filesink error):
1. Block the role tee's src pad for that branch.
2. Send EOS into the branch's queue.
3. Release the tee request pad, then remove the branch.
4. Emit `OutputFailed`.

**Sink isolation:** remove the stream branch the same way. Emit `SinkFailed`. Files are untouched.

**Fallback:** if isolation hasn't completed within 5 s, run a whole-pipeline stop (§5.3) and emit `Stopped{IsolationTimeout}`.

**Bug fixes carried over:**
- Only `StateChanged` messages from the pipeline itself are considered. Today any element going to NULL ends the loop.
- Errors are returned, not swallowed.
- The `.error.json` files go away. Errors are recorded in the manifest and the per-file JSON instead.

---

## 7. Scenarios

### 7.1 Configuration scenarios

| # | Configuration | Graph built | Files | Tracks (if `stream`) |
|---|---|---|---|---|
| C1 | AV mix, 1 mic | dev(cam), dev(mic) → camera-1, mic-1 → `av-mix` (mono AAC) | `av-mix.mp4`, `mic-1_*.m4a` | av-mix-video, av-mix-audio (mono, rnnoise) |
| C2 | AV mix, 2 mics | + `mix.lr` (L = mic-1, R = mic-2) | `av-mix.mp4` (stereo L/R), `mic-1_*.m4a`, `mic-2_*.m4a` | av-mix-video, av-mix-audio (L+R → mono, rnnoise) |
| C3 | C2 + screen | + dev(screen) → screen-1 | + `screen-1_*.mp4` | + screen-1 (Screenshare) |
| C4 | C2 + extra camera + extra mic | + camera-2, mic-3 | + `camera-2_*.mp4`, `mic-3_*.m4a` (device channels) | + camera-2, mic-3 (mono, rnnoise), each per its toggle |
| C5 | No AV mix, extras only | camera-N / mic-N chains only; no `mix.lr`, no `av-mix` | per-device files | per-device tracks |
| C6 | Shared multichannel device (UMC1820: ch1 → mic-1 L, ch2 → mic-2 R, ch3 → mic-3) | **one** `dev0:src` → tee → three role chains, each with its own `pick` matrix | as in C2/C4 | as toggled |
| C7 | Everything record-only | No `sink.*` branches built at all | all files | none. In session mode the participant still joins the room, so `ROOM_EVENTS` still works. |
| C8 | Stream-only (library: `output_dir: None`) | No `file.*` branches; no manifest files | none | all sinks. The app always records, so this is library-only. |
| C9 | Two identical camera models | Distinct `DeviceKey`s and roles | `camera-1_…` and `camera-2_…` (unique by role prefix) | distinct track names |

### 7.2 Lifecycle scenarios

| # | Scenario | Behaviour | Events / UI / room |
|---|---|---|---|
| L1 | Normal start | §5.1. The manifest is written with status `Recording`. | UI: streaming or recording status, as today |
| L2 | Device missing or busy at start (BestEffort) | The device is dropped from the plan, and the rest starts | `DeviceFailed{phase: Start}` → UI Alert, plus `ROOM_EVENTS` once the room is connected |
| L3 | Device missing at start (AllOrNothing) | `start()` returns an error naming the device | Start error → UI Alert and room event |
| L4 | User stop, cancel, or room `Disconnected` | §5.3 | `OutputFinalized`s, then `Stopped{Requested}`. Upload follows. |
| L5 | App killed (`kill -9`) or power loss | Fragmented MP4s stay readable. `manifest.json` stays at `Recording` with each output's last known state. | On the next launch, the UI could flag folders still marked `Recording` (future) |
| L6 | Stop hangs | 10 s timeout, then NULL. Unfinished outputs are marked `Partial`. | `Stopped{IsolationTimeout}` warning in the UI |

### 7.3 Failure scenarios (runtime)

| # | Failure | Isolated unit | Result | Events |
|---|---|---|---|---|
| F1 | Extra mic (mic-3) unplugged | dev(mic-3) | `mic-3_*.m4a` finalizes, the mic-3 track is unpublished, everything else continues | `DeviceFailed{recording_continues: true}` |
| F2 | Mix mic-2 unplugged | dev(mic-2) | `mic-2_*.m4a` finalizes. `av-mix.mp4` continues with R silent. The stream continues as mono, at about half level, because one input is now zero. | DeviceFailed |
| F3 | Both mix mics unplugged | both devs | Both `.m4a` files finalize. Once all inputs hit EOS the mixer goes EOS, so the `av-mix` audio pad ends and the av-mix-audio track is unpublished. `av-mix.mp4` continues as video only. | 2 × DeviceFailed |
| F4 | AV mix camera unplugged | dev(camera-1) | The `av-mix` video pad ends and `av-mix.mp4` continues as audio only. av-mix-video is unpublished. | DeviceFailed |
| F5 | All AV mix devices fail | all | `av-mix.mp4` finalizes. Other outputs continue. | DeviceFailed × n, OutputFinalized |
| F6 | Shared UMC1820 fails | dev0 → mic-1, mic-2, mic-3 | All three roles end together, which is F2 + F3 + F1 combined | **One** DeviceFailed listing 3 roles and their outputs |
| F7 | Screen capture lost (permission revoked or display unplugged) | dev(screen) | `screen-1_*.mp4` finalizes, and the screen track is unpublished | DeviceFailed |
| F8 | Encoder or mux error on one output | `file.X` | Only that file branch is removed. The device keeps feeding its other branches. | OutputFailed |
| F9 | Disk full | every `file.*` eventually | Each output fails in turn. When none are left, the pipeline stops. | OutputFailed × n, then `Stopped{AllOutputsDone}` |
| F10 | `audiornnoise` missing (some Linux installs) | none | The stream branch is built without denoising | Warning (UI only) |
| F11 | LiveKit network drop | none (in the pipeline) | The leaky stream queues drop frames, and files are unaffected. The LiveKit SDK reconnects. | Not a pipeline event. The publisher shows the room state as today. |
| F12 | Pipeline-level or untagged error | everything | Whole stop (§5.3) | `Stopped{Fatal}` → UI and room |
| F13 | Isolation exceeds 5 s | everything | Whole stop | `Stopped{IsolationTimeout}` |
| F14 | Device plugged back in | not supported in v1. It stays failed until the next session. | — | — |

---

## 8. Naming, folders and manifest

- **Folder** (built in the publisher): `recordings/<YYYY-MM-DD_HH-MM>_<session-slug>_<short-id>/`.
  - `short-id` is the first 6 characters of the session id, or `local` in offline mode.
  - The session slug uses the same rules as `device_slug`, up to 40 characters.
  - This replaces `syncflow_publisher.rs:128-131` and `:516-521`.
- **Files** (built in the library, `naming.rs`):
  - `av-mix.mp4`, `mic-1_USB-Mic-A.m4a`, `mic-2_USB-Mic-B.m4a`, `camera-2_Logitech-C920.mp4`, `mic-3_UMC1820-ch3.m4a`, `screen-1_Display-1.mp4`.
  - Each file has a `<stem>.json` (`RecordingMetadata`, `media_device.rs:42`), extended with `status`, `first_pts` and `last_pts`. The pts values are filled by probes on each muxer pad, fixing `FileSinkTiming`, which was never set before.
  - The AV mix `source` is fixed; today it is `"screen-share"` (`av_mix_stream.rs:161`).
- **`manifest.json`:**
  - The library produces a `RecordingManifest`:
    ```
    { status: "recording" | "completed" | "completed-with-errors",
      clock: "monotonic-system", base_time_ns, wallclock_at_base_ns,
      devices: [DeviceRef], roles: [RoleRef],
      outputs: [{ id, file, roles, audio_channels: {L: "mic-1", R: "mic-2"} | null,
                  status: "finalized" | "failed" | "partial", first_pts, last_pts, error? }],
      audio_continuity: [{ role, samples_added, samples_dropped }],
      events: [CompositeEvent…] }
    ```
  - It is written at start, rewritten on every failure event and finalized at stop.
  - The library writes `manifest.json` itself. The publisher passes `{ session: {id, name}, project, participant: {identity, name}, recording_mode, app_version }` as `CompositeStreamSpec::manifest_context`, which is stored verbatim under `context`.
- **Upload:** the zip is still built next to the folder. **The S3 key is unchanged** (`:393-398`, `:705-708`).
  - New rule: upload when at least one output finalized. Today any failure skips the upload.
  - Nothing is deleted locally.

---

## 9. LiveKit publishing (`lk_participant.rs`)

- **`publish_composite(&mut self, &CompositeStream) -> Result<Vec<SinkId>, LKParticipantError>`** publishes every entry in `sinks()`:

  | Sink | Track source | Format |
  |---|---|---|
  | AvMixVideo, Camera(n) | Camera | VP9, 640x480 |
  | Screen(n) | Screenshare | VP9, 640x480 |
  | AvMixAudio, Mic(n) | Microphone | `NativeAudioSource(48000, 1ch)` |

- **Naming:** the track name is `<participant>-<sink-id>`. `published_tracks` is keyed by `SinkId`.
- **Unpublishing:** a watcher task on `events()` unpublishes the `sinks_closed` of each `DeviceFailed`, `OutputFailed` and `SinkFailed`.
- **`publish_data(&self, topic: &str, payload: Vec<u8>, reliable: bool)`:** a generic wrapper with nothing SyncFlow-specific.
- **Bug fixes:**
  - `video_track_task`/`audio_track_task` (`:462`, `:504`) spin on `RecvError::Closed`. They should break.
  - `unpublish_track` should remove the map entry.

---

## 10. syncflow-publisher

### Config

- `CompositeConfig { av_mix: Option<{camera, mic_1, mic_2?, stream}>, screen: Option<{options, stream}>, cameras: Vec<{options, stream}>, mics: Vec<{options, stream}>, recording_mode }` replaces `Vec<DeviceRecordingAndStreamingConfig>` and `AvMixMode`.
- It is saved to `composite_config.json`. The old `selected_devices.json` is ignored, not deleted.
- The command names stay the same.

### Flows

- One `build_composite(config, dir)` is shared by `record_publish_to_syncflow_with_cancellation` and `local_recording_only`. It replaces `:133-222` and `:540-628`, along with their `panic!`s.
- The token grant gets `can_publish_data: true` (`:75-81`).
- The wait loop also exits on `Stopped`.

### `pipeline_events.rs` (event forwarder)

For each event and each start error, it does two things.

**UI**, in both modes:
- `publication-notification` gets new variants:
  - `deviceFailure { sessionId, device, roles, outputs, reason, recordingContinues }`
  - `outputFailure`
  - `deviceWarning`

**Room**, in session mode, for the same failures:
- `publish_data("ROOM_EVENTS", json, reliable=true)`, with this payload:
  ```json
  { "type": "LocalRecorderStreamFailure", "occurredAt": 1760459882000000000, "phase": "running",
    "participant": { "identity": "Host-1a2b3c4d(grp)", "name": "...", "sid": "PA_..." },
    "session": { "id": "...", "name": "Lecture 3" },
    "device": { "role": "mic-3", "kind": "mic", "displayName": "UMC1820", "deviceId": "...", "codec": "audio/x-raw", "selectedChannel": 3 },
    "outputsAffected": ["mic-3"], "tracksUnpublished": ["mic-3"], "streaming": true, "recordingContinues": true,
    "gst": { "element": "dev0:src", "domain": "resource", "code": 3, "message": "...", "debug": "..." } }
  ```
- Record-only sessions publish too, because they still join the room.
- Local mode has no room, so failures there go to the UI and the manifest only.
- The type moves into `syncflow_shared` later, in the SyncFlow repo.
- No credentials are ever included.

### UI (runes, `docs/design-rules.md`)

**`CompositeSetup.svelte`** replaces `Devices`, `DeviceSelector` and `SelectedDevices`:

```
┌ Recording setup ─────────────────────────────────────── [⟳ Refresh] ┐
│ AV mix  → av-mix.mp4 (L = Mic 1, R = Mic 2)          [Use ◉] [Stream ◉] │
│   Camera  [FaceTime HD ▾] [MJPEG 1280×720 @30 ▾]                       │
│   Mic 1 (L) [USB Mic A ▾] [48 kHz ▾]   Mic 2 (R) [USB Mic B ▾] [48 kHz ▾] │
│ Screen share                                         [Use ○] [Stream ○] │
│ Additional cameras                                       [+ Add camera] │
│   Camera 2  Logitech C920 · MJPEG 720p@30            [Stream ○]  [✕]   │
│ Additional microphones                                      [+ Add mic] │
│   Mic 3  UMC1820 · ch 3 · 48 kHz                     [Stream ○]  [✕]   │
│ ⓘ Streamed audio is mono with noise suppression; files keep raw audio. │
└──────────────────────────────────────────────────────────────────────┘
```

- The format options come from `DeviceSelector.svelte:15-106`, extracted into `lib/publishOptions.ts`.
- `store.svelte.ts` holds one composite `$state`.
- `types.ts` gains the new types.
- Small fix: `+page.ts:18-19` loads devices even when the app isn't registered.

**Stream page:**
- `CompositeSummary.svelte` shows each output with a `SessionStatus`, including a new `failed` state.
- `deviceFailure` renders as a red Alert, for example "Mic 3 (UMC1820 ch 3) disconnected. Other devices are still recording."
- The `listen()` handles are cleaned up on destroy.

---

## 11. Delivery

**Status (2026-10-14):** steps 0–2 are implemented in `livekit-gstreamer/src/composite/` and `LKParticipant::publish_composite`/`publish_data`, verified on a MacBook with its camera and two USB mics. Step 3 is split in three: **backend** (done: both flows run on `CompositeStream`, readable folders `<YYYY-MM-DD_HH-MM>_<session>_<short-id>` with `-2`… on collision, manifest context, `can_publish_data`, failures forwarded to the UI and `ROOM_EVENTS`, upload when any file finalized; the room is joined before recording starts, and a failed join stops the session as before), **UI** (next; the backend still reads the per-device configs and converts them with `CompositeConfig::from_device_configs`), then **cleanup** (delete the old API).


Each PR builds on its own. Per CLAUDE.md, public API changes update the publisher in the same PR.

0. **Scaffolding:** this document, plus the `composite` module types and signatures (`todo!` bodies). No behaviour change.
1. **PR 1, library core (additive):**
   - `ids`, `tags`, `spec`/`validate`, `naming`, `manifest` and the builders (§4).
   - `CompositeStream` start and stop.
   - The bus task and isolation (§6).
   - Examples: `stream_composite.rs`, `stream_two_cameras.rs`.
   - Tests.
2. **PR 2, publishing:**
   - `publish_composite`, `publish_data` and the task fixes.
   - Examples: `lk_publish_composite.rs`, and `lk_listen_data.rs`, which prints data payloads for a topic given as an argument.
3. **PR 3, switch over:**
   - Publisher config, flows, naming, manifest, events and `ROOM_EVENTS`.
   - UI.
   - Delete `GstMediaStream`, `AvMixStream`, `handle_streams`, `MediaStream`, `StreamingHandlingConfig`, the standalone `*_pipeline` builders and the shared-clock fields.
   - Port the examples.
   - Update the CLAUDE.md package structure.

## Verification

- **Static checks:** run `cargo fmt --all`, then `cargo clippy --workspace --all-targets -- -D warnings`, then `cargo build --workspace`.
- **Library tests:** `cargo test -p livekit-gstreamer --features test-sources`. The `test-sources` feature maps the device ids `test:video`, `test:audio:<hz>:<channels>` and `test:screen` to live `videotestsrc`/`audiotestsrc`.
  - **Pure:** `validate()` cases, `ResolvedSpec` edges for C1–C9, `ElementTag` round-trips, slugs.
  - **C2:** `av-mix.mp4` has 2 AAC channels, with L at 440 Hz and R at 880 Hz (`Discoverer` plus decode and FFT/zero-crossing). The stream appsink caps are mono 48k and contain `audiornnoise`.
  - **C6:** one 4-channel test source feeds 3 roles, and each `.m4a` carries the expected tone.
  - **F1, F2, F4, F6, F8:** `post_error_message` on the tagged element yields the expected event. The listed files finalize and are playable, and the remaining outputs keep growing.
  - **L4:** all files finalize, every output shares `base_time`, and `manifest.json` reaches `completed`.
- **Hardware:** run `stream_composite` with 2 cameras, a UMC1820 (3 roles), a USB mic and a screen for 5 minutes.
  - `ffprobe` each file.
  - Unplug the USB mic mid-run, and confirm the others continue.
  - `kill -9` once, and confirm the files are readable.
- **LiveKit:**
  1. Run `lk_publish_composite`, and confirm the dashboard shows the expected tracks with mono audio.
  2. Run `lk_listen_data -- ROOM_EVENTS` while a **record-only** app session runs, then unplug a mic.
  3. Expect a `LocalRecorderStreamFailure` with the participant identity and the device, and a red Alert in the UI.
- **App:** in `syncflow-publisher`, run `pnpm check`, `pnpm check:design` and `pnpm tauri dev`.
  - Exercise C1–C7 in local and session mode.
  - Confirm the folder and file names, `manifest.json`, and that the S3 key is unchanged.

## Open items (defaults chosen; flag if you disagree)

- **Start policy:** defaults to `BestEffort`, meaning start without a missing device and report it. `AllOrNothing` is available.
- **H.264 encoder:** `Auto` prefers hardware (`vtenc_h264_hw` / `mfh264enc`), falling back to `x264enc`. Hardware encoder availability in the bundled framework is checked in PR 1.
- **Mono stream level:** with 2 mix mics, the stream downmix averages L and R. When one mic fails, the stream drops about 6 dB. The alternative is a summing downmix, which risks clipping.
- **`audiornnoise` voice-activity gating:** off by default (threshold 0.0).
