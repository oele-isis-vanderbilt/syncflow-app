use std::path::PathBuf;
use std::vec;

use livekit_gstreamer::get_devices_info;
use livekit_gstreamer::GstMediaDevice;
use livekit_gstreamer::MediaCapability;
use livekit_gstreamer::MediaDeviceInfo;
use livekit_gstreamer::PublishOptions;

use crate::errors::SyncFlowPublisherError;
use crate::models;
use crate::models::DeviceRecordingAndStreamingConfig;
use crate::session_listener::initialize_session_listener;
use crate::utils::save_json;

fn verify_device_exists_and_supports_codec(
    option: &PublishOptions,
) -> Result<bool, SyncFlowPublisherError> {
    let existing_devices = get_devices();

    let device_id = match option {
        PublishOptions::Audio(details) => &details.device_id,
        PublishOptions::Video(details) => &details.device_id,
        PublishOptions::Screen(details) => &details.screen_id_or_name,
    };

    let device = existing_devices
        .iter()
        .find(|d| d.device_path == *device_id)
        .ok_or_else(|| {
            SyncFlowPublisherError::ConfigError(format!("Device with path {} not found", device_id))
        })?;

    let media_device = match device.device_class.as_str() {
        "Audio/Source" | "Video/Source" => GstMediaDevice::from_device_path(&device.device_path)?,
        _ => GstMediaDevice::from_screen_id_or_name(&device.device_path)?,
    };

    let supports_codec = match option {
        PublishOptions::Audio(details) => {
            media_device.supports_audio(&details.codec, details.channels, details.framerate)
        }
        PublishOptions::Video(details) => media_device.supports_video(
            &details.codec,
            details.width,
            details.height,
            details.framerate,
        ),
        PublishOptions::Screen(details) => media_device.supports_screen_share(
            &details.codec,
            details.width,
            details.height,
            details.framerate,
        ),
    };

    Ok(supports_codec)
}

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
                        MediaCapability::Video(video_cap) => {
                            #[cfg(target_os = "macos")]
                            {
                                video_cap.codec == "image/jpeg" || video_cap.codec == "video/x-raw"
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                video_cap.codec == "image/jpeg"
                            }
                        }
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

#[tauri::command(async)]
pub async fn set_streaming_config(
    configs: Vec<DeviceRecordingAndStreamingConfig>,
    app_state: tauri::State<'_, models::AppState>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<DeviceRecordingAndStreamingConfig>, SyncFlowPublisherError> {
    let device_exists_and_supports_codec =
        |option: &PublishOptions| -> Result<bool, SyncFlowPublisherError> {
            verify_device_exists_and_supports_codec(option)
        };

    let mut all_errors = vec![];

    for config in &configs {
        let supports_codec = device_exists_and_supports_codec(&config.publish_options);
        if let Err(e) = supports_codec {
            all_errors.push(e.to_string());
        }
    }

    if !all_errors.is_empty() {
        return Err(SyncFlowPublisherError::ConfigError(all_errors.join(", ")));
    }

    save_json(&configs, &app_state.app_dir.join("selected_devices.json"))?;
    {
        let mut guard = app_state.recording_and_streaming_config.lock().unwrap();
        *guard = Some(configs.clone());
    }

    let mut session_listener_guard = app_state.session_listener.lock().await;
    if session_listener_guard.is_none() {
        let listener = initialize_session_listener(&app_state.app_dir, app_handle.clone())
            .await
            .ok_or(SyncFlowPublisherError::InitializationError(
                "Failed to initialize session listener".to_string(),
            ))?;
        *session_listener_guard = Some(listener);
    }

    Ok(configs)
}

pub fn initialize_streaming_config(
    app_dir: &PathBuf,
) -> Option<Vec<DeviceRecordingAndStreamingConfig>> {
    let config_file = app_dir.join("selected_devices.json");
    if config_file.exists() {
        let config_str = std::fs::read_to_string(&config_file).ok()?;
        let configs: Vec<DeviceRecordingAndStreamingConfig> =
            serde_json::from_str(&config_str).ok()?;
        configs
            .iter()
            .all(|config| verify_device_exists_and_supports_codec(&config.publish_options).is_ok())
            .then_some(configs)
    } else {
        None
    }
}

#[tauri::command]
pub fn get_streaming_config(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<Vec<DeviceRecordingAndStreamingConfig>, SyncFlowPublisherError> {
    let guard = app_state.recording_and_streaming_config.lock().unwrap();
    if let Some(configs) = &*guard {
        Ok(configs.clone())
    } else {
        Err(SyncFlowPublisherError::NotIntialized(
            "Streaming config not initialized".to_string(),
        ))
    }
}

#[tauri::command(async)]
pub async fn delete_streaming_config(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<(), SyncFlowPublisherError> {
    let config_file = app_state.app_dir.join("selected_devices.json");
    if config_file.exists() {
        std::fs::remove_file(config_file)?;
    }
    {
        let mut guard = app_state.recording_and_streaming_config.lock().unwrap();
        *guard = None;
    }

    let mut listener_to_stop = {
        let mut session_listener_guard = app_state.session_listener.lock().await;
        session_listener_guard.take()
    };

    if let Some(ref mut listener) = listener_to_stop {
        listener.stop().await?;
    }

    Ok(())
}
