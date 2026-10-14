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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecordingAndStreamingConfig {
    pub publish_options: PublishOptions,
    pub enable_streaming: bool,
    pub av_mix_mode: Option<AvMixMode>,
    pub recording_mode: RecordingMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RecordingMode {
    SessionMode, // Listen for remote sessions
    LocalMode,   // Manual local recording
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AvMixMode {
    Primary, // Main video source
    Mic1,    // Primary audio
    Mic2,    // Secondary audio
}

// The composite config replaces `DeviceRecordingAndStreamingConfig` and `AvMixMode`
// when the app switches to `CompositeStream` (composite-stream-plan.md §10). Unused until then.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvMixConfig {
    pub camera: VideoPublishOptions,
    pub mic_1: AudioPublishOptions,
    pub mic_2: Option<AudioPublishOptions>,
    pub stream: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputConfig<T> {
    pub options: T,
    pub stream: bool,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
impl CompositeConfig {
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

        let s3_client = rusoto_s3::S3Client::new_with(client, credentials_provider, region);

        s3_client
    }
}

#[derive(Debug)]
pub struct ActiveSession {
    pub session_id: String,
    pub session_name: String,
    pub cancel_token: CancellationToken,
    pub task_handle: tauri::async_runtime::JoinHandle<()>,
}

pub struct AppState {
    pub client: Arc<Mutex<Option<ProjectClient>>>,
    pub app_dir: PathBuf,
    pub registration: Arc<Mutex<Option<RegistrationResponse>>>,
    pub recording_and_streaming_config: Arc<Mutex<Option<Vec<DeviceRecordingAndStreamingConfig>>>>,
    pub session_listener: Arc<AsyncMutex<Option<SessionListener>>>,
    pub active_sessions: Arc<AsyncMutex<HashMap<String, ActiveSession>>>,
    pub currently_joined_session: Arc<AsyncMutex<Option<String>>>,
    pub local_recording_session: Arc<AsyncMutex<Option<ActiveSession>>>,
}
