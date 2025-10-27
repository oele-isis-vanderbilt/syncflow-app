use std::sync::Arc;

use gstreamer::prelude::*;
use gstreamer::{Buffer, Pipeline};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    run_pipeline, AudioPublishOptions, LocalFileSaveOptions, PublishOptions, RecordingMetadata,
};
use crate::{utils::random_string, GStreamerError};

fn build_pipeline_string(
    device_id: &str,
    framerate: i32,
    channels: i32,
    selected_channels: &[i32],
    audio_format: &str,
    output_file: &Option<String>,
) -> String {
    // Main pipeline with mixer output going to tee
    let mut pipeline = format!(
        "alsasrc device={} name=alsasrc0 ! audio/x-raw,format={},channels={},rate={} ! deinterleave name=di \
         audiomixer name=mix ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,channels=1,rate={} ! tee name=t",
        device_id, audio_format, channels, framerate, framerate
    );

    // Link selected channels to mixer
    for &ch in selected_channels {
        pipeline.push_str(&format!(
            " di.src_{} ! queue ! audioconvert ! audioresample ! mix.",
            ch
        ));
    }

    // Add appsink branch
    pipeline.push_str(" t. ! queue ! appsink name=appsink emit-signals=true sync=false");

    // Add optional file output branch
    if let Some(file) = output_file {
        pipeline.push_str(&format!(
            " t. ! queue ! audioconvert ! avenc_aac ! mp4mux ! filesink location={}",
            file
        ));
    }

    pipeline
}

pub struct AlsaMultiChannelMixStreamHandle {
    close_tx: broadcast::Sender<()>,
    frame_tx: broadcast::Sender<Arc<Buffer>>,
    task: tokio::task::JoinHandle<Result<(), GStreamerError>>,
    pipeline: Pipeline,
}

pub struct AlsaMultiChannelMixStream {
    pub device_id: String,
    pub framerate: i32,
    pub channels: i32,
    pub selected_channels: Vec<i32>,
    pub outdir: Option<String>,
    pub audio_format: String,
    pub handle: Option<AlsaMultiChannelMixStreamHandle>,
}

impl AlsaMultiChannelMixStream {
    pub fn new(
        device_id: String,
        framerate: i32,
        channels: i32,
        selected_channels: Vec<i32>,
        audio_format: String,
        outdir: Option<String>,
    ) -> Self {
        Self {
            device_id,
            framerate,
            channels,
            selected_channels,
            audio_format,
            outdir,
            handle: None,
        }
    }

    pub async fn start(&mut self) -> Result<(), GStreamerError> {
        self.stop().await?;
        let mut filename = None;
        let mut metadata = None;
        if let Some(dir) = &self.outdir {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("Failed to create output directory {}: {}", dir, e);
            }
            filename = self.outdir.as_ref().map(|dir| {
                format!(
                    "{}/recording_{}_channels-{}_{}.m4a",
                    dir,
                    self.device_id.replace("/", "_"),
                    self.selected_channels
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join("-"),
                    chrono::Local::now().format("%Y%m%d_%H%M%S")
                )
            });

            metadata = Some(RecordingMetadata::new(
                filename.clone().unwrap_or_default(),
                dir.clone(),
                "microphone".to_string(),
                "audio".to_string(),
                "audio/x-raw".to_string(),
                Some(1),
                Some(self.device_id.clone()),
            ));
        }
        let pipeline = self.get_alsa_multi_channel_mix_pipeline(&filename);

        let (frame_tx, _) = broadcast::channel::<Arc<Buffer>>(1);
        let (close_tx, _) = broadcast::channel::<()>(1);
        let frame_tx_arc = Arc::new(frame_tx.clone());
        let appsink = pipeline
            .by_name("appsink")
            .expect("Failed to get appsink from pipeline")
            .downcast::<gstreamer_app::AppSink>()
            .expect("Failed to downcast to AppSink");
        let frame_tx_arc_clone = frame_tx_arc.clone();
        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = match sink.pull_sample() {
                        Ok(s) => s,
                        Err(_) => return Err(gstreamer::FlowError::Eos),
                    };

                    let buffer = sample.buffer().ok_or(gstreamer::FlowError::Error)?;

                    if frame_tx_arc_clone.receiver_count() > 0 {
                        let _ = frame_tx_arc_clone.send(Arc::new(buffer.copy()));
                    }
                    Ok(gstreamer::FlowSuccess::Ok)
                })
                .build(),
        );

        let pipeline_task = tokio::spawn(run_pipeline(
            pipeline.clone(),
            close_tx.clone(),
            metadata.clone(),
        ));

        let handle = AlsaMultiChannelMixStreamHandle {
            close_tx,
            frame_tx,
            task: pipeline_task,
            pipeline,
        };

        self.handle = Some(handle);

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), GStreamerError> {
        if let Some(handle) = self.handle.take() {
            handle.pipeline.send_event(gstreamer::event::Eos::new());
            let _ = handle.task.await;
        }
        self.handle = None;
        Ok(())
    }

    pub fn subscribe(&self) -> Option<(broadcast::Receiver<Arc<Buffer>>, broadcast::Receiver<()>)> {
        self.handle
            .as_ref()
            .map(|h| (h.frame_tx.subscribe(), h.close_tx.subscribe()))
    }

    pub fn get_device_name(&self) -> Option<String> {
        self.handle.as_ref().and_then(|h| {
            let device = h
                .pipeline
                .by_name("alsasrc0")
                .and_then(|e| e.property::<Option<String>>("device-name"));
            device
        })
    }

    pub fn details(&self) -> Option<AudioPublishOptions> {
        if self.has_started() {
            Some(AudioPublishOptions {
                codec: "audio/x-raw".to_string(),
                device_id: self.device_id.clone(),
                framerate: self.framerate,
                channels: self.channels,
                selected_channel: None,
                local_file_save_options: self.outdir.as_ref().map(|dir| LocalFileSaveOptions {
                    output_dir: dir.clone(),
                }),
            })
        } else {
            None
        }
    }

    fn get_alsa_multi_channel_mix_pipeline(&self, filename: &Option<String>) -> Pipeline {
        let pipeline_str = build_pipeline_string(
            &self.device_id,
            self.framerate,
            self.channels,
            &self.selected_channels,
            &self.audio_format,
            filename,
        );
        let pipeline = gstreamer::parse::launch(&pipeline_str)
            .expect("Failed to create GStreamer pipeline")
            .downcast::<gstreamer::Pipeline>()
            .expect("Failed to downcast to Pipeline");

        pipeline
    }

    pub fn has_started(&self) -> bool {
        self.handle.is_some()
    }
}
