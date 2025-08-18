use livekit_gstreamer::PublishOptions;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use syncflow_client::ProjectClient;

use crate::register::RegistrationResponse;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecordingAndStreamingConfig {
    pub publish_options: PublishOptions,
    pub disable_streaming: bool,
}

pub struct AppState {
    pub client: Arc<Mutex<Option<ProjectClient>>>,
    pub app_dir: PathBuf,
    pub registration: Arc<Mutex<Option<RegistrationResponse>>>,
}
