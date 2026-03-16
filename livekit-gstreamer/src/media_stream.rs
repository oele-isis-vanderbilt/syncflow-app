use crate::{
    media_device::{GStreamerError, GstMediaDevice},
    play_pipeline, preroll_pipeline, run_bus_loop, set_pipeline_clock,
    utils::random_string,
    RecordingMetadata,
};
use gstreamer::{prelude::*, Buffer, Pipeline};
use serde::{Deserialize, Serialize};
use std::{
    path::{self, PathBuf},
    sync::Arc,
};
use tokio::{fs, sync::broadcast};

#[derive(Debug)]
struct StreamHandle {
    close_tx: broadcast::Sender<()>,
    frame_tx: broadcast::Sender<Arc<Buffer>>,
    task: Option<tokio::task::JoinHandle<Result<(), GStreamerError>>>,
    pipeline: Pipeline,
    device: GstMediaDevice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFileSaveOptions {
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSaveFileMetadata {
    pub file_name: String,
    pub codec: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPublishOptions {
    pub codec: String,
    pub device_id: String,
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
    pub local_file_save_options: Option<LocalFileSaveOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPublishOptions {
    pub codec: String,
    pub device_id: String,
    pub framerate: i32,
    pub channels: i32,
    pub selected_channel: Option<i32>,
    pub local_file_save_options: Option<LocalFileSaveOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenPublishOptions {
    pub codec: String,
    pub screen_id_or_name: String,
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
    pub local_file_save_options: Option<LocalFileSaveOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PublishOptions {
    Video(VideoPublishOptions),
    Audio(AudioPublishOptions),
    Screen(ScreenPublishOptions),
}

#[derive(Debug)]
pub struct GstMediaStream {
    handle: Option<StreamHandle>,
    publish_options: PublishOptions,
    frame_tx: Option<broadcast::Sender<Arc<Buffer>>>,
    close_tx: Option<broadcast::Sender<()>>,
    pipeline: Option<Pipeline>,
    metadata: Option<RecordingMetadata>,
}

pub async fn create_dir(options: &LocalFileSaveOptions) -> Result<PathBuf, GStreamerError> {
    let output_dir = PathBuf::from(&options.output_dir);
    fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| GStreamerError::PipelineError(format!("Failed to create directory: {}", e)))?;
    Ok(output_dir)
}

fn strict_sanitize_filename<S: AsRef<str>>(filename: S) -> String {
    let s = filename
        .as_ref()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();

    // Return the last 10 characters of the sanitized filename appended with a random string
    if s.len() > 10 {
        let trimmed = &s[s.len().saturating_sub(10)..];
        random_string(trimmed)
    } else {
        s
    }
}

impl GstMediaStream {
    pub fn new(publish_options: PublishOptions) -> Self {
        Self {
            handle: None,
            publish_options,
            frame_tx: None,
            close_tx: None,
            pipeline: None,
            metadata: None,
        }
    }

    pub fn has_started(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| h.task.is_some())
    }

    pub fn kind(&self) -> &str {
        match &self.publish_options {
            PublishOptions::Video(_) => "Video",
            PublishOptions::Audio(_) => "Audio",
            PublishOptions::Screen(_) => "Screen",
        }
    }

    pub async fn stop(&mut self) -> Result<(), GStreamerError> {
        if let Some(handle) = self.handle.take() {
            handle.close_tx.send(()).ok();
            handle.pipeline.send_event(gstreamer::event::Eos::new());
            if let Some(task) = handle.task {
                let _ = task.await;
            }
        }
        self.handle = None;
        Ok(())
    }

    pub async fn preroll_pipeline(&mut self) -> Result<(), GStreamerError> {
        let pipeline = self.pipeline.clone().ok_or(GStreamerError::PipelineError(
            "Please call build pipeline first".into(),
        ))?;

        preroll_pipeline(&pipeline).await?;
        Ok(())
    }

    pub fn set_pipeline_clock(
        &mut self,
        clock: &gstreamer::Clock,
        base_time: gstreamer::ClockTime,
    ) -> Result<(), GStreamerError> {
        let pipeline = self.pipeline.clone().ok_or(GStreamerError::PipelineError(
            "Please call build pipeline first".into(),
        ))?;
        set_pipeline_clock(&pipeline, clock, base_time, self.metadata.as_mut())?;
        Ok(())
    }

    pub fn play_pipeline(&mut self) -> Result<(), GStreamerError> {
        let pipeline = self.pipeline.clone().ok_or(GStreamerError::PipelineError(
            "Please call build pipeline first".into(),
        ))?;
        play_pipeline(&pipeline, self.metadata.as_mut())?;
        Ok(())
    }

    pub async fn run_bus_loop(&mut self) -> Result<(), GStreamerError> {
        let pipeline = self.pipeline.clone().ok_or(GStreamerError::PipelineError(
            "Please call build pipeline first".into(),
        ))?;

        let (close_tx, _) = broadcast::channel::<()>(1);
        let metadata = self.metadata.as_mut();
        let pipeline_task = tokio::spawn(run_bus_loop(
            pipeline.clone(),
            close_tx.clone(),
            metadata.cloned(),
        ));

        let frame_tx =
            self.frame_tx.as_ref().cloned().ok_or_else(|| {
                GStreamerError::PipelineError("Frame channel not initialized".into())
            })?;
        let device = self.get_device()?;
        let handle = StreamHandle {
            close_tx,
            frame_tx,
            task: Some(pipeline_task),
            pipeline,
            device,
        };
        self.handle = Some(handle);
        Ok(())
    }

    fn get_device(&self) -> Result<GstMediaDevice, GStreamerError> {
        match &self.publish_options {
            PublishOptions::Video(video_options) => {
                GstMediaDevice::from_device_path(video_options.device_id.as_str())
            }
            PublishOptions::Audio(audio_options) => {
                GstMediaDevice::from_device_path(audio_options.device_id.as_str())
            }
            PublishOptions::Screen(screen_options) => {
                GstMediaDevice::from_screen_id_or_name(&screen_options.screen_id_or_name)
            }
        }
    }

    pub async fn build_pipeline(
        &mut self,
    ) -> Result<(Pipeline, Option<RecordingMetadata>), GStreamerError> {
        let device = match &self.publish_options {
            PublishOptions::Video(video_options) => {
                GstMediaDevice::from_device_path(video_options.device_id.as_str())?
            }
            PublishOptions::Audio(audio_options) => {
                GstMediaDevice::from_device_path(audio_options.device_id.as_str())?
            }
            PublishOptions::Screen(screen_options) => {
                GstMediaDevice::from_screen_id_or_name(&screen_options.screen_id_or_name)?
            }
        };

        let mut metadata = None;
        let (frame_tx, _) = broadcast::channel::<Arc<Buffer>>(1);
        let frame_tx_arc = Arc::new(frame_tx.clone());
        let pipeline = match &self.publish_options {
            PublishOptions::Video(video_options) => {
                let mut filename = None;
                if let Some(local_file_save_options) = &video_options.local_file_save_options {
                    let op_dir = create_dir(local_file_save_options).await?;
                    let filename_str = format!(
                        "{}-{}-{}-{}.mp4",
                        "video",
                        device.display_name.replace(" ", "_"),
                        strict_sanitize_filename(&video_options.device_id),
                        chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
                    );

                    metadata = Some(RecordingMetadata::new(
                        filename_str.clone(),
                        path::absolute(&op_dir)
                            .unwrap()
                            .to_string_lossy()
                            .to_string(),
                        "camera".into(),
                        "video".into(),
                        video_options.codec.clone(),
                        None, // No audio channel for video,
                        Some(device.display_name.clone()),
                    ));

                    filename = Some(op_dir.join(filename_str).to_string_lossy().to_string());
                }
                device.video_pipeline(
                    &video_options.codec,
                    video_options.width,
                    video_options.height,
                    video_options.framerate,
                    frame_tx_arc.clone(),
                    filename,
                )?
            }
            PublishOptions::Audio(audio_options) => {
                let mut filename = None;
                if let Some(local_file_save_options) = &audio_options.local_file_save_options {
                    let op_dir = create_dir(local_file_save_options).await?;
                    let filename_str = format!(
                        "{}-{}-{}-{}.m4a",
                        "audio",
                        match audio_options.selected_channel {
                            Some(channel) => format!(
                                "{}-{}",
                                strict_sanitize_filename(&device.display_name),
                                channel
                            ),
                            None => strict_sanitize_filename(&device.display_name),
                        },
                        strict_sanitize_filename(&audio_options.device_id),
                        chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
                    );

                    metadata = Some(RecordingMetadata::new(
                        filename_str.clone(),
                        path::absolute(&op_dir)
                            .unwrap()
                            .to_string_lossy()
                            .to_string(),
                        "microphone".into(),
                        "audio".into(),
                        audio_options.codec.clone(),
                        audio_options.selected_channel,
                        Some(device.display_name.clone()),
                    ));

                    filename = Some(op_dir.join(filename_str).to_string_lossy().to_string());
                }
                match audio_options.selected_channel {
                    Some(selected_channel) => device.deinterleaved_audio_pipeline(
                        &audio_options.codec,
                        audio_options.channels,
                        selected_channel,
                        audio_options.framerate,
                        frame_tx_arc.clone(),
                        filename,
                    )?,
                    None => device.audio_pipeline(
                        &audio_options.codec,
                        audio_options.channels,
                        audio_options.framerate,
                        frame_tx_arc.clone(),
                        filename,
                    )?,
                }
            }
            PublishOptions::Screen(screen_options) => {
                let mut filename = None;
                if let Some(local_file_save_options) = &screen_options.local_file_save_options {
                    let op_dir = create_dir(local_file_save_options).await?;
                    let filename_str = format!(
                        "{}-{}-{}.mp4",
                        "screen-share",
                        screen_options.screen_id_or_name.replace(" ", "_"),
                        chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
                    );

                    metadata = Some(RecordingMetadata::new(
                        filename_str.clone(),
                        path::absolute(&op_dir)
                            .unwrap()
                            .to_string_lossy()
                            .to_string(),
                        "screen-share".into(),
                        "video".into(),
                        screen_options.codec.clone(),
                        None,
                        Some(device.display_name.clone()),
                    ));
                    filename = Some(op_dir.join(filename_str).to_string_lossy().to_string());
                }
                device.screen_share_pipeline(
                    &screen_options.codec,
                    screen_options.width,
                    screen_options.height,
                    screen_options.framerate,
                    frame_tx_arc.clone(),
                    filename,
                )?
            }
        };

        self.frame_tx = Some(frame_tx);
        self.pipeline = Some(pipeline.clone());
        self.metadata = metadata.clone();
        Ok((pipeline, metadata))
    }

    pub async fn start_with_clock(
        &mut self,
        clock: gstreamer::Clock,
        base_time: gstreamer::ClockTime,
    ) -> Result<(), GStreamerError> {
        self.stop().await?;
        let (close_tx, _) = broadcast::channel::<()>(1);

        let device = match &self.publish_options {
            PublishOptions::Video(video_options) => {
                GstMediaDevice::from_device_path(video_options.device_id.as_str())?
            }
            PublishOptions::Audio(audio_options) => {
                GstMediaDevice::from_device_path(audio_options.device_id.as_str())?
            }
            PublishOptions::Screen(screen_options) => {
                GstMediaDevice::from_screen_id_or_name(&screen_options.screen_id_or_name)?
            }
        };

        let (pipeline, mut metadata) = self.build_pipeline().await?;

        preroll_pipeline(&pipeline).await?;
        set_pipeline_clock(&pipeline, &clock, base_time, metadata.as_mut())?;
        play_pipeline(&pipeline, metadata.as_mut())?;

        let pipeline_task = tokio::spawn(run_bus_loop(
            pipeline.clone(),
            close_tx.clone(),
            metadata.clone(),
        ));

        let frame_tx =
            self.frame_tx.as_ref().cloned().ok_or_else(|| {
                GStreamerError::PipelineError("Frame channel not initialized".into())
            })?;

        let handle = StreamHandle {
            close_tx,
            frame_tx,
            task: Some(pipeline_task),
            pipeline,
            device,
        };
        self.handle = Some(handle);
        Ok(())
    }

    pub async fn start(&mut self) -> Result<(), GStreamerError> {
        self.stop().await?;

        let (close_tx, _) = broadcast::channel::<()>(1);

        let device = match &self.publish_options {
            PublishOptions::Video(video_options) => {
                GstMediaDevice::from_device_path(video_options.device_id.as_str())?
            }
            PublishOptions::Audio(audio_options) => {
                GstMediaDevice::from_device_path(audio_options.device_id.as_str())?
            }
            PublishOptions::Screen(screen_options) => {
                GstMediaDevice::from_screen_id_or_name(&screen_options.screen_id_or_name)?
            }
        };

        let (pipeline, mut metadata) = self.build_pipeline().await?;

        preroll_pipeline(&pipeline).await?;
        let clock = gstreamer::SystemClock::obtain();
        let base_time = clock.time();
        set_pipeline_clock(&pipeline, &clock, base_time, metadata.as_mut())?;
        play_pipeline(&pipeline, metadata.as_mut())?;

        let pipeline_task = tokio::spawn(run_bus_loop(
            pipeline.clone(),
            close_tx.clone(),
            metadata.clone(),
        ));

        let frame_tx =
            self.frame_tx.as_ref().cloned().ok_or_else(|| {
                GStreamerError::PipelineError("Frame channel not initialized".into())
            })?;

        let handle = StreamHandle {
            close_tx,
            frame_tx,
            task: Some(pipeline_task),
            pipeline,
            device,
        };
        self.handle = Some(handle);

        Ok(())
    }

    pub fn subscribe(&self) -> Option<(broadcast::Receiver<Arc<Buffer>>, broadcast::Receiver<()>)> {
        self.handle
            .as_ref()
            .map(|h| (h.frame_tx.subscribe(), h.close_tx.subscribe()))
    }

    pub fn details(&self) -> Option<PublishOptions> {
        self.handle.as_ref().map(|_| self.publish_options.clone())
    }

    pub fn get_device_name(&self) -> Option<String> {
        self.handle.as_ref().map(|h| h.device.display_name.clone())
    }
}

impl Drop for GstMediaStream {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle
                .pipeline
                .set_state(gstreamer::State::Null)
                .map_err(|_| GStreamerError::PipelineError("Failed to stop pipeline".into()));
        }
    }
}
