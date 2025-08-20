use livekit_gstreamer::PublishOptions;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use syncflow_client::ProjectClient;
use tokio::sync::Mutex as AsyncMutex;

use crate::{register::RegistrationResponse, session_listener::SessionListener};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecordingAndStreamingConfig {
    pub publish_options: PublishOptions,
    pub enable_streaming: bool,
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

pub struct AppState {
    pub client: Arc<Mutex<Option<ProjectClient>>>,
    pub app_dir: PathBuf,
    pub registration: Arc<Mutex<Option<RegistrationResponse>>>,
    pub recording_and_streaming_config: Arc<Mutex<Option<Vec<DeviceRecordingAndStreamingConfig>>>>,
    pub session_listener: Arc<AsyncMutex<Option<SessionListener>>>,
}
