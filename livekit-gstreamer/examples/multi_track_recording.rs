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
            width: 1920,
            height: 1080,
            framerate: 30,
            bitrate: 8000000,
            preset: "veryfast".to_string(),
            camera_codec: "image/jpeg".to_string(),
        }
    ];

    let screen_config = vec![ScreenTrackConfig {
        width: 1920,
        height: 1080,
        framerate: 30,
        bitrate: 8000000,
        preset: "veryfast".to_string(),
    }];

    let audio_tracks = vec![
        AudioTrackConfig {
            device_id: r"{0.0.1.00000000}.{7aca3bda-52f4-46ac-90a6-d8bbd9cff454}".to_string(),
            device_name: Some("Microphone (USB Audio Device)".to_string()),
            bitrate: 128000,
            sample_rate: 48000,
            channels: 1,
        },
        AudioTrackConfig {
            device_id: r"{0.0.1.00000000}.{0ef77635-3426-49fd-ae68-687626f31e83}".to_string(),
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
    std::thread::sleep(std::time::Duration::from_secs(60));
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
