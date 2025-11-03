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
    gstreamer::init().unwrap();
    
    if env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "debug");
    }
    env_logger::init();

    let url = env::var("LIVEKIT_URL").expect("LIVEKIT_URL is not set");
    let api_key = env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY is not set");
    let api_secret = env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET is not set");

    let rtsp_location = "rtsp://10.2.40.141/vaddio-avb-nano-stream".to_string();
    

    let token = access_token::AccessToken::with_api_key(&api_key, &api_secret)
        .with_identity("vaddio-camera-publisher")
        .with_name("Vaddio Camera Publisher")
        .with_grants(access_token::VideoGrants {
            room_join: true,
            room: "demo-room".to_string(),
            ..Default::default()
        })
        .to_jwt()
        .unwrap();

    let monitor_token = access_token::AccessToken::with_api_key(&api_key, &api_secret)
        .with_identity("vaddio-camera-monitor")
        .with_name("Vaddio Camera Monitor")
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

    // Test with just video first
    println!("Creating video stream...");
    let mut video_stream = RtspStream::new(
        rtsp_location.clone(),
        RtspStreamType::Video {
            width: 1920,
            height: 1080,
            framerate: 34,
        },
        None, // No recording for test
    );

    println!("Starting video stream...");
    video_stream.start().await.unwrap();
    println!("Video stream started");

    let mut participant = LKParticipant::new(new_room.clone());

    println!("Publishing video...");
    let video_track_id = participant.publish_rtsp_stream(&mut video_stream, Some("Vaddio Camera".to_string())).await?;
    println!("Published video track: {}", video_track_id);

    println!("Waiting for frames... (press Ctrl+C to exit)");
    
    // Wait for the room to close or Ctrl+C
    wait::wait_rtsp(&mut video_stream, new_room.clone(), &mut room_rx).await
}