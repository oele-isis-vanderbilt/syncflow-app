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

use crate::{
    errors::SyncFlowPublisherError, register::RegistrationResponse,
    session_listener::SessionListener,
};

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

/// What one recording contains: the AV mix (camera plus up to two mics recorded L/R), an
/// optional screen, and additional cameras and mics, each with its own stream toggle.
/// Built from the per-device configs the UI saves (`from_device_configs`).
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
    /// Converts the per-device configs the UI saves: the `primary`/`mic1`/`mic2` AV mix
    /// roles become the AV mix (streamed when the primary or mic 1 streams, as before),
    /// and every other device becomes its own output with its own stream toggle.
    pub fn from_device_configs(
        configs: &[DeviceRecordingAndStreamingConfig],
    ) -> Result<Self, SyncFlowPublisherError> {
        let config_error = |message: &str| SyncFlowPublisherError::ConfigError(message.into());
        let role = |mode: AvMixMode| configs.iter().find(|c| c.av_mix_mode == Some(mode.clone()));
        let audio = |config: &DeviceRecordingAndStreamingConfig, what: &str| match &config
            .publish_options
        {
            PublishOptions::Audio(options) => Ok(options.clone()),
            _ => Err(config_error(&format!(
                "the AV mix {what} must be a microphone"
            ))),
        };

        let av_mix = match role(AvMixMode::Primary) {
            None if configs.iter().any(|c| c.av_mix_mode.is_some()) => {
                return Err(config_error("the AV mix needs a camera"))
            }
            None => None,
            Some(primary) => {
                let camera = match &primary.publish_options {
                    PublishOptions::Video(options) => options.clone(),
                    _ => return Err(config_error("the AV mix video must be a camera")),
                };
                let mic_1 =
                    role(AvMixMode::Mic1).ok_or_else(|| config_error("the AV mix needs mic 1"))?;
                let mic_2 = role(AvMixMode::Mic2)
                    .map(|c| audio(c, "mic 2"))
                    .transpose()?;
                Some(AvMixConfig {
                    camera,
                    mic_1: audio(mic_1, "mic 1")?,
                    mic_2,
                    stream: primary.enable_streaming || mic_1.enable_streaming,
                })
            }
        };

        let mut screen = None;
        let mut cameras = Vec::new();
        let mut mics = Vec::new();
        for config in configs.iter().filter(|c| c.av_mix_mode.is_none()) {
            let stream = config.enable_streaming;
            match &config.publish_options {
                PublishOptions::Video(options) => cameras.push(OutputConfig {
                    options: options.clone(),
                    stream,
                }),
                PublishOptions::Audio(options) => mics.push(OutputConfig {
                    options: options.clone(),
                    stream,
                }),
                PublishOptions::Screen(options) => {
                    if screen.is_some() {
                        return Err(config_error("only one screen can be recorded"));
                    }
                    screen = Some(OutputConfig {
                        options: options.clone(),
                        stream,
                    });
                }
            }
        }

        Ok(Self {
            av_mix,
            screen,
            cameras,
            mics,
            recording_mode: configs
                .first()
                .map_or(RecordingMode::SessionMode, |c| c.recording_mode.clone()),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str) -> PublishOptions {
        PublishOptions::Video(VideoPublishOptions {
            codec: "video/x-raw".into(),
            device_id: id.into(),
            width: 1280,
            height: 720,
            framerate: 30,
            local_file_save_options: None,
        })
    }

    fn audio(id: &str) -> PublishOptions {
        PublishOptions::Audio(AudioPublishOptions {
            codec: "audio/x-raw".into(),
            device_id: id.into(),
            framerate: 48000,
            channels: 1,
            selected_channel: None,
            local_file_save_options: None,
        })
    }

    fn screen(id: &str) -> PublishOptions {
        PublishOptions::Screen(ScreenPublishOptions {
            codec: "video/x-raw".into(),
            screen_id_or_name: id.into(),
            width: 1512,
            height: 982,
            framerate: 15,
            local_file_save_options: None,
        })
    }

    fn device(
        options: PublishOptions,
        enable_streaming: bool,
        av_mix_mode: Option<AvMixMode>,
    ) -> DeviceRecordingAndStreamingConfig {
        DeviceRecordingAndStreamingConfig {
            publish_options: options,
            enable_streaming,
            av_mix_mode,
            recording_mode: RecordingMode::LocalMode,
        }
    }

    fn device_id(options: &impl Serialize) -> String {
        let json = serde_json::to_value(options).unwrap();
        json["deviceId"].as_str().unwrap().to_string()
    }

    #[test]
    fn av_mix_roles_become_the_av_mix() {
        let config = CompositeConfig::from_device_configs(&[
            device(video("cam"), false, Some(AvMixMode::Primary)),
            device(audio("mic-a"), true, Some(AvMixMode::Mic1)),
            device(audio("mic-b"), false, Some(AvMixMode::Mic2)),
        ])
        .unwrap();
        let mix = config.av_mix.as_ref().unwrap();
        assert_eq!(device_id(&mix.camera), "cam");
        assert_eq!(device_id(&mix.mic_1), "mic-a");
        assert_eq!(device_id(mix.mic_2.as_ref().unwrap()), "mic-b");
        assert!(mix.stream, "streams when the primary or mic 1 streams");
        assert!(config.cameras.is_empty() && config.mics.is_empty());
        assert_eq!(config.recording_mode, RecordingMode::LocalMode);
    }

    #[test]
    fn other_devices_become_their_own_outputs() {
        let config = CompositeConfig::from_device_configs(&[
            device(video("cam"), true, Some(AvMixMode::Primary)),
            device(audio("mic-a"), false, Some(AvMixMode::Mic1)),
            device(video("c920"), false, None),
            device(audio("umc"), true, None),
            device(screen("1"), false, None),
        ])
        .unwrap();
        assert!(config.av_mix.as_ref().unwrap().mic_2.is_none());
        assert_eq!(config.cameras.len(), 1);
        assert!(!config.cameras[0].stream);
        assert_eq!(config.mics.len(), 1);
        assert!(config.mics[0].stream);
        assert!(config.screen.is_some());
        assert!(config.streams_anything());
    }

    #[test]
    fn without_av_mix_roles_there_is_no_av_mix() {
        let config =
            CompositeConfig::from_device_configs(&[device(audio("mic-a"), false, None)]).unwrap();
        assert!(config.av_mix.is_none());
        assert!(!config.streams_anything());
    }

    #[test]
    fn invalid_av_mix_roles_are_config_errors() {
        let cases = [
            vec![device(audio("mic-a"), false, Some(AvMixMode::Mic1))],
            vec![
                device(screen("1"), false, Some(AvMixMode::Primary)),
                device(audio("mic-a"), false, Some(AvMixMode::Mic1)),
            ],
            vec![device(video("cam"), false, Some(AvMixMode::Primary))],
            vec![
                device(video("cam"), false, Some(AvMixMode::Primary)),
                device(video("c920"), false, Some(AvMixMode::Mic1)),
            ],
            vec![
                device(screen("1"), false, None),
                device(screen("2"), false, None),
            ],
        ];
        for configs in cases {
            assert!(matches!(
                CompositeConfig::from_device_configs(&configs),
                Err(SyncFlowPublisherError::ConfigError(_))
            ));
        }
    }
}
