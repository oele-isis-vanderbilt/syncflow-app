//! Joins a LiveKit room and prints every data message, or only those on the topic given
//! as the first argument:
//! `cargo run -p livekit-gstreamer --example lk_listen_data -- composite-events`
//!
//! Needs `LIVEKIT_URL`, `LIVEKIT_API_KEY` and `LIVEKIT_API_SECRET` (`LIVEKIT_ROOM` is
//! optional).

use dotenvy::dotenv;
use livekit::{Room, RoomEvent, RoomOptions};
use livekit_api::access_token;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let topic = env::args().nth(1);
    let url = env::var("LIVEKIT_URL")?;
    let token = access_token::AccessToken::with_api_key(
        &env::var("LIVEKIT_API_KEY")?,
        &env::var("LIVEKIT_API_SECRET")?,
    )
    .with_identity("rust-bot-listener")
    .with_name("Rust Bot Listener")
    .with_grants(access_token::VideoGrants {
        room_join: true,
        room: env::var("LIVEKIT_ROOM").unwrap_or_else(|_| "composite-room".into()),
        ..Default::default()
    })
    .to_jwt()?;

    let (_room, mut events) = Room::connect(&url, &token, RoomOptions::default()).await?;
    println!("Listening for {}", topic.as_deref().unwrap_or("all topics"));
    while let Some(event) = events.recv().await {
        match event {
            RoomEvent::DataReceived {
                payload,
                topic: message_topic,
                participant,
                ..
            } => {
                if topic.is_some() && topic != message_topic {
                    continue;
                }
                let from = participant
                    .map(|p| p.identity().to_string())
                    .unwrap_or_default();
                println!(
                    "[{}] {from}: {}",
                    message_topic.unwrap_or_default(),
                    String::from_utf8_lossy(&payload)
                );
            }
            RoomEvent::Disconnected { .. } => break,
            _ => {}
        }
    }
    Ok(())
}
