use livekit_gstreamer::{
    initialize_gstreamer, AudioPublishOptions, AvMixStream, GStreamerError, LocalFileSaveOptions,
    VideoPublishOptions,
};

#[tokio::main]
async fn main() -> Result<(), GStreamerError> {
    initialize_gstreamer();
    std::env::set_var("GST_DEBUG", "3");

    let mic1 = AudioPublishOptions {
        #[cfg(target_os = "windows")]
        device_id: "{0.0.1.00000000}.{be006906-f26f-4a69-ac72-e0216303e6cc}".into(),
        #[cfg(target_os = "macos")]
        device_id: r"AppleUSBAudioEngine:C-Media Electronics Inc.:USB PnP Sound Device:124000:1"
            .into(),
        codec: "audio/x-raw".into(),
        #[cfg(target_os = "windows")]
        framerate: 44100,
        #[cfg(target_os = "macos")]
        framerate: 48000,
        #[cfg(target_os = "windows")]
        channels: 1,
        #[cfg(target_os = "macos")]
        channels: 2,
        selected_channel: None,
        local_file_save_options: Some(LocalFileSaveOptions {
            output_dir: "recordings".into(),
        }),
    };

    let mic2 = AudioPublishOptions {
        #[cfg(target_os = "windows")]
        device_id: "{0.0.1.00000000}.{be006906-f26f-4a69-ac72-e0216303e6cc}".into(),
        #[cfg(target_os = "macos")]
        device_id: r"AppleUSBAudioEngine:C-Media Electronics Inc.:USB PnP Sound Device:123000:1"
            .into(),
        codec: "audio/x-raw".into(),
        #[cfg(target_os = "windows")]
        framerate: 44100,
        #[cfg(target_os = "macos")]
        framerate: 48000,
        #[cfg(target_os = "windows")]
        channels: 1,
        #[cfg(target_os = "macos")]
        channels: 2,
        selected_channel: None,
        local_file_save_options: Some(LocalFileSaveOptions {
            output_dir: "recordings".into(),
        }),
    };

    let camera = VideoPublishOptions {
        #[cfg(target_os = "windows")]
        device_id: r"\\?\usb#vid_1c45&pid_6200&mi_00#6&1b461b34&0&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global".into(),
        #[cfg(target_os = "macos")]
        device_id: r"0x1300001c456200".into(),
        #[cfg(target_os = "windows")]
        codec: "image/jpeg".into(),
        #[cfg(target_os = "macos")]
        codec: "video/x-raw".into(),
        framerate: 30,
        width: 1920,
        height: 1080,
        local_file_save_options: Some(LocalFileSaveOptions {
            output_dir: "recordings".into(),
        }),
    };

    let mut stream = AvMixStream::new(camera, mic1, Some(mic2));

    stream.build_pipeline().await?;
    stream.preroll_pipeline().await?;
    stream.play_pipeline()?;
    stream.run_bus_loop().await?;
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    stream.stop().await?;

    Ok(())
}
