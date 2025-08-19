use std::path::PathBuf;

use crate::errors::SyncFlowPublisherError;

pub fn host_name() -> String {
    gethostname::gethostname().to_string_lossy().to_string()
}

pub fn get_ip_address() -> Option<String> {
    local_ip_address::local_ip().ok().map(|ip| ip.to_string())
}

pub fn save_json(
    data: &impl serde::Serialize,
    path: &PathBuf,
) -> Result<(), SyncFlowPublisherError> {
    let file = std::fs::File::create(path).map_err(|e| SyncFlowPublisherError::IoError(e))?;
    serde_json::to_writer(file, data).map_err(|e| SyncFlowPublisherError::JsonError(e))?;
    Ok(())
}

pub fn load_json<T: serde::de::DeserializeOwned>(
    file_loc: &PathBuf,
) -> Result<T, SyncFlowPublisherError> {
    let file = std::fs::File::open(file_loc)?;
    serde_json::from_reader(file).map_err(|e| SyncFlowPublisherError::JsonError(e))
}
