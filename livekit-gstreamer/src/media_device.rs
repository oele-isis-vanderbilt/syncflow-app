use gstreamer::{prelude::*, Buffer};
use gstreamer_app::AppSink;
use serde::de;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use thiserror::Error;
use tokio::sync::broadcast;

use crate::utils::random_string;
use crate::utils::system_time_nanos;
use crate::{get_device_capabilities, AudioPublishOptions, VideoPublishOptions};
use crate::{get_gst_device, get_monitor};

#[cfg(target_os = "macos")]
const SUPPORTED_VIDEO_CODECS: [&str; 3] = ["video/x-h264", "image/jpeg", "video/x-raw"];

#[cfg(not(target_os = "macos"))]
const SUPPORTED_VIDEO_CODECS: [&str; 3] = ["video/x-h264", "image/jpeg", "video/x-raw"];

const SUPPORTED_AUDIO_CODECS: [&str; 1] = ["audio/x-raw"];
const VIDEO_FRAME_FORMAT: &str = "I420";

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
struct FileSinkTiming {
    start_time: Option<i64>,
    end_time: Option<i64>,
}

/// A struct representing a GStreamer device
/// This implementation assumes that GStreamer is initialized elsewhere
#[derive(Debug, Clone)]
pub struct GstMediaDevice {
    pub display_name: String,
    #[allow(dead_code)]
    pub device_class: String,
    pub device_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingMetadata {
    pub filename: String,
    pub parent_dir: String,
    pub source: String,
    pub media_type: String,
    start_time: Option<i64>,
    end_time: Option<i64>,
    pub codec: String,
    pub audio_channel: Option<i32>,
    pub device_name: Option<String>,
    pub base_time: Option<i64>,
    pub first_pts: Option<i64>,
}

impl RecordingMetadata {
    pub fn new(
        filename: String,
        parent_dir: String,
        source: String,
        media_type: String,
        codec: String,
        audio_channel: Option<i32>,
        device_name: Option<String>,
    ) -> Self {
        RecordingMetadata {
            filename,
            parent_dir,
            source,
            media_type,
            start_time: None,
            end_time: None,
            codec,
            audio_channel,
            device_name: device_name,
            base_time: None,
            first_pts: None,
        }
    }

    pub fn set_base_time(&mut self, base_time: i64) {
        self.base_time = Some(base_time);
    }

    pub fn set_first_pts(&mut self, first_pts: i64) {
        self.first_pts = Some(first_pts);
    }

    pub fn set_start_time(&mut self, time: i64) {
        self.start_time = Some(time);
    }

    pub fn set_end_time(&mut self, time: i64) {
        self.end_time = Some(time);
    }

    pub fn start_time(&self) -> Option<i64> {
        self.start_time
    }

    pub fn end_time(&self) -> Option<i64> {
        self.end_time
    }

    pub fn write_success(&self) -> Result<bool, GStreamerError> {
        let parent_dir = PathBuf::from(&self.parent_dir);

        let string_content = serde_json::to_string(&self).map_err(|e| {
            GStreamerError::PipelineError(format!("Failed to serialize metadata: {}", e))
        })?;

        let metadata_file = format!("{}.json", self.filename);

        std::fs::write(parent_dir.join(metadata_file), string_content).map_err(|e| {
            GStreamerError::PipelineError(format!("Failed to write metadata: {}", e))
        })?;

        Ok(true)
    }

    pub fn write_error(&self, error: &str) -> Result<bool, GStreamerError> {
        let parent_dir = PathBuf::from(&self.parent_dir);

        let error_object = serde_json::json!({
            "error": error,
            "filename": self.filename,
            "parent_dir": self.parent_dir,
            "source": self.source,
            "media_type": self.media_type,
            "codec": self.codec,
            "audio_channel": self.audio_channel,
        });

        let string_content = serde_json::to_string(&error_object).map_err(|e| {
            GStreamerError::PipelineError(format!("Failed to serialize error metadata: {}", e))
        })?;

        let metadata_file = format!("{}.error.json", self.filename);

        std::fs::write(parent_dir.join(metadata_file), string_content).map_err(|e| {
            GStreamerError::PipelineError(format!("Failed to write metadata: {}", e))
        })?;

        Ok(true)
    }
}

pub async fn preroll_pipeline(pipeline: &gstreamer::Pipeline) -> Result<(), GStreamerError> {
    pipeline.set_state(gstreamer::State::Paused).map_err(|_| {
        GStreamerError::PipelineError("Failed to set pipeline to Paused state".to_string())
    })?;

    let pipeline_clone = pipeline.clone();
    tokio::task::spawn_blocking(move || {
        let bus = pipeline_clone.bus().unwrap();
        for msg in bus.iter_timed(gstreamer::ClockTime::from_seconds(10)) {
            use gstreamer::MessageView;
            match msg.view() {
                MessageView::StateChanged(e) => {
                    if e.src()
                        .map(|s| s == pipeline_clone.upcast_ref::<gstreamer::Object>())
                        .unwrap_or(false)
                        && e.current() == gstreamer::State::Paused
                    {
                        break;
                    }
                }
                MessageView::AsyncDone(e) => {
                    if e.src()
                        .map(|s| s == pipeline_clone.upcast_ref::<gstreamer::Object>())
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
                MessageView::Error(err) => {
                    return Err(GStreamerError::PipelineError(format!(
                        "Pipeline error during preroll: {}",
                        err.error().message()
                    )));
                }
                _ => (),
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| GStreamerError::PipelineError("spawn_blocking panicked".into()))?
}

pub fn set_pipeline_clock(
    pipeline: &gstreamer::Pipeline,
    clock: &gstreamer::Clock,
    base_time: gstreamer::ClockTime,
    metadata: Option<&mut RecordingMetadata>,
) -> Result<(), GStreamerError> {
    pipeline
        .set_clock(Some(clock))
        .map_err(|_| GStreamerError::PipelineError("Failed to set clock".to_string()))?;
    pipeline.set_base_time(base_time);

    // FixMe: This never works with wasapi2 on Windows and Macos, need to investigate why
    // Only enable for linux
    #[cfg(target_os = "linux")]
    pipeline.set_start_time(gstreamer::ClockTime::NONE);

    if let Some(meta) = metadata {
        meta.set_base_time(base_time.nseconds() as i64);
    }
    Ok(())
}

pub fn play_pipeline(
    pipeline: &gstreamer::Pipeline,
    metadata: Option<&mut RecordingMetadata>,
) -> Result<(), GStreamerError> {
    pipeline
        .set_state(gstreamer::State::Playing)
        .map_err(|_| GStreamerError::PipelineError("Failed to set Playing".to_string()))?;

    if let Some(meta) = metadata {
        meta.set_start_time(system_time_nanos());
    }
    Ok(())
}

pub async fn run_bus_loop(
    pipeline: gstreamer::Pipeline,
    tx: broadcast::Sender<()>,
    mut recording_metadata: Option<RecordingMetadata>,
) -> Result<(), GStreamerError> {
    let pipeline_clone = pipeline.clone();
    let timing = Arc::new(Mutex::new(FileSinkTiming::default()));

    tokio::task::spawn_blocking(
        move || -> Result<Option<RecordingMetadata>, GStreamerError> {
            let bus = pipeline_clone.bus().unwrap();
            let timing_clone = timing.clone();
            for msg in bus.iter_timed(gstreamer::ClockTime::NONE) {
                use gstreamer::MessageView;
                match msg.view() {
                    MessageView::Eos(..) => {
                        if let Some(metadata) = recording_metadata.as_mut() {
                            let t = timing_clone.lock().unwrap();
                            if let Some(start) = t.start_time {
                                metadata.set_start_time(start);
                                metadata.set_first_pts(start);
                            }
                            if let Some(end) = t.end_time {
                                metadata.set_end_time(end);
                            }
                        }
                        break;
                    }
                    MessageView::Error(err) => {
                        if let Some(metadata) = recording_metadata.as_mut() {
                            let _ = metadata
                                .write_error(&format!("Pipeline error: {}", err.error().message()));
                        }
                        break;
                    }
                    MessageView::StateChanged(e) => {
                        if e.current() == gstreamer::State::Null {
                            break;
                        }
                    }
                    _ => (),
                }
            }

            pipeline_clone
                .set_state(gstreamer::State::Null)
                .map_err(|_| GStreamerError::PipelineError("Failed to set Null".to_string()))?;

            Ok(recording_metadata)
        },
    )
    .await
    .map_err(|_| GStreamerError::PipelineError("spawn_blocking panicked".to_string()))??
    .map(|metadata| metadata.write_success());

    tx.send(()).ok();
    Ok(())
}

// In media_device.rs - add this to GstMediaDevice impl

/// Result of adding video to a pipeline - holds references needed for file branch
pub struct VideoChainHandles {
    pub tee: gstreamer::Element,
    pub appsink_tx: Arc<broadcast::Sender<Arc<Buffer>>>,
}

impl GstMediaDevice {
    pub fn add_video_to_pipeline(
        &self,
        pipeline: &gstreamer::Pipeline,
        options: &VideoPublishOptions,
        stream_tx: Arc<broadcast::Sender<Arc<Buffer>>>,
    ) -> Result<VideoChainHandles, GStreamerError> {
        if self.device_class == "Audio/Source" {
            return Err(GStreamerError::PipelineError(
                "Device is an audio source".into(),
            ));
        }

        if !SUPPORTED_VIDEO_CODECS.contains(&options.codec.as_str()) {
            return Err(GStreamerError::PipelineError(format!(
                "Unsupported codec {}",
                options.codec
            )));
        }

        let can_support = self.supports_video(
            &options.codec,
            options.width,
            options.height,
            options.framerate,
        );
        if !can_support {
            return Err(GStreamerError::PipelineError(
                "Device does not support requested configuration".into(),
            ));
        }

        // Source element
        let source = self.get_video_element()?;

        eprintln!(
            "[DEBUG] Video source created: {} (factory: {:?})",
            source.name(),
            source.factory().map(|f| f.name().to_string())
        );

        // Source caps — request specific format from camera
        let source_caps = gstreamer::Caps::builder(&options.codec)
            .field("width", options.width)
            .field("height", options.height)
            .field("framerate", gstreamer::Fraction::new(options.framerate, 1))
            .build();
        let source_capsfilter = Self::make_capsfilter(&source_caps)?;

        // Decode chain if needed (jpeg → jpegdec, h264 → h264parse + avdec_h264)
        let decode_elements = Self::build_video_decode_chain(&options.codec)?;

        // Convert to I420
        let convert = Self::make_element("videoconvert")?;
        let scale = Self::make_element("videoscale")?;

        let i420_caps = gstreamer::Caps::builder("video/x-raw")
            .field("format", VIDEO_FRAME_FORMAT)
            .field("width", options.width)
            .field("height", options.height)
            .field("framerate", gstreamer::Fraction::new(options.framerate, 1))
            .field("pixel-aspect-ratio", gstreamer::Fraction::new(1, 1))
            .build();
        let i420_filter = Self::make_capsfilter(&i420_caps)?;

        // Tee — fan out to stream branch and (later) file branch
        let tee = Self::make_element("tee")?;

        // Stream branch: scale down for appsink
        let queue_stream = Self::make_element("queue")?;
        queue_stream.set_property_from_str("leaky", "downstream");

        let stream_scale = Self::make_element("videoscale")?;

        let stream_caps = gstreamer::Caps::builder("video/x-raw")
            .field("width", 640)
            .field("height", 480)
            .field("framerate", gstreamer::Fraction::new(options.framerate, 1))
            .field("format", VIDEO_FRAME_FORMAT)
            .build();
        let stream_capsfilter = Self::make_capsfilter(&stream_caps)?;

        let appsink = self.broadcast_appsink(stream_tx.clone(), Some(&stream_caps))?;

        // Add all to pipeline
        let mut elements: Vec<&gstreamer::Element> = vec![&source, &source_capsfilter];
        for el in &decode_elements {
            elements.push(el);
        }
        elements.extend_from_slice(&[
            &convert,
            &scale,
            &i420_filter,
            &tee,
            &queue_stream,
            &stream_scale,
            &stream_capsfilter,
            appsink.upcast_ref(),
        ]);

        pipeline.add_many(elements.as_slice()).map_err(|_| {
            GStreamerError::PipelineError("Failed to add video elements to pipeline".into())
        })?;

        // Link source chain: source → caps → [decode] → convert → scale → i420 → tee
        let mut chain: Vec<&gstreamer::Element> = vec![&source, &source_capsfilter];
        for el in &decode_elements {
            chain.push(el);
        }
        chain.extend_from_slice(&[&convert, &scale, &i420_filter, &tee]);

        gstreamer::Element::link_many(chain).map_err(|_| {
            GStreamerError::PipelineError("Failed to link video source chain".into())
        })?;

        // Link tee → stream branch
        let tee_pad = tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request tee pad for video stream".into())
        })?;
        let queue_sink = queue_stream.static_pad("sink").ok_or_else(|| {
            GStreamerError::PipelineError("Video stream queue has no sink pad".into())
        })?;
        tee_pad.link(&queue_sink).map_err(|_| {
            GStreamerError::PipelineError("Failed to link video tee to stream branch".into())
        })?;

        gstreamer::Element::link_many([
            &queue_stream,
            &stream_scale,
            &stream_capsfilter,
            appsink.upcast_ref(),
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link video stream branch".into()))?;

        Ok(VideoChainHandles {
            tee,
            appsink_tx: stream_tx,
        })
    }

    // Helper methods
    fn make_element(factory_name: &str) -> Result<gstreamer::Element, GStreamerError> {
        gstreamer::ElementFactory::make(factory_name)
            .name(random_string(factory_name))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError(format!("Failed to create {}", factory_name))
            })
    }

    fn make_capsfilter(caps: &gstreamer::Caps) -> Result<gstreamer::Element, GStreamerError> {
        let filter = Self::make_element("capsfilter")?;
        filter.set_property("caps", caps);
        Ok(filter)
    }

    fn build_video_decode_chain(codec: &str) -> Result<Vec<gstreamer::Element>, GStreamerError> {
        match codec {
            "image/jpeg" => Ok(vec![Self::make_element("jpegdec")?]),
            "video/x-h264" => Ok(vec![
                Self::make_element("h264parse")?,
                Self::make_element("avdec_h264")?,
            ]),
            "video/x-raw" => Ok(vec![]),
            _ => Err(GStreamerError::PipelineError(format!(
                "Unsupported video codec: {}",
                codec
            ))),
        }
    }
}

pub struct AudioChainHandles {
    pub tee: gstreamer::Element,
    pub appsink_tx: Arc<broadcast::Sender<Arc<Buffer>>>,
    pub num_channels: i32,
}

impl GstMediaDevice {
    /// Add one or more audio sources to a pipeline, mixed into a single stream.
    /// Returns handles to the tee after the mixer output.
    pub fn add_audio_to_pipeline(
        devices: &[(&GstMediaDevice, &AudioPublishOptions)],
        pipeline: &gstreamer::Pipeline,
        stream_tx: Arc<broadcast::Sender<Arc<Buffer>>>,
    ) -> Result<AudioChainHandles, GStreamerError> {
        if devices.is_empty() {
            return Err(GStreamerError::PipelineError(
                "No audio devices provided".into(),
            ));
        }

        let output_rate = devices[0].1.framerate;

        let output_caps = gstreamer::Caps::builder("audio/x-raw")
            .field("format", "S16LE")
            .field("channels", 1i32)
            .field("rate", output_rate)
            .field("layout", "interleaved")
            .build();

        let output_element: gstreamer::Element = if devices.len() == 1 {
            // Single mic — direct chain, no mixer
            let (device, _options) = &devices[0];
            let source = device.get_audio_element()?;
            let convert = Self::make_element("audioconvert")?;
            let resample = Self::make_element("audioresample")?;
            let capsfilter = Self::make_capsfilter(&output_caps)?;

            pipeline
                .add_many([&source, &convert, &resample, &capsfilter])
                .map_err(|_| {
                    GStreamerError::PipelineError("Failed to add audio elements".into())
                })?;

            gstreamer::Element::link_many([&source, &convert, &resample, &capsfilter]).map_err(
                |_| GStreamerError::PipelineError("Failed to link audio source chain".into()),
            )?;

            capsfilter
        } else {
            // Multiple mics — use audiomixer
            let mixer = Self::make_element("audiomixer")?;
            pipeline
                .add(&mixer)
                .map_err(|_| GStreamerError::PipelineError("Failed to add audiomixer".into()))?;

            for (i, (device, _options)) in devices.iter().enumerate() {
                let source = device.get_audio_element()?;
                let convert = Self::make_element("audioconvert")?;
                let resample = Self::make_element("audioresample")?;

                let mic_caps = gstreamer::Caps::builder("audio/x-raw")
                    .field("format", "S16LE")
                    .field("channels", 1i32)
                    .field("rate", output_rate)
                    .build();
                let capsfilter = Self::make_capsfilter(&mic_caps)?;

                pipeline
                    .add_many([&source, &convert, &resample, &capsfilter])
                    .map_err(|_| {
                        GStreamerError::PipelineError(format!(
                            "Failed to add mic {} elements",
                            i + 1
                        ))
                    })?;

                gstreamer::Element::link_many([&source, &convert, &resample, &capsfilter])
                    .map_err(|_| {
                        GStreamerError::PipelineError(format!("Failed to link mic {} chain", i + 1))
                    })?;

                let mixer_pad = mixer.request_pad_simple("sink_%u").ok_or_else(|| {
                    GStreamerError::PipelineError(format!(
                        "Failed to request mixer pad for mic {}",
                        i + 1
                    ))
                })?;
                // Reduce volume to prevent clipping when summing
                mixer_pad.set_property("volume", 0.5f64);

                let capsfilter_src = capsfilter.static_pad("src").unwrap();
                capsfilter_src.link(&mixer_pad).map_err(|_| {
                    GStreamerError::PipelineError(format!("Failed to link mic {} to mixer", i + 1))
                })?;
            }

            let output_filter = Self::make_capsfilter(&output_caps)?;
            pipeline.add(&output_filter).map_err(|_| {
                GStreamerError::PipelineError("Failed to add output capsfilter".into())
            })?;
            mixer.link(&output_filter).map_err(|_| {
                GStreamerError::PipelineError("Failed to link mixer to output".into())
            })?;

            output_filter
        };

        // Tee
        let tee = Self::make_element("tee")?;
        pipeline
            .add(&tee)
            .map_err(|_| GStreamerError::PipelineError("Failed to add audio tee".into()))?;
        output_element
            .link(&tee)
            .map_err(|_| GStreamerError::PipelineError("Failed to link output to tee".into()))?;

        // Stream branch: tee → queue → appsink
        let queue_stream = Self::make_element("queue")?;
        let appsink = devices[0]
            .0
            .broadcast_appsink(stream_tx.clone(), Some(&output_caps))?;

        pipeline
            .add_many([&queue_stream, appsink.upcast_ref()])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add audio stream elements".into())
            })?;

        let tee_pad = tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request audio tee pad".into())
        })?;
        let queue_sink = queue_stream
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Audio queue has no sink pad".into()))?;
        tee_pad.link(&queue_sink).map_err(|_| {
            GStreamerError::PipelineError("Failed to link audio tee to stream".into())
        })?;

        gstreamer::Element::link_many([&queue_stream, appsink.upcast_ref()])
            .map_err(|_| GStreamerError::PipelineError("Failed to link audio appsink".into()))?;

        Ok(AudioChainHandles {
            tee,
            appsink_tx: stream_tx,
            num_channels: 1,
        })
    }
}

// In media_device.rs

pub struct FileBranchHandles {
    pub video_mux_pad: gstreamer::Pad,
    pub audio_mux_pad: gstreamer::Pad,
    pub muxer: gstreamer::Element,
}

impl GstMediaDevice {
    pub fn add_av_file_branch(
        pipeline: &gstreamer::Pipeline,
        video_tee: &gstreamer::Element,
        audio_tee: &gstreamer::Element,
        path: &str,
        video_framerate: i32,
    ) -> Result<FileBranchHandles, GStreamerError> {
        // ===== VIDEO FILE CHAIN =====
        let vq = Self::make_element("queue")?;
        vq.set_property_from_str("leaky", "downstream");

        let vrate = Self::make_element("videorate")?;
        vrate.set_property("max-rate", 30i32);

        let vconvert = Self::make_element("videoconvert")?;

        let vencoder = Self::make_element("x264enc")?;
        vencoder.set_property("bitrate", 3000u32);
        vencoder.set_property_from_str("speed-preset", "ultrafast");

        let vparser = Self::make_element("h264parse")?;

        // ===== AUDIO FILE CHAIN =====
        let aq = Self::make_element("queue")?;

        let aconvert = Self::make_element("audioconvert")?;
        let aresample = Self::make_element("audioresample")?;

        let arate = Self::make_element("audiorate")?;
        arate.set_property("tolerance", 40000000u64);

        let aencoder = Self::make_element("avenc_aac")?;
        aencoder.set_property("bitrate", 128000i32);

        let aparser = Self::make_element("aacparse")?;

        // ===== MUXER + FILESINK =====
        let muxer = Self::make_element("mp4mux")?;
        muxer.set_property_from_str("faststart", "true");

        let filesink = Self::make_element("filesink")?;
        filesink.set_property("location", path);
        filesink.set_property("sync", false);

        // Add all to pipeline
        pipeline
            .add_many([
                &vq, &vrate, &vconvert, &vencoder, &vparser, &aq, &aconvert, &aresample, &arate,
                &aencoder, &aparser, &muxer, &filesink,
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add file branch elements".into())
            })?;

        // Link video file chain (up to parser)
        gstreamer::Element::link_many([&vq, &vrate, &vconvert, &vencoder, &vparser])
            .map_err(|_| GStreamerError::PipelineError("Failed to link video file chain".into()))?;

        // Link audio file chain (up to parser)
        gstreamer::Element::link_many([&aq, &aconvert, &aresample, &arate, &aencoder, &aparser])
            .map_err(|_| GStreamerError::PipelineError("Failed to link audio file chain".into()))?;

        // Request muxer pads explicitly — probe them for timing
        let video_mux_pad = muxer.request_pad_simple("video_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request video mux pad".into())
        })?;

        let audio_mux_pad = muxer.request_pad_simple("audio_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request audio mux pad".into())
        })?;

        // Link parsers to muxer pads
        let vparser_src = vparser
            .static_pad("src")
            .ok_or_else(|| GStreamerError::PipelineError("Video parser has no src pad".into()))?;
        vparser_src.link(&video_mux_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link video parser to muxer".into())
        })?;

        let aparser_src = aparser
            .static_pad("src")
            .ok_or_else(|| GStreamerError::PipelineError("Audio parser has no src pad".into()))?;
        aparser_src.link(&audio_mux_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link audio parser to muxer".into())
        })?;

        // Link muxer to filesink
        muxer.link(&filesink).map_err(|_| {
            GStreamerError::PipelineError("Failed to link muxer to filesink".into())
        })?;

        // Connect video tee to video file chain
        let vtee_pad = video_tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request video tee pad".into())
        })?;
        let vq_sink = vq
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Video queue has no sink pad".into()))?;
        vtee_pad.link(&vq_sink).map_err(|_| {
            GStreamerError::PipelineError("Failed to link video tee to file branch".into())
        })?;

        // Connect audio tee to audio file chain
        let atee_pad = audio_tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request audio tee pad".into())
        })?;
        let aq_sink = aq
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Audio queue has no sink pad".into()))?;
        atee_pad.link(&aq_sink).map_err(|_| {
            GStreamerError::PipelineError("Failed to link audio tee to file branch".into())
        })?;

        // // After linking parsers to muxer pads, add debug probes:
        // let vparser_src_probe = vparser.static_pad("src").unwrap();
        // // vparser_src_probe.add_probe(gstreamer::PadProbeType::BUFFER, |_pad, _info| {
        // //     eprintln!("[DEBUG] Video buffer reaching muxer");
        // //     gstreamer::PadProbeReturn::Ok
        // // });

        // let aparser_src_probe = aparser.static_pad("src").unwrap();
        // aparser_src_probe.add_probe(gstreamer::PadProbeType::BUFFER, |_pad, _info| {
        //     eprintln!("[DEBUG] Audio buffer reaching muxer");
        //     gstreamer::PadProbeReturn::Ok
        // });

        Ok(FileBranchHandles {
            video_mux_pad,
            audio_mux_pad,
            muxer,
        })
    }
}

impl GstMediaDevice {
    pub fn from_device_path(path: &str) -> Result<Self, GStreamerError> {
        let device = get_gst_device(path);
        let device =
            device.ok_or_else(|| GStreamerError::DeviceError("No device found".to_string()))?;
        let display_name: String = device.display_name().into();

        let device = GstMediaDevice {
            display_name,
            device_class: device.device_class().into(),
            device_path: path.into(),
        };
        Ok(device)
    }

    pub fn from_screen_id_or_name(screen_id_or_name: &str) -> Result<Self, GStreamerError> {
        let monitor = {
            #[cfg(target_os = "windows")]
            {
                let (monitor, _) = get_monitor(screen_id_or_name)
                    .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?;
                monitor
            }
            #[cfg(target_os = "linux")]
            {
                get_monitor(screen_id_or_name)
                    .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?
            }
            #[cfg(target_os = "macos")]
            {
                get_monitor(screen_id_or_name).ok_or_else(|| {
                    GStreamerError::DeviceError(format!(
                        "Screen with ID or name '{}' not found",
                        screen_id_or_name
                    ))
                })?
            }
        };

        let device = GstMediaDevice {
            display_name: monitor.display_name,
            device_class: "Screen/Source".to_string(),
            device_path: monitor.device_path,
        };

        Ok(device)
    }

    pub fn capabilities(&self) -> Vec<MediaCapability> {
        if self.device_class == "Screen/Source" {
            #[cfg(target_os = "windows")]
            {
                if let Some((monitor, _)) = get_monitor(&self.device_path) {
                    return monitor.capabilities;
                } else {
                    return vec![];
                }
            }

            #[cfg(target_os = "linux")]
            {
                return get_monitor(&self.device_path).map_or(vec![], |m| m.capabilities);
            }

            #[cfg(target_os = "macos")]
            {
                return get_monitor(&self.device_path).map_or(vec![], |m| m.capabilities);
            }
        }
        let device = get_gst_device(&self.device_path).unwrap();
        get_device_capabilities(&device)
    }

    pub fn screen_share_pipeline(
        &self,
        codec: &str,
        width: i32,
        height: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        if self.device_class != "Screen/Source" {
            return Err(GStreamerError::PipelineError(
                "Device is not a screen source".to_string(),
            ));
        }

        let can_support = self.supports_screen_share(codec, width, height, framerate);
        if !can_support {
            return Err(GStreamerError::PipelineError(
                "Device does not support requested configuration".to_string(),
            ));
        }

        let element = self.get_screen_element()?;

        // Single convert before tee — BGR→I420 once, shared by both branches
        let video_convert = gstreamer::ElementFactory::make("videoconvert")
            .name(random_string("videoconvert"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create videoconvert".to_string())
            })?;

        let video_scale = gstreamer::ElementFactory::make("videoscale")
            .name(random_string("videoscale"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create videoscale".to_string())
            })?;

        let caps = gstreamer::Caps::builder("video/x-raw")
            .field("format", VIDEO_FRAME_FORMAT)
            .field("width", width)
            .field("height", height)
            .field("framerate", gstreamer::Fraction::new(framerate, 1))
            .field("pixel-aspect-ratio", gstreamer::Fraction::new(1, 1))
            .build();

        let caps_filter = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;
        caps_filter.set_property("caps", &caps);

        let tee = gstreamer::ElementFactory::make("tee")
            .name(random_string("tee"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create tee".to_string()))?;

        // --- Stream branch ---
        let queue_appsink = gstreamer::ElementFactory::make("queue")
            .name(random_string("queue-appsink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create queue".to_string()))?;
        queue_appsink.set_property_from_str("leaky", "downstream");

        // No stream_convert needed — already I420 from before the tee
        let stream_scale = gstreamer::ElementFactory::make("videoscale")
            .name(random_string("stream-videoscale"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create stream videoscale".to_string())
            })?;

        let stream_caps = gstreamer::Caps::builder("video/x-raw")
            .field("width", 640)
            .field("height", 480)
            .field("framerate", gstreamer::Fraction::new(framerate, 1))
            .field("format", VIDEO_FRAME_FORMAT)
            .build();

        let stream_capsfilter = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("stream-capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create stream capsfilter".to_string())
            })?;
        stream_capsfilter.set_property("caps", &stream_caps);

        let broadcast_appsink = self.broadcast_appsink(tx, Some(&stream_caps))?;

        let pipeline = gstreamer::Pipeline::with_name(&random_string("stream-screen-share"));

        pipeline
            .add_many([
                &element,
                &video_convert,
                &video_scale,
                &caps_filter,
                &tee,
                &queue_appsink,
                &stream_scale,
                &stream_capsfilter,
                broadcast_appsink.upcast_ref(),
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add elements to pipeline".to_string())
            })?;

        // Link source chain up to tee
        gstreamer::Element::link_many([&element, &video_convert, &video_scale, &caps_filter, &tee])
            .map_err(|e| GStreamerError::PipelineError(e.to_string()))?;

        // Explicitly link tee → stream branch
        let tee_stream_pad = tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request tee pad for appsink".into())
        })?;
        let queue_appsink_sink_pad = queue_appsink
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Appsink queue has no sink pad".into()))?;
        tee_stream_pad.link(&queue_appsink_sink_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link tee to appsink queue".into())
        })?;

        gstreamer::Element::link_many([
            &queue_appsink,
            &stream_scale,
            &stream_capsfilter,
            broadcast_appsink.upcast_ref(),
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link stream branch".to_string()))?;

        // Explicitly link tee → file branch (videorate + ultrafast x264 lives inside)
        if let Some(ref path) = filename {
            self.add_video_file_branch(&pipeline, &tee, path)?;
        }

        pipeline
            .iterate_elements()
            .foreach(|e| {
                let _ = e.sync_state_with_parent();
            })
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to sync state with parent".to_string())
            })?;

        Ok(pipeline)
    }

    pub fn video_pipeline(
        &self,
        codec: &str,
        width: i32,
        height: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        if self.device_class == "Audio/Source" {
            return Err(GStreamerError::PipelineError(
                "Device is an audio source".to_string(),
            ));
        }

        if !SUPPORTED_VIDEO_CODECS.contains(&codec) {
            return Err(GStreamerError::PipelineError(format!(
                "Unsupported codec {}",
                codec
            )));
        }

        let can_support = self.supports_video(codec, width, height, framerate);
        if !can_support {
            return Err(GStreamerError::PipelineError(
                "Device does not support requested configuration".to_string(),
            ));
        }
        if codec == "video/x-raw" {
            return self.video_xraw_pipeline(width, height, framerate, tx, filename);
        } else if codec == "video/x-h264" {
            return self.video_xh264_pipeline(width, height, framerate, tx, filename);
        } else if codec == "image/jpeg" {
            return self.image_jpeg_pipeline(width, height, framerate, tx, filename);
        }

        Err(GStreamerError::PipelineError(
            "Failed to create pipeline".to_string(),
        ))
    }

    pub fn audio_pipeline(
        &self,
        codec: &str,
        channels: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        if self.device_class == "Video/Source" {
            return Err(GStreamerError::PipelineError(
                "Device is a video source".to_string(),
            ));
        }

        if !SUPPORTED_AUDIO_CODECS.contains(&codec) {
            return Err(GStreamerError::PipelineError(format!(
                "Unsupported codec {}",
                codec
            )));
        }

        let can_support = self.supports_audio(codec, channels, framerate);
        if !can_support {
            return Err(GStreamerError::PipelineError(
                "Device does not support requested configuration".to_string(),
            ));
        }
        self.audio_xraw_pipeline(channels, framerate, tx, filename)
    }

    pub fn deinterleaved_audio_pipeline(
        &self,
        codec: &str,
        channels: i32,
        selected_channel: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        if self.device_class == "Video/Source" {
            return Err(GStreamerError::PipelineError(
                "Device is a video source".to_string(),
            ));
        }

        if !SUPPORTED_AUDIO_CODECS.contains(&codec) {
            return Err(GStreamerError::PipelineError(format!(
                "Unsupported codec {}",
                codec
            )));
        }

        let can_support = self.supports_audio(codec, channels, framerate);
        if !can_support {
            return Err(GStreamerError::PipelineError(
                "Device does not support requested configuration".to_string(),
            ));
        }

        self.audio_deinterleaved_pipeline(selected_channel, channels, framerate, tx, filename)
    }

    fn audio_deinterleaved_pipeline(
        &self,
        selected_channel: i32,
        channels: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        let audio_el = self.get_audio_element()?;
        let convert = gstreamer::ElementFactory::make("audioconvert")
            .name(random_string("audioconvert"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create audioconvert".to_string())
            })?;

        let caps = gstreamer::Caps::builder("audio/x-raw")
            .field("format", "S16LE")
            .field("channels", channels)
            .field("rate", framerate)
            .field("channel-mask", gstreamer::Bitmask::new((1 << channels) - 1))
            .build();

        let caps_element = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;

        caps_element.set_property("caps", caps);

        let deinterleave_element = gstreamer::ElementFactory::make("deinterleave")
            .name(random_string("deinterleave"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create deinterleave".to_string())
            })?;

        let queue = gstreamer::ElementFactory::make("queue")
            .name(random_string("queue"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create queue".to_string()))?;

        let tee = gstreamer::ElementFactory::make("tee")
            .name(random_string("tee"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create tee".to_string()))?;

        let queue_appsink = gstreamer::ElementFactory::make("queue")
            .name(random_string("queue-appsink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create queue".to_string()))?;

        let broadcast_appsink = self.broadcast_appsink(tx, None)?;

        let pipeline = gstreamer::Pipeline::with_name(&random_string("deinterleaved-audio-xraw"));

        pipeline
            .add_many([
                &audio_el,
                &convert,
                &caps_element,
                &deinterleave_element,
                &queue,
                &tee,
                &queue_appsink,
                (broadcast_appsink.upcast_ref()),
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add elements to pipeline".to_string())
            })?;

        gstreamer::Element::link_many([&audio_el, &convert, &caps_element, &deinterleave_element])
            .map_err(|_| GStreamerError::PipelineError("Failed to link elements".to_string()))?;

        let cloned = queue.clone();

        deinterleave_element.connect_pad_added(move |_, src_pad| {
            let pad_name = src_pad.name();
            if pad_name == format!("src_{}", selected_channel - 1) {
                let queue_sink_pad = cloned.static_pad("sink").unwrap();
                if queue_sink_pad.is_linked() {
                    return;
                }
                src_pad.link(&queue_sink_pad).unwrap();
            }
        });

        gstreamer::Element::link_many([&queue, &tee]).map_err(|_| {
            GStreamerError::PipelineError("Failed to link queue and tee".to_string())
        })?;

        let tee_appsink_pad = tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request tee pad for appsink".into())
        })?;

        let queue_appsink_pad = queue_appsink
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Appsink queue has no sink pad".into()))?;

        tee_appsink_pad.link(&queue_appsink_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link tee to appsink queue".into())
        })?;

        gstreamer::Element::link_many([&queue_appsink, broadcast_appsink.upcast_ref()])
            .map_err(|_| GStreamerError::PipelineError("Failed to link appsink".to_string()))?;

        if let Some(ref path) = filename {
            self.add_audio_file_branch(&pipeline, &tee, path)?;
        }

        Ok(pipeline)
    }

    fn audio_xraw_pipeline(
        &self,
        channels: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        let audio_el = self.get_audio_element()?;
        let convert = gstreamer::ElementFactory::make("audioconvert")
            .name(random_string("audioconvert"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create audioconvert".to_string())
            })?;

        let resample = gstreamer::ElementFactory::make("audioresample")
            .name(random_string("audioresample"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create audioresample".to_string())
            })?;

        let caps = gstreamer::Caps::builder("audio/x-raw")
            .field("format", "S16LE")
            .field("channels", channels)
            .field("rate", framerate)
            .build();

        let caps_element = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;

        caps_element.set_property("caps", caps);

        let audiorate = gstreamer::ElementFactory::make("audiorate")
            .name(random_string("audiorate"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create audiorate".to_string()))?;

        audiorate.set_property("tolerance", 40000000u64);
        // audiorate.set_property("skip-to-first", true);

        let tee = gstreamer::ElementFactory::make("tee")
            .name(random_string("tee"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create tee".to_string()))?;

        let queue_appsink = gstreamer::ElementFactory::make("queue")
            .name(random_string("queue-appsink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create queue".to_string()))?;

        let broadcast_appsink = self.broadcast_appsink(tx, None)?;

        let pipeline = gstreamer::Pipeline::with_name(&random_string("stream-audio-xraw"));

        pipeline
            .add_many([
                &audio_el,
                &convert,
                &resample,
                &caps_element,
                &audiorate,
                &tee,
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add elements to pipeline".to_string())
            })?;

        gstreamer::Element::link_many([
            &audio_el,
            &convert,
            &resample,
            &caps_element,
            &audiorate,
            &tee,
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link elements".to_string()))?;

        pipeline
            .add_many([&queue_appsink, broadcast_appsink.upcast_ref()])
            .map_err(|_| GStreamerError::PipelineError("Failed to add appsink".to_string()))?;
        gstreamer::Element::link_many([&queue_appsink, broadcast_appsink.upcast_ref()])
            .map_err(|_| GStreamerError::PipelineError("Failed to link appsink".to_string()))?;

        let tee_appsink_pad = tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request tee pad for appsink".into())
        })?;

        let queue_appsink_pad = queue_appsink
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Appsink queue has no sink pad".into()))?;

        tee_appsink_pad.link(&queue_appsink_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link tee to appsink queue".into())
        })?;

        if let Some(ref path) = filename {
            self.add_audio_file_branch(&pipeline, &tee, path)?;
        }

        pipeline
            .iterate_elements()
            .foreach(|e| {
                let _ = e.sync_state_with_parent();
            })
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to sync state with parent".to_string())
            })?;

        Ok(pipeline)
    }

    pub fn supports_video(&self, codec: &str, width: i32, height: i32, framerate: i32) -> bool {
        let caps = self.capabilities();
        if self.device_class == "Audio/Source" {
            return false;
        }
        let caps = caps
            .iter()
            .filter_map(|c| match c {
                MediaCapability::Video(c) => Some(c),
                _ => None,
            })
            .collect::<Vec<_>>();

        caps.iter().any(|c| {
            c.codec == codec
                && c.width == width
                && c.height == height
                && c.framerates.contains(&framerate)
        })
    }

    pub fn preroll_video_pipeline(
        &self,
        codec: &str,
        width: i32,
        height: i32,
        framerate: i32,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        let pipeline = self.video_pipeline(
            codec,
            width,
            height,
            framerate,
            Arc::new(broadcast::channel(1).0),
            None,
        )?;
        pipeline.set_state(gstreamer::State::Paused).map_err(|_| {
            GStreamerError::PipelineError(
                "Failed to set preroll pipeline to Paused state".to_string(),
            )
        })?;
        Ok(pipeline)
    }

    pub fn supports_audio(&self, codec: &str, channels: i32, framerate: i32) -> bool {
        let caps = self.capabilities();
        if self.device_class == "Video/Source" {
            return false;
        }
        let caps = caps
            .iter()
            .filter_map(|c| match c {
                MediaCapability::Audio(c) => Some(c),
                _ => None,
            })
            .collect::<Vec<_>>();

        caps.iter().any(|c| {
            c.codec == codec
                && c.channels == channels
                && c.framerates.0 <= framerate
                && c.framerates.1 >= framerate
        })
    }

    pub fn supports_screen_share(
        &self,
        codec: &str,
        width: i32,
        height: i32,
        framerate: i32,
    ) -> bool {
        if self.device_class != "Screen/Source" {
            return false;
        }
        let caps = self.capabilities();
        let caps = caps
            .iter()
            .filter_map(|c| match c {
                MediaCapability::Screen(c) => Some(c),
                _ => None,
            })
            .collect::<Vec<_>>();
        println!("Screen share capabilities: {:?}", caps);
        println!(
            "Checking codec: {}, width: {}, height: {}, framerate: {}",
            codec, width, height, framerate
        );
        caps.iter().any(|c| {
            c.codec == codec
                && c.width >= width
                && c.height >= height
                && c.framerates.contains(&framerate)
        })
    }

    fn video_xraw_pipeline(
        &self,
        width: i32,
        height: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        let input = self.get_video_element()?;

        let caps_element = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;
        let caps = gstreamer::Caps::builder("video/x-raw")
            .field("width", width)
            .field("height", height)
            .field("format", VIDEO_FRAME_FORMAT)
            .field("framerate", gstreamer::Fraction::new(framerate, 1))
            .build();
        caps_element.set_property("caps", caps);

        let convert = gstreamer::ElementFactory::make("videoconvert")
            .name(random_string("videoconvert"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create videoconvert".to_string())
            })?;

        // let rate = gstreamer::ElementFactory::make("videorate")
        //     .name(random_string("videorate"))
        //     .build()
        //     .map_err(|_| GStreamerError::PipelineError("Failed to create videorate".to_string()))?;

        // rate.set_property("max-rate", framerate);

        let i420_caps = gstreamer::Caps::builder("video/x-raw")
            .field("format", "I420")
            .build();

        let caps_filter = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;

        caps_filter.set_property("caps", &i420_caps);

        let tee = gstreamer::ElementFactory::make("tee")
            .name(random_string("tee"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create tee".to_string()))?;

        let queue_appsink = gstreamer::ElementFactory::make("queue")
            .name(random_string("queue-appsink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create queue".to_string()))?;

        let stream_convert = gstreamer::ElementFactory::make("videoconvert")
            .name(random_string("videoconvert"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create videoconvert".to_string())
            })?;

        let stream_scale = gstreamer::ElementFactory::make("videoscale")
            .name(random_string("videoscale"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create videoscale".to_string())
            })?;

        let stream_caps = gstreamer::Caps::builder("video/x-raw")
            .field("width", 640)
            .field("height", 480)
            .field("framerate", gstreamer::Fraction::new(framerate, 1))
            .field("format", VIDEO_FRAME_FORMAT)
            .build();

        let stream_capsfilter = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;
        stream_capsfilter.set_property("caps", &stream_caps);

        let appsink = self.broadcast_appsink(tx, Some(&stream_caps))?;

        let pipeline = gstreamer::Pipeline::with_name(&random_string("stream-xraw"));

        pipeline
            .add_many([
                &input,
                &convert,
                // &rate,
                &caps_element,
                &caps_filter,
                &tee,
                &queue_appsink,
                &stream_convert,
                &stream_scale,
                &stream_capsfilter,
                appsink.upcast_ref(),
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add elements to pipeline".to_string())
            })?;

        gstreamer::Element::link_many([&input, &convert, &caps_element, &caps_filter, &tee])
            .map_err(|_| GStreamerError::PipelineError("Failed to link elements".to_string()))?;

        let tee_appsink_pad = tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request tee pad for appsink".into())
        })?;

        let queue_appsink_pad = queue_appsink
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Appsink queue has no sink pad".into()))?;

        tee_appsink_pad.link(&queue_appsink_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link tee to appsink queue".into())
        })?;

        gstreamer::Element::link_many([
            &queue_appsink,
            &stream_convert,
            &stream_scale,
            &stream_capsfilter,
            appsink.upcast_ref(),
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link appsink".to_string()))?;

        if let Some(ref path) = filename {
            self.add_video_file_branch(&pipeline, &tee, path)?;
        }

        pipeline
            .iterate_elements()
            .foreach(|e| {
                let _ = e.sync_state_with_parent();
            })
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to sync state with parent".to_string())
            })?;

        Ok(pipeline)
    }
    fn video_xh264_pipeline(
        &self,
        width: i32,
        height: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        if filename.is_some() {
            return Err(GStreamerError::PipelineError(
                "Filename not supported for H264 pipeline".to_string(),
            ));
        }

        let input = self.get_video_element()?;
        let caps_element = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;
        let caps = gstreamer::Caps::builder("video/x-h264")
            .field("width", width)
            .field("height", height)
            .field("framerate", gstreamer::Fraction::new(framerate, 1))
            .build();
        caps_element.set_property("caps", caps);

        let h264parse = gstreamer::ElementFactory::make("h264parse")
            .name(random_string("h264parse"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create h264parse".to_string()))?;

        let avdec_h264 = gstreamer::ElementFactory::make("avdec_h264")
            .name(random_string("avdec_h264"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create avdec_h264".to_string())
            })?;

        let i420_caps = gstreamer::Caps::builder("video/x-raw")
            .field("format", "I420")
            .build();
        let appsink = self.broadcast_appsink(tx, Some(&i420_caps))?;

        let pipeline = gstreamer::Pipeline::with_name(&random_string("stream-h264"));

        pipeline
            .add_many([
                &input,
                &caps_element,
                &h264parse,
                &avdec_h264,
                appsink.upcast_ref(),
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add elements to pipeline".to_string())
            })?;

        gstreamer::Element::link_many([
            &input,
            &caps_element,
            &h264parse,
            &avdec_h264,
            appsink.upcast_ref(),
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link elements".to_string()))?;

        Ok(pipeline)
    }

    fn image_jpeg_pipeline(
        &self,
        width: i32,
        height: i32,
        framerate: i32,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        filename: Option<String>,
    ) -> Result<gstreamer::Pipeline, GStreamerError> {
        let input = self.get_video_element()?;

        let caps_element = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create capsfilter".to_string())
            })?;
        let caps = gstreamer::Caps::builder("image/jpeg")
            .field("width", width)
            .field("height", height)
            .field("framerate", gstreamer::Fraction::new(framerate, 1))
            .build();
        caps_element.set_property("caps", caps);

        let jpegdec = gstreamer::ElementFactory::make("jpegdec")
            .name(random_string("jpegdec"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create jpegdec".to_string()))?;

        let convert = gstreamer::ElementFactory::make("videoconvert")
            .name(random_string("videoconvert"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create videoconvert".to_string())
            })?;

        let i420_caps = gstreamer::Caps::builder("video/x-raw")
            .field("format", "I420")
            .build();
        let i420_filter = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create i420 capsfilter".to_string())
            })?;
        i420_filter.set_property("caps", &i420_caps);

        let tee = gstreamer::ElementFactory::make("tee")
            .name(random_string("tee"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create tee".to_string()))?;

        let queue_appsink = gstreamer::ElementFactory::make("queue")
            .name(random_string("queue-appsink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create queue".to_string()))?;

        queue_appsink.set_property_from_str("leaky", "downstream");

        let stream_scale = gstreamer::ElementFactory::make("videoscale")
            .name(random_string("stream-videoscale"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create stream videoscale".to_string())
            })?;

        let stream_caps = gstreamer::Caps::builder("video/x-raw")
            .field("width", 640)
            .field("height", 480)
            .field("framerate", gstreamer::Fraction::new(framerate, 1))
            .field("format", VIDEO_FRAME_FORMAT)
            .build();
        let stream_capsfilter = gstreamer::ElementFactory::make("capsfilter")
            .name(random_string("stream-capsfilter"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create stream capsfilter".to_string())
            })?;
        stream_capsfilter.set_property("caps", &stream_caps);

        let appsink = self.broadcast_appsink(tx, Some(&stream_caps))?;

        let pipeline = gstreamer::Pipeline::with_name(&random_string("stream-jpeg"));

        pipeline
            .add_many([
                &input,
                &caps_element,
                &jpegdec,
                &convert,
                &i420_filter,
                &tee,
                &queue_appsink,
                &stream_scale,
                &stream_capsfilter,
                appsink.upcast_ref(),
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to add elements to pipeline".to_string())
            })?;

        gstreamer::Element::link_many([
            &input,
            &caps_element,
            &jpegdec,
            &convert,
            &i420_filter,
            &tee,
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link source chain".to_string()))?;

        let tee_stream_pad = tee.request_pad_simple("src_%u").ok_or_else(|| {
            GStreamerError::PipelineError("Failed to request tee pad for appsink".into())
        })?;
        let queue_appsink_sink_pad = queue_appsink
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Appsink queue has no sink pad".into()))?;
        tee_stream_pad.link(&queue_appsink_sink_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link tee to appsink queue".into())
        })?;

        gstreamer::Element::link_many([
            &queue_appsink,
            &stream_scale,
            &stream_capsfilter,
            appsink.upcast_ref(),
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link appsink branch".to_string()))?;

        if let Some(ref path) = filename {
            self.add_video_file_branch(&pipeline, &tee, path)?;
        }

        pipeline
            .iterate_elements()
            .foreach(|e| {
                let _ = e.sync_state_with_parent();
            })
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to sync state with parent".to_string())
            })?;

        Ok(pipeline)
    }

    #[cfg(target_os = "windows")]
    fn get_screen_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let (_, idx) = get_monitor(&self.device_path)
            .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?;

        if gstreamer::ElementFactory::find("dx9screencapsrc").is_some() {
            let element = gstreamer::ElementFactory::make("dx9screencapsrc")
                .name(random_string("screen-source"))
                .build()
                .map_err(|_| {
                    GStreamerError::PipelineError("Failed to create dxgiscreencapsrc".to_string())
                })?;

            element.set_property("monitor", idx);
            element.set_property("cursor", true);

            Ok(element)
        } else {
            Err(GStreamerError::PipelineError(
                "dx9screencapsrc not found".to_string(),
            ))
        }
    }

    #[cfg(target_os = "macos")]
    fn get_screen_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let monitor = get_monitor(&self.device_path).ok_or_else(|| {
            GStreamerError::DeviceError(format!("No screen found {}", self.device_path))
        })?;

        let element = gstreamer::ElementFactory::make("avfvideosrc")
            .name(random_string("screen-source"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create osxvideosrc".to_string())
            })?;

        if let Some(MediaCapability::Screen(cap)) = monitor.capabilities.first() {
            element.set_property("capture-screen", true);
            element.set_property("capture-screen-cursor", true);
            element.set_property("screen-crop-x", cap.startx as u32);
            element.set_property("screen-crop-y", cap.starty as u32);
            element.set_property("screen-crop-width", cap.endx as u32 - cap.startx as u32);
            element.set_property("screen-crop-height", cap.endy as u32 - cap.starty as u32);
        } else {
            return Err(GStreamerError::PipelineError(
                "No screen capability found".to_string(),
            ));
        }

        Ok(element)
    }

    #[cfg(target_os = "linux")]
    fn get_screen_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let monitor = get_monitor(&self.device_path)
            .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?;

        let element = gstreamer::ElementFactory::make("ximagesrc")
            .name(random_string("screen-source"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create ximagesrc".to_string()))?;

        if let Some(MediaCapability::Screen(cap)) = monitor.capabilities.first() {
            element.set_property("use-damage", false);
            element.set_property("show-pointer", true);
            element.set_property("startx", cap.startx as u32);
            element.set_property("starty", cap.starty as u32);
            element.set_property("endx", cap.endx as u32 - 1);
            element.set_property("endy", cap.endy as u32 - 1);
        } else {
            return Err(GStreamerError::PipelineError(
                "No screen capability found".to_string(),
            ));
        }

        Ok(element)
    }

    fn get_video_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let device = get_gst_device(&self.device_path).unwrap();
        let random_source_name = random_string("source");
        let element = device
            .create_element(Some(random_source_name.as_str()))
            .unwrap();
        Ok(element)
    }

    fn get_audio_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let device = get_gst_device(&self.device_path).unwrap();
        println!("Device: {:?}", device);
        println!("Device props: {:?}", device.caps());
        let random_source_name = random_string("source");
        let element = device
            .create_element(Some(random_source_name.as_str()))
            .unwrap();
        Ok(element)
    }

    fn broadcast_appsink(
        &self,
        tx: Arc<broadcast::Sender<Arc<Buffer>>>,
        caps: Option<&gstreamer::Caps>,
    ) -> Result<AppSink, GStreamerError> {
        let appsink = gstreamer::ElementFactory::make("appsink")
            .name(random_string("xraw-appsink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create appsink".to_string()))?;
        let appsink = appsink
            .dynamic_cast::<AppSink>()
            .map_err(|_| GStreamerError::PipelineError("Failed to cast appsink".to_string()))?;

        appsink.set_property("emit-signals", true);
        appsink.set_property("drop", true);
        appsink.set_property("max-buffers", 1u32);

        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = match sink.pull_sample() {
                        Ok(s) => s,
                        Err(_) => return Err(gstreamer::FlowError::Eos),
                    };

                    let buffer = sample.buffer().ok_or(gstreamer::FlowError::Error)?;

                    if tx.receiver_count() > 0 {
                        let _ = tx.send(Arc::new(buffer.copy()));
                    }
                    Ok(gstreamer::FlowSuccess::Ok)
                })
                .build(),
        );
        if caps.is_some() {
            appsink.set_caps(caps);
        }

        Ok(appsink)
    }

    fn add_video_file_branch(
        &self,
        pipeline: &gstreamer::Pipeline,
        tee: &gstreamer::Element,
        path: &str,
    ) -> Result<(), GStreamerError> {
        let queue_file = gstreamer::ElementFactory::make("queue")
            .name(random_string("file-queue"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("queue".into()))?;
        queue_file.set_property_from_str("leaky", "downstream"); // leaky downstream

        let file_rate = gstreamer::ElementFactory::make("videorate")
            .name(random_string("file-videorate"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create file videorate".into()))?;
        file_rate.set_property("max-rate", 30i32); // low frequency recording, tune as needed

        let convert = gstreamer::ElementFactory::make("videoconvert")
            .name(random_string("file-videoconvert"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("videoconvert".into()))?;

        let encoder = gstreamer::ElementFactory::make("x264enc")
            .name(random_string("file-x264enc"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("x264enc".into()))?;
        encoder.set_property("bitrate", 3000u32);
        encoder.set_property_from_str("speed-preset", "ultrafast"); // much cheaper than zerolatency tune

        let parser = gstreamer::ElementFactory::make("h264parse")
            .name(random_string("file-h264parse"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("h264parse".into()))?;

        let muxer = gstreamer::ElementFactory::make("mp4mux")
            .name(random_string("file-mp4mux"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("mp4mux".into()))?;

        let filesink = gstreamer::ElementFactory::make("filesink")
            .name(random_string("file-filesink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("filesink".into()))?;
        filesink.set_property("location", path);
        filesink.set_property("sync", false);

        pipeline
            .add_many([
                &queue_file,
                &file_rate,
                &convert,
                &encoder,
                &parser,
                &muxer,
                &filesink,
            ])
            .map_err(|_| GStreamerError::PipelineError("Failed to add file branch".into()))?;

        gstreamer::Element::link_many([
            &queue_file,
            &file_rate,
            &convert,
            &encoder,
            &parser,
            &muxer,
            &filesink,
        ])
        .map_err(|_| GStreamerError::PipelineError("Failed to link file branch".into()))?;

        let tee_src_pad = tee
            .request_pad_simple("src_%u")
            .ok_or_else(|| GStreamerError::PipelineError("Failed to request tee pad".into()))?;
        let queue_sink_pad = queue_file
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Queue has no sink pad".into()))?;
        tee_src_pad.link(&queue_sink_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link tee to file branch".into())
        })?;

        Ok(())
    }

    fn add_audio_file_branch(
        &self,
        pipeline: &gstreamer::Pipeline,
        tee: &gstreamer::Element,
        path: &str,
    ) -> Result<(), GStreamerError> {
        let queue_file = gstreamer::ElementFactory::make("queue")
            .name(random_string("file-queue"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("queue".into()))?;

        let convert = gstreamer::ElementFactory::make("audioconvert")
            .name(random_string("file-audioconvert"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("audioconvert".into()))?;

        let resample = gstreamer::ElementFactory::make("audioresample")
            .name(random_string("file-audioresample"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("audioresample".into()))?;

        let encoder = gstreamer::ElementFactory::make("avenc_aac")
            .name(random_string("file-avenc_aac"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("avenc_aac".into()))?;
        encoder.set_property("bitrate", 128000i32);

        let parser = gstreamer::ElementFactory::make("aacparse")
            .name(random_string("file-aacparse"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("aacparse".into()))?;

        let muxer = gstreamer::ElementFactory::make("mp4mux")
            .name(random_string("file-mp4mux"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("mp4mux".into()))?;

        let filesink = gstreamer::ElementFactory::make("filesink")
            .name(random_string("file-filesink"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("filesink".into()))?;
        filesink.set_property("location", path);
        filesink.set_property("sync", false);

        pipeline
            .add_many([
                &queue_file,
                &convert,
                &resample,
                &encoder,
                &parser,
                &muxer,
                &filesink,
            ])
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to ad elements to the file branch".into())
            })?;

        gstreamer::Element::link_many([
            &queue_file,
            &convert,
            &resample,
            &encoder,
            &parser,
            &muxer,
            &filesink,
        ])
        .map_err(|_| {
            GStreamerError::PipelineError("Failed to link elements in file branch".into())
        })?;

        let tee_src_pad = tee
            .request_pad_simple("src_%u")
            .ok_or_else(|| GStreamerError::PipelineError("Failed to request tee pad".into()))?;
        let queue_sink_pad = queue_file
            .static_pad("sink")
            .ok_or_else(|| GStreamerError::PipelineError("Queue has no sink pad".into()))?;

        tee_src_pad.link(&queue_sink_pad).map_err(|_| {
            GStreamerError::PipelineError("Failed to link tee to file branch".into())
        })?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCapability {
    pub width: i32,
    pub height: i32,
    pub framerates: Vec<i32>,
    pub codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCapability {
    pub channels: i32,
    pub framerates: (i32, i32),
    pub codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCapability {
    pub width: i32,
    pub height: i32,
    pub framerates: Vec<i32>,
    pub codec: String,
    pub startx: i32,
    pub starty: i32,
    pub endx: i32,
    pub endy: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDeviceInfo {
    pub device_path: String,
    pub display_name: String,
    pub capabilities: Vec<MediaCapability>,
    pub device_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum MediaCapability {
    Video(VideoCapability),
    Audio(AudioCapability),
    Screen(ScreenCapability), // For screen capture capabilities
}

#[derive(Debug, Clone, Error)]
pub enum GStreamerError {
    #[error("Failed to create pipeline: {0}")]
    PipelineError(String),
    #[error("Devices: {0}")]
    DeviceError(String),
}

mod tests {
    #[cfg(test)]
    use super::*;

    #[test]
    fn test_from_path() {
        gstreamer::init().unwrap();
        let path = "/dev/video4";
        let device = GstMediaDevice::from_device_path(path);
        assert!(device.is_ok());
        let device = device.unwrap();
        assert_eq!(device.device_path, path);
    }
}
