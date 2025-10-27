use livekit_gstreamer::{
    AlsaMultiChannelMixStream, AudioPublishOptions, GStreamerError, GstMediaStream,
    LocalFileSaveOptions, PublishOptions,
};

#[path = "./helper/wait.rs"]
mod wait;

#[tokio::main]
async fn main() -> Result<(), GStreamerError> {
    gstreamer::init().map_err(|e| {
        GStreamerError::PipelineError(format!("Failed to initialize gstreamer: {}", e))
    })?;

    if !cfg!(target_os = "linux") {
        panic!("This example is only supported on Linux");
    }

    let mut stream = AlsaMultiChannelMixStream::new(
        "hw:1,0".to_string(),
        96000,
        2,
        vec![0, 1],
        "S32LE".to_string(),
        Some("recordings-alsa-multi-channel-mix".to_string()),
    );

    stream.start().await?;

    let (frame_rx, close_rx) = stream.subscribe().unwrap();

    wait::wait_alsa_mixstreams(&mut [stream], vec![frame_rx], vec![close_rx]).await
}
