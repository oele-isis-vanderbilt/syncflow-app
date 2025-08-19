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

pub struct AppState {
    pub client: Arc<Mutex<Option<ProjectClient>>>,
    pub app_dir: PathBuf,
    pub registration: Arc<Mutex<Option<RegistrationResponse>>>,
    pub recording_and_streaming_config: Arc<Mutex<Option<Vec<DeviceRecordingAndStreamingConfig>>>>,
    pub session_listener: Arc<AsyncMutex<Option<SessionListener>>>,
}
