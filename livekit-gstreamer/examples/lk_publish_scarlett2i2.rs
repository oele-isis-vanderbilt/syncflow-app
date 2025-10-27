use dotenvy::dotenv;
use livekit::{Room, RoomOptions};
use livekit_gstreamer::{
    AlsaMultiChannelMixStream, AudioPublishOptions, GstMediaStream, LKParticipant,
    LKParticipantError, LocalFileSaveOptions, PublishOptions,
};

use livekit_api::access_token;
use std::{env, sync::Arc};

#[path = "./helper/wait.rs"]
mod wait;

#[tokio::main]
async fn main() -> Result<(), LKParticipantError> {
    // Only run on Linux
    if !cfg!(target_os = "linux") {
        panic!("This example is only supported on Linux");
    }
    dotenv().ok();

    // Initialize gstreamer
    gstreamer::init().unwrap();
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();

    let url = env::var("LIVEKIT_URL").expect("LIVEKIT_URL is not set");
    let api_key = env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY is not set");
    let api_secret = env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET is not set");

    let token = access_token::AccessToken::with_api_key(&api_key, &api_secret)
        .with_identity("rust-bot-microphone")
        .with_name("Rust Bot Microphone")
        .with_grants(access_token::VideoGrants {
            room_join: true,
            room: "demo-room".to_string(),
            ..Default::default()
        })
        .to_jwt()
        .unwrap();

    let monitor_token = access_token::AccessToken::with_api_key(&api_key, &api_secret)
        .with_identity("room-monitor")
        .with_name("Room Monitor")
        .with_grants(access_token::VideoGrants {
            room_join: true,
            room: "demo-room".to_string(),
            ..Default::default()
        })
        .to_jwt()
        .unwrap();

    println!("Monitor Token: {}", monitor_token);

    let (room, mut room_rx) = Room::connect(&url, &token, RoomOptions::default())
        .await
        .unwrap();

    let new_room = Arc::new(room);

    let mut stream1 = AlsaMultiChannelMixStream::new(
        "hw:1,0".to_string(),
        96000,
        2,
        vec![0, 1],
        "S32LE".to_string(),
        Some("recordings-scarlett-2i2".to_string()),
    );

    stream1.start().await?;

    let mut participant = LKParticipant::new(new_room.clone());
    participant
        .publish_alsa_stream(&mut stream1, Some("Scarlett Multichannel Mix".into()))
        .await?;

    log::info!(
        "Connected to room: {} - {}",
        new_room.name(),
        String::from(new_room.sid().await)
    );

    wait::wait_alsa_lk(&mut [stream1], new_room.clone(), &mut room_rx).await
}
