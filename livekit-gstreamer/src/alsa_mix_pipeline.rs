use std::path::PathBuf;
use std::sync::Arc;

use gstreamer::{glib::object::Cast, prelude::GstBinExt};
use gstreamer::{prelude::*, Buffer};
use tokio::sync::broadcast;

use crate::{
    run_pipeline, AudioPublishOptions, GStreamerError, LocalFileSaveOptions, PublishOptions,
    RecordingMetadata,
};

#[derive(Debug)]
struct UMC1820StreamHandle {
    close_tx: broadcast::Sender<()>,
    frame_tx: broadcast::Sender<Arc<Buffer>>,
    task: tokio::task::JoinHandle<Result<(), GStreamerError>>,
    pipeline: gstreamer::Pipeline,
}

pub struct UMC1820MixStream {
    device_id: String,
    framerate: u32,
    total_channels: usize,
    mix_channels: Vec<usize>,
    opdir: PathBuf,
    handle: Option<UMC1820StreamHandle>,
}

fn get_pipeline_head(device_id: &str, sample_rate: u32, total_channels: usize) -> String {
    format!(
        "alsasrc device={device_id} buffer-time=200000 latency-time=10000 ! \
         audio/x-raw,format=S32LE,rate={sample_rate},channels={total_channels},layout=interleaved ! \
         deinterleave name=di \\",
        device_id = device_id,
        sample_rate = sample_rate,
        total_channels = total_channels
    )
}

fn get_channel_and_mix(
    channel: usize,
    sample_rate: u32,
    amplification: f64,
    channel_prefix: usize,
    outdir: &PathBuf,
    mixer_name: &str,
    is_first_mixer: bool,
) -> String {
    format!(
        "di.src_{channel} ! tee name=t{channel} \\
         t{channel}. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate={sample_rate},channels=1 \\
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location={outdir}/ch{channel_prefix}.m4a \\
         t{channel}. ! queue ! audioconvert ! audioresample ! audioamplify amplification={amplification} ! {mixer_head} \\",
         mixer_head = if is_first_mixer {
            format!("audiomixer name={}", mixer_name)
         } else {
            format!("{}.", mixer_name)
         },
        channel = channel,
        sample_rate = sample_rate,
        amplification = amplification,
        channel_prefix = channel_prefix,
        outdir = outdir.to_str().unwrap(),
    )
}

fn appsink_str(sample_rate: u32, mixer_name: &str, appsink_name: &str, op_dir: &str) -> String {
    format!(
        " {mixer_name}. ! tee name=appsink_tee \\
          appsink_tee. ! queue name=file_queue ! \\
          audioconvert ! audioresample ! audio/x-raw,channels=1,rate={sample_rate},format=S16LE ! \\
          voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location={op_dir}/mix.m4a \\
          appsink_tee. ! queue name=appsink_queue ! audioconvert ! audioresample ! audio/x-raw,channels=1,rate={sample_rate},format=S16LE ! \\
          appsink name={appsink_name} emit-signals=true sync=false max-buffers=1 drop=true",
        sample_rate = sample_rate,
        mixer_name = mixer_name,
        appsink_name = appsink_name,
        op_dir = op_dir
    )
}
impl UMC1820MixStream {
    pub fn new(
        device_id: &str,
        framerate: u32,
        total_channels: usize,
        mix_channels: Vec<usize>,
        output_dir: &str,
    ) -> Self {
        Self {
            device_id: device_id.to_string(),
            framerate,
            total_channels,
            mix_channels,
            opdir: PathBuf::from(output_dir),
            handle: None,
        }
    }

    pub fn has_started(&self) -> bool {
        self.handle.is_some()
    }

    pub async fn start(&mut self) -> Result<(), GStreamerError> {
        let recordings_dir = self.opdir.join(&format!(
            "UMC1820_{}",
            chrono::Local::now().format("%Y-%m-%d-%H-%M-%S")
        ));

        std::fs::create_dir_all(&recordings_dir)
            .map_err(|e| GStreamerError::PipelineError(e.to_string()))?;

        let pipeline_head = get_pipeline_head(&self.device_id, self.framerate, self.total_channels);

        let mixer_heads = self
            .mix_channels
            .iter()
            .enumerate()
            .map(|(index, &channel)| {
                get_channel_and_mix(
                    channel,
                    self.framerate,
                    0.125,
                    channel + 1,
                    &recordings_dir,
                    "mix",
                    index == 0,
                )
            })
            .collect::<Vec<String>>();

        let appsink_name = "broadcast_appsink";

        let appsink = appsink_str(
            self.framerate,
            "mix",
            appsink_name,
            self.opdir.to_str().unwrap(),
        );

        let pipeline_str = format!(
            "#!/usr/bin/env bash\ngst-launch-1.0 -e {}\n{}\n{}",
            pipeline_head,
            mixer_heads.join("\n"),
            appsink
        );

        let pipeline_description =
            format!("{}\n{}\n{}", pipeline_head, mixer_heads.join("\n"), appsink);

        let pipeline_path = self.opdir.join("umc1820_pipeline.sh");
        std::fs::write(&pipeline_path, pipeline_str)
            .map_err(|e| GStreamerError::PipelineError(e.to_string()))?;

        let pipeline = gstreamer::parse::launch(&pipeline_description)
            .map_err(|e| GStreamerError::PipelineError(e.to_string()))?;

        let pipeline: gstreamer::Pipeline = pipeline
            .downcast()
            .map_err(|_| GStreamerError::PipelineError("Failed to cast pipeline".to_string()))?;

        let (frame_tx, _) = broadcast::channel::<Arc<Buffer>>(1);
        let (close_tx, _) = broadcast::channel::<()>(1);

        let appsink = pipeline
            .by_name(appsink_name)
            .ok_or_else(|| {
                GStreamerError::PipelineError("Appsink not found in pipeline".to_string())
            })?
            .downcast::<gstreamer_app::AppSink>()
            .map_err(|_| GStreamerError::PipelineError("Failed to cast to AppSink".to_string()))?;

        let frame_tx_arc = Arc::new(frame_tx.clone());
        let tx = frame_tx_arc.clone();
        let frame_tx_cloned = frame_tx.clone();

        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = match sink.pull_sample() {
                        Ok(s) => s,
                        Err(_) => return Err(gstreamer::FlowError::Eos),
                    };

                    let buffer = sample.buffer().ok_or(gstreamer::FlowError::Error)?;

                    if frame_tx_cloned.receiver_count() > 0 {
                        let _ = tx.send(Arc::new(buffer.copy()));
                    }
                    Ok(gstreamer::FlowSuccess::Ok)
                })
                .build(),
        );

        let recording_metadata = RecordingMetadata::new(
            "recording_details".to_string(),
            recordings_dir.to_str().unwrap().to_string(),
            format!(
                "UMC1820-microphone-channels-{}",
                self.mix_channels
                    .iter()
                    .map(|c| (c + 1).to_string())
                    .collect::<Vec<String>>()
                    .join("-")
            ),
            "audio".to_string(),
            "audio/aac".to_string(),
            None,
        );

        let pipeline_task = tokio::spawn(run_pipeline(
            pipeline.clone(),
            close_tx.clone(),
            Some(recording_metadata.clone()),
        ));

        let handle = UMC1820StreamHandle {
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

    pub fn get_details(&self) -> AudioPublishOptions {
        AudioPublishOptions {
            codec: "audio/x-raw".to_string(),
            device_id: self.device_id.clone(),
            framerate: self.framerate as i32,
            channels: 1,
            selected_channel: None,
            local_file_save_options: Some(LocalFileSaveOptions {
                output_dir: self.opdir.to_str().unwrap().to_string(),
            }),
        }
    }

    pub fn get_device_name(&self) -> String {
        "UMC 1820 Mix Stream".to_string()
    }
}
