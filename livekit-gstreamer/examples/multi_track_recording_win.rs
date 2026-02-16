use livekit_gstreamer::GStreamerError;

fn main() -> Result<(), GStreamerError> {
    // Only run on Windows for now
    if !cfg!(target_os = "windows") {
        panic!("This example is only supported on Windows");
    }

    gstreamer::init().map_err(|e| {
        GStreamerError::PipelineError(format!("Failed to initialize gstreamer: {}", e))
    })?;
    use livekit_gstreamer::single_container_pipeline::SingleContainerPipeline;
    use livekit_gstreamer::single_container_pipeline::{
        AudioTrackConfig, ScreenTrackConfig, VideoTrackConfig,
    };

    gstreamer::init().map_err(|e| {
        GStreamerError::PipelineError(format!("Failed to initialize gstreamer: {}", e))
    })?;

    let video_config = vec![
        VideoTrackConfig {
            device_id: r"\\\\?\\usb#vid_1c45&pid_6200&mi_00#6&1b461b34&0&0000#{6994ad05-93ef-11d0-a3cc-00a0c9223196}\\global".to_string(),
            device_name: Some("HD Pro Webcam C920".to_string()),
            width: 1920,
            height: 1080,
            framerate: 30,
            bitrate: 4000000,
            preset: "ultrafast".to_string(),
            camera_codec: "image/jpeg".to_string(),
        }
    ];

    let screen_config = vec![ScreenTrackConfig {
        device_id: "screen:0".to_string(),
        device_name: Some("Screen Capture".to_string()),
        width: 1920,
        height: 1080,
        framerate: 30,
        bitrate: 4000000,
        preset: "ultrafast".to_string(),
    }];

    let audio_tracks = vec![
        AudioTrackConfig {
            device_id: r"{0.0.1.00000000}.{632c169a-b754-45ef-86f5-ed6b73e606c3}".to_string(),
            device_name: Some("Microphone (USB Audio Device)".to_string()),
            bitrate: 128000,
            sample_rate: 48000,
            channels: 1,
        },
        AudioTrackConfig {
            device_id: r"{0.0.1.00000000}.{d1e05b75-1b7c-442e-bdc8-e15fe7fbe710}".to_string(),
            device_name: Some("Microphone (USB Audio Device)".to_string()),
            bitrate: 128000,
            sample_rate: 48000,
            channels: 1,
        },
    ];

    let mut pipeline = SingleContainerPipeline::new(
        "test_recording".to_string(),
        video_config,
        screen_config,
        audio_tracks,
        "output".to_string(),
    );

    pipeline
        .initialize()
        .expect("Failed to initialize pipeline");

    // Sleep 1 minute to check filename
    pipeline.start().expect("Failed to start pipeline");

    // Wait for user input to stop the pipeline
    println!("Press Enter to stop recording...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    if let Err(e) = pipeline.stop() {
        eprintln!("Failed to stop pipeline: {}", e);
    } else {
        println!("Pipeline stopped successfully");
    }

    Ok(())
}
