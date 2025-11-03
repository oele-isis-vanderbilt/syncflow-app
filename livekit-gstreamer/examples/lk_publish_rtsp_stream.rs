use dotenvy::dotenv;
use livekit::{Room, RoomOptions};

use livekit_api::access_token;
use livekit_gstreamer::{
    LKParticipant, LKParticipantError, RtspStream, RtspStreamType,
};
use std::{env, sync::Arc};

#[path = "./helper/wait.rs"]
mod wait;

#[tokio::main]
async fn main() -> Result<(), LKParticipantError> {
    dotenv().ok();
    // Initialize gstreamer
    gstreamer::init().unwrap();
    
    // Set logging level - can be overridden by RUST_LOG env var
    if env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }
    env_logger::init();
    
    // Enable GStreamer debug logging if requested
    // if env::var("GST_DEBUG").is_ok() {
    //     log::info!("GStreamer debug logging enabled");
    // } else {
    //     log::info!("Set GST_DEBUG=3 environment variable for GStreamer debug output");
    // }

    let url = env::var("LIVEKIT_URL").expect("LIVEKIT_URL is not set");
    let api_key = env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY is not set");
    let api_secret = env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET is not set");

    // RTSP stream configuration - hardcoded based on ffprobe output
    let rtsp_location = env::var("RTSP_LOCATION")
        .unwrap_or_else(|_| "rtsp://10.2.40.141/vaddio-avb-nano-stream".to_string());
    
    // Video configuration (from ffprobe: h264 Main, yuv420p, 1920x1080, 34 tbr)
    let width: i32 = 1920;
    let height: i32 = 1080;
    let video_framerate: i32 = 34;
    
    // Audio configuration (from ffprobe: aac LC, 48000 Hz, stereo)
    let audio_framerate: i32 = 48000;
    let channels: i32 = 2;
    
    // Recording directory (optional)
    let recording_dir = env::var("RECORDING_DIR").ok();

    let token = access_token::AccessToken::with_api_key(&api_key, &api_secret)
        .with_identity("rust-rtsp-publisher")
        .with_name("Rust RTSP Publisher")
        .with_grants(access_token::VideoGrants {
            room_join: true,
            room: "rtsp-demo-room".to_string(),
            ..Default::default()
        })
        .to_jwt()
        .unwrap();

    let monitor_token = access_token::AccessToken::with_api_key(&api_key, &api_secret)
        .with_identity("room-monitor")
        .with_name("Room Monitor")
        .with_grants(access_token::VideoGrants {
            room_join: true,
            room: "rtsp-demo-room".to_string(),
            ..Default::default()
        })
        .to_jwt()
        .unwrap();

    println!("Monitor Token: {}", monitor_token);

    let (room, mut room_rx) = Room::connect(&url, &token, RoomOptions::default())
        .await
        .unwrap();

    let new_room = Arc::new(room);

    // Create both video and audio RTSP streams from the same source
    log::info!("Creating RTSP video stream: {}x{} @ {}fps", width, height, video_framerate);
    let mut video_stream = RtspStream::new(
        rtsp_location.clone(),
        RtspStreamType::Video {
            width,
            height,
            framerate: video_framerate,
        },
        recording_dir.clone(),
    );

    log::info!("Creating RTSP audio stream: {} channels @ {}Hz", channels, audio_framerate);
    let mut audio_stream = RtspStream::new(
        rtsp_location.clone(),
        RtspStreamType::Audio {
            framerate: audio_framerate,
            channels,
        },
        recording_dir.clone(),
    );

    // Start video stream first
    println!("Starting video stream...");
    video_stream.start().await.unwrap();
    log::info!("RTSP video stream started successfully");
    
    // Wait a bit before starting audio to avoid conflicts
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    println!("Starting audio stream...");
    audio_stream.start().await.unwrap();
    log::info!("RTSP audio stream started successfully");

    let mut participant = LKParticipant::new(new_room.clone());

    // Publish video stream first
    println!("Publishing video stream...");
    let video_track_id = participant.publish_rtsp_stream(&mut video_stream, Some("Vaddio Camera".to_string())).await?;
    log::info!("Published video track: {}", video_track_id);
    
    // Wait a bit before publishing audio
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    println!("Publishing audio stream...");
    let audio_track_id = participant.publish_rtsp_stream(&mut audio_stream, Some("Vaddio Audio".to_string())).await?;
    log::info!("Published audio track: {}", audio_track_id);

    log::info!(
        "Connected to room: {} - {}",
        new_room.name(),
        String::from(new_room.sid().await)
    );

    log::info!("Publishing RTSP video and audio streams from: {}", video_stream.get_stream_location());

    // Wait for the room to close or Ctrl+C - we'll just monitor one stream since they share the same lifecycle
    wait::wait_rtsp(&mut video_stream, new_room.clone(), &mut room_rx).await
}