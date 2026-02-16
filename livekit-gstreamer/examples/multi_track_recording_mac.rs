use livekit_gstreamer::GStreamerError;

fn main() -> Result<(), GStreamerError> {
    // Only run on Windows for now
    if !cfg!(target_os = "macos") {
        panic!("This example is only supported on MacOs");
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

    let video_config = vec![VideoTrackConfig {
        device_id: r"FDF90FEB-59E5-4FCF-AABD-DA03C4E19BFB".to_string(),
        device_name: Some("FaceTime HD Camera".to_string()),
        width: 1920,
        height: 1080,
        framerate: 30,
        bitrate: 4000000,
        preset: "ultrafast".to_string(),
        camera_codec: "video/x-raw".to_string(),
    }];

    let screen_config = vec![ScreenTrackConfig {
        device_id: "1".to_string(),
        device_name: Some("Screen Capture".to_string()),
        width: 1470,
        height: 956,
        framerate: 30,
        bitrate: 4000000,
        preset: "ultrafast".to_string(),
    }];

    let audio_tracks = vec![
        AudioTrackConfig {
            device_id:
                "AppleUSBAudioEngine:C-Media Electronics Inc.      :USB PnP Sound Device:100000:1"
                    .to_string(),
            device_name: Some("Microphone (USB Audio Device)".to_string()),
            bitrate: 128000,
            sample_rate: 48000,
            channels: 1,
        },
        AudioTrackConfig {
            device_id:
                "AppleUSBAudioEngine:C-Media Electronics Inc.      :USB PnP Sound Device:1100000:1"
                    .to_string(),
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
    std::thread::sleep(std::time::Duration::from_secs(5));
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
