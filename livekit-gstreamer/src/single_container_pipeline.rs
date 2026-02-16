use gstreamer::prelude::*;
use serde::{Deserialize, Serialize};

use crate::GStreamerError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackConfig {
    pub device_id: String,
    pub device_name: Option<String>,
    pub sample_rate: i32,
    pub bitrate: i32,  // e.g., 128000
    pub channels: i32, // e.g., 2 for stereo
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTrackConfig {
    pub device_id: String,
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
    pub bitrate: i32,         // e.g., 4000 kbps
    pub preset: String,       // e.g., "veryfast"
    pub camera_codec: String, // e.g., "MJPEG" or "YUY2"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenTrackConfig {
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
    pub bitrate: i32,   // e.g., 4000 kbps
    pub preset: String, // e.g., "veryfast"
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
    pub fn build_pipeline_description(&self) -> String {
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
                screen.preset
            );
            pipeline_parts.push(screen_branch);
        }

        for (idx, audio) in self.audio_tracks.iter().enumerate() {
            let audio_branch = format!(
                "wasapisrc device=\"{}\" low-latency=false buffer-time=200000 latency-time=10000 ! \
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
        format!("{} {}", pipeline_parts.join(" "), muxer)
    }

    pub fn initialize(&mut self) -> Result<(), GStreamerError> {
        if self.pipeline.is_some() {
            return Ok(());
        }

        let pipeline_description = self.build_pipeline_description();
        println!("Pipeline description: {}", pipeline_description);

        let pipeline = gstreamer::parse::launch(&pipeline_description).map_err(|e| {
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
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.pipeline.is_some()
    }
}
