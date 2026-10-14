use livekit_gstreamer::composite::{AvMixSpec, CompositeStreamSpec, Output};
use livekit_gstreamer::{
    AudioPublishOptions, PublishOptions, ScreenPublishOptions, VideoPublishOptions,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use syncflow_client::ProjectClient;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::{register::RegistrationResponse, session_listener::SessionListener};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RecordingMode {
    SessionMode, // Listen for remote sessions
    LocalMode,   // Manual local recording
}

/// What one recording contains: the AV mix (camera plus up to two mics recorded L/R), an
/// optional screen, and additional cameras and mics, each with its own stream toggle.
/// Saved to `composite_config.json` by the setup page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvMixConfig {
    pub camera: VideoPublishOptions,
    pub mic_1: AudioPublishOptions,
    pub mic_2: Option<AudioPublishOptions>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputConfig<T> {
    pub options: T,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositeConfig {
    pub av_mix: Option<AvMixConfig>,
    pub screen: Option<OutputConfig<ScreenPublishOptions>>,
    #[serde(default)]
    pub cameras: Vec<OutputConfig<VideoPublishOptions>>,
    #[serde(default)]
    pub mics: Vec<OutputConfig<AudioPublishOptions>>,
    pub recording_mode: RecordingMode,
}

impl CompositeConfig {
    /// Every device of the config with its requested format, for checking that the
    /// devices exist and support it.
    pub fn publish_options(&self) -> Vec<PublishOptions> {
        let mut options = Vec::new();
        if let Some(mix) = &self.av_mix {
            options.push(PublishOptions::Video(mix.camera.clone()));
            options.push(PublishOptions::Audio(mix.mic_1.clone()));
            options.extend(mix.mic_2.clone().map(PublishOptions::Audio));
        }
        options.extend(
            self.screen
                .iter()
                .map(|s| PublishOptions::Screen(s.options.clone())),
        );
        options.extend(
            self.cameras
                .iter()
                .map(|c| PublishOptions::Video(c.options.clone())),
        );
        options.extend(
            self.mics
                .iter()
                .map(|m| PublishOptions::Audio(m.options.clone())),
        );
        options
    }

    /// Whether any output is streamed.
    pub fn streams_anything(&self) -> bool {
        self.av_mix.as_ref().is_some_and(|m| m.stream)
            || self.screen.as_ref().is_some_and(|s| s.stream)
            || self.cameras.iter().any(|c| c.stream)
            || self.mics.iter().any(|m| m.stream)
    }

    pub fn to_stream_spec(
        &self,
        output_dir: PathBuf,
        manifest_context: serde_json::Value,
    ) -> CompositeStreamSpec {
        fn output<T: Clone>(config: &OutputConfig<T>) -> Output<T> {
            Output {
                options: config.options.clone(),
                stream: config.stream,
            }
        }

        CompositeStreamSpec {
            av_mix: self.av_mix.as_ref().map(|mix| AvMixSpec {
                camera: mix.camera.clone(),
                mics: std::iter::once(mix.mic_1.clone())
                    .chain(mix.mic_2.clone())
                    .collect(),
                stream: mix.stream,
            }),
            screen: self.screen.as_ref().map(output),
            cameras: self.cameras.iter().map(output).collect(),
            mics: self.mics.iter().map(output).collect(),
            output_dir: Some(output_dir),
            stream_audio: Default::default(),
            encoders: Default::default(),
            start_policy: Default::default(),
            manifest_context: Some(manifest_context),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Config {
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_region: String,
    pub s3_bucket: String,
    pub s3_endpoint: String,
}

impl From<S3Config> for rusoto_s3::S3Client {
    fn from(value: S3Config) -> Self {
        let client = rusoto_core::HttpClient::new().expect("Failed to create request dispatcher");
        let region = rusoto_core::Region::Custom {
            name: value.s3_region.to_string(),
            endpoint: value.s3_endpoint.to_string(),
        };
        let credentials_provider = rusoto_credential::StaticProvider::new_minimal(
            value.s3_access_key.to_string(),
            value.s3_secret_key.to_string(),
        );

        rusoto_s3::S3Client::new_with(client, credentials_provider, region)
    }
}

#[derive(Debug)]
pub struct ActiveSession {
    pub session_id: String,
    pub session_name: String,
    pub cancel_token: CancellationToken,
}

pub struct AppState {
    pub client: Arc<Mutex<Option<ProjectClient>>>,
    pub app_dir: PathBuf,
    pub registration: Arc<Mutex<Option<RegistrationResponse>>>,
    pub recording_and_streaming_config: Arc<Mutex<Option<CompositeConfig>>>,
    pub session_listener: Arc<AsyncMutex<Option<SessionListener>>>,
    pub active_sessions: Arc<AsyncMutex<HashMap<String, ActiveSession>>>,
    pub currently_joined_session: Arc<AsyncMutex<Option<String>>>,
    pub local_recording_session: Arc<AsyncMutex<Option<ActiveSession>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JSON the setup page sends (`CompositeConfig` in `types.ts`).
    const UI_CONFIG: &str = r#"{
        "avMix": {
            "camera": { "codec": "video/x-raw", "deviceId": "cam", "width": 1280, "height": 720, "framerate": 30 },
            "mic1": { "codec": "audio/x-raw", "deviceId": "mic-a", "framerate": 48000, "channels": 1 },
            "mic2": null,
            "stream": true
        },
        "screen": null,
        "cameras": [],
        "mics": [{ "options": { "codec": "audio/x-raw", "deviceId": "umc", "framerate": 48000, "channels": 4, "selectedChannel": 3 }, "stream": false }],
        "recordingMode": "localMode"
    }"#;

    #[test]
    fn parses_the_setup_page_config() {
        let config: CompositeConfig = serde_json::from_str(UI_CONFIG).unwrap();
        let mix = config.av_mix.as_ref().unwrap();
        assert_eq!(mix.mic_1.device_id, "mic-a");
        assert!(mix.mic_2.is_none());
        assert_eq!(config.mics[0].options.selected_channel, Some(3));
        assert_eq!(config.recording_mode, RecordingMode::LocalMode);
        assert_eq!(config.publish_options().len(), 3);
        assert!(config.streams_anything());

        let round_trip = serde_json::to_value(&config).unwrap();
        assert_eq!(round_trip["avMix"]["mic1"]["deviceId"], "mic-a");
        assert_eq!(round_trip["recordingMode"], "localMode");
    }
}
