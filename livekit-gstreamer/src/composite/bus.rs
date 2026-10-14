//! The bus task: attributes errors to a device, file or appsink and isolates it while
//! the rest of the pipeline keeps recording (composite-stream-plan.md §6).

use super::builder::BuiltGraph;
use super::events::{CompositeError, CompositeEvent, ErrorInfo, FailurePhase, StopReason};
use super::ids::{DeviceKey, OutputId, SinkId};
use super::manifest::{OutputStatus, RecordingManifest};
use super::naming::metadata_file_name;
use super::spec::ResolvedSpec;
use super::tags::{ElementTag, Scope};
use gstreamer::prelude::*;
use gstreamer::{Pad, PadProbeReturn, PadProbeType, Pipeline};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// How long isolating one failure may take before the whole pipeline is stopped.
pub const ISOLATION_TIMEOUT: Duration = Duration::from_secs(5);
/// Name of the bus message a file sink posts when EOS reaches it.
pub(crate) const OUTPUT_EOS_MESSAGE: &str = "composite-output-eos";
const POLL_INTERVAL: u64 = 100;
/// How long to wait for a branch's tee pad to go idle before removing it regardless.
const UNLINK_TIMEOUT: Duration = Duration::from_secs(2);

/// The smallest part of the graph a failure is contained to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureUnit {
    Device(DeviceKey),
    Output(OutputId),
    Sink(SinkId),
    /// The L/R mix: ends the AV mix audio, both in the file and in the stream.
    Mix,
    /// Untagged or pipeline-level error: stop everything.
    Fatal,
}

/// First and last buffer times reaching one file's muxer, in nanoseconds of pipeline
/// running time (shared by every file of the recording).
#[derive(Debug)]
pub(crate) struct OutputTiming {
    pub first: AtomicU64,
    pub last: AtomicU64,
}

impl Default for OutputTiming {
    fn default() -> Self {
        Self {
            first: AtomicU64::new(u64::MAX),
            last: AtomicU64::new(0),
        }
    }
}

impl OutputTiming {
    /// Records a buffer reaching the muxer on `pad`, in pipeline running time. Raw
    /// timestamps are not comparable: H.264 encoders shift theirs by 1000 hours.
    pub fn record(&self, pad: &gstreamer::Pad, buffer: &gstreamer::BufferRef) {
        let Some(pts) = buffer.pts() else {
            return;
        };
        let running = pad
            .sticky_event::<gstreamer::event::Segment>(0)
            .and_then(|event| {
                event
                    .segment()
                    .downcast_ref::<gstreamer::ClockTime>()
                    .and_then(|segment| segment.to_running_time(pts))
            });
        if let Some(start) = running {
            let end = start + buffer.duration().unwrap_or_default();
            self.first.fetch_min(start.nseconds(), Ordering::Relaxed);
            self.last.fetch_max(end.nseconds(), Ordering::Relaxed);
        }
    }

    fn range(&self) -> (Option<u64>, Option<u64>) {
        let first = self.first.load(Ordering::Relaxed);
        let last = self.last.load(Ordering::Relaxed);
        (
            (first != u64::MAX).then_some(first),
            (last != 0).then_some(last),
        )
    }
}

/// State shared by `CompositeStream` and its bus task.
pub(crate) struct Shared {
    pub pipeline: Pipeline,
    pub spec: Arc<ResolvedSpec>,
    pub graph: Mutex<BuiltGraph>,
    pub manifest: Mutex<RecordingManifest>,
    pub events: broadcast::Sender<CompositeEvent>,
    pub timings: HashMap<OutputId, Arc<OutputTiming>>,
    /// Devices that failed, at start or while running.
    pub dead_devices: Mutex<HashSet<DeviceKey>>,
    /// Set by `stop`: the next EOS is the requested one.
    pub stop_requested: AtomicBool,
    /// Set when `stop` gives up waiting: the task returns at once.
    pub abort: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    /// Records `event` in the manifest, saves the manifest and sends the event.
    pub fn emit(&self, event: CompositeEvent) {
        lock(&self.manifest).events.push(event.clone());
        self.save_manifest();
        let _ = self.events.send(event);
    }

    pub fn save_manifest(&self) {
        if let Some(dir) = &self.spec.output_dir {
            let manifest = lock(&self.manifest);
            if let Err(e) = manifest.write(dir) {
                let _ = self.events.send(CompositeEvent::Warning {
                    scope: None,
                    message: format!("failed to write manifest: {e}"),
                });
            }
        }
    }

    /// Writes the metadata file of `output` next to it.
    pub fn save_output_metadata(&self, output: OutputId) {
        let Some(dir) = &self.spec.output_dir else {
            return;
        };
        let manifest = lock(&self.manifest);
        let Some(record) = manifest.outputs.iter().find(|o| o.id == output) else {
            return;
        };
        let metadata = serde_json::json!({
            "output": record,
            "clock": manifest.clock,
            "baseTimeNs": manifest.base_time_ns,
            "wallclockAtBaseNs": manifest.wallclock_at_base_ns,
        });
        let path = dir.join(metadata_file_name(&record.file));
        if let Ok(json) = serde_json::to_vec_pretty(&metadata) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Copies the timestamps seen by `output`'s muxer into the manifest.
    pub fn record_timing(&self, output: OutputId) {
        if let Some(timing) = self.timings.get(&output) {
            let (first, last) = timing.range();
            if let Some(record) = lock(&self.manifest).output_mut(output) {
                record.first_pts_ns = first;
                record.last_pts_ns = last;
            }
        }
    }

    /// Outputs and sinks are alive while some device feeding them is.
    fn alive(&self, roles: &[super::ids::Role]) -> bool {
        let dead = lock(&self.dead_devices);
        roles.iter().any(|role| {
            self.spec
                .device_of(*role)
                .is_some_and(|key| !dead.contains(&key))
        })
    }

    /// Whether anything is still being recorded (or streamed, when nothing is recorded).
    pub fn recording_continues(&self) -> bool {
        let graph = lock(&self.graph);
        let manifest = lock(&self.manifest);
        let live_files = self.spec.outputs.iter().any(|o| {
            graph.outputs.contains_key(&o.id)
                && manifest
                    .outputs
                    .iter()
                    .any(|r| r.id == o.id && r.status != OutputStatus::Failed)
                && self.alive(&o.roles)
        });
        if !self.spec.outputs.is_empty() {
            return live_files;
        }
        self.spec
            .sinks
            .iter()
            .any(|s| graph.sinks.contains_key(&s.id) && self.alive(&s.roles))
    }
}

/// Maps the tag of the element that posted an error to the unit to isolate.
pub fn attribute(spec: &ResolvedSpec, tag: Option<&ElementTag>) -> FailureUnit {
    match tag.map(|t| t.scope) {
        Some(Scope::Device(key)) => FailureUnit::Device(key),
        Some(Scope::Role(role)) => spec
            .device_of(role)
            .map_or(FailureUnit::Fatal, FailureUnit::Device),
        Some(Scope::Mix) => FailureUnit::Mix,
        Some(Scope::File(output)) => FailureUnit::Output(output),
        Some(Scope::Sink(sink)) => FailureUnit::Sink(sink),
        None => FailureUnit::Fatal,
    }
}

/// Runs the bus loop until the pipeline stops, isolating failures as they arrive.
pub(crate) fn spawn_bus_task(
    shared: Arc<Shared>,
) -> JoinHandle<Result<StopReason, CompositeError>> {
    tokio::task::spawn_blocking(move || BusLoop::new(shared).run())
}

struct BusLoop {
    shared: Arc<Shared>,
    playing: bool,
    /// Files ended by a device failure, with the time by which they must finalize.
    pending: Vec<(OutputId, Instant)>,
    /// A whole-pipeline stop in progress: why, and how long to wait for its EOS.
    stopping: Option<(StopReason, Instant)>,
}

impl BusLoop {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            playing: false,
            pending: Vec::new(),
            stopping: None,
        }
    }

    fn run(mut self) -> Result<StopReason, CompositeError> {
        let bus = self
            .shared
            .pipeline
            .bus()
            .ok_or_else(|| CompositeError::Timeout("pipeline has no bus".into()))?;
        loop {
            if self.shared.abort.load(Ordering::SeqCst) {
                return Ok(StopReason::Requested);
            }
            let now = Instant::now();
            if let Some((reason, deadline)) = &self.stopping {
                if now > *deadline {
                    return Ok(self.unrequested(reason.clone()));
                }
            } else if self.pending.iter().any(|(_, deadline)| now > *deadline) {
                self.stop_all(StopReason::IsolationTimeout);
            }

            let Some(message) = bus.timed_pop(gstreamer::ClockTime::from_mseconds(POLL_INTERVAL))
            else {
                continue;
            };
            match message.view() {
                gstreamer::MessageView::Eos(_) => {
                    let reason = match self.stopping.take() {
                        Some((reason, _)) => reason,
                        None if self.shared.stop_requested.load(Ordering::SeqCst) => {
                            StopReason::Requested
                        }
                        None => StopReason::AllOutputsDone,
                    };
                    return Ok(self.unrequested(reason));
                }
                gstreamer::MessageView::Error(error) => {
                    if self.stopping.is_none() {
                        self.on_error(error);
                    }
                }
                gstreamer::MessageView::Warning(warning) => {
                    let scope = warning
                        .src()
                        .and_then(ElementTag::find)
                        .map(|tag| tag.scope.to_string());
                    self.shared.emit(CompositeEvent::Warning {
                        scope,
                        message: warning.error().message().to_string(),
                    });
                }
                gstreamer::MessageView::StateChanged(change) => {
                    let from_pipeline = change.src().is_some_and(|src| {
                        src == self.shared.pipeline.upcast_ref::<gstreamer::Object>()
                    });
                    if from_pipeline && change.current() == gstreamer::State::Playing {
                        self.playing = true;
                    }
                }
                gstreamer::MessageView::Application(app) => {
                    let output = app
                        .structure()
                        .filter(|s| s.name() == OUTPUT_EOS_MESSAGE)
                        .and_then(|s| s.get::<&str>("output").ok())
                        .and_then(|id| id.parse::<OutputId>().ok());
                    if let Some(output) = output {
                        self.on_output_eos(output);
                    }
                }
                _ => {}
            }
        }
    }

    /// Emits `Stopped` for stops nobody asked for; `stop` reports requested ones.
    fn unrequested(&self, reason: StopReason) -> StopReason {
        if reason != StopReason::Requested {
            self.shared.emit(CompositeEvent::Stopped {
                reason: reason.clone(),
            });
        }
        reason
    }

    fn stop_all(&mut self, reason: StopReason) {
        self.shared
            .pipeline
            .send_event(gstreamer::event::Eos::new());
        self.stopping = Some((reason, Instant::now() + ISOLATION_TIMEOUT));
    }

    fn on_error(&mut self, error: &gstreamer::message::Error) {
        let info = ErrorInfo::from_error_message(error);
        let tag = error.src().and_then(ElementTag::find);
        let phase = if self.playing {
            FailurePhase::Running
        } else {
            FailurePhase::Start
        };
        match attribute(&self.shared.spec, tag.as_ref()) {
            FailureUnit::Device(key) => {
                let ended = isolate_device(&self.shared, key, info, phase);
                let deadline = Instant::now() + ISOLATION_TIMEOUT;
                self.pending
                    .extend(ended.into_iter().map(|o| (o, deadline)));
            }
            FailureUnit::Output(output) => isolate_output(&self.shared, output, info),
            FailureUnit::Sink(sink) => isolate_sink(&self.shared, sink, info),
            FailureUnit::Mix => {
                isolate_output(&self.shared, OutputId::AvMix, info.clone());
                isolate_sink(&self.shared, SinkId::AvMixAudio, info);
            }
            FailureUnit::Fatal => self.stop_all(StopReason::Fatal(info)),
        }
    }

    fn on_output_eos(&mut self, output: OutputId) {
        self.pending.retain(|(id, _)| *id != output);
        let path = {
            let graph = lock(&self.shared.graph);
            graph.outputs.get(&output).map(|b| b.path.clone())
        };
        self.shared.record_timing(output);
        {
            let mut manifest = lock(&self.shared.manifest);
            match manifest.output_mut(output) {
                Some(record) if record.status == OutputStatus::Recording => {
                    record.status = OutputStatus::Finalized;
                }
                _ => return,
            }
        }
        self.shared.save_output_metadata(output);
        if let Some(path) = path {
            self.shared
                .emit(CompositeEvent::OutputFinalized { output, path });
        }
    }
}

/// Blocks the device source, pushes EOS down every branch it feeds, then removes the
/// device chain, releasing the hardware. Files fed only by this device then finalize;
/// mixers and muxers with other inputs keep going. Returns the files that will finalize.
pub(crate) fn isolate_device(
    shared: &Shared,
    key: DeviceKey,
    error: ErrorInfo,
    phase: FailurePhase,
) -> Vec<OutputId> {
    let Some(chain) = lock(&shared.graph).devices.remove(&key) else {
        return Vec::new(); // Already isolated: a follow-up error from the same device.
    };
    lock(&shared.dead_devices).insert(key);

    if let Some(src) = chain.source.static_pad("src") {
        src.add_probe(PadProbeType::BLOCK_DOWNSTREAM, |_, _| PadProbeReturn::Ok);
    }
    if let Some(next) = chain.elements.get(1).and_then(|e| e.static_pad("sink")) {
        next.send_event(gstreamer::event::Eos::new());
    }
    for element in &chain.elements {
        let _ = element.set_state(gstreamer::State::Null);
        let _ = shared.pipeline.remove(element);
    }

    let spec = &shared.spec;
    let ended: Vec<OutputId> = spec
        .outputs
        .iter()
        .filter(|o| lock(&shared.graph).outputs.contains_key(&o.id) && !shared.alive(&o.roles))
        .map(|o| o.id)
        .collect();
    let sinks_closed: Vec<SinkId> = spec
        .sinks
        .iter()
        .filter(|s| lock(&shared.graph).sinks.contains_key(&s.id) && !shared.alive(&s.roles))
        .map(|s| s.id)
        .collect();
    {
        let mut manifest = lock(&shared.manifest);
        for output in &ended {
            if let Some(record) = manifest.output_mut(*output) {
                record.error = Some(error.clone());
            }
        }
    }
    let Some(device) = spec.device(key).map(|d| d.device.clone()) else {
        return ended;
    };
    shared.emit(CompositeEvent::DeviceFailed {
        device,
        roles: spec.roles_of(key),
        outputs_affected: spec.outputs_fed_by(key),
        sinks_closed,
        phase,
        error,
        recording_continues: shared.recording_continues(),
    });
    ended
}

/// Unlinks the tee pad feeding a branch once it is idle, pushes EOS into the branch,
/// then releases the pad. A pad that never goes idle (its branch is stuck) is released
/// anyway once the branch is removed.
fn unlink_branch(tee_pad: &Pad) {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    tee_pad.add_probe(PadProbeType::IDLE, move |pad, _| {
        if let Some(peer) = pad.peer() {
            let _ = pad.unlink(&peer);
            peer.send_event(gstreamer::event::Eos::new());
        }
        let _ = done_tx.send(());
        PadProbeReturn::Remove
    });
    let _ = done_rx.recv_timeout(UNLINK_TIMEOUT);
    if let Some(tee) = tee_pad.parent_element() {
        tee.release_request_pad(tee_pad);
    }
}

fn remove_elements(shared: &Shared, elements: &[gstreamer::Element]) {
    for element in elements {
        let _ = element.set_state(gstreamer::State::Null);
        let _ = shared.pipeline.remove(element);
    }
}

/// Unlinks the file branch of `output` from its tees and removes it. The device keeps
/// feeding its other branches.
pub(crate) fn isolate_output(shared: &Shared, output: OutputId, error: ErrorInfo) {
    let Some(branch) = lock(&shared.graph).outputs.remove(&output) else {
        return;
    };
    for pad in &branch.tee_pads {
        unlink_branch(pad);
    }
    remove_elements(shared, &branch.elements);
    shared.record_timing(output);
    if let Some(record) = lock(&shared.manifest).output_mut(output) {
        record.status = OutputStatus::Failed;
        record.error = Some(error.clone());
    }
    shared.save_output_metadata(output);
    shared.emit(CompositeEvent::OutputFailed {
        output,
        error,
        recording_continues: shared.recording_continues(),
    });
}

/// Unlinks the stream branch of `sink` from its tee and removes it, closing its track.
/// Files are untouched.
pub(crate) fn isolate_sink(shared: &Shared, sink: SinkId, error: ErrorInfo) {
    let Some(branch) = lock(&shared.graph).sinks.remove(&sink) else {
        return;
    };
    unlink_branch(&branch.tee_pad);
    remove_elements(shared, &branch.elements);
    let _ = branch.closed.send(());
    shared.emit(CompositeEvent::SinkFailed { sink, error });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composite::ids::Role;
    use crate::composite::spec::{AvMixSpec, CompositeStreamSpec, Output};
    use crate::{AudioPublishOptions, VideoPublishOptions};

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

    #[test]
    fn errors_are_attributed_to_the_smallest_unit() {
        let spec = CompositeStreamSpec {
            av_mix: Some(AvMixSpec {
                camera: VideoPublishOptions {
                    codec: "video/x-raw".into(),
                    device_id: "cam".into(),
                    width: 320,
                    height: 240,
                    framerate: 30,
                    local_file_save_options: None,
                },
                mics: vec![mic("a"), mic("b")],
                stream: true,
            }),
            mics: vec![Output {
                options: mic("c"),
                stream: true,
            }],
            output_dir: Some("/tmp".into()),
            ..Default::default()
        }
        .validate_with_names(&HashMap::new())
        .unwrap();
        let unit = |name: &str| attribute(&spec, ElementTag::parse(name).as_ref());

        assert_eq!(unit("dev2:src"), FailureUnit::Device(DeviceKey(2)));
        assert_eq!(
            unit("mic-3:pick"),
            FailureUnit::Device(spec.device_of(Role::Mic(3)).unwrap())
        );
        assert_eq!(unit("mix.lr:mixer"), FailureUnit::Mix);
        assert_eq!(
            unit("file.mic-1:aenc"),
            FailureUnit::Output(OutputId::Mic(1))
        );
        assert_eq!(
            unit("sink.av-mix-video:q"),
            FailureUnit::Sink(SinkId::AvMixVideo)
        );
        assert_eq!(unit("pipeline0"), FailureUnit::Fatal);
        assert_eq!(unit("mic-9:pick"), FailureUnit::Fatal);
    }
}
