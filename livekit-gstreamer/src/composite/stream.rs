//! `CompositeStream`: every configured device in one pipeline, with one clock and one
//! base time (composite-stream-plan.md §5).

use super::builder::build_graph;
use super::bus::{spawn_bus_task, OutputTiming, Shared, OUTPUT_EOS_MESSAGE};
use super::events::{CompositeError, CompositeEvent, ErrorInfo, FailurePhase, StopReason};
use super::ids::{DeviceRef, OutputId, Role, SinkId};
use super::manifest::{AudioContinuity, OutputStatus, RecordingManifest};
use super::spec::{CompositeStreamSpec, ResolvedSpec, SinkInfo};
use super::tags::{element_name, Scope};
use crate::utils::system_time_nanos;
use gstreamer::prelude::*;
use gstreamer::{Buffer, PadProbeReturn, PadProbeType, Pipeline};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

const EVENT_CHANNEL_CAPACITY: usize = 64;
/// How long `start` waits for the pipeline to reach PLAYING.
const START_TIMEOUT: u64 = 10;
/// How long `stop` waits for every file to finalize.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// How long dropping a running stream waits for files to finalize.
const DROP_TIMEOUT: Duration = Duration::from_secs(5);

/// Devices that failed to open under `StartPolicy::BestEffort`.
#[derive(Debug, Clone, Default)]
pub struct StartReport {
    pub failed_at_start: Vec<(DeviceRef, ErrorInfo)>,
}

/// Frames from one appsink, plus a signal that fires when its branch closes.
pub struct SinkSubscription {
    pub frames: broadcast::Receiver<Arc<Buffer>>,
    pub closed: broadcast::Receiver<()>,
}

struct Running {
    shared: Arc<Shared>,
    bus_task: JoinHandle<Result<StopReason, CompositeError>>,
}

/// One pipeline driving every configured device.
///
/// Subscribe to [`events`](Self::events) before calling [`start`](Self::start) to see
/// devices that fail to open. Call [`stop`](Self::stop) to finalize the files; dropping a
/// running stream also tries to, for up to 5 seconds.
pub struct CompositeStream {
    spec: CompositeStreamSpec,
    events: broadcast::Sender<CompositeEvent>,
    running: Option<Running>,
}

impl CompositeStream {
    pub fn new(spec: CompositeStreamSpec) -> Self {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            spec,
            events,
            running: None,
        }
    }

    pub fn spec(&self) -> &CompositeStreamSpec {
        &self.spec
    }

    /// Validates the spec (reading device names from the system), builds the pipeline on
    /// the system clock, plays it, and starts handling errors.
    pub async fn start(&mut self) -> Result<StartReport, CompositeError> {
        let spec = self.spec.validate()?;
        self.start_resolved(spec).await
    }

    /// Like [`start`](Self::start), with device display names given by device id
    /// instead of read from the system.
    pub async fn start_with_names(
        &mut self,
        names: &HashMap<String, String>,
    ) -> Result<StartReport, CompositeError> {
        let spec = self.spec.validate_with_names(names)?;
        self.start_resolved(spec).await
    }

    async fn start_resolved(&mut self, spec: ResolvedSpec) -> Result<StartReport, CompositeError> {
        if self.running.is_some() {
            return Err(CompositeError::AlreadyStarted);
        }
        if let Some(dir) = &spec.output_dir {
            std::fs::create_dir_all(dir)?;
        }
        let spec = Arc::new(spec);
        let pipeline = Pipeline::new();
        let (graph, failed) = build_graph(&pipeline, &spec)?;

        let timings: HashMap<OutputId, Arc<OutputTiming>> = graph
            .outputs
            .keys()
            .map(|id| (*id, Arc::new(OutputTiming::default())))
            .collect();
        for (id, branch) in &graph.outputs {
            watch_output(*id, &branch.mux, &branch.filesink, timings[id].clone());
        }
        let built_outputs: HashSet<OutputId> = graph.outputs.keys().copied().collect();
        let shared = Arc::new(Shared {
            pipeline: pipeline.clone(),
            spec: spec.clone(),
            graph: Mutex::new(graph),
            manifest: Mutex::new(RecordingManifest::new(&spec)),
            events: self.events.clone(),
            timings,
            dead_devices: Mutex::new(failed.iter().map(|(key, _)| *key).collect()),
            stop_requested: AtomicBool::new(false),
            abort: AtomicBool::new(false),
        });

        let mut report = StartReport::default();
        for (key, error) in &failed {
            let Some(device) = spec.device(*key).map(|d| d.device.clone()) else {
                continue;
            };
            let info =
                ErrorInfo::from_composite_error(element_name(Scope::Device(*key), "src"), error);
            report.failed_at_start.push((device.clone(), info.clone()));
            {
                let mut manifest = shared.manifest.lock().unwrap_or_else(|e| e.into_inner());
                for output in spec.outputs_fed_by(*key) {
                    if built_outputs.contains(&output) {
                        continue;
                    }
                    if let Some(record) = manifest.output_mut(output) {
                        record.status = OutputStatus::Failed;
                        record.error = Some(info.clone());
                    }
                }
            }
            let sinks_closed = spec
                .sinks_fed_by(*key)
                .into_iter()
                .filter(|sink| {
                    !shared
                        .graph
                        .lock()
                        .is_ok_and(|g| g.sinks.contains_key(sink))
                })
                .collect();
            shared.emit(CompositeEvent::DeviceFailed {
                device,
                roles: spec.roles_of(*key),
                outputs_affected: spec.outputs_fed_by(*key),
                sinks_closed,
                phase: FailurePhase::Start,
                error: info,
                recording_continues: true,
            });
        }

        pipeline.use_clock(Some(&gstreamer::SystemClock::obtain()));
        let bus_task = spawn_bus_task(shared.clone());
        let running = Running { shared, bus_task };
        if let Err(e) = pipeline.set_state(gstreamer::State::Playing) {
            shutdown(&running);
            return Err(CompositeError::GStreamer(
                crate::media_device::GStreamerError::PipelineError(format!(
                    "failed to start the pipeline: {e}"
                )),
            ));
        }
        let waiting = pipeline.clone();
        let (_, state, _) = tokio::task::spawn_blocking(move || {
            waiting.state(gstreamer::ClockTime::from_seconds(START_TIMEOUT))
        })
        .await
        .map_err(|e| CompositeError::Timeout(format!("waiting for PLAYING: {e}")))?;
        if state != gstreamer::State::Playing {
            shutdown(&running);
            return Err(CompositeError::Timeout(format!(
                "the pipeline did not reach PLAYING within {START_TIMEOUT} s"
            )));
        }

        {
            let mut manifest = running
                .shared
                .manifest
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let (Some(base), Some(now)) =
                (pipeline.base_time(), pipeline.clock().map(|c| c.time()))
            {
                let since_base = now.saturating_sub(base).nseconds();
                manifest.base_time_ns = Some(base.nseconds());
                manifest.wallclock_at_base_ns =
                    Some(system_time_nanos() - i64::try_from(since_base).unwrap_or(0));
            }
        }
        running.shared.save_manifest();
        self.running = Some(running);
        Ok(report)
    }

    /// Sends EOS, waits for every file to finalize (10 s budget), stops the pipeline and
    /// returns the final manifest, also written to `manifest.json`.
    pub async fn stop(&mut self) -> Result<RecordingManifest, CompositeError> {
        let mut running = self.running.take().ok_or(CompositeError::NotStarted)?;
        let shared = running.shared.clone();
        shared.stop_requested.store(true, Ordering::SeqCst);
        shared.pipeline.send_event(gstreamer::event::Eos::new());
        let reason = match tokio::time::timeout(STOP_TIMEOUT, &mut running.bus_task).await {
            Ok(Ok(Ok(reason))) => reason,
            Ok(_) => StopReason::Requested,
            Err(_) => {
                shared.abort.store(true, Ordering::SeqCst);
                let _ = (&mut running.bus_task).await;
                StopReason::Requested
            }
        };
        let _ = shared.pipeline.set_state(gstreamer::State::Null);

        for output in shared.spec.outputs.iter().map(|o| o.id) {
            shared.record_timing(output);
        }
        {
            let mut manifest = shared.manifest.lock().unwrap_or_else(|e| e.into_inner());
            manifest.audio_continuity = audio_continuity(&shared);
            manifest.finish();
        }
        if reason == StopReason::Requested {
            shared.emit(CompositeEvent::Stopped { reason });
        } else {
            shared.save_manifest();
        }
        for output in shared.spec.outputs.iter().map(|o| o.id) {
            shared.save_output_metadata(output);
        }
        let manifest = shared
            .manifest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Ok(manifest)
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Appsinks still streaming.
    pub fn sinks(&self) -> Vec<SinkInfo> {
        self.with_graph(|graph| graph.sinks.values().map(|b| b.info.clone()).collect())
            .unwrap_or_default()
    }

    pub fn subscribe(&self, id: &SinkId) -> Option<SinkSubscription> {
        self.with_graph(|graph| {
            graph.sinks.get(id).map(|branch| SinkSubscription {
                frames: branch.frames.subscribe(),
                closed: branch.closed.subscribe(),
            })
        })
        .flatten()
    }

    pub fn events(&self) -> broadcast::Receiver<CompositeEvent> {
        self.events.subscribe()
    }

    pub fn devices(&self) -> Vec<DeviceRef> {
        self.running
            .as_ref()
            .map(|r| {
                r.shared
                    .spec
                    .devices
                    .iter()
                    .map(|d| d.device.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Files still being written.
    pub fn live_outputs(&self) -> Vec<OutputId> {
        self.running
            .as_ref()
            .map(|r| {
                r.shared
                    .manifest
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .live_outputs()
            })
            .unwrap_or_default()
    }

    /// A snapshot of the manifest as it stands.
    pub fn manifest(&self) -> Option<RecordingManifest> {
        self.running.as_ref().map(|r| {
            r.shared
                .manifest
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        })
    }

    fn with_graph<T>(&self, f: impl FnOnce(&super::builder::BuiltGraph) -> T) -> Option<T> {
        let running = self.running.as_ref()?;
        let graph = running
            .shared
            .graph
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Some(f(&graph))
    }
}

impl Drop for CompositeStream {
    /// Sends EOS and waits up to 5 seconds for files to finalize. Prefer `stop`, which
    /// also writes the final manifest.
    fn drop(&mut self) {
        let Some(running) = self.running.take() else {
            return;
        };
        running.shared.stop_requested.store(true, Ordering::SeqCst);
        running
            .shared
            .pipeline
            .send_event(gstreamer::event::Eos::new());
        let deadline = Instant::now() + DROP_TIMEOUT;
        while !running.bus_task.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        shutdown(&running);
    }
}

fn shutdown(running: &Running) {
    running.shared.abort.store(true, Ordering::SeqCst);
    let _ = running.shared.pipeline.set_state(gstreamer::State::Null);
}

/// Posts a bus message when EOS reaches `filesink` (the file is complete), and records
/// the first and last timestamps reaching the muxer.
fn watch_output(
    output: OutputId,
    mux: &gstreamer::Element,
    filesink: &gstreamer::Element,
    timing: Arc<OutputTiming>,
) {
    if let Some(pad) = filesink.static_pad("sink") {
        pad.add_probe(PadProbeType::EVENT_DOWNSTREAM, move |pad, info| {
            let is_eos = info
                .event()
                .is_some_and(|e| e.type_() == gstreamer::EventType::Eos);
            if let (true, Some(sink)) = (is_eos, pad.parent_element()) {
                let structure = gstreamer::Structure::builder(OUTPUT_EOS_MESSAGE)
                    .field("output", output.to_string())
                    .build();
                let _ = sink.post_message(
                    gstreamer::message::Application::builder(structure)
                        .src(&sink)
                        .build(),
                );
            }
            PadProbeReturn::Ok
        });
    }
    for pad in mux.sink_pads() {
        let timing = timing.clone();
        pad.add_probe(PadProbeType::BUFFER, move |pad, info| {
            if let Some(buffer) = info.buffer() {
                timing.record(pad, buffer);
            }
            PadProbeReturn::Ok
        });
    }
}

/// Samples each mic role's `audiorate` added and dropped to stay continuous.
fn audio_continuity(shared: &Shared) -> Vec<AudioContinuity> {
    shared
        .spec
        .roles
        .iter()
        .filter(|r| matches!(r.role, Role::Mic(_)))
        .filter_map(|r| {
            let rate = shared
                .pipeline
                .by_name(&element_name(Scope::Role(r.role), "rate"))?;
            Some(AudioContinuity {
                role: r.role,
                samples_added: rate.property::<u64>("add"),
                samples_dropped: rate.property::<u64>("drop"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composite::manifest::ManifestStatus;
    use crate::composite::spec::{AvMixSpec, EncoderOptions, H264Encoder, Output};
    use crate::{AudioPublishOptions, ScreenPublishOptions, VideoPublishOptions};

    fn mic(id: &str) -> AudioPublishOptions {
        AudioPublishOptions {
            codec: "audio/x-raw".into(),
            device_id: id.into(),
            framerate: 48000,
            channels: 1,
            selected_channel: None,
            local_file_save_options: None,
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("composite-stream-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// AV mix (2 mics, streamed), a recorded screen and an extra streamed mic (mic-3).
    fn spec(dir: &std::path::Path, extra_mic: &str) -> CompositeStreamSpec {
        CompositeStreamSpec {
            av_mix: Some(AvMixSpec {
                camera: VideoPublishOptions {
                    codec: "video/x-raw".into(),
                    device_id: "test:video".into(),
                    width: 320,
                    height: 240,
                    framerate: 30,
                    local_file_save_options: None,
                },
                mics: vec![mic("test:audio:440"), mic("test:audio:880")],
                stream: true,
            }),
            screen: Some(Output {
                options: ScreenPublishOptions {
                    codec: "video/x-raw".into(),
                    screen_id_or_name: "test:screen".into(),
                    width: 640,
                    height: 360,
                    framerate: 15,
                    local_file_save_options: None,
                },
                stream: false,
            }),
            mics: vec![Output {
                options: mic(extra_mic),
                stream: true,
            }],
            output_dir: Some(dir.to_path_buf()),
            encoders: EncoderOptions {
                h264: H264Encoder::X264,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    async fn start(
        spec: CompositeStreamSpec,
    ) -> (CompositeStream, broadcast::Receiver<CompositeEvent>) {
        gstreamer::init().unwrap();
        let mut stream = CompositeStream::new(spec);
        let events = stream.events();
        stream.start_with_names(&HashMap::new()).await.unwrap();
        (stream, events)
    }

    /// Waits up to 10 s for an event matching `matches`.
    async fn wait_for(
        events: &mut broadcast::Receiver<CompositeEvent>,
        matches: impl Fn(&CompositeEvent) -> bool,
    ) -> CompositeEvent {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match events.recv().await {
                    Ok(event) if matches(&event) => return event,
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(e) => panic!("event channel closed: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for an event")
    }

    /// Posts a GStreamer error from the element `name`, or from the pipeline itself.
    fn post_error(stream: &CompositeStream, name: Option<&str>) {
        let pipeline = &stream.running.as_ref().unwrap().shared.pipeline;
        let src: gstreamer::Element = match name {
            Some(name) => pipeline
                .by_name(name)
                .unwrap_or_else(|| panic!("no {name}")),
            None => pipeline.clone().upcast(),
        };
        let message =
            gstreamer::message::Error::builder(gstreamer::ResourceError::Read, "unplugged")
                .src(&src)
                .build();
        src.post_message(message).unwrap();
    }

    fn device_src(stream: &CompositeStream, device_id: &str) -> String {
        let device = stream
            .devices()
            .into_iter()
            .find(|d| d.device_id == device_id)
            .unwrap();
        element_name(Scope::Device(device.key), "src")
    }

    fn output(manifest: &RecordingManifest, id: OutputId) -> &super::super::manifest::OutputRecord {
        manifest.outputs.iter().find(|o| o.id == id).unwrap()
    }

    fn audio_channels(path: &std::path::Path) -> Vec<u32> {
        let uri = gstreamer::glib::filename_to_uri(path, None).unwrap();
        gstreamer_pbutils::Discoverer::new(gstreamer::ClockTime::from_seconds(10))
            .unwrap()
            .discover_uri(&uri)
            .unwrap_or_else(|e| panic!("{} is not readable: {e}", path.display()))
            .audio_streams()
            .iter()
            .map(|a| a.channels())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn records_and_writes_the_manifest() {
        let dir = temp_dir("records");
        let (mut stream, mut events) = start(spec(&dir, "test:audio:660")).await;
        let mut outputs = stream.live_outputs();
        outputs.sort();
        assert_eq!(
            outputs,
            [
                OutputId::AvMix,
                OutputId::Mic(1),
                OutputId::Mic(2),
                OutputId::Mic(3),
                OutputId::Screen(1)
            ]
        );
        let mut frames = stream.subscribe(&SinkId::Mic(3)).unwrap().frames;
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(
            matches!(
                frames.try_recv(),
                Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_))
            ),
            "mic-3 delivered no frames"
        );

        let manifest = stream.stop().await.unwrap();
        assert_eq!(manifest.status, ManifestStatus::Completed);
        assert!(manifest.base_time_ns.is_some() && manifest.wallclock_at_base_ns.is_some());
        for record in &manifest.outputs {
            assert_eq!(record.status, OutputStatus::Finalized, "{}", record.id);
            let (first, last) = (record.first_pts_ns.unwrap(), record.last_pts_ns.unwrap());
            assert!(
                last > first + 1_000_000_000,
                "{} spans {first}..{last}",
                record.id
            );
            // Running time, not raw timestamps (H.264 encoders add 1000 hours to theirs).
            assert!(last < 60_000_000_000, "{} ends at {last} ns", record.id);
        }
        assert_eq!(manifest.audio_continuity.len(), 3);

        let on_disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(on_disk["status"], "completed");
        assert!(dir.join("av-mix.json").exists());
        assert_eq!(audio_channels(&dir.join("av-mix.mp4")), [2]);

        let mut finalized = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(event, CompositeEvent::OutputFinalized { .. }) {
                finalized += 1;
            }
        }
        assert_eq!(finalized, 5);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_device_is_isolated() {
        let dir = temp_dir("device");
        let (mut stream, mut events) = start(spec(&dir, "test:audio:660")).await;
        let mut closed = stream.subscribe(&SinkId::Mic(3)).unwrap().closed;
        tokio::time::sleep(Duration::from_secs(1)).await;

        post_error(&stream, Some(&device_src(&stream, "test:audio:660")));
        let failed = wait_for(&mut events, |e| {
            matches!(e, CompositeEvent::DeviceFailed { .. })
        })
        .await;
        let CompositeEvent::DeviceFailed {
            roles,
            outputs_affected,
            sinks_closed,
            phase,
            recording_continues,
            ..
        } = failed
        else {
            unreachable!()
        };
        assert_eq!(roles, [Role::Mic(3)]);
        assert_eq!(outputs_affected, [OutputId::Mic(3)]);
        assert_eq!(sinks_closed, [SinkId::Mic(3)]);
        assert_eq!(phase, FailurePhase::Running);
        assert!(recording_continues);
        wait_for(&mut events, |e| {
            matches!(
                e,
                CompositeEvent::OutputFinalized {
                    output: OutputId::Mic(3),
                    ..
                }
            )
        })
        .await;
        tokio::time::timeout(Duration::from_secs(5), closed.recv())
            .await
            .expect("mic-3 stream did not close")
            .unwrap();
        assert!(!stream.live_outputs().contains(&OutputId::Mic(3)));
        assert!(stream.live_outputs().contains(&OutputId::AvMix));

        tokio::time::sleep(Duration::from_secs(1)).await;
        let manifest = stream.stop().await.unwrap();
        assert_eq!(manifest.status, ManifestStatus::CompletedWithErrors);
        let mic3 = output(&manifest, OutputId::Mic(3));
        assert_eq!(mic3.status, OutputStatus::Finalized);
        assert!(mic3.error.is_some());
        let av_mix = output(&manifest, OutputId::AvMix);
        assert_eq!(av_mix.status, OutputStatus::Finalized);
        assert!(
            av_mix.last_pts_ns.unwrap() > mic3.last_pts_ns.unwrap(),
            "av-mix kept recording"
        );
        assert_eq!(audio_channels(&dir.join(&mic3.file)), [1]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_mix_mic_keeps_the_av_mix() {
        let dir = temp_dir("mix-mic");
        let (mut stream, mut events) = start(spec(&dir, "test:audio:660")).await;
        tokio::time::sleep(Duration::from_secs(1)).await;

        post_error(&stream, Some(&device_src(&stream, "test:audio:880")));
        let failed = wait_for(&mut events, |e| {
            matches!(e, CompositeEvent::DeviceFailed { .. })
        })
        .await;
        let CompositeEvent::DeviceFailed {
            outputs_affected,
            sinks_closed,
            ..
        } = failed
        else {
            unreachable!()
        };
        assert_eq!(outputs_affected, [OutputId::AvMix, OutputId::Mic(2)]);
        assert!(sinks_closed.is_empty(), "the AV mix audio still has mic-1");
        wait_for(&mut events, |e| {
            matches!(
                e,
                CompositeEvent::OutputFinalized {
                    output: OutputId::Mic(2),
                    ..
                }
            )
        })
        .await;
        assert!(stream.live_outputs().contains(&OutputId::AvMix));

        tokio::time::sleep(Duration::from_secs(1)).await;
        let manifest = stream.stop().await.unwrap();
        let av_mix = output(&manifest, OutputId::AvMix);
        assert_eq!(av_mix.status, OutputStatus::Finalized);
        assert_eq!(audio_channels(&dir.join(&av_mix.file)), [2]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_file_branch_leaves_its_stream() {
        let dir = temp_dir("output");
        let (mut stream, mut events) = start(spec(&dir, "test:audio:660")).await;
        tokio::time::sleep(Duration::from_secs(1)).await;

        post_error(&stream, Some("file.mic-3:aenc"));
        let failed = wait_for(&mut events, |e| {
            matches!(e, CompositeEvent::OutputFailed { .. })
        })
        .await;
        assert!(matches!(
            failed,
            CompositeEvent::OutputFailed {
                output: OutputId::Mic(3),
                recording_continues: true,
                ..
            }
        ));
        assert!(stream.sinks().iter().any(|s| s.id == SinkId::Mic(3)));
        assert!(stream
            .running
            .as_ref()
            .unwrap()
            .shared
            .pipeline
            .by_name("file.mic-3:mux")
            .is_none());

        let manifest = stream.stop().await.unwrap();
        assert_eq!(
            output(&manifest, OutputId::Mic(3)).status,
            OutputStatus::Failed
        );
        assert_eq!(
            output(&manifest, OutputId::AvMix).status,
            OutputStatus::Finalized
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_stream_branch_leaves_its_file() {
        let dir = temp_dir("sink");
        let (mut stream, mut events) = start(spec(&dir, "test:audio:660")).await;
        let mut closed = stream.subscribe(&SinkId::Mic(3)).unwrap().closed;
        tokio::time::sleep(Duration::from_secs(1)).await;

        post_error(&stream, Some("sink.mic-3:q"));
        wait_for(&mut events, |e| {
            matches!(
                e,
                CompositeEvent::SinkFailed {
                    sink: SinkId::Mic(3),
                    ..
                }
            )
        })
        .await;
        // Closing is signalled by the isolation and again by the branch's EOS.
        assert!(matches!(
            closed.recv().await,
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_))
        ));
        assert!(!stream.sinks().iter().any(|s| s.id == SinkId::Mic(3)));
        assert!(stream.live_outputs().contains(&OutputId::Mic(3)));

        let manifest = stream.stop().await.unwrap();
        assert_eq!(
            output(&manifest, OutputId::Mic(3)).status,
            OutputStatus::Finalized
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_untagged_error_stops_everything() {
        let dir = temp_dir("fatal");
        let (mut stream, mut events) = start(spec(&dir, "test:audio:660")).await;
        tokio::time::sleep(Duration::from_secs(1)).await;

        post_error(&stream, None);
        wait_for(&mut events, |e| {
            matches!(
                e,
                CompositeEvent::Stopped {
                    reason: StopReason::Fatal(_)
                }
            )
        })
        .await;
        let manifest = stream.stop().await.unwrap();
        assert_eq!(manifest.status, ManifestStatus::CompletedWithErrors);
        assert_eq!(
            output(&manifest, OutputId::AvMix).status,
            OutputStatus::Finalized
        );
        assert_eq!(audio_channels(&dir.join("av-mix.mp4")), [2]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn losing_the_only_device_stops_the_pipeline() {
        let dir = temp_dir("only-device");
        let spec = CompositeStreamSpec {
            mics: vec![Output {
                options: mic("test:audio:660"),
                stream: false,
            }],
            output_dir: Some(dir.clone()),
            ..Default::default()
        };
        let (mut stream, mut events) = start(spec).await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        post_error(&stream, Some(&device_src(&stream, "test:audio:660")));
        let failed = wait_for(&mut events, |e| {
            matches!(e, CompositeEvent::DeviceFailed { .. })
        })
        .await;
        assert!(matches!(
            failed,
            CompositeEvent::DeviceFailed {
                recording_continues: false,
                ..
            }
        ));
        wait_for(&mut events, |e| {
            matches!(
                e,
                CompositeEvent::Stopped {
                    reason: StopReason::AllOutputsDone
                }
            )
        })
        .await;
        let manifest = stream.stop().await.unwrap();
        assert_eq!(
            output(&manifest, OutputId::Mic(1)).status,
            OutputStatus::Finalized
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `osxaudiosrc` on the system clock stamps every buffer 0; without re-stamping,
    /// `audiorate` drops all but the first ~50 ms and the mic file is nearly empty.
    #[tokio::test(flavor = "multi_thread")]
    async fn mics_with_device_clock_timestamps_record_fully() {
        let dir = temp_dir("zero-ts");
        let spec = CompositeStreamSpec {
            mics: vec![Output {
                options: mic("test:zero-ts:440"),
                stream: false,
            }],
            output_dir: Some(dir.clone()),
            ..Default::default()
        };
        let (mut stream, _) = start(spec).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let manifest = stream.stop().await.unwrap();

        let mic = output(&manifest, OutputId::Mic(1));
        let span = mic.last_pts_ns.unwrap() - mic.first_pts_ns.unwrap();
        assert!(span > 1_500_000_000, "mic recorded only {span} ns");
        let continuity = &manifest.audio_continuity[0];
        assert!(
            continuity.samples_dropped < 48_000 / 10,
            "audiorate dropped {} samples",
            continuity.samples_dropped
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_device_missing_at_start_is_reported() {
        let dir = temp_dir("missing");
        gstreamer::init().unwrap();
        let mut stream = CompositeStream::new(spec(&dir, "test:missing"));
        let mut events = stream.events();
        let report = stream.start_with_names(&HashMap::new()).await.unwrap();
        assert_eq!(report.failed_at_start.len(), 1);
        let failed = wait_for(&mut events, |e| {
            matches!(e, CompositeEvent::DeviceFailed { .. })
        })
        .await;
        assert!(matches!(
            failed,
            CompositeEvent::DeviceFailed {
                phase: FailurePhase::Start,
                recording_continues: true,
                ..
            }
        ));
        assert!(!stream.sinks().iter().any(|s| s.id == SinkId::Mic(3)));
        tokio::time::sleep(Duration::from_secs(1)).await;

        let manifest = stream.stop().await.unwrap();
        assert_eq!(
            output(&manifest, OutputId::Mic(3)).status,
            OutputStatus::Failed
        );
        assert_eq!(
            output(&manifest, OutputId::AvMix).status,
            OutputStatus::Finalized
        );
        assert_eq!(manifest.status, ManifestStatus::CompletedWithErrors);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
