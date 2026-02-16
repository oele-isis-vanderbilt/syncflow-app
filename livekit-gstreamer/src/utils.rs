use rand::{distributions::Alphanumeric, thread_rng, Rng};

use crate::{GStreamerError, GstMediaDevice};

pub fn random_string(prefix: &str) -> String {
    let random_string: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect();
    format!("{}-{}", prefix, random_string)
}

pub fn system_time_nanos() -> i64 {
    let now = std::time::SystemTime::now();
    now.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

pub fn get_device_name(device_id_or_path: &str, is_screen: bool) -> Result<String, GStreamerError> {
    if is_screen {
        GstMediaDevice::from_screen_id_or_name(device_id_or_path).map(|device| device.display_name)
    } else {
        GstMediaDevice::from_device_path(device_id_or_path).map(|device| device.display_name)
    }
}
