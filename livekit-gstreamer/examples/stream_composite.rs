//! Records every configured device through one composite pipeline for 60 seconds (or
//! until Ctrl+C), printing pipeline events, then prints the manifest.
//!
//! Devices come from environment variables; see `helper/composite_env.rs`.
//! `cargo run -p livekit-gstreamer --example stream_composite`

use dotenvy::dotenv;
use livekit_gstreamer::composite::{CompositeError, CompositeStream};
use std::time::Duration;

#[path = "./helper/composite_env.rs"]
mod composite_env;

#[tokio::main]
async fn main() -> Result<(), CompositeError> {
    dotenv().ok();
    livekit_gstreamer::initialize_gstreamer();

    let output_dir = std::env::current_dir()?.join("recordings").join(format!(
        "{}_composite",
        chrono::Local::now().format("%Y-%m-%d_%H-%M")
    ));
    let mut stream = CompositeStream::new(composite_env::spec_from_env(Some(output_dir.clone())));
    let mut events = stream.events();
    let report = stream.start().await?;
    for (device, error) in &report.failed_at_start {
        eprintln!("{} did not open: {}", device.display_name, error.message);
    }
    println!("Recording into {}", output_dir.display());

    let deadline = tokio::time::sleep(Duration::from_secs(60));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => println!("{}", serde_json::to_string(&event).unwrap_or_default()),
                Err(_) => break,
            },
            _ = &mut deadline => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    let manifest = stream.stop().await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&manifest).unwrap_or_default()
    );
    Ok(())
}
