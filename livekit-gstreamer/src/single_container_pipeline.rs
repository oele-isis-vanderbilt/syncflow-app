use gstreamer::prelude::*;
use serde::{de, Deserialize, Serialize};

use crate::{
    get_gst_device, utils::get_device_name, AudioPublishOptions, GStreamerError, GstMediaDevice,
    ScreenPublishOptions, VideoPublishOptions,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleContainerPipelineRecordingMetadata {
    pub device_names: Vec<String>,
    pub video_configs: Vec<VideoTrackConfig>,
    pub screen_configs: Vec<ScreenTrackConfig>,
    pub audio_configs: Vec<AudioTrackConfig>,
    pub start_time: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackConfig {
    pub device_id: String,
    pub device_name: Option<String>,
    pub sample_rate: i32,
    pub bitrate: i32,
    pub channels: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTrackConfig {
    pub device_id: String,
    pub device_name: Option<String>,
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
    pub bitrate: i32,
    pub preset: String,
    pub camera_codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenTrackConfig {
    pub device_id: String,
    pub device_name: Option<String>,
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
    pub bitrate: i32,   // e.g., 4000 kbps
    pub preset: String, // e.g., "veryfast"
}

impl TryFrom<VideoPublishOptions> for VideoTrackConfig {
    type Error = GStreamerError;
    fn try_from(options: VideoPublishOptions) -> Result<Self, Self::Error> {
        let pixels_per_second = options.height * options.width * options.framerate;
        let bits_per_pixel = 0.1; // ToDo: tune this constant
        let bitrate = (pixels_per_second as f64 * bits_per_pixel) as i32;
        let device_name = get_device_name(&options.device_id, false)?;
        Ok(Self {
            device_id: options.device_id,
            device_name: Some(device_name),
            width: options.width,
            height: options.height,
            framerate: options.framerate,
            bitrate: bitrate.into(),
            preset: "ultrafast".to_string(),
            camera_codec: options.codec,
        })
    }
}

impl TryFrom<ScreenPublishOptions> for ScreenTrackConfig {
    type Error = GStreamerError;
    fn try_from(options: ScreenPublishOptions) -> Result<Self, Self::Error> {
        let pixels_per_second = options.height * options.width * options.framerate;
        let bits_per_pixel = 0.05; // Screens can often get away with lower bitrate
        let bitrate = (pixels_per_second as f64 * bits_per_pixel) as i32;
        let device_name = get_device_name(&options.screen_id_or_name, true)?;

        Ok(Self {
            device_id: options.screen_id_or_name,
            device_name: Some(device_name),
            width: options.width,
            height: options.height,
            framerate: options.framerate,
            bitrate: bitrate.into(),
            preset: "ultrafast".to_string(),
        })
    }
}

impl TryFrom<AudioPublishOptions> for AudioTrackConfig {
    type Error = GStreamerError;
    fn try_from(options: AudioPublishOptions) -> Result<Self, Self::Error> {
        let device_name = get_device_name(&options.device_id, false)?;
        Ok(Self {
            device_id: options.device_id,
            channels: options.channels,
            sample_rate: options.framerate,
            bitrate: 128000, // ToDo: make this configurable
            device_name: Some(device_name),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleContainerPipeline {
    pub name: String,
    pub video_config: Vec<VideoTrackConfig>,
    pub screen_config: Vec<ScreenTrackConfig>,
    pub audio_tracks: Vec<AudioTrackConfig>,
    pub output_dir: String,

    // Runtime state
    #[serde(skip)]
    pipeline: Option<gstreamer::Pipeline>,

    #[serde(skip)]
    pub metadata: Option<SingleContainerPipelineRecordingMetadata>,
}

struct PipelineDescription {
    gst_launch_string: String,
}

impl SingleContainerPipeline {
    pub fn new(
        name: String,
        video_config: Vec<VideoTrackConfig>,
        screen_config: Vec<ScreenTrackConfig>,
        audio_tracks: Vec<AudioTrackConfig>,
        output_dir: String,
    ) -> Self {
        Self {
            name,
            video_config,
            screen_config,
            audio_tracks,
            output_dir,
            pipeline: None,
            metadata: None,
        }
    }

    #[cfg(target_os = "windows")]
    fn build_pipeline_description(&self) -> PipelineDescription {
        let output_file = format!(
            "{}/{}-{}.mkv",
            self.output_dir,
            self.name,
            chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
        );

        let mut pipeline_parts = Vec::new();

        for (idx, video) in self.video_config.iter().enumerate() {
            let video_branch = format!(
                "ksvideosrc device-path=\"{}\" ! \
                 {},width={},height={},framerate={}/1 ! \
                 jpegdec ! \
                 videoconvert ! \
                 videorate ! \
                 video/x-raw,width={},height={},framerate={}/1 ! \
                 queue ! \
                 x264enc bitrate={} speed-preset={} ! h264parse ! queue ! mux.",
                video.device_id,
                video.camera_codec,
                video.width,
                video.height,
                video.framerate,
                video.width,
                video.height,
                video.framerate,
                video.bitrate / 1000, // Convert to kbps
                video.preset
            );
            pipeline_parts.push(video_branch);
        }

        for (idx, screen) in self.screen_config.iter().enumerate() {
            let screen_branch = format!(
                "dx9screencapsrc ! \
                video/x-raw,framerate={}/1 ! \
                videoconvert ! videorate ! \
                video/x-raw,width={},height={},framerate={}/1 ! \
                queue ! \
                x264enc bitrate={} speed-preset={} ! h264parse ! queue ! mux.",
                screen.framerate,
                screen.width,
                screen.height,
                screen.framerate,
                screen.bitrate / 1000,
                screen.preset,
            );
            pipeline_parts.push(screen_branch);
        }

        for (idx, audio) in self.audio_tracks.iter().enumerate() {
            let audio_branch = format!(
                "wasapisrc exclusive=true device=\"{}\" low-latency=false buffer-time=200000 latency-time=10000 ! \
                audioconvert ! audioresample ! audiorate ! \
                audio/x-raw,channels={},rate={} ! \
                queue ! \
                voaacenc bitrate={} !    aacparse ! queue ! mux.",
                audio.device_id,
                audio.channels,   // <-- just use channels directly
                audio.sample_rate,
                audio.bitrate
            );
            pipeline_parts.push(audio_branch);
        }

        // Muxer and sink
        let muxer = format!(
            "matroskamux name=mux ! filesink name=filesink0 location=\"{}\"",
            output_file
        );

        // Combine all parts
        let gst_string = format!("{} {}", pipeline_parts.join(" "), muxer);
        PipelineDescription {
            gst_launch_string: gst_string,
        }
    }

    pub fn initialize(&mut self) -> Result<(), GStreamerError> {
        if self.pipeline.is_some() {
            return Ok(());
        }

        let pipeline_description = self.build_pipeline_description();
        println!(
            "Pipeline description: {}",
            pipeline_description.gst_launch_string
        );

        let pipeline =
            gstreamer::parse::launch(&pipeline_description.gst_launch_string).map_err(|e| {
                GStreamerError::PipelineError(format!("Failed to create pipeline: {}", e))
            })?;

        let pipeline_dc = pipeline
            .downcast::<gstreamer::Pipeline>()
            .map_err(|_| GStreamerError::PipelineError("Failed to downcast to Pipeline".into()))?;

        let clock = gstreamer::SystemClock::obtain();
        pipeline_dc.use_clock(Some(&clock));
        self.pipeline = Some(pipeline_dc);
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), GStreamerError> {
        if let Some(ref pipeline) = self.pipeline {
            let _ = self.set_output_file(&format!(
                "{}/{}-{}.mkv",
                self.output_dir,
                self.name,
                chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
            ));

            pipeline
                .set_state(gstreamer::State::Paused)
                .map_err(|e| GStreamerError::PipelineError(format!("Failed to pause: {}", e)))?;

            let (res, current, _pending) = pipeline.state(gstreamer::ClockTime::from_seconds(5));

            pipeline.set_state(gstreamer::State::Playing).map_err(|e| {
                GStreamerError::PipelineError(format!("Failed to start pipeline: {}", e))
            })?;

            // Todo: Error handling mid pipeline
            self.metadata = Some(SingleContainerPipelineRecordingMetadata {
                device_names: self
                    .audio_tracks
                    .iter()
                    .filter_map(|a| a.device_name.clone())
                    .collect(),
                video_configs: self.video_config.clone(),
                screen_configs: self.screen_config.clone(),
                audio_configs: self.audio_tracks.clone(),
                start_time: Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                ),
                error: None,
            });

            let (res, current, _) = pipeline.state(gstreamer::ClockTime::from_seconds(5));
            if let Some(clock) = pipeline.clock() {
                let now = clock.time();
                pipeline.set_base_time(now);
                pipeline.set_start_time(gstreamer::ClockTime::NONE);
            } else {
                println!("Warning: Pipeline has no clock");
            }
        }
        Ok(())
    }

    pub fn set_output_file(&self, path: &str) -> Result<(), GStreamerError> {
        if let Some(ref pipeline) = self.pipeline {
            let filesink = pipeline
                .by_name("filesink0")
                .ok_or_else(|| GStreamerError::PipelineError("filesink not found".into()))?;

            filesink.set_property("location", path);
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), GStreamerError> {
        if let Some(ref pipeline) = self.pipeline {
            pipeline.send_event(gstreamer::event::Eos::new());

            let bus = pipeline.bus().unwrap();
            for msg in bus.iter_timed(gstreamer::ClockTime::NONE) {
                use gstreamer::MessageView;
                match msg.view() {
                    MessageView::Eos(..) => {
                        println!("EOS received, pipeline finished writing");
                        break;
                    }
                    MessageView::Error(err) => {
                        println!("Error during stop: {}", err.error());
                        break;
                    }
                    _ => {}
                }
            }
            pipeline.set_state(gstreamer::State::Null).map_err(|e| {
                GStreamerError::PipelineError(format!("Failed to stop pipeline: {}", e))
            })?;
        }
        self.pipeline = None;
        if self.metadata.is_some() {
            let cloned_metadata = self.metadata.clone().unwrap();
            // Write JSON metadata file
            let metadata_path = format!(
                "{}/{}-{}.json",
                self.output_dir,
                self.name,
                chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
            );
            let metadata_json = serde_json::to_string_pretty(&cloned_metadata).unwrap();
            std::fs::write(&metadata_path, metadata_json).map_err(|e| {
                GStreamerError::PipelineError(format!("Failed to write metadata file: {}", e))
            })?;
        }
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.pipeline.is_some()
    }
}
