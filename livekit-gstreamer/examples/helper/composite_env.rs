//! Builds a composite spec from environment variables, so the composite examples run on
//! any machine. Find device ids with `cargo run -p livekit-gstreamer --example get_devices`.
//!
//! - `CAMERA_ID` (+ `CAMERA_CODEC`, `CAMERA_WIDTH`, `CAMERA_HEIGHT`, `CAMERA_FPS`): AV mix camera
//! - `MIC1_ID`, `MIC2_ID` (+ `MIC_RATE`, `MIC1_CHANNELS`, `MIC1_CHANNEL`, `MIC2_…`): AV mix mics,
//!   recorded L/R
//! - `SCREEN_ID` (+ `SCREEN_WIDTH`, `SCREEN_HEIGHT`, `SCREEN_FPS`): optional screenshare
//! - `EXTRA_CAMERA_ID`, `EXTRA_MIC_ID` (+ `EXTRA_MIC_CHANNELS`, `EXTRA_MIC_CHANNEL`): optional
//! - `STREAM_SCREEN`, `STREAM_EXTRA`: `true` to stream those too (the AV mix always streams)

use livekit_gstreamer::composite::{AvMixSpec, CompositeStreamSpec, Output};
use livekit_gstreamer::{AudioPublishOptions, ScreenPublishOptions, VideoPublishOptions};
use std::env;
use std::path::PathBuf;

fn var(name: &str) -> Option<String> {
    println!("{}-{:?}", name, env::var(name).ok());
    env::var(name).ok().filter(|v| !v.is_empty())
}

fn num(name: &str, default: i32) -> i32 {
    var(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn flag(name: &str) -> bool {
    var(name).is_some_and(|v| v == "true" || v == "1")
}

fn camera(prefix: &str, id: String) -> VideoPublishOptions {
    VideoPublishOptions {
        codec: var(&format!("{prefix}_CODEC")).unwrap_or_else(|| "image/jpeg".into()),
        device_id: id,
        width: num(&format!("{prefix}_WIDTH"), 1280),
        height: num(&format!("{prefix}_HEIGHT"), 720),
        framerate: num(&format!("{prefix}_FPS"), 30),
    }
}

fn mic(prefix: &str, id: String) -> AudioPublishOptions {
    AudioPublishOptions {
        codec: "audio/x-raw".into(),
        device_id: id,
        framerate: num("MIC_RATE", 48000),
        channels: num(&format!("{prefix}_CHANNELS"), 1),
        selected_channel: var(&format!("{prefix}_CHANNEL")).and_then(|v| v.parse().ok()),
    }
}

/// The spec described by the environment, recording into `output_dir`.
pub fn spec_from_env(output_dir: Option<PathBuf>) -> CompositeStreamSpec {
    let av_mix = var("CAMERA_ID").map(|camera_id| AvMixSpec {
        camera: camera("CAMERA", camera_id),
        mics: ["MIC1", "MIC2"]
            .iter()
            .filter_map(|prefix| var(&format!("{prefix}_ID")).map(|id| mic(prefix, id)))
            .collect(),
        stream: true,
    });
    let screen = var("SCREEN_ID").map(|id| Output {
        options: ScreenPublishOptions {
            codec: "video/x-raw".into(),
            screen_id_or_name: id,
            width: num("SCREEN_WIDTH", 1920),
            height: num("SCREEN_HEIGHT", 1080),
            framerate: num("SCREEN_FPS", 15),
        },
        stream: flag("STREAM_SCREEN"),
    });
    CompositeStreamSpec {
        av_mix,
        screen,
        cameras: var("EXTRA_CAMERA_ID")
            .map(|id| Output {
                options: camera("EXTRA_CAMERA", id),
                stream: flag("STREAM_EXTRA"),
            })
            .into_iter()
            .collect(),
        mics: var("EXTRA_MIC_ID")
            .map(|id| Output {
                options: mic("EXTRA_MIC", id),
                stream: flag("STREAM_EXTRA"),
            })
            .into_iter()
            .collect(),
        output_dir,
        ..Default::default()
    }
}
