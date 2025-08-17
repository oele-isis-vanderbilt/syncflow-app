use dotenvy::dotenv;
use livekit_gstreamer::{
    GStreamerError, GstMediaStream, LocalFileSaveOptions, PublishOptions, ScreenPublishOptions,
};

#[path = "./helper/wait.rs"]
mod wait;

#[tokio::main]
async fn main() -> Result<(), GStreamerError> {
    dotenv().ok();
    // Initialize gstreamer
    gstreamer::init().unwrap();
    let mut stream = if cfg!(target_os = "linux") {
        GstMediaStream::new(PublishOptions::Screen(ScreenPublishOptions {
            codec: "video/x-raw".to_string(),
            width: 1920,
            height: 1080,
            framerate: 30,
            screen_id_or_name: "DP-3-2".to_string(),
            local_file_save_options: Some(LocalFileSaveOptions {
                output_dir: "recordings".to_string(),
            }),
        }))
    } else if cfg!(target_os = "macos") {
        GstMediaStream::new(PublishOptions::Screen(ScreenPublishOptions {
            codec: "video/x-raw".to_string(),
            width: 2560,
            height: 1664,
            framerate: 30,
            screen_id_or_name: "Built-in Display".to_string(),
            local_file_save_options: Some(LocalFileSaveOptions {
                output_dir: "recordings".to_string(),
            }),
        }))
    } else {
        GstMediaStream::new(PublishOptions::Screen(ScreenPublishOptions {
            codec: "video/x-raw".to_string(),
            width: 1920,
            height: 1080,
            framerate: 30,
            screen_id_or_name: "131073".to_string(),
            local_file_save_options: Some(LocalFileSaveOptions {
                output_dir: "recordings".to_string(),
            }),
        }))
    };

    stream.start().await.unwrap();

    let (frame_rx, close_rx) = stream.subscribe().unwrap();

    wait::wait_streams(&mut [stream], vec![frame_rx], vec![close_rx]).await
}
