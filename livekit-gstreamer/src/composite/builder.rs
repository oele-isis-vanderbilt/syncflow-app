//! Graph construction, one function per layer: device chains, role chains, the L/R mix,
//! file branches and stream branches (composite-stream-plan.md §4).

use super::events::CompositeError;
use super::ids::{DeviceKey, DeviceKind, OutputId, Role, SinkId};
use super::spec::{
    DeviceOptions, EncoderOptions, H264Encoder, ResolvedDevice, ResolvedOutput, ResolvedSpec,
    SinkInfo, SinkKind, StartPolicy,
};
use super::tags::{element_name, ElementTag, Scope};
use crate::get_gst_device;
use crate::media_device::{GStreamerError, GstMediaDevice};
use gstreamer::prelude::*;
use gstreamer::{Buffer, Caps, Element, Pad, Pipeline};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Source → caps → decode/normalize → tee for one physical device.
#[derive(Debug)]
pub struct DeviceChainHandles {
    pub key: DeviceKey,
    pub source: Element,
    pub elements: Vec<Element>,
    pub tee: Element,
}

/// queue → channel pick → audiorate → tee for one role. Cameras and screens use
/// their device tee directly and have no elements of their own.
#[derive(Debug)]
pub struct RoleChainHandles {
    pub role: Role,
    pub elements: Vec<Element>,
    /// Requested device tee pad feeding this role (mics only), released on isolation.
    pub source_pad: Option<Pad>,
    pub tee: Element,
}

/// Per-mic L/R placement into `audiomixer`, then a tee.
#[derive(Debug)]
pub struct LrMixHandles {
    pub elements: Vec<Element>,
    /// Requested role tee pads feeding the mix inputs.
    pub source_pads: Vec<Pad>,
    pub mixer: Element,
    /// Requested mixer sink pads. A side whose mic failed to open has none and stays
    /// silent.
    pub left_pad: Option<Pad>,
    pub right_pad: Option<Pad>,
    pub tee: Element,
}

/// queue → encoder → mp4mux → filesink for one file.
#[derive(Debug)]
pub struct OutputBranchHandles {
    pub output: OutputId,
    pub path: PathBuf,
    pub elements: Vec<Element>,
    /// Requested tee src pads feeding this branch, released on isolation.
    pub tee_pads: Vec<Pad>,
    pub mux: Element,
    pub filesink: Element,
}

/// Leaky queue → convert (→ audiornnoise) → appsink for one track.
#[derive(Debug)]
pub struct StreamBranchHandles {
    pub sink: SinkId,
    pub info: SinkInfo,
    pub elements: Vec<Element>,
    pub tee_pad: Pad,
    pub appsink: gstreamer_app::AppSink,
    pub frames: broadcast::Sender<Arc<Buffer>>,
    /// Fires when the branch reaches EOS (stop or isolation), so its track can unpublish.
    pub closed: broadcast::Sender<()>,
    /// Whether `audiornnoise` is in the chain. False for video, or when it isn't installed.
    pub denoised: bool,
}

/// Everything added to the pipeline, keyed by identity. Isolation removes entries.
#[derive(Debug, Default)]
pub struct BuiltGraph {
    pub devices: HashMap<DeviceKey, DeviceChainHandles>,
    pub roles: HashMap<Role, RoleChainHandles>,
    pub mix: Option<LrMixHandles>,
    pub outputs: HashMap<OutputId, OutputBranchHandles>,
    pub sinks: HashMap<SinkId, StreamBranchHandles>,
}

/// Format of every mic after the device chain: what `audiornnoise` needs.
const AUDIO_FORMAT: &str = "F32LE";
const AUDIO_RATE: i32 = 48_000;
const VIDEO_FORMAT: &str = "I420";
/// Buffering between a tee and each role, so one slow role can't stall its siblings.
const ROLE_QUEUE_TIME: u64 = 1_000_000_000;
/// `audiorate` fills gaps and drops overlaps larger than this.
const AUDIO_RATE_TOLERANCE: u64 = 40_000_000;
/// How long `audiomixer` waits for a late input before mixing without it.
const MIX_LATENCY: u64 = 40_000_000;
/// Size of each mixed buffer.
const MIX_BUFFER_DURATION: u64 = 10_000_000;
/// Front left | front right.
const STEREO_MASK: u64 = 0b11;
/// Buffering in front of each encoder. File queues never drop data.
const FILE_QUEUE_TIME: u64 = 3_000_000_000;
/// MP4 fragment length: a file cut off by a crash stays readable up to the last fragment.
const MP4_FRAGMENT_MS: u32 = 1000;
const MUX_LATENCY: u64 = 40_000_000;
/// H.264 encoders in order of preference for `H264Encoder::Auto`.
const HARDWARE_H264_ENCODERS: [&str; 2] = ["vtenc_h264_hw", "mfh264enc"];
/// Stream queues drop the oldest data beyond this, so streaming never holds files back.
const STREAM_VIDEO_QUEUE_BUFFERS: u32 = 2;
const STREAM_AUDIO_QUEUE_TIME: u64 = 200_000_000;
/// Frames buffered per subscriber before it starts skipping.
const VIDEO_FRAME_CAPACITY: usize = 2;
const AUDIO_FRAME_CAPACITY: usize = 32;

fn pipeline_error(message: impl Into<String>) -> CompositeError {
    CompositeError::GStreamer(GStreamerError::PipelineError(message.into()))
}

/// Creates an element named `<scope>:<purpose>`.
pub fn make_tagged_element(
    factory: &str,
    scope: Scope,
    purpose: &str,
) -> Result<Element, CompositeError> {
    let name = element_name(scope, purpose);
    gstreamer::ElementFactory::make(factory)
        .name(name.as_str())
        .build()
        .map_err(|_| pipeline_error(format!("failed to create {factory} ({name})")))
}

fn make_capsfilter(scope: Scope, purpose: &str, caps: &Caps) -> Result<Element, CompositeError> {
    let filter = make_tagged_element("capsfilter", scope, purpose)?;
    filter.set_property("caps", caps);
    Ok(filter)
}

fn make_tee(scope: Scope) -> Result<Element, CompositeError> {
    let tee = make_tagged_element("tee", scope, "tee")?;
    tee.set_property("allow-not-linked", true);
    Ok(tee)
}

/// Adds `elements` to `pipeline` and links them in order.
fn add_and_link(pipeline: &Pipeline, elements: &[Element]) -> Result<(), CompositeError> {
    pipeline
        .add_many(elements)
        .map_err(|e| pipeline_error(format!("failed to add elements: {e}")))?;
    Element::link_many(elements).map_err(|e| {
        let names: Vec<String> = elements.iter().map(|el| el.name().to_string()).collect();
        pipeline_error(format!("failed to link {}: {e}", names.join(" ! ")))
    })
}

/// Links a new request pad of `tee` to the static sink pad of `downstream`.
fn link_tee(tee: &Element, downstream: &Element) -> Result<Pad, CompositeError> {
    let tee_pad = tee
        .request_pad_simple("src_%u")
        .ok_or_else(|| pipeline_error(format!("{} gave no src pad", tee.name())))?;
    let sink = downstream
        .static_pad("sink")
        .ok_or_else(|| pipeline_error(format!("{} has no sink pad", downstream.name())))?;
    tee_pad.link(&sink).map_err(|e| {
        pipeline_error(format!(
            "failed to link {} to {}: {e:?}",
            tee.name(),
            downstream.name()
        ))
    })?;
    Ok(tee_pad)
}

/// Caps of an audio stream with `channels` channels. No channel mask is set, so
/// multichannel interfaces pass through whether their channels are positioned or not:
/// `audioconvert` cannot turn unpositioned channels into positioned ones, and the role
/// chain picks channels with `mix-matrix`, which works for either.
fn audio_caps(channels: i32) -> Caps {
    Caps::builder("audio/x-raw")
        .field("format", AUDIO_FORMAT)
        .field("rate", AUDIO_RATE)
        .field("channels", channels)
        .field("layout", "interleaved")
        .build()
}

fn stereo_caps() -> Caps {
    Caps::builder("audio/x-raw")
        .field("format", AUDIO_FORMAT)
        .field("rate", AUDIO_RATE)
        .field("channels", 2)
        .field("layout", "interleaved")
        .field("channel-mask", gstreamer::Bitmask::new(STEREO_MASK))
        .build()
}

fn i420_caps(width: i32, height: i32, framerate: i32) -> Caps {
    Caps::builder("video/x-raw")
        .field("format", VIDEO_FORMAT)
        .field("width", width)
        .field("height", height)
        .field("framerate", gstreamer::Fraction::new(framerate, 1))
        .field("pixel-aspect-ratio", gstreamer::Fraction::new(1, 1))
        .build()
}

/// `audioconvert` `mix-matrix` value for `matrix` (rows are output channels).
fn mix_matrix_value(matrix: &[Vec<f32>]) -> gstreamer::Array {
    gstreamer::Array::new(
        matrix
            .iter()
            .map(|row| gstreamer::Array::new(row.iter().copied()).to_send_value()),
    )
}

/// Creates the live source element of `device`, named `devK:src`.
fn make_source(device: &ResolvedDevice) -> Result<Element, CompositeError> {
    let scope = Scope::Device(device.device.key);
    #[cfg(any(test, feature = "test-sources"))]
    if let Some(source) = test_sources::make(device, scope)? {
        return Ok(source);
    }
    let name = element_name(scope, "src");
    let id = &device.device.device_id;
    let not_found = || {
        CompositeError::GStreamer(GStreamerError::DeviceError(format!(
            "device not found: {id}"
        )))
    };
    match device.device.kind {
        DeviceKind::Camera | DeviceKind::Mic => get_gst_device(id)
            .ok_or_else(not_found)?
            .create_element(Some(&name))
            .map_err(|e| pipeline_error(format!("failed to open {id}: {e}"))),
        DeviceKind::Screen => {
            let source = GstMediaDevice::from_screen_id_or_name(id)
                .map_err(|_| not_found())?
                .get_screen_element()?;
            source.set_property("name", &name);
            Ok(source)
        }
    }
}

/// `test:video`, `test:screen` and `test:audio:<hz>` device ids map to live test sources,
/// so pipelines can be exercised without hardware.
#[cfg(any(test, feature = "test-sources"))]
mod test_sources {
    use super::*;

    pub(super) fn make(
        device: &ResolvedDevice,
        scope: Scope,
    ) -> Result<Option<Element>, CompositeError> {
        let Some(spec) = device.device.device_id.strip_prefix("test:") else {
            return Ok(None);
        };
        let mut parts = spec.split(':');
        let source = match parts.next() {
            Some("video") | Some("screen") => {
                let source = make_tagged_element("videotestsrc", scope, "src")?;
                source.set_property_from_str("pattern", "ball");
                source
            }
            Some("audio") => {
                let source = make_tagged_element("audiotestsrc", scope, "src")?;
                if let Some(freq) = parts.next().and_then(|f| f.parse::<f64>().ok()) {
                    source.set_property("freq", freq);
                }
                source
            }
            Some("tones") => {
                let freqs: Vec<f64> = parts.filter_map(|f| f.parse().ok()).collect();
                return tones(scope, &freqs).map(Some);
            }
            Some("zero-ts") => {
                let freq = parts.next().and_then(|f| f.parse::<f64>().ok());
                return zero_timestamps(scope, freq).map(Some);
            }
            Some("missing") => {
                return Err(CompositeError::GStreamer(GStreamerError::DeviceError(
                    "device not found: test:missing".into(),
                )))
            }
            _ => return Err(pipeline_error(format!("unknown test device {spec}"))),
        };
        source.set_property("is-live", true);
        Ok(Some(source))
    }

    /// A mic whose buffers all carry timestamp 0 (`test:zero-ts:<hz>`), as `osxaudiosrc`
    /// produces on a pipeline running on the system clock.
    fn zero_timestamps(scope: Scope, freq: Option<f64>) -> Result<Element, CompositeError> {
        let bin = gstreamer::Bin::with_name(&element_name(scope, "src"));
        let tone = gstreamer::ElementFactory::make("audiotestsrc")
            .property("is-live", true)
            .build()
            .map_err(|_| pipeline_error("failed to create audiotestsrc"))?;
        if let Some(freq) = freq {
            tone.set_property("freq", freq);
        }
        bin.add(&tone).map_err(|e| pipeline_error(e.to_string()))?;
        let src = tone
            .static_pad("src")
            .ok_or_else(|| pipeline_error("audiotestsrc has no src pad"))?;
        src.add_probe(gstreamer::PadProbeType::BUFFER, |_, info| {
            if let Some(buffer) = info.buffer_mut() {
                buffer.make_mut().set_pts(gstreamer::ClockTime::ZERO);
            }
            gstreamer::PadProbeReturn::Ok
        });
        let ghost =
            gstreamer::GhostPad::with_target(&src).map_err(|e| pipeline_error(e.to_string()))?;
        bin.add_pad(&ghost)
            .map_err(|e| pipeline_error(e.to_string()))?;
        Ok(bin.upcast())
    }

    /// A multichannel device with one sine tone per channel (`test:tones:440:880:…`),
    /// so tests can check which channel ends up where.
    fn tones(scope: Scope, freqs: &[f64]) -> Result<Element, CompositeError> {
        let bin = gstreamer::Bin::with_name(&element_name(scope, "src"));
        let interleave = gstreamer::ElementFactory::make("interleave")
            .property("channel-positions-from-input", false)
            .build()
            .map_err(|_| pipeline_error("failed to create interleave"))?;
        bin.add(&interleave)
            .map_err(|e| pipeline_error(e.to_string()))?;
        let mono = Caps::builder("audio/x-raw")
            .field("format", AUDIO_FORMAT)
            .field("rate", AUDIO_RATE)
            .field("channels", 1)
            .build();
        for freq in freqs {
            let tone = gstreamer::ElementFactory::make("audiotestsrc")
                .property("is-live", true)
                .property("freq", *freq)
                .build()
                .map_err(|_| pipeline_error("failed to create audiotestsrc"))?;
            let caps = gstreamer::ElementFactory::make("capsfilter")
                .property("caps", &mono)
                .build()
                .map_err(|_| pipeline_error("failed to create capsfilter"))?;
            bin.add_many([&tone, &caps])
                .map_err(|e| pipeline_error(e.to_string()))?;
            Element::link_many([&tone, &caps, &interleave])
                .map_err(|e| pipeline_error(e.to_string()))?;
        }
        let src = interleave
            .static_pad("src")
            .ok_or_else(|| pipeline_error("interleave has no src pad"))?;
        let ghost =
            gstreamer::GhostPad::with_target(&src).map_err(|e| pipeline_error(e.to_string()))?;
        bin.add_pad(&ghost)
            .map_err(|e| pipeline_error(e.to_string()))?;
        Ok(bin.upcast())
    }
}

/// Builds every layer of `spec` into `pipeline`, in order: device chains, role chains,
/// the L/R mix, file branches, then stream branches.
///
/// With `StartPolicy::BestEffort`, a device that fails to open is returned in the second
/// list and left out, along with whatever depends only on it: its roles, its files and its
/// streams. An AV mix missing one mic keeps its L/R layout with that side silent; one
/// missing its camera records audio only. With `AllOrNothing`, the first failure is
/// returned. Fails if nothing can be recorded or streamed.
pub fn build_graph(
    pipeline: &Pipeline,
    spec: &ResolvedSpec,
) -> Result<(BuiltGraph, Vec<(DeviceKey, CompositeError)>), CompositeError> {
    let mut graph = BuiltGraph::default();
    let mut failed = Vec::new();

    for device in &spec.devices {
        let key = device.device.key;
        match add_device_chain(pipeline, device) {
            Ok(chain) => {
                graph.devices.insert(key, chain);
            }
            Err(error) if spec.start_policy == StartPolicy::BestEffort => {
                remove_scope(pipeline, Scope::Device(key));
                failed.push((key, error));
            }
            Err(error) => return Err(error),
        }
    }

    for role in &spec.roles {
        if let Some(device) = graph.devices.get(&role.device) {
            let chain = add_role_chain(pipeline, spec, role.role, device)?;
            graph.roles.insert(role.role, chain);
        }
    }

    if let Some(lr) = spec.lr_mix() {
        let (left, right) = (graph.roles.get(&lr.left), graph.roles.get(&lr.right));
        if left.is_some() || right.is_some() {
            graph.mix = Some(add_lr_mix(pipeline, left, right)?);
        }
    }

    for output in &spec.outputs {
        let (video, audio) = output_inputs(output, &graph.roles, graph.mix.as_ref());
        if video.is_some() || audio.is_some() {
            let branch = add_output_branch(pipeline, spec, output, video.as_ref(), audio.as_ref())?;
            graph.outputs.insert(output.id, branch);
        }
    }

    for info in &spec.sinks {
        if let Some(tee) = sink_input(info, &graph.roles, graph.mix.as_ref()) {
            let branch = add_stream_branch(pipeline, spec, info, &tee)?;
            graph.sinks.insert(info.id, branch);
        }
    }

    if graph.outputs.is_empty() && graph.sinks.is_empty() {
        return Err(match failed.into_iter().next() {
            Some((_, error)) => error,
            None => pipeline_error("nothing to record or stream"),
        });
    }
    Ok((graph, failed))
}

/// Removes every element of `scope` from `pipeline`, e.g. a device chain that failed
/// halfway through being built.
fn remove_scope(pipeline: &Pipeline, scope: Scope) {
    for element in pipeline.children() {
        if ElementTag::parse(&element.name()).is_some_and(|tag| tag.scope == scope) {
            let _ = element.set_state(gstreamer::State::Null);
            let _ = pipeline.remove(&element);
        }
    }
}

/// Stamps every buffer leaving an audio source with the pipeline clock's running time at
/// arrival, minus the buffer's duration (when its first sample was captured).
///
/// The pipeline runs on the system clock. Sources such as `osxaudiosrc` (and likely
/// `wasapi2src`) timestamp from their own device clock, which counts from zero; GStreamer
/// skips its `slave-method` handling when a source supplies its own timestamps and clamps
/// them all to zero against the system clock's base time, so `audiorate` drops nearly
/// every sample. Re-stamping on arrival puts every mic on the pipeline clock; `audiorate`
/// downstream smooths the arrival jitter into continuous samples.
fn retimestamp_on_pipeline_clock(source: &Element) {
    let Some(pad) = source.static_pad("src") else {
        return;
    };
    pad.add_probe(gstreamer::PadProbeType::BUFFER, |pad, info| {
        let now = pad
            .parent_element()
            .and_then(|element| Some((element.clock()?.time(), element.base_time()?)));
        if let (Some((now, base)), Some(buffer)) = (now, info.buffer_mut()) {
            let duration = buffer.duration().unwrap_or_default();
            let pts = now.saturating_sub(base).saturating_sub(duration);
            let buffer = buffer.make_mut();
            buffer.set_pts(pts);
            buffer.set_dts(gstreamer::ClockTime::NONE);
        }
        gstreamer::PadProbeReturn::Ok
    });
}

/// Adds the source chain of one physical device, ending in its tee:
///
/// - camera: `src ! caps(codec) ! [decode] ! videoconvert ! videoscale ! caps(I420) ! tee`
/// - screen: `src ! videoconvert ! videoscale ! caps(I420) ! tee`
/// - mic: `src ! caps(rate, channels) ! audioconvert ! audioresample ! caps(F32LE 48k) ! tee`
pub fn add_device_chain(
    pipeline: &Pipeline,
    device: &ResolvedDevice,
) -> Result<DeviceChainHandles, CompositeError> {
    let scope = Scope::Device(device.device.key);
    let source = make_source(device)?;
    let mut chain = vec![source.clone()];

    match &device.options {
        DeviceOptions::Video(options) => {
            let source_caps = Caps::builder(options.codec.as_str())
                .field("width", options.width)
                .field("height", options.height)
                .field("framerate", gstreamer::Fraction::new(options.framerate, 1))
                .build();
            chain.push(make_capsfilter(scope, "src-caps", &source_caps)?);
            match options.codec.as_str() {
                "image/jpeg" => chain.push(make_tagged_element("jpegdec", scope, "decode")?),
                "video/x-h264" => {
                    chain.push(make_tagged_element("h264parse", scope, "parse")?);
                    chain.push(make_tagged_element("avdec_h264", scope, "decode")?);
                }
                "video/x-raw" => {}
                codec => return Err(pipeline_error(format!("unsupported video codec {codec}"))),
            }
            chain.push(make_tagged_element("videoconvert", scope, "convert")?);
            chain.push(make_tagged_element("videoscale", scope, "scale")?);
            chain.push(make_capsfilter(
                scope,
                "caps",
                &i420_caps(options.width, options.height, options.framerate),
            )?);
        }
        DeviceOptions::Screen(options) => {
            chain.push(make_tagged_element("videoconvert", scope, "convert")?);
            chain.push(make_tagged_element("videoscale", scope, "scale")?);
            chain.push(make_capsfilter(
                scope,
                "caps",
                &i420_caps(options.width, options.height, options.framerate),
            )?);
        }
        DeviceOptions::Audio(options) => {
            retimestamp_on_pipeline_clock(&source);
            let source_caps = Caps::builder("audio/x-raw")
                .field("rate", options.framerate)
                .field("channels", options.channels)
                .build();
            chain.push(make_capsfilter(scope, "src-caps", &source_caps)?);
            chain.push(make_tagged_element("audioconvert", scope, "convert")?);
            chain.push(make_tagged_element("audioresample", scope, "resample")?);
            chain.push(make_capsfilter(
                scope,
                "caps",
                &audio_caps(options.channels),
            )?);
        }
    }

    let tee = make_tee(scope)?;
    chain.push(tee.clone());
    add_and_link(pipeline, &chain)?;

    Ok(DeviceChainHandles {
        key: device.device.key,
        source,
        elements: chain,
        tee,
    })
}

/// Adds the chain of one role. Cameras and screens use the device tee directly. Mics get
/// `queue ! audioconvert(mix-matrix) ! caps ! audiorate ! tee`, where the matrix picks
/// the selected channel or averages all channels to mono. Extra mics without a selected
/// channel keep every device channel.
pub fn add_role_chain(
    pipeline: &Pipeline,
    spec: &ResolvedSpec,
    role: Role,
    device: &DeviceChainHandles,
) -> Result<RoleChainHandles, CompositeError> {
    let role_ref = spec
        .roles
        .iter()
        .find(|r| r.role == role)
        .ok_or_else(|| pipeline_error(format!("{role} is not in the spec")))?;
    let channels = match spec.device(role_ref.device).map(|d| &d.options) {
        Some(DeviceOptions::Audio(options)) => options.channels,
        _ => {
            return Ok(RoleChainHandles {
                role,
                elements: Vec::new(),
                source_pad: None,
                tee: device.tee.clone(),
            })
        }
    };

    let scope = Scope::Role(role);
    let queue = make_tagged_element("queue", scope, "q")?;
    queue.set_property("max-size-time", ROLE_QUEUE_TIME);
    queue.set_property("max-size-buffers", 0u32);
    queue.set_property("max-size-bytes", 0u32);

    let pick = make_tagged_element("audioconvert", scope, "pick")?;
    let to_mono = role_ref.selected_channel.is_some() || spec.is_mix_role(role);
    let out_channels = if to_mono { 1 } else { channels };
    if to_mono && channels > 1 {
        let channels = usize::try_from(channels).unwrap_or(1);
        pick.set_property(
            "mix-matrix",
            mix_matrix_value(&mono_pick_matrix(channels, role_ref.selected_channel)),
        );
    }
    let caps = make_capsfilter(scope, "caps", &audio_caps(out_channels))?;

    let rate = make_tagged_element("audiorate", scope, "rate")?;
    rate.set_property("tolerance", AUDIO_RATE_TOLERANCE);
    rate.set_property("skip-to-first", true);

    let tee = make_tee(scope)?;
    let elements = vec![queue.clone(), pick, caps, rate, tee.clone()];
    add_and_link(pipeline, &elements)?;
    let source_pad = link_tee(&device.tee, &queue)?;

    Ok(RoleChainHandles {
        role,
        elements,
        source_pad: Some(source_pad),
        tee,
    })
}

/// Adds the L/R mix of two mono mic roles, ending in its tee:
///
/// ```text
/// left role tee  ! queue ! audioconvert(mix-matrix [[1],[0]]) ! caps(stereo) ─┐
///                                                                              audiomixer ! caps(stereo) ! tee
/// right role tee ! queue ! audioconvert(mix-matrix [[0],[1]]) ! caps(stereo) ─┘
/// ```
///
/// Each output channel has exactly one non-zero input, so the mixer places rather than
/// sums, and pads keep volume 1.0. `audiomixer` is an aggregator: it times out on a
/// stalled input and keeps running when one input ends, leaving that channel silent.
/// A missing side (its mic failed to open) is left out and stays silent, so the file
/// keeps its L/R layout.
pub fn add_lr_mix(
    pipeline: &Pipeline,
    left: Option<&RoleChainHandles>,
    right: Option<&RoleChainHandles>,
) -> Result<LrMixHandles, CompositeError> {
    if left.is_none() && right.is_none() {
        return Err(pipeline_error("the L/R mix has no inputs"));
    }
    let scope = Scope::Mix;
    let mixer = make_tagged_element("audiomixer", scope, "mixer")?;
    mixer.set_property("latency", MIX_LATENCY);
    mixer.set_property_from_str("start-time-selection", "first");
    mixer.set_property("ignore-inactive-pads", true);
    mixer.set_property("output-buffer-duration", MIX_BUFFER_DURATION);
    let out_caps = make_capsfilter(scope, "caps", &stereo_caps())?;
    let tee = make_tee(scope)?;
    let mut elements = vec![mixer.clone(), out_caps, tee.clone()];
    add_and_link(pipeline, &elements)?;

    let mut place = |role: &RoleChainHandles, side: &str, to_left: bool| {
        let queue = make_tagged_element("queue", scope, &format!("q-{side}"))?;
        queue.set_property("max-size-time", ROLE_QUEUE_TIME);
        queue.set_property("max-size-buffers", 0u32);
        queue.set_property("max-size-bytes", 0u32);
        let convert = make_tagged_element("audioconvert", scope, &format!("to-{side}"))?;
        convert.set_property(
            "mix-matrix",
            mix_matrix_value(&lr_placement_matrix(to_left)),
        );
        let caps = make_capsfilter(scope, &format!("caps-{side}"), &stereo_caps())?;
        let input = [queue.clone(), convert, caps.clone()];
        add_and_link(pipeline, &input)?;
        elements.extend(input);

        let mixer_pad = mixer
            .request_pad_simple("sink_%u")
            .ok_or_else(|| pipeline_error("audiomixer gave no sink pad"))?;
        let caps_src = caps
            .static_pad("src")
            .ok_or_else(|| pipeline_error("capsfilter has no src pad"))?;
        caps_src
            .link(&mixer_pad)
            .map_err(|e| pipeline_error(format!("failed to link {side} input to mixer: {e:?}")))?;
        let source_pad = link_tee(&role.tee, &queue)?;
        Ok::<_, CompositeError>((source_pad, mixer_pad))
    };
    let left = left.map(|role| place(role, "l", true)).transpose()?;
    let right = right.map(|role| place(role, "r", false)).transpose()?;
    let source_pads = left
        .iter()
        .chain(right.iter())
        .map(|(source, _)| source.clone())
        .collect();

    Ok(LrMixHandles {
        elements,
        source_pads,
        mixer,
        left_pad: left.map(|(_, pad)| pad),
        right_pad: right.map(|(_, pad)| pad),
        tee,
    })
}

/// Adds the file branch of one output, fed from `video_tee` and/or `audio_tee`:
///
/// ```text
/// video tee ! queue ! videorate ! h264 encoder ! h264parse ─┐
///                                                            mp4mux ! filesink
/// audio tee ! queue ! audioconvert ! avenc_aac ! aacparse ──┘
/// ```
///
/// Queues are not leaky: files never drop data. `mp4mux` writes 1 s fragments while
/// recording and rewrites them into a regular MP4 on EOS, so a file cut off by a crash
/// stays readable. The caller creates `spec.output_dir` before playing.
pub fn add_output_branch(
    pipeline: &Pipeline,
    spec: &ResolvedSpec,
    output: &ResolvedOutput,
    video_tee: Option<&Element>,
    audio_tee: Option<&Element>,
) -> Result<OutputBranchHandles, CompositeError> {
    let output_dir = spec
        .output_dir
        .as_deref()
        .ok_or_else(|| pipeline_error("file branches need an output directory"))?;
    let path = output_dir.join(&output.file_name);
    let scope = Scope::File(output.id);

    let mux = make_tagged_element("mp4mux", scope, "mux")?;
    mux.set_property("fragment-duration", MP4_FRAGMENT_MS);
    mux.set_property_from_str("fragment-mode", "first-moov-then-finalise");
    mux.set_property("latency", MUX_LATENCY);
    mux.set_property_from_str("start-time-selection", "first");
    let filesink = make_tagged_element("filesink", scope, "sink")?;
    filesink.set_property("location", path.to_string_lossy().as_ref());
    filesink.set_property("sync", false);
    filesink.set_property("async", false);
    let mut elements = vec![mux.clone(), filesink.clone()];
    add_and_link(pipeline, &elements)?;

    let mut tee_pads = Vec::new();
    if let Some(tee) = video_tee {
        let framerate = output_framerate(spec, output)?;
        let encoder = make_tagged_element(pick_h264_encoder(&spec.encoders)?, scope, "venc")?;
        configure_h264_encoder(&encoder, spec.encoders.video_kbps, framerate);
        let rate = make_tagged_element("videorate", scope, "vrate")?;
        rate.set_property("max-rate", framerate);
        let parse = make_tagged_element("h264parse", scope, "vparse")?;
        parse.set_property("config-interval", -1i32);
        let chain = vec![file_queue(scope, "vq")?, rate, encoder, parse];
        tee_pads.push(add_mux_input(pipeline, tee, &chain, &mux, "video_%u")?);
        elements.extend(chain);
    }
    if let Some(tee) = audio_tee {
        let channels = output_channels(spec, output)?;
        let mut chain = vec![
            file_queue(scope, "aq")?,
            make_tagged_element("audioconvert", scope, "aconv")?,
        ];
        if channels > 2 {
            // AAC needs positioned channels; an identity matrix lets audioconvert assign
            // positions to unpositioned interface channels.
            let n = usize::try_from(channels).unwrap_or(0);
            let identity: Vec<Vec<f32>> = (0..n)
                .map(|row| {
                    (0..n)
                        .map(|col| if row == col { 1.0 } else { 0.0 })
                        .collect()
                })
                .collect();
            chain[1].set_property("mix-matrix", mix_matrix_value(&identity));
            let positioned = Caps::builder("audio/x-raw")
                .field("channels", channels)
                .field("channel-mask", gstreamer::Bitmask::new((1 << channels) - 1))
                .build();
            chain.push(make_capsfilter(scope, "apos", &positioned)?);
        }
        let encoder = make_tagged_element("avenc_aac", scope, "aenc")?;
        let bitrate = u64::from(spec.encoders.aac_kbps_per_channel) * 1000 * channels as u64;
        encoder.set_property_from_str("bitrate", &bitrate.to_string());
        chain.push(encoder);
        chain.push(make_tagged_element("aacparse", scope, "aparse")?);
        tee_pads.push(add_mux_input(pipeline, tee, &chain, &mux, "audio_%u")?);
        elements.extend(chain);
    }
    if tee_pads.is_empty() {
        return Err(pipeline_error(format!("{} has no inputs", output.id)));
    }

    Ok(OutputBranchHandles {
        output: output.id,
        path,
        elements,
        tee_pads,
        mux,
        filesink,
    })
}

/// The tees feeding `output`: `(video, audio)`. The AV mix takes the camera tee and the
/// L/R mix tee (or the single mic's tee); camera, screen and mic files take their role tee.
/// Inputs whose devices failed to open are `None`.
pub fn output_inputs(
    output: &ResolvedOutput,
    roles: &HashMap<Role, RoleChainHandles>,
    mix: Option<&LrMixHandles>,
) -> (Option<Element>, Option<Element>) {
    let tee = |role: &Role| roles.get(role).map(|r| r.tee.clone());
    let first = output.roles.first().and_then(tee);
    match output.id {
        OutputId::AvMix => {
            let audio = match output.audio_channels {
                Some(_) => mix.map(|m| m.tee.clone()),
                None => output.roles.get(1).and_then(tee),
            };
            (first, audio)
        }
        OutputId::Camera(_) | OutputId::Screen(_) => (first, None),
        OutputId::Mic(_) => (None, first),
    }
}

fn file_queue(scope: Scope, purpose: &str) -> Result<Element, CompositeError> {
    let queue = make_tagged_element("queue", scope, purpose)?;
    queue.set_property("max-size-time", FILE_QUEUE_TIME);
    queue.set_property("max-size-buffers", 0u32);
    queue.set_property("max-size-bytes", 0u32);
    Ok(queue)
}

/// Links `tee ! chain…` into a new request pad of `mux` and returns the tee pad.
fn add_mux_input(
    pipeline: &Pipeline,
    tee: &Element,
    chain: &[Element],
    mux: &Element,
    pad_template: &str,
) -> Result<Pad, CompositeError> {
    add_and_link(pipeline, chain)?;
    let (first, last) = (&chain[0], &chain[chain.len() - 1]);
    let mux_pad = mux
        .request_pad_simple(pad_template)
        .ok_or_else(|| pipeline_error(format!("{} gave no {pad_template} pad", mux.name())))?;
    last.static_pad("src")
        .ok_or_else(|| pipeline_error(format!("{} has no src pad", last.name())))?
        .link(&mux_pad)
        .map_err(|e| {
            pipeline_error(format!(
                "failed to link {} to {}: {e:?}",
                last.name(),
                mux.name()
            ))
        })?;
    link_tee(tee, first)
}

/// Frame rate of the camera or screen recorded in `output`.
fn output_framerate(spec: &ResolvedSpec, output: &ResolvedOutput) -> Result<i32, CompositeError> {
    output
        .roles
        .iter()
        .filter_map(|role| spec.device(spec.device_of(*role)?))
        .find_map(|device| match &device.options {
            DeviceOptions::Video(o) => Some(o.framerate),
            DeviceOptions::Screen(o) => Some(o.framerate),
            DeviceOptions::Audio(_) => None,
        })
        .ok_or_else(|| pipeline_error(format!("{} has no video role", output.id)))
}

/// Channel count of the audio recorded in `output`: stereo for a two-mic AV mix, mono for
/// one, and the role's channels for a mic file.
fn output_channels(spec: &ResolvedSpec, output: &ResolvedOutput) -> Result<i32, CompositeError> {
    if output.id == OutputId::AvMix {
        return Ok(if output.audio_channels.is_some() {
            2
        } else {
            1
        });
    }
    let role = output
        .roles
        .first()
        .ok_or_else(|| pipeline_error(format!("{} has no role", output.id)))?;
    role_channels(spec, *role)
}

/// Channels a mic role produces: mono for mix mics and picked channels, otherwise every
/// device channel.
fn role_channels(spec: &ResolvedSpec, role: Role) -> Result<i32, CompositeError> {
    let role_ref = spec
        .roles
        .iter()
        .find(|r| r.role == role)
        .ok_or_else(|| pipeline_error(format!("{role} is not in the spec")))?;
    if role_ref.selected_channel.is_some() || spec.is_mix_role(role) {
        return Ok(1);
    }
    match spec.device(role_ref.device).map(|d| &d.options) {
        Some(DeviceOptions::Audio(options)) => Ok(options.channels),
        _ => Err(pipeline_error(format!("{role} has no audio device"))),
    }
}

/// Channels arriving at the stream branch of `info`: stereo from the L/R mix, otherwise
/// what the role produces.
fn stream_input_channels(spec: &ResolvedSpec, info: &SinkInfo) -> Result<i32, CompositeError> {
    match (info.id, info.roles.as_slice()) {
        (SinkId::AvMixAudio, [_, _]) => Ok(2),
        (_, [role, ..]) => role_channels(spec, *role),
        _ => Err(pipeline_error(format!("{} has no roles", info.id))),
    }
}

/// Sets bitrate and a keyframe every second (matching the MP4 fragments) on whichever
/// H.264 encoder was picked; property names differ between encoders.
fn configure_h264_encoder(encoder: &Element, kbps: u32, framerate: i32) {
    let set = |name: &str, value: String| {
        if encoder.find_property(name).is_some() {
            encoder.set_property_from_str(name, &value);
        }
    };
    set("bitrate", kbps.to_string());
    set("speed-preset", "ultrafast".into());
    set("key-int-max", framerate.to_string());
    set("max-keyframe-interval", framerate.to_string());
    set("gop-size", framerate.to_string());
    set("realtime", "true".into());
    set("allow-frame-reordering", "false".into());
}

/// Adds the stream branch of one appsink, fed from `tee`:
///
/// - video: `queue(leaky) ! videoscale ! videoconvert ! caps(I420 640x480) ! appsink`
/// - audio: `queue(leaky) ! audioconvert ! audioresample ! caps(F32LE 48k mono)
///   ! audiornnoise ! audioconvert ! caps(S16LE 48k mono) ! appsink`
///
/// Multichannel input (the L/R mix, or an interface mic) is downmixed to mono by
/// averaging with an explicit `mix-matrix`, which works whether or not the channels are
/// positioned. `audiornnoise` runs only here, so files keep the raw audio; without it
/// installed the branch is built without noise suppression and `denoised` is false.
pub fn add_stream_branch(
    pipeline: &Pipeline,
    spec: &ResolvedSpec,
    info: &SinkInfo,
    tee: &Element,
) -> Result<StreamBranchHandles, CompositeError> {
    let audio = &spec.stream_audio;
    let scope = Scope::Sink(info.id);
    let queue = make_tagged_element("queue", scope, "q")?;
    queue.set_property_from_str("leaky", "downstream");
    queue.set_property("max-size-bytes", 0u32);
    let mut chain = vec![queue.clone()];
    let mut denoised = false;

    let (caps, capacity) = match info.kind {
        SinkKind::Video { width, height } => {
            queue.set_property("max-size-buffers", STREAM_VIDEO_QUEUE_BUFFERS);
            queue.set_property("max-size-time", 0u64);
            chain.push(make_tagged_element("videoscale", scope, "scale")?);
            chain.push(make_tagged_element("videoconvert", scope, "convert")?);
            let caps = Caps::builder("video/x-raw")
                .field("format", VIDEO_FORMAT)
                .field("width", i32::try_from(width).unwrap_or(i32::MAX))
                .field("height", i32::try_from(height).unwrap_or(i32::MAX))
                .field("pixel-aspect-ratio", gstreamer::Fraction::new(1, 1))
                .build();
            (caps, VIDEO_FRAME_CAPACITY)
        }
        SinkKind::Audio { rate, channels } => {
            queue.set_property("max-size-buffers", 0u32);
            queue.set_property("max-size-time", STREAM_AUDIO_QUEUE_TIME);
            let rate = i32::try_from(rate).unwrap_or(AUDIO_RATE);
            let channels = i32::try_from(channels).unwrap_or(1);
            let downmix = make_tagged_element("audioconvert", scope, "downmix")?;
            let input_channels = stream_input_channels(spec, info)?;
            if channels == 1 && input_channels > 1 {
                let n = usize::try_from(input_channels).unwrap_or(1);
                downmix.set_property("mix-matrix", mix_matrix_value(&mono_pick_matrix(n, None)));
            }
            chain.push(downmix);
            chain.push(make_tagged_element("audioresample", scope, "resample")?);
            let float = Caps::builder("audio/x-raw")
                .field("format", AUDIO_FORMAT)
                .field("rate", rate)
                .field("channels", channels)
                .field("layout", "interleaved")
                .build();
            chain.push(make_capsfilter(scope, "float", &float)?);
            if audio.noise_suppression && rnnoise_available() {
                let rnnoise = make_tagged_element("audiornnoise", scope, "rnnoise")?;
                rnnoise.set_property("voice-activity-threshold", audio.vad_threshold);
                chain.push(rnnoise);
                denoised = true;
            }
            chain.push(make_tagged_element("audioconvert", scope, "convert")?);
            let caps = Caps::builder("audio/x-raw")
                .field("format", "S16LE")
                .field("rate", rate)
                .field("channels", channels)
                .field("layout", "interleaved")
                .build();
            (caps, AUDIO_FRAME_CAPACITY)
        }
    };
    chain.push(make_capsfilter(scope, "caps", &caps)?);

    let (frames, _) = broadcast::channel(capacity);
    let (closed, _) = broadcast::channel(1);
    let appsink = make_tagged_element("appsink", scope, "appsink")?
        .dynamic_cast::<gstreamer_app::AppSink>()
        .map_err(|_| pipeline_error("appsink is not an AppSink"))?;
    configure_appsink(&appsink, &info.kind, frames.clone(), closed.clone());
    chain.push(appsink.clone().upcast());

    add_and_link(pipeline, &chain)?;
    let tee_pad = link_tee(tee, &queue)?;

    Ok(StreamBranchHandles {
        sink: info.id,
        info: info.clone(),
        elements: chain,
        tee_pad,
        appsink,
        frames,
        closed,
        denoised,
    })
}

/// Broadcasts every sample to subscribers and signals `closed` on EOS. The sink never
/// syncs or prerolls: a live pipeline must not wait on it, or it would block its tee and
/// starve the file branches.
fn configure_appsink(
    appsink: &gstreamer_app::AppSink,
    kind: &SinkKind,
    frames: broadcast::Sender<Arc<Buffer>>,
    closed: broadcast::Sender<()>,
) {
    appsink.set_property("sync", false);
    appsink.set_property("async", false);
    match kind {
        SinkKind::Video { .. } => {
            appsink.set_property("drop", true);
            appsink.set_property("max-buffers", 1u32);
        }
        SinkKind::Audio { .. } => {
            appsink.set_property("drop", false);
            appsink.set_property("max-buffers", 8u32);
        }
    }
    appsink.set_callbacks(
        gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gstreamer::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gstreamer::FlowError::Error)?;
                if frames.receiver_count() > 0 {
                    let _ = frames.send(Arc::new(buffer.copy()));
                }
                Ok(gstreamer::FlowSuccess::Ok)
            })
            .eos(move |_| {
                let _ = closed.send(());
            })
            .build(),
    );
}

/// The tee feeding the stream of `info`: the camera tee for the AV mix video, the L/R
/// mix tee (or the single mic's tee) for the AV mix audio, and the role tee otherwise.
/// `None` when the devices feeding it failed to open.
pub fn sink_input(
    info: &SinkInfo,
    roles: &HashMap<Role, RoleChainHandles>,
    mix: Option<&LrMixHandles>,
) -> Option<Element> {
    match (info.id, info.roles.as_slice()) {
        (SinkId::AvMixAudio, [_, _]) => mix.map(|m| m.tee.clone()),
        (_, [role, ..]) => roles.get(role).map(|r| r.tee.clone()),
        _ => None,
    }
}

/// Picks the H.264 encoder factory for `options`, preferring hardware for `Auto`.
pub fn pick_h264_encoder(options: &EncoderOptions) -> Result<&'static str, CompositeError> {
    let available = |name: &&str| gstreamer::ElementFactory::find(name).is_some();
    let hardware = HARDWARE_H264_ENCODERS.into_iter().find(available);
    match options.h264 {
        H264Encoder::Auto => hardware.or(Some("x264enc").filter(available)),
        H264Encoder::Hardware => hardware,
        H264Encoder::X264 => Some("x264enc").filter(available),
    }
    .ok_or_else(|| pipeline_error(format!("no {:?} H.264 encoder is installed", options.h264)))
}

/// Whether `audiornnoise` (gst-plugins-rs `rsaudiofx`) is installed.
pub fn rnnoise_available() -> bool {
    gstreamer::ElementFactory::find("audiornnoise").is_some()
}

/// `audioconvert` `mix-matrix` taking one input channel to mono. With no channel
/// selected, all input channels are averaged.
pub fn mono_pick_matrix(channels: usize, selected_channel: Option<u8>) -> Vec<Vec<f32>> {
    let row = match selected_channel {
        Some(channel) => (1..=channels)
            .map(|c| if c == usize::from(channel) { 1.0 } else { 0.0 })
            .collect(),
        None => vec![1.0 / channels.max(1) as f32; channels],
    };
    vec![row]
}

/// `audioconvert` `mix-matrix` placing a mono input on the left or right channel only.
pub fn lr_placement_matrix(left: bool) -> Vec<Vec<f32>> {
    if left {
        vec![vec![1.0], vec![0.0]]
    } else {
        vec![vec![0.0], vec![1.0]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_matrices() {
        assert_eq!(mono_pick_matrix(4, Some(3)), vec![vec![0.0, 0.0, 1.0, 0.0]]);
        assert_eq!(mono_pick_matrix(2, None), vec![vec![0.5, 0.5]]);
        assert_eq!(mono_pick_matrix(1, None), vec![vec![1.0]]);
    }

    use crate::composite::spec::{AvMixSpec, CompositeStreamSpec, Output};
    use crate::{AudioPublishOptions, ScreenPublishOptions, VideoPublishOptions};
    use std::time::Duration;

    fn test_mic(device: &str, channels: i32, selected_channel: Option<i32>) -> AudioPublishOptions {
        AudioPublishOptions {
            codec: "audio/x-raw".into(),
            device_id: device.into(),
            framerate: 48000,
            channels,
            selected_channel,
            local_file_save_options: None,
        }
    }

    /// Camera, a 4-channel interface feeding both mix mics, an extra stereo mic and a screen.
    fn test_spec() -> ResolvedSpec {
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
                mics: vec![
                    test_mic("test:audio:440", 4, Some(1)),
                    test_mic("test:audio:440", 4, Some(2)),
                ],
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
                options: test_mic("test:audio:880", 2, None),
                stream: false,
            }],
            output_dir: Some("/tmp/unused".into()),
            ..Default::default()
        }
        .validate_with_names(&HashMap::new())
        .unwrap()
    }

    /// Builds the device and role chains of `spec`, hangs a fakesink on every role tee,
    /// plays for a moment and returns the caps each role produced.
    fn role_caps(spec: &ResolvedSpec) -> (Pipeline, HashMap<Role, Caps>) {
        gstreamer::init().unwrap();
        let pipeline = Pipeline::new();
        let devices: HashMap<DeviceKey, DeviceChainHandles> = spec
            .devices
            .iter()
            .map(|d| (d.device.key, add_device_chain(&pipeline, d).unwrap()))
            .collect();
        let mut sinks = Vec::new();
        for role_ref in &spec.roles {
            let device = &devices[&role_ref.device];
            let role = add_role_chain(&pipeline, spec, role_ref.role, device).unwrap();
            let sink = gstreamer::ElementFactory::make("fakesink")
                .property("sync", false)
                .build()
                .unwrap();
            pipeline.add(&sink).unwrap();
            link_tee(&role.tee, &sink).unwrap();
            sinks.push((role_ref.role, sink));
        }
        pipeline.set_state(gstreamer::State::Playing).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let caps = sinks
            .iter()
            .map(|(role, sink)| {
                let pad = sink.static_pad("sink").unwrap();
                (*role, pad.current_caps().expect("role produced no caps"))
            })
            .collect();
        pipeline.set_state(gstreamer::State::Null).unwrap();
        (pipeline, caps)
    }

    fn field<'a, T: gstreamer::glib::value::FromValue<'a>>(caps: &'a Caps, name: &str) -> T {
        caps.structure(0).unwrap().get(name).unwrap()
    }

    #[test]
    fn device_and_role_chains_negotiate() {
        let spec = test_spec();
        let (pipeline, caps) = role_caps(&spec);

        let camera = &caps[&Role::Camera(1)];
        assert_eq!(field::<&str>(camera, "format"), "I420");
        assert_eq!(field::<i32>(camera, "width"), 320);

        let screen = &caps[&Role::Screen(1)];
        assert_eq!(field::<i32>(screen, "width"), 640);
        assert_eq!(field::<i32>(screen, "height"), 360);

        // Mix mics are picked to mono from the shared 4-channel device.
        for mic in [Role::Mic(1), Role::Mic(2)] {
            assert_eq!(field::<&str>(&caps[&mic], "format"), "F32LE");
            assert_eq!(field::<i32>(&caps[&mic], "rate"), 48000);
            assert_eq!(field::<i32>(&caps[&mic], "channels"), 1);
        }
        // An extra mic without a selected channel keeps its channels.
        assert_eq!(field::<i32>(&caps[&Role::Mic(3)], "channels"), 2);

        // One source for the shared interface, feeding two roles.
        let shared = spec.device_of(Role::Mic(1)).unwrap();
        assert_eq!(spec.device_of(Role::Mic(2)), Some(shared));
        let tee = pipeline.by_name(&format!("{shared}:tee")).unwrap();
        assert_eq!(tee.src_pads().len(), 2);
    }

    #[test]
    fn chain_elements_are_tagged() {
        let spec = test_spec();
        let (pipeline, _) = role_caps(&spec);
        for name in [
            "dev0:src",
            "dev0:tee",
            "mic-1:pick",
            "mic-2:rate",
            "mic-3:tee",
        ] {
            assert!(pipeline.by_name(name).is_some(), "missing {name}");
        }
    }

    fn mix_spec(mics: Vec<AudioPublishOptions>) -> ResolvedSpec {
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
                mics,
                stream: true,
            }),
            ..Default::default()
        }
        .validate_with_names(&HashMap::new())
        .unwrap()
    }

    /// Builds the mic devices, roles and L/R mix of `spec`, plays for a moment and
    /// returns the mixer caps and its interleaved samples.
    fn capture_mix(spec: &ResolvedSpec) -> (Caps, Vec<f32>) {
        capture_mix_sides(spec, true)
    }

    /// Like `capture_mix`, optionally leaving out the right input.
    fn capture_mix_sides(spec: &ResolvedSpec, with_right: bool) -> (Caps, Vec<f32>) {
        gstreamer::init().unwrap();
        let pipeline = Pipeline::new();
        let mut devices = HashMap::new();
        for device in spec
            .devices
            .iter()
            .filter(|d| d.device.kind == DeviceKind::Mic)
        {
            devices.insert(
                device.device.key,
                add_device_chain(&pipeline, device).unwrap(),
            );
        }
        let role = |n| {
            let device = &devices[&spec.device_of(Role::Mic(n)).unwrap()];
            add_role_chain(&pipeline, spec, Role::Mic(n), device).unwrap()
        };
        let (left, right) = (role(1), role(2));
        let mix = add_lr_mix(&pipeline, Some(&left), with_right.then_some(&right)).unwrap();

        let appsink = gstreamer_app::AppSink::builder().sync(false).build();
        pipeline.add(&appsink).unwrap();
        link_tee(&mix.tee, appsink.upcast_ref()).unwrap();
        pipeline.set_state(gstreamer::State::Playing).unwrap();

        let mut caps = None;
        let mut samples = Vec::new();
        while samples.len() < 2 * AUDIO_RATE as usize / 2 {
            let sample = appsink
                .try_pull_sample(gstreamer::ClockTime::from_seconds(2))
                .expect("mixer produced no audio");
            caps = sample.caps().map(|c| c.to_owned());
            let buffer = sample.buffer().unwrap().map_readable().unwrap();
            samples.extend(
                buffer
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b)),
            );
        }
        pipeline.set_state(gstreamer::State::Null).unwrap();
        (caps.unwrap(), samples)
    }

    /// Frequency of one channel of interleaved stereo, from its zero crossings.
    fn channel_freq(samples: &[f32], channel: usize) -> f64 {
        let values: Vec<f32> = samples.iter().skip(channel).step_by(2).copied().collect();
        let crossings = values
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        crossings as f64 / 2.0 / (values.len() as f64 / f64::from(AUDIO_RATE))
    }

    fn assert_near(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < expected * 0.05,
            "expected about {expected} Hz, got {actual:.1} Hz"
        );
    }

    #[test]
    fn lr_mix_places_each_mic_on_its_own_channel() {
        let spec = mix_spec(vec![
            test_mic("test:audio:440", 1, None),
            test_mic("test:audio:880", 1, None),
        ]);
        let (caps, samples) = capture_mix(&spec);
        assert_eq!(field::<i32>(&caps, "channels"), 2);
        assert_eq!(field::<&str>(&caps, "format"), "F32LE");
        assert_near(channel_freq(&samples, 0), 440.0);
        assert_near(channel_freq(&samples, 1), 880.0);
    }

    #[test]
    fn lr_mix_picks_channels_of_a_shared_interface() {
        let interface = "test:tones:440:880:1320:1760";
        let spec = mix_spec(vec![
            test_mic(interface, 4, Some(2)),
            test_mic(interface, 4, Some(4)),
        ]);
        let (_, samples) = capture_mix(&spec);
        assert_near(channel_freq(&samples, 0), 880.0);
        assert_near(channel_freq(&samples, 1), 1760.0);
    }

    #[test]
    fn mix_elements_are_tagged() {
        let spec = mix_spec(vec![
            test_mic("test:audio:440", 1, None),
            test_mic("test:audio:880", 1, None),
        ]);
        gstreamer::init().unwrap();
        let pipeline = Pipeline::new();
        let mut roles = Vec::new();
        for (n, device) in (1..).zip(spec.devices.iter().skip(1)) {
            let chain = add_device_chain(&pipeline, device).unwrap();
            roles.push(add_role_chain(&pipeline, &spec, Role::Mic(n), &chain).unwrap());
        }
        add_lr_mix(&pipeline, Some(&roles[0]), Some(&roles[1])).unwrap();
        for name in ["mix.lr:mixer", "mix.lr:to-l", "mix.lr:to-r", "mix.lr:tee"] {
            assert!(pipeline.by_name(name).is_some(), "missing {name}");
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("composite-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct Built {
        pipeline: Pipeline,
        paths: HashMap<OutputId, std::path::PathBuf>,
        sinks: HashMap<SinkId, StreamBranchHandles>,
    }

    /// Builds `spec` with `build_graph`; every device must open.
    fn build_all(spec: &ResolvedSpec) -> Built {
        let (built, failed) = build_with_failures(spec);
        assert!(failed.is_empty(), "devices failed to open: {failed:?}");
        built
    }

    fn build_with_failures(spec: &ResolvedSpec) -> (Built, Vec<(DeviceKey, CompositeError)>) {
        gstreamer::init().unwrap();
        let pipeline = Pipeline::new();
        let (graph, failed) = build_graph(&pipeline, spec).unwrap();
        let paths = graph
            .outputs
            .iter()
            .map(|(id, branch)| (*id, branch.path.clone()))
            .collect();
        let built = Built {
            pipeline,
            paths,
            sinks: graph.sinks,
        };
        (built, failed)
    }

    /// Plays for `seconds`, then stops with EOS (or, with `crash`, straight to NULL).
    fn run(pipeline: &Pipeline, seconds: u64, crash: bool) {
        pipeline.set_state(gstreamer::State::Playing).unwrap();
        std::thread::sleep(Duration::from_millis(seconds * 1000));
        if !crash {
            pipeline.send_event(gstreamer::event::Eos::new());
            let bus = pipeline.bus().unwrap();
            let msg = bus
                .timed_pop_filtered(
                    gstreamer::ClockTime::from_seconds(10),
                    &[gstreamer::MessageType::Eos, gstreamer::MessageType::Error],
                )
                .expect("no EOS after stopping");
            if let gstreamer::MessageView::Error(err) = msg.view() {
                panic!("pipeline error: {} ({:?})", err.error(), err.debug());
            }
        }
        pipeline.set_state(gstreamer::State::Null).unwrap();
    }

    /// Builds every layer, records for `seconds` and returns each file's path.
    fn record(
        spec: &ResolvedSpec,
        seconds: u64,
        crash: bool,
    ) -> HashMap<OutputId, std::path::PathBuf> {
        let built = build_all(spec);
        run(&built.pipeline, seconds, crash);
        built.paths
    }

    fn discover(path: &std::path::Path) -> gstreamer_pbutils::DiscovererInfo {
        let uri = gstreamer::glib::filename_to_uri(path, None).unwrap();
        gstreamer_pbutils::Discoverer::new(gstreamer::ClockTime::from_seconds(10))
            .unwrap()
            .discover_uri(&uri)
            .unwrap_or_else(|e| panic!("{} is not readable: {e}", path.display()))
    }

    fn audio_channels(info: &gstreamer_pbutils::DiscovererInfo) -> Vec<u32> {
        info.audio_streams().iter().map(|a| a.channels()).collect()
    }

    fn video_sizes(info: &gstreamer_pbutils::DiscovererInfo) -> Vec<(u32, u32)> {
        info.video_streams()
            .iter()
            .map(|v| (v.width(), v.height()))
            .collect()
    }

    fn full_spec(output_dir: std::path::PathBuf) -> ResolvedSpec {
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
                mics: vec![
                    test_mic("test:audio:440", 1, None),
                    test_mic("test:audio:880", 1, None),
                ],
                stream: false,
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
                options: test_mic("test:tones:440:880:1320:1760", 4, None),
                stream: false,
            }],
            output_dir: Some(output_dir),
            encoders: crate::composite::EncoderOptions {
                h264: crate::composite::H264Encoder::X264,
                ..Default::default()
            },
            ..Default::default()
        }
        .validate_with_names(&HashMap::new())
        .unwrap()
    }

    #[test]
    fn file_branches_write_every_output() {
        let dir = temp_dir("files");
        let paths = record(&full_spec(dir.clone()), 2, false);

        let av_mix = discover(&paths[&OutputId::AvMix]);
        assert_eq!(video_sizes(&av_mix), [(320, 240)]);
        assert_eq!(audio_channels(&av_mix), [2], "av-mix audio is L/R stereo");
        assert!(av_mix.duration().unwrap() >= gstreamer::ClockTime::from_mseconds(1500));

        assert_eq!(audio_channels(&discover(&paths[&OutputId::Mic(1)])), [1]);
        assert_eq!(audio_channels(&discover(&paths[&OutputId::Mic(2)])), [1]);
        assert_eq!(audio_channels(&discover(&paths[&OutputId::Mic(3)])), [4]);
        assert_eq!(
            video_sizes(&discover(&paths[&OutputId::Screen(1)])),
            [(640, 360)]
        );

        let names: Vec<String> = paths
            .values()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"av-mix.mp4".to_string()));
        assert!(names.contains(&"screen-1_test-screen.mp4".to_string()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn av_mix_with_one_mic_is_mono() {
        let dir = temp_dir("mono");
        let mut spec = CompositeStreamSpec {
            av_mix: Some(AvMixSpec {
                camera: VideoPublishOptions {
                    codec: "video/x-raw".into(),
                    device_id: "test:video".into(),
                    width: 320,
                    height: 240,
                    framerate: 30,
                    local_file_save_options: None,
                },
                mics: vec![test_mic("test:audio:440", 1, None)],
                stream: false,
            }),
            output_dir: Some(dir.clone()),
            ..Default::default()
        };
        spec.encoders.h264 = crate::composite::H264Encoder::X264;
        let paths = record(
            &spec.validate_with_names(&HashMap::new()).unwrap(),
            1,
            false,
        );
        assert_eq!(audio_channels(&discover(&paths[&OutputId::AvMix])), [1]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn default_encoder_records_video() {
        let dir = temp_dir("auto-encoder");
        let mut spec = full_spec(dir.clone());
        spec.encoders = crate::composite::EncoderOptions::default();
        gstreamer::init().unwrap();
        let encoder = pick_h264_encoder(&spec.encoders).unwrap();
        let paths = record(&spec, 2, false);
        let av_mix = discover(&paths[&OutputId::AvMix]);
        assert_eq!(video_sizes(&av_mix), [(320, 240)], "encoder {encoder}");
        assert!(av_mix.duration().unwrap() >= gstreamer::ClockTime::from_mseconds(1500));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn files_stay_readable_without_eos() {
        let dir = temp_dir("crash");
        let paths = record(&full_spec(dir.clone()), 3, true);
        let av_mix = discover(&paths[&OutputId::AvMix]);
        assert_eq!(video_sizes(&av_mix), [(320, 240)]);
        assert_eq!(audio_channels(&av_mix), [2]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Everything recorded and streamed: AV mix (2 mics), screen, an extra camera and an
    /// extra 4-channel mic.
    fn streaming_spec(output_dir: Option<std::path::PathBuf>) -> CompositeStreamSpec {
        let camera = |id: &str| VideoPublishOptions {
            codec: "video/x-raw".into(),
            device_id: id.into(),
            width: 320,
            height: 240,
            framerate: 30,
            local_file_save_options: None,
        };
        CompositeStreamSpec {
            av_mix: Some(AvMixSpec {
                camera: camera("test:video"),
                mics: vec![
                    test_mic("test:audio:440", 1, None),
                    test_mic("test:audio:880", 1, None),
                ],
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
                stream: true,
            }),
            cameras: vec![Output {
                options: camera("test:video:2"),
                stream: true,
            }],
            mics: vec![Output {
                options: test_mic("test:tones:440:880:1320:1760", 4, None),
                stream: true,
            }],
            output_dir,
            encoders: crate::composite::EncoderOptions {
                h264: crate::composite::H264Encoder::X264,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn sink_caps(branch: &StreamBranchHandles) -> Caps {
        branch
            .appsink
            .static_pad("sink")
            .unwrap()
            .current_caps()
            .unwrap_or_else(|| panic!("{} negotiated no caps", branch.sink))
    }

    /// Plays `built` for a moment and checks every stream delivered frames in the
    /// format LiveKit expects, then stops with EOS and checks every stream closed.
    fn assert_streams(built: Built, seconds: u64) {
        let mut frames: Vec<_> = built
            .sinks
            .values()
            .map(|b| (b.sink, b.frames.subscribe(), b.closed.subscribe()))
            .collect();
        built.pipeline.set_state(gstreamer::State::Playing).unwrap();
        std::thread::sleep(Duration::from_millis(seconds * 1000));

        for branch in built.sinks.values() {
            let caps = sink_caps(branch);
            match branch.info.kind {
                SinkKind::Video { .. } => {
                    assert_eq!(field::<&str>(&caps, "format"), "I420", "{}", branch.sink);
                    assert_eq!(field::<i32>(&caps, "width"), 640, "{}", branch.sink);
                    assert_eq!(field::<i32>(&caps, "height"), 480, "{}", branch.sink);
                }
                SinkKind::Audio { .. } => {
                    assert_eq!(field::<&str>(&caps, "format"), "S16LE", "{}", branch.sink);
                    assert_eq!(field::<i32>(&caps, "rate"), 48000, "{}", branch.sink);
                    assert_eq!(field::<i32>(&caps, "channels"), 1, "{}", branch.sink);
                }
            }
        }
        for (sink, rx, _) in &mut frames {
            use tokio::sync::broadcast::error::TryRecvError;
            match rx.try_recv() {
                Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                Err(e) => panic!("{sink} delivered no frames: {e:?}"),
            }
        }

        built.pipeline.send_event(gstreamer::event::Eos::new());
        let bus = built.pipeline.bus().unwrap();
        let msg = bus
            .timed_pop_filtered(
                gstreamer::ClockTime::from_seconds(10),
                &[gstreamer::MessageType::Eos, gstreamer::MessageType::Error],
            )
            .expect("no EOS after stopping");
        if let gstreamer::MessageView::Error(err) = msg.view() {
            panic!("pipeline error: {} ({:?})", err.error(), err.debug());
        }
        for (sink, _, closed) in &mut frames {
            assert!(
                closed.try_recv().is_ok(),
                "{sink} did not signal closed on EOS"
            );
        }
        built.pipeline.set_state(gstreamer::State::Null).unwrap();
    }

    #[test]
    fn streams_deliver_livekit_formats_while_recording() {
        let dir = temp_dir("streams");
        let spec = streaming_spec(Some(dir.clone()))
            .validate_with_names(&HashMap::new())
            .unwrap();
        let built = build_all(&spec);
        let mut ids: Vec<SinkId> = built.sinks.keys().copied().collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                SinkId::AvMixVideo,
                SinkId::AvMixAudio,
                SinkId::Camera(2),
                SinkId::Mic(3),
                SinkId::Screen(1)
            ]
        );
        // Noise suppression runs on every audio stream, and only there.
        for (id, branch) in &built.sinks {
            let is_audio = matches!(id, SinkId::AvMixAudio | SinkId::Mic(_));
            assert_eq!(branch.denoised, is_audio && rnnoise_available(), "{id}");
            let rnnoise = built.pipeline.by_name(&format!("sink.{id}:rnnoise"));
            assert_eq!(rnnoise.is_some(), branch.denoised, "{id}");
        }
        let paths = built.paths.clone();
        assert_streams(built, 2);

        // Files still finalize with streaming running alongside.
        let av_mix = discover(&paths[&OutputId::AvMix]);
        assert_eq!(audio_channels(&av_mix), [2]);
        assert_eq!(video_sizes(&av_mix), [(320, 240)]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stream_only_builds_no_files() {
        let spec = streaming_spec(None)
            .validate_with_names(&HashMap::new())
            .unwrap();
        let built = build_all(&spec);
        assert!(built.paths.is_empty());
        assert!(built.pipeline.by_name("file.av-mix:mux").is_none());
        assert_streams(built, 1);
    }

    #[test]
    fn noise_suppression_can_be_turned_off() {
        let mut spec = streaming_spec(None);
        spec.stream_audio.noise_suppression = false;
        let built = build_all(&spec.validate_with_names(&HashMap::new()).unwrap());
        assert!(built.sinks.values().all(|b| !b.denoised));
        assert!(built
            .pipeline
            .by_name("sink.av-mix-audio:rnnoise")
            .is_none());
    }

    #[test]
    fn lr_mix_with_one_side_leaves_the_other_silent() {
        let spec = mix_spec(vec![
            test_mic("test:audio:440", 1, None),
            test_mic("test:audio:880", 1, None),
        ]);
        let (caps, samples) = capture_mix_sides(&spec, false);
        assert_eq!(field::<i32>(&caps, "channels"), 2);
        assert_near(channel_freq(&samples, 0), 440.0);
        let right_peak = samples
            .iter()
            .skip(1)
            .step_by(2)
            .fold(0f32, |peak, v| peak.max(v.abs()));
        assert_eq!(right_peak, 0.0, "right channel should be silent");
    }

    fn camera_options(id: &str) -> VideoPublishOptions {
        VideoPublishOptions {
            codec: "video/x-raw".into(),
            device_id: id.into(),
            width: 320,
            height: 240,
            framerate: 30,
            local_file_save_options: None,
        }
    }

    /// AV mix with mic-2 missing, plus a missing extra camera and a working extra mic.
    fn partly_missing_spec(output_dir: std::path::PathBuf) -> CompositeStreamSpec {
        CompositeStreamSpec {
            av_mix: Some(AvMixSpec {
                camera: camera_options("test:video"),
                mics: vec![
                    test_mic("test:audio:440", 1, None),
                    test_mic("test:missing", 1, None),
                ],
                stream: true,
            }),
            cameras: vec![Output {
                options: camera_options("test:missing:camera"),
                stream: true,
            }],
            mics: vec![Output {
                options: test_mic("test:audio:880", 1, None),
                stream: true,
            }],
            output_dir: Some(output_dir),
            encoders: crate::composite::EncoderOptions {
                h264: crate::composite::H264Encoder::X264,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn best_effort_leaves_out_missing_devices() {
        let dir = temp_dir("best-effort");
        let spec = partly_missing_spec(dir.clone())
            .validate_with_names(&HashMap::new())
            .unwrap();
        let (built, failed) = build_with_failures(&spec);

        let mut failed_keys: Vec<DeviceKey> = failed.iter().map(|(key, _)| *key).collect();
        failed_keys.sort();
        let mut expected = vec![
            spec.device_of(Role::Mic(2)).unwrap(),
            spec.device_of(Role::Camera(2)).unwrap(),
        ];
        expected.sort();
        assert_eq!(failed_keys, expected);

        let mut outputs: Vec<OutputId> = built.paths.keys().copied().collect();
        outputs.sort();
        assert_eq!(
            outputs,
            [OutputId::AvMix, OutputId::Mic(1), OutputId::Mic(3)]
        );
        let mut sinks: Vec<SinkId> = built.sinks.keys().copied().collect();
        sinks.sort();
        assert_eq!(
            sinks,
            [SinkId::AvMixVideo, SinkId::AvMixAudio, SinkId::Mic(3)]
        );

        // Nothing of the failed devices is left in the pipeline; the mix has only L.
        for key in &failed_keys {
            let prefix = format!("{key}:");
            assert!(built
                .pipeline
                .children()
                .iter()
                .all(|e| !e.name().starts_with(&prefix)));
        }
        assert!(built.pipeline.by_name("mix.lr:to-l").is_some());
        assert!(built.pipeline.by_name("mix.lr:to-r").is_none());

        let paths = built.paths.clone();
        run(&built.pipeline, 2, false);
        let av_mix = discover(&paths[&OutputId::AvMix]);
        assert_eq!(video_sizes(&av_mix), [(320, 240)]);
        assert_eq!(audio_channels(&av_mix), [2], "keeps L/R with R silent");
        assert_eq!(audio_channels(&discover(&paths[&OutputId::Mic(3)])), [1]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn av_mix_without_its_camera_records_audio_only() {
        let dir = temp_dir("no-camera");
        let mut spec = partly_missing_spec(dir.clone());
        if let Some(mix) = spec.av_mix.as_mut() {
            mix.camera = camera_options("test:missing");
            mix.mics[1] = test_mic("test:audio:660", 1, None);
        }
        let spec = spec.validate_with_names(&HashMap::new()).unwrap();
        let (built, _) = build_with_failures(&spec);
        assert!(!built.sinks.contains_key(&SinkId::AvMixVideo));
        let paths = built.paths.clone();
        run(&built.pipeline, 2, false);
        let av_mix = discover(&paths[&OutputId::AvMix]);
        assert!(video_sizes(&av_mix).is_empty());
        assert_eq!(audio_channels(&av_mix), [2]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn all_or_nothing_fails_on_a_missing_device() {
        let mut spec = partly_missing_spec(std::env::temp_dir());
        spec.start_policy = StartPolicy::AllOrNothing;
        gstreamer::init().unwrap();
        let error = build_graph(
            &Pipeline::new(),
            &spec.validate_with_names(&HashMap::new()).unwrap(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("test:missing"), "{error}");
    }

    #[test]
    fn fails_when_nothing_can_be_built() {
        let spec = CompositeStreamSpec {
            mics: vec![Output {
                options: test_mic("test:missing", 1, None),
                stream: true,
            }],
            ..Default::default()
        };
        gstreamer::init().unwrap();
        let error = build_graph(
            &Pipeline::new(),
            &spec.validate_with_names(&HashMap::new()).unwrap(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("test:missing"), "{error}");
    }

    #[test]
    fn placement_matrices() {
        assert_eq!(lr_placement_matrix(true), vec![vec![1.0], vec![0.0]]);
        assert_eq!(lr_placement_matrix(false), vec![vec![0.0], vec![1.0]]);
    }
}
