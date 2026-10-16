use std::path::{Path, PathBuf};

use livekit_gstreamer::get_devices_info;
use livekit_gstreamer::GstMediaDevice;
use livekit_gstreamer::MediaCapability;
use livekit_gstreamer::MediaDeviceInfo;
use livekit_gstreamer::PublishOptions;

use crate::errors::SyncFlowPublisherError;
use crate::models;
use crate::models::{CompositeConfig, RecordingMode};
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
                println!("Device: {:?}", device.display_name);
                device.capabilities.retain(|cap| match cap {
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
                });
                if !device.capabilities.is_empty() {
                    Some(device)
                } else {
                    None
                }
            } else if device.device_class == "Audio/Source" {
                device.capabilities.retain(|cap| match cap {
                    MediaCapability::Audio(audio_cap) => audio_cap.codec == "audio/x-raw",
                    _ => false,
                });
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

const CONFIG_FILE: &str = "composite_config.json";

/// Checks every device of `config`: it exists and supports the requested format, and the
/// combination is valid (no device or channel used twice, an AV mix with a camera and
/// mic 1, shared interfaces at one format).
fn verify_config(config: &CompositeConfig) -> Result<(), SyncFlowPublisherError> {
    let mut errors = Vec::new();
    for option in config.publish_options() {
        match verify_device_exists_and_supports_codec(&option) {
            Ok(true) => {}
            Ok(false) => errors.push(format!(
                "{} does not support the selected format",
                device_id(&option)
            )),
            Err(e) => errors.push(e.to_string()),
        }
    }
    let spec = config.to_stream_spec(PathBuf::new(), serde_json::Value::Null);
    if let Err(e) = spec.validate() {
        errors.push(e.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncFlowPublisherError::ConfigError(errors.join(", ")))
    }
}

fn device_id(option: &PublishOptions) -> &str {
    match option {
        PublishOptions::Audio(details) => &details.device_id,
        PublishOptions::Video(details) => &details.device_id,
        PublishOptions::Screen(details) => &details.screen_id_or_name,
    }
}

#[tauri::command(async)]
pub async fn set_streaming_config(
    config: CompositeConfig,
    app_state: tauri::State<'_, models::AppState>,
    app_handle: tauri::AppHandle,
) -> Result<CompositeConfig, SyncFlowPublisherError> {
    verify_config(&config)?;
    save_json(&config, &app_state.app_dir.join(CONFIG_FILE))?;
    {
        let mut guard = app_state.recording_and_streaming_config.lock().unwrap();
        *guard = Some(config.clone());
    }

    // Only initialize session listener for session mode
    if config.recording_mode == RecordingMode::SessionMode {
        let mut session_listener_guard = app_state.session_listener.lock().await;
        if session_listener_guard.is_none() {
            let listener = initialize_session_listener(&app_state.app_dir, app_handle.clone())
                .await
                .ok_or(SyncFlowPublisherError::InitializationError(
                    "Failed to initialize session listener".to_string(),
                ))?;
            *session_listener_guard = Some(listener);
        }
    }

    Ok(config)
}

/// Loads the saved config if every device in it is still usable. A config saved by an
/// older version (`selected_devices.json`) is not read: devices are configured again.
pub fn initialize_streaming_config(app_dir: &Path) -> Option<CompositeConfig> {
    let config_str = std::fs::read_to_string(app_dir.join(CONFIG_FILE)).ok()?;
    let config: CompositeConfig = serde_json::from_str(&config_str).ok()?;
    verify_config(&config).is_ok().then_some(config)
}

#[tauri::command]
pub fn get_streaming_config(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<CompositeConfig, SyncFlowPublisherError> {
    let guard = app_state.recording_and_streaming_config.lock().unwrap();
    guard.clone().ok_or_else(|| {
        SyncFlowPublisherError::NotIntialized("Streaming config not initialized".to_string())
    })
}

#[tauri::command(async)]
pub async fn delete_streaming_config(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<(), SyncFlowPublisherError> {
    let config_file = app_state.app_dir.join(CONFIG_FILE);
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

/// What the recording pipeline can do on this machine.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineCapabilities {
    /// Whether mic audio can be noise suppressed (`audiornnoise` is installed).
    noise_suppression: bool,
}

#[tauri::command]
pub fn get_pipeline_capabilities() -> PipelineCapabilities {
    PipelineCapabilities {
        noise_suppression: livekit_gstreamer::composite::builder::rnnoise_available(),
    }
}
