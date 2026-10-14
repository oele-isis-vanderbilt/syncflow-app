//! Records every configured device through one composite pipeline and publishes its
//! streams to a LiveKit room until Ctrl+C or the room disconnects. Pipeline events are
//! printed and also published to the room as data on the `composite-events` topic
//! (watch them with the `lk_listen_data` example).
//!
//! Needs `LIVEKIT_URL`, `LIVEKIT_API_KEY` and `LIVEKIT_API_SECRET` (`LIVEKIT_ROOM` is
//! optional). Devices come from environment variables; see `helper/composite_env.rs`.

use dotenvy::dotenv;
use livekit::{Room, RoomEvent, RoomOptions};
use livekit_api::access_token;
use livekit_gstreamer::composite::{CompositeEvent, CompositeStream};
use livekit_gstreamer::LKParticipant;
use std::{env, sync::Arc};

#[path = "./helper/composite_env.rs"]
mod composite_env;

const EVENTS_TOPIC: &str = "composite-events";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    livekit_gstreamer::initialize_gstreamer();

    let url = env::var("LIVEKIT_URL")?;
    let token = access_token::AccessToken::with_api_key(
        &env::var("LIVEKIT_API_KEY")?,
        &env::var("LIVEKIT_API_SECRET")?,
    )
    .with_identity("rust-bot-composite")
    .with_name("Rust Bot Composite")
    .with_grants(access_token::VideoGrants {
        room_join: true,
        can_publish_data: true,
        room: env::var("LIVEKIT_ROOM").unwrap_or_else(|_| "composite-room".into()),
        ..Default::default()
    })
    .to_jwt()?;

    let output_dir = env::current_dir()?.join("recordings").join(format!(
        "{}_composite",
        chrono::Local::now().format("%Y-%m-%d_%H-%M")
    ));
    let mut stream = CompositeStream::new(composite_env::spec_from_env(Some(output_dir)));
    let mut events = stream.events();
    stream.start().await?;

    let (room, mut room_events) = Room::connect(&url, &token, RoomOptions::default()).await?;
    let mut participant = LKParticipant::new(Arc::new(room));
    for sink in participant.publish_composite(&stream).await? {
        println!("Published {sink}");
    }

    loop {
        tokio::select! {
            event = events.recv() => {
                let Ok(event) = event else { break };
                let json = serde_json::to_vec(&event)?;
                println!("{}", String::from_utf8_lossy(&json));
                participant.publish_data(EVENTS_TOPIC, json, true).await?;
                if matches!(event, CompositeEvent::Stopped { .. }) {
                    break;
                }
            }
            event = room_events.recv() => {
                if matches!(event, None | Some(RoomEvent::Disconnected { .. })) {
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    let manifest = stream.stop().await?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
