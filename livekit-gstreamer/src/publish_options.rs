//! The formats a device is asked for: the shape the app saves and sends over Tauri.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPublishOptions {
    pub codec: String,
    pub device_id: String,
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPublishOptions {
    pub codec: String,
    pub device_id: String,
    pub framerate: i32,
    pub channels: i32,
    pub selected_channel: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenPublishOptions {
    pub codec: String,
    pub screen_id_or_name: String,
    pub width: i32,
    pub height: i32,
    pub framerate: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PublishOptions {
    Video(VideoPublishOptions),
    Audio(AudioPublishOptions),
    Screen(ScreenPublishOptions),
}
