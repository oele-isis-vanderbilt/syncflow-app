use std::sync::Arc;

use crate::{
    create_dir, play_pipeline, preroll_pipeline, run_bus_loop, set_pipeline_clock,
    utils::random_string, AudioPublishOptions, GStreamerError, GstMediaDevice, RecordingMetadata,
    VideoPublishOptions,
};
use gstreamer::{prelude::ElementExtManual, Buffer, Pipeline};
use std::path;
use tokio::sync::broadcast;

#[derive(Debug)]
pub struct AvMixStream {
    pub video_publish_options: VideoPublishOptions,
    pub mic1: AudioPublishOptions,
    pub mic2: Option<AudioPublishOptions>,
    pipeline: Option<Pipeline>,
    video_frame_tx: Option<broadcast::Sender<Arc<Buffer>>>,
    audio_frame_tx: Option<broadcast::Sender<Arc<Buffer>>>,
    metadata: Option<RecordingMetadata>,
    handle: Option<StreamHandle>,
}

#[derive(Debug)]
struct StreamHandle {
    close_tx: broadcast::Sender<()>,
    video_frame_tx: broadcast::Sender<Arc<Buffer>>,
    audio_frame_tx: broadcast::Sender<Arc<Buffer>>,
    task: Option<tokio::task::JoinHandle<Result<(), GStreamerError>>>,
    pipeline: Pipeline,
}

impl AvMixStream {
    pub fn new(
        video_publish_options: VideoPublishOptions,
        mic1: AudioPublishOptions,
        mic2: Option<AudioPublishOptions>,
    ) -> Self {
        Self {
            video_publish_options,
            mic1,
            mic2,
            pipeline: None,
            video_frame_tx: None,
            audio_frame_tx: None,
            metadata: None,
            handle: None,
        }
    }

    fn get_video_device(&self) -> Result<GstMediaDevice, GStreamerError> {
        let device = GstMediaDevice::from_device_path(&self.video_publish_options.device_id)?;

        Ok(device)
    }

    fn get_audio_devices(&self) -> Result<Vec<GstMediaDevice>, GStreamerError> {
        let mut devices = vec![GstMediaDevice::from_device_path(&self.mic1.device_id)?];

        if let Some(mic2) = &self.mic2 {
            devices.push(GstMediaDevice::from_device_path(&mic2.device_id)?);
        }

        Ok(devices)
    }
}

impl AvMixStream {
    pub fn kind(&self) -> String {
        "av-mix".into()
    }

    pub fn display_name(&self) -> String {
        "AV Mix Stream".to_string()
    }

    pub fn has_started(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| h.task.is_some())
    }

    pub fn get_audio_publish_options(&self) -> AudioPublishOptions {
        let mut options = self.mic1.clone();
        options.channels = 1;
        options
    }

    pub fn get_video_publish_options(&self) -> VideoPublishOptions {
        self.video_publish_options.clone()
    }

    pub async fn build_pipeline(&mut self) -> Result<(), GStreamerError> {
        let video_device = self.get_video_device()?;
        let audio_devices = self.get_audio_devices()?;

        let pipeline = gstreamer::Pipeline::with_name(&random_string("av-mix"));

        let (video_tx, _) = broadcast::channel::<Arc<Buffer>>(1);
        let (audio_tx, _) = broadcast::channel::<Arc<Buffer>>(1);

        // Step 1: Add video
        let video_handles = video_device.add_video_to_pipeline(
            &pipeline,
            &self.video_publish_options,
            Arc::new(video_tx.clone()),
        )?;

        // Step 2: Add audio (1 or 2 mics, mixed)
        let mic_pairs: Vec<(&GstMediaDevice, &AudioPublishOptions)> = {
            let mut pairs = vec![(&audio_devices[0], &self.mic1)];
            if let Some(ref mic2_opts) = self.mic2 {
                if audio_devices.len() > 1 {
                    pairs.push((&audio_devices[1], mic2_opts));
                }
            }
            pairs
        };

        let audio_handles = GstMediaDevice::add_audio_to_pipeline(
            &mic_pairs,
            &pipeline,
            Arc::new(audio_tx.clone()),
        )?;

        // Step 3: File branch (single mp4 with both tracks)
        let mut metadata = None;
        if let Some(ref save_opts) = self.video_publish_options.local_file_save_options {
            let op_dir = create_dir(save_opts).await?;
            let filename = format!(
                "av-mix-{}.mp4",
                chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
            );
            let path = op_dir.join(&filename).to_string_lossy().to_string();

            let file_handles = GstMediaDevice::add_av_file_branch(
                &pipeline,
                &video_handles.tee,
                &audio_handles.tee,
                &path,
                self.video_publish_options.framerate,
            )?;

            // Attach muxer probes for timing
            // (store timing arc in self for later metadata extraction)
            // self.attach_muxer_probes(&file_handles.video_mux_pad, &file_handles.audio_mux_pad);

            let video_device_name = video_device.display_name.clone();
            let audio_deivice_names = audio_devices
                .iter()
                .map(|d| d.display_name.clone())
                .collect::<Vec<String>>();

            let composite_device_name =
                format!("{}-{}", video_device_name, audio_deivice_names.join("-"));

            metadata = Some(RecordingMetadata::new(
                filename.clone(),
                path::absolute(&op_dir)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                "screen-share".into(),
                "video".into(),
                "mp4mux".into(),
                Some(audio_handles.num_channels),
                Some(composite_device_name),
            ));
        }

        self.pipeline = Some(pipeline);
        self.video_frame_tx = Some(video_tx);
        self.audio_frame_tx = Some(audio_tx);
        self.metadata = metadata;

        Ok(())
    }

    pub async fn preroll_pipeline(&self) -> Result<(), GStreamerError> {
        let pipeline = self.pipeline.clone().ok_or(GStreamerError::PipelineError(
            "Please call build pipeline first".into(),
        ))?;

        preroll_pipeline(&pipeline).await?;
        Ok(())
    }

    pub fn play_pipeline(&mut self) -> Result<(), GStreamerError> {
        let pipeline = self.pipeline.clone().ok_or(GStreamerError::PipelineError(
            "Please call build pipeline first".into(),
        ))?;
        play_pipeline(&pipeline, self.metadata.as_mut())?;
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

    pub async fn run_bus_loop(&mut self) -> Result<(), GStreamerError> {
        let pipeline = self.pipeline.clone().ok_or(GStreamerError::PipelineError(
            "Please call build pipeline first".into(),
        ))?;

        let (close_tx, _) = broadcast::channel::<()>(1);
        let pipeline_task = tokio::spawn(run_bus_loop(
            pipeline.clone(),
            close_tx.clone(),
            self.metadata.clone(),
        ));

        let video_frame_tx = self
            .video_frame_tx
            .clone()
            .ok_or(GStreamerError::PipelineError(
                "Please call build pipeline first".into(),
            ))?;
        let audio_frame_tx = self
            .audio_frame_tx
            .clone()
            .ok_or(GStreamerError::PipelineError(
                "Please call build pipeline first".into(),
            ))?;
        let handle = StreamHandle {
            close_tx,
            audio_frame_tx,
            video_frame_tx,
            task: Some(pipeline_task),
            pipeline,
        };
        self.handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), GStreamerError> {
        if let Some(handle) = self.handle.take() {
            handle.pipeline.send_event(gstreamer::event::Eos::new());
            if let Some(task) = handle.task {
                let _ = task.await;
            }
        }
        self.handle = None;
        Ok(())
    }

    pub fn subscribe_video(
        &self,
    ) -> Option<(broadcast::Receiver<Arc<Buffer>>, broadcast::Receiver<()>)> {
        self.handle
            .as_ref()
            .map(|h| (h.video_frame_tx.subscribe(), h.close_tx.subscribe()))
    }

    pub fn subscribe_audio(
        &self,
    ) -> Option<(broadcast::Receiver<Arc<Buffer>>, broadcast::Receiver<()>)> {
        self.handle
            .as_ref()
            .map(|h| (h.audio_frame_tx.subscribe(), h.close_tx.subscribe()))
    }

    pub fn num_audio_channels(&self) -> Option<i32> {
        self.metadata.as_ref().and_then(|m| m.audio_channel)
    }
}
