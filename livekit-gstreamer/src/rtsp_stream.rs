use std::sync::Arc;

use gstreamer::prelude::*;
use gstreamer::{Buffer, Pipeline};
use tokio::sync::broadcast;

use crate::{
    run_pipeline, AudioPublishOptions, VideoPublishOptions, LocalFileSaveOptions, PublishOptions, RecordingMetadata,
};
use crate::GStreamerError;

fn build_video_pipeline_string(
    rtsp_location: &str,
    width: i32,
    height: i32,
    _framerate: i32,
    output_file: &Option<String>,
) -> String {
    // Adaptive pipeline - let GStreamer negotiate the best format
    let mut pipeline = format!(
        "rtspsrc location={} latency=0 drop-on-latency=true protocols=tcp name=rtspsrc0 ! \
         decodebin ! videoconvert ! \
         videoscale ! video/x-raw,width={},height={} ! \
         tee name=t",
        rtsp_location, width, height
    );

    // Add appsink branch with I420 format for LiveKit compatibility
    pipeline.push_str(" t. ! queue max-size-buffers=3 leaky=downstream ! \
                       videoconvert ! video/x-raw,format=I420 ! \
                       appsink name=appsink emit-signals=true sync=false max-buffers=3 drop=true");

    // Add optional file output branch
    if let Some(file) = output_file {
        pipeline.push_str(&format!(
            " t. ! queue ! videoconvert ! x264enc tune=zerolatency speed-preset=ultrafast ! \
             mp4mux ! filesink location={}",
            file
        ));
    }

    println!("Built adaptive video pipeline: {}", pipeline);
    pipeline
}

fn build_simple_video_pipeline_string(
    rtsp_location: &str,
    output_file: &Option<String>,
) -> String {
    // Minimal pipeline that just passes through whatever format the stream provides
    let mut pipeline = format!(
        "rtspsrc location={} latency=0 protocols=tcp name=rtspsrc0 ! \
         decodebin ! videoconvert ! \
         tee name=t",
        rtsp_location
    );

    // Add appsink branch with I420 format for LiveKit compatibility
    pipeline.push_str(" t. ! queue ! videoconvert ! video/x-raw,format=I420 ! \
                       appsink name=appsink emit-signals=true sync=false max-buffers=5 drop=true");

    // Add optional file output branch
    if let Some(file) = output_file {
        pipeline.push_str(&format!(
            " t. ! queue ! videoconvert ! x264enc tune=zerolatency ! \
             mp4mux ! filesink location={}",
            file
        ));
    }

    println!("Built simple video pipeline: {}", pipeline);
    pipeline
}

fn build_audio_pipeline_string(
    rtsp_location: &str,
    framerate: i32,
    channels: i32,
    output_file: &Option<String>,
) -> String {
    let mut pipeline = format!(
        "rtspsrc location={} latency=0 name=rtspsrc0 ! decodebin ! audioconvert ! audioresample ! \
         audio/x-raw,format=S16LE,channels={},rate={} ! tee name=t",
        rtsp_location, channels, framerate
    );

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


pub struct RtspStreamHandle {
    close_tx: broadcast::Sender<()>,
    frame_tx: broadcast::Sender<Arc<Buffer>>,
    task: tokio::task::JoinHandle<Result<(), GStreamerError>>,
    pipeline: Pipeline,
}

pub struct RtspStream {
    pub rtsp_location: String,
    pub stream_type: RtspStreamType,
    pub outdir: Option<String>,
    pub handle: Option<RtspStreamHandle>,
}

#[derive(Clone, Debug)]
pub enum RtspStreamType {
    Video {
        width: i32,
        height: i32,
        framerate: i32,
    },
    Audio {
        framerate: i32,
        channels: i32,
    },
}

impl RtspStream {
    pub fn new(
        rtsp_location: String,
        stream_type: RtspStreamType,
        outdir: Option<String>,
    ) -> Self {
        Self {
            rtsp_location,
            stream_type,
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
            let (file_ext, media_type, codec) = match &self.stream_type {
                RtspStreamType::Video { .. } => ("mp4", "video", "video/x-raw"),
                RtspStreamType::Audio { .. } => ("m4a", "audio", "audio/x-raw"),
            };
            filename = self.outdir.as_ref().map(|dir| {
                format!(
                    "{}/recording_rtsp_{}_{}.{}",
                    dir,
                    self.rtsp_location.replace("://", "_").replace("/", "_").replace(":", "_"),
                    chrono::Local::now().format("%Y%m%d_%H%M%S"),
                    file_ext
                )
            });

            metadata = Some(RecordingMetadata::new(
                filename.clone().unwrap_or_default(),
                dir.clone(),
                "rtsp_stream".to_string(),
                media_type.to_string(),
                codec.to_string(),
                Some(1),
                Some(self.rtsp_location.clone()),
            ));
        }
        
        println!("Creating RTSP pipeline for location: {}", self.rtsp_location);
        let pipeline = self.get_rtsp_pipeline(&filename);
        
        // Set pipeline to PAUSED state first to check for errors
        match pipeline.set_state(gstreamer::State::Paused) {
            Ok(_) => println!("Pipeline state change initiated"),
            Err(e) => {
                return Err(GStreamerError::PipelineError(format!("Failed to set pipeline state: {:?}", e)));
            }
        }
        
        // Wait for the state change to complete or fail
        let (state_change_result, current_state, pending_state) = pipeline.state(Some(gstreamer::ClockTime::from_seconds(10)));
        match state_change_result {
            Ok(_) if current_state == gstreamer::State::Paused => {
                println!("Pipeline successfully reached PAUSED state");
            }
            Ok(_) => {
                println!("Pipeline state change async, waiting...");
                // Wait a bit more for async state changes and check for messages
                let bus = pipeline.bus().unwrap();
                if let Some(msg) = bus.timed_pop_filtered(
                    Some(gstreamer::ClockTime::from_seconds(10)),
                    &[gstreamer::MessageType::StateChanged, gstreamer::MessageType::Error, gstreamer::MessageType::Warning]
                ) {
                    match msg.view() {
                        gstreamer::MessageView::Error(err) => {
                            println!("Pipeline error: {} ({})", err.error(), err.debug().unwrap_or_default());
                        }
                        gstreamer::MessageView::Warning(warn) => {
                            println!("Pipeline warning: {} ({})", warn.error(), warn.debug().unwrap_or_default());
                        }
                        gstreamer::MessageView::StateChanged(state_changed) => {
                            println!("State changed from {:?} to {:?}", state_changed.old(), state_changed.current());
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                println!("Pipeline state change failed: {:?}, current: {:?}, pending: {:?}", 
                          e, current_state, pending_state);
            }
        }

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
                        Err(e) => {
                            eprintln!("Failed to pull sample from appsink: {:?}", e);
                            return Err(gstreamer::FlowError::Eos);
                        }
                    };

                    let buffer = sample.buffer().ok_or(gstreamer::FlowError::Error)?;
                    
                    // Get caps info for debugging
                    let caps = sample.caps();
                    
                    // Only print every 30th frame to avoid spam
                    static mut FRAME_COUNT: u64 = 0;
                    static mut CAPS_PRINTED: bool = false;
                    unsafe {
                        FRAME_COUNT += 1;
                        if !CAPS_PRINTED {
                            if let Some(caps) = caps {
                                println!("Video caps: {}", caps);
                            }
                            CAPS_PRINTED = true;
                        }
                        if FRAME_COUNT % 30 == 0 {
                            println!("Received frame #{}: {} bytes, pts: {:?}", 
                                      FRAME_COUNT,
                                      buffer.size(),
                                      buffer.pts().map(|pts| pts.useconds()));
                        }
                    }

                    if frame_tx_arc_clone.receiver_count() > 0 {
                        let _ = frame_tx_arc_clone.send(Arc::new(buffer.copy()));
                    } else if unsafe { FRAME_COUNT % 100 == 0 } {
                        println!("No receivers for frame data (frame #{})", unsafe { FRAME_COUNT });
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

        let handle = RtspStreamHandle {
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

    pub fn get_stream_location(&self) -> String {
        self.rtsp_location.clone()
    }

    pub fn details(&self) -> Option<PublishOptions> {
        if self.has_started() {
            match &self.stream_type {
                RtspStreamType::Video { width, height, framerate } => {
                    Some(PublishOptions::Video(VideoPublishOptions {
                        codec: "video/x-raw".to_string(),
                        device_id: self.rtsp_location.clone(),
                        width: *width,
                        height: *height,
                        framerate: *framerate,
                        local_file_save_options: self.outdir.as_ref().map(|dir| LocalFileSaveOptions {
                            output_dir: dir.clone(),
                        }),
                    }))
                }
                RtspStreamType::Audio { framerate, channels } => {
                    Some(PublishOptions::Audio(AudioPublishOptions {
                        codec: "audio/x-raw".to_string(),
                        device_id: self.rtsp_location.clone(),
                        framerate: *framerate,
                        channels: *channels,
                        selected_channel: None,
                        local_file_save_options: self.outdir.as_ref().map(|dir| LocalFileSaveOptions {
                            output_dir: dir.clone(),
                        }),
                    }))
                }
            }
        } else {
            None
        }
    }

    fn get_rtsp_pipeline(&self, filename: &Option<String>) -> Pipeline {
        let pipeline_str = match &self.stream_type {
            RtspStreamType::Video { width, height, framerate } => {
                // Try the full pipeline first
                let full_pipeline = build_video_pipeline_string(
                    &self.rtsp_location,
                    *width,
                    *height,
                    *framerate,
                    filename,
                );
                
                // Try to parse the pipeline - if it fails, fall back to simple
                match gstreamer::parse::launch(&full_pipeline) {
                    Ok(pipeline) => return pipeline.downcast::<gstreamer::Pipeline>()
                        .expect("Failed to downcast to Pipeline"),
                    Err(e) => {
                        println!("Failed to create complex pipeline: {:?}, trying simple pipeline", e);
                        build_simple_video_pipeline_string(&self.rtsp_location, filename)
                    }
                }
            }
            RtspStreamType::Audio { framerate, channels } => {
                build_audio_pipeline_string(
                    &self.rtsp_location,
                    *framerate,
                    *channels,
                    filename,
                )
            }
        };
        
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
