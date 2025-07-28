use livekit_gstreamer::get_devices_info;
use livekit_gstreamer::MediaCapability;
use livekit_gstreamer::MediaDeviceInfo;

#[tauri::command]
pub fn get_devices() -> Vec<MediaDeviceInfo> {
    get_devices_info()
        .into_iter()
        .filter_map(|mut device| {
            if device.device_class == "Video/Source" {
                device.capabilities = device
                    .capabilities
                    .into_iter()
                    .filter(|cap| match cap {
                        MediaCapability::Video(video_cap) => video_cap.codec == "image/jpeg",
                        _ => false,
                    })
                    .collect();
                if !device.capabilities.is_empty() {
                    Some(device)
                } else {
                    None
                }
            } else if device.device_class == "Audio/Source" {
                device.capabilities = device
                    .capabilities
                    .into_iter()
                    .filter(|cap| match cap {
                        MediaCapability::Audio(audio_cap) => audio_cap.codec == "audio/x-raw",
                        _ => false,
                    })
                    .collect();
                if !device.capabilities.is_empty() {
                    Some(device)
                } else {
                    None
                }
            } else if device.device_class == "Screen/Source" {
                Some(device)
            } else {
                None
            }
        })
        .collect()
}
