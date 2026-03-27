use std::path::PathBuf;
use std::vec;

use livekit_gstreamer::get_devices_info;
use livekit_gstreamer::AudioCapability;
use livekit_gstreamer::AudioPublishOptions;
use livekit_gstreamer::GstMediaDevice;
use livekit_gstreamer::MediaCapability;
use livekit_gstreamer::MediaDeviceInfo;
use livekit_gstreamer::PublishOptions;
use livekit_gstreamer::ScreenPublishOptions;
use livekit_gstreamer::VideoCapability;
use livekit_gstreamer::VideoPublishOptions;

use crate::errors::SyncFlowPublisherError;
use crate::models;
use crate::models::DeviceRecordingAndStreamingConfig;
use crate::session_listener::initialize_session_listener;
use crate::utils::save_json;
use serde::{Deserialize, Serialize};

// Add this struct (e.g., in models.rs or at the top of this file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SciStoryDevices {
    pub camera_name: String,
    pub primary_microphone_name: String,
    pub usb_microphone_names: Vec<String>, // partial match used to find both USB mics
}

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

    // Only initialize session listener for session mode
    let is_session_mode = configs.iter().any(|config| {
        matches!(
            config.recording_mode,
            crate::models::RecordingMode::SessionMode
        )
    });
    if is_session_mode {
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

    Ok(configs)
}

pub fn get_best_publish_options_for_device(device: &MediaDeviceInfo) -> Option<PublishOptions> {
    match device.device_class.as_str() {
        "Video/Source" => {
            let mut best: Option<&VideoCapability> = None;
            for cap in device.capabilities.iter().filter_map(|c| match c {
                MediaCapability::Video(v) => Some(v),
                _ => None,
            }) {
                best = Some(match best {
                    None => cap,
                    Some(b) => {
                        let cap_fps = cap.framerates.iter().max().copied().unwrap_or(0);
                        let b_fps = b.framerates.iter().max().copied().unwrap_or(0);
                        if cap.width * cap.height > b.width * b.height
                            || (cap.width * cap.height == b.width * b.height && cap_fps > b_fps)
                        {
                            cap
                        } else {
                            b
                        }
                    }
                });
            }
            let cap = best?;
            let framerate = *cap.framerates.iter().max()?;
            Some(PublishOptions::Video(VideoPublishOptions {
                codec: cap.codec.clone(),
                width: cap.width,
                height: cap.height,
                framerate,
                device_id: device.device_path.clone(),
                local_file_save_options: None,
            }))
        }

        "Audio/Source" => {
            let mut best: Option<&AudioCapability> = None;
            for cap in device.capabilities.iter().filter_map(|c| match c {
                MediaCapability::Audio(a) => Some(a),
                _ => None,
            }) {
                best = Some(match best {
                    None => cap,
                    Some(b) => {
                        // Prefer higher sample rate, then more channels
                        if cap.framerates.1 > b.framerates.1
                            || (cap.framerates.1 == b.framerates.1 && cap.channels > b.channels)
                        {
                            cap
                        } else {
                            b
                        }
                    }
                });
            }
            let cap = best?;
            Some(PublishOptions::Audio(AudioPublishOptions {
                codec: cap.codec.clone(),
                channels: cap.channels,
                framerate: cap.framerates.1, // max sample rate
                device_id: device.device_path.clone(),
                selected_channel: None,
                local_file_save_options: None,
            }))
        }

        "Screen/Source" => {
            let cap = device.capabilities.iter().find_map(|c| match c {
                MediaCapability::Screen(s) => Some(s),
                _ => None,
            })?;
            // Cap at 30fps for screen
            let framerate = cap
                .framerates
                .iter()
                .filter(|&&f| f <= 30)
                .max()
                .copied()
                .unwrap_or(30);
            Some(PublishOptions::Screen(ScreenPublishOptions {
                codec: cap.codec.clone(),
                width: cap.width,
                height: cap.height,
                framerate,
                screen_id_or_name: device.device_path.clone(),
                local_file_save_options: None,
            }))
        }

        _ => None,
    }
}

pub fn initialize_streaming_config(
    app_dir: &PathBuf,
) -> Option<Vec<DeviceRecordingAndStreamingConfig>> {
    #[cfg(target_os = "windows")]
    {
        let sci_story_devices_file = app_dir.join("scistory_devices.json");
        if sci_story_devices_file.exists() {
            let sci_story_devices: SciStoryDevices =
                serde_json::from_str(&std::fs::read_to_string(&sci_story_devices_file).ok()?)
                    .ok()?;

            let devices = get_devices();

            let usb_camera = devices
                .iter()
                .find(|d| d.display_name.contains(&sci_story_devices.camera_name))?;
            let primary_microphone = devices.iter().find(|d| {
                d.display_name
                    .contains(&sci_story_devices.primary_microphone_name)
            })?;
            let usb_mics = devices
                .iter()
                .filter(|d| {
                    sci_story_devices
                        .usb_microphone_names
                        .iter()
                        .any(|name| d.display_name.contains(name))
                })
                .collect::<Vec<_>>();

            if usb_mics.len() != 2 {
                return None;
            }

            let screen = devices.iter().find(|d| d.device_class == "Screen/Source")?;

            let mut configs = vec![
                DeviceRecordingAndStreamingConfig {
                    publish_options: get_best_publish_options_for_device(usb_camera)?,
                    enable_streaming: false,
                    av_mix_mode: Some(models::AvMixMode::Primary),
                    recording_mode: models::RecordingMode::LocalMode,
                },
                DeviceRecordingAndStreamingConfig {
                    publish_options: get_best_publish_options_for_device(primary_microphone)?,
                    enable_streaming: false,
                    av_mix_mode: Some(models::AvMixMode::Mic1),
                    recording_mode: models::RecordingMode::LocalMode,
                },
                DeviceRecordingAndStreamingConfig {
                    publish_options: get_best_publish_options_for_device(screen)?,
                    enable_streaming: false,
                    av_mix_mode: None,
                    recording_mode: models::RecordingMode::LocalMode,
                },
            ];

            for mic in usb_mics {
                if let Some(opts) = get_best_publish_options_for_device(mic) {
                    configs.push(DeviceRecordingAndStreamingConfig {
                        publish_options: opts,
                        enable_streaming: true,
                        av_mix_mode: None,
                        recording_mode: models::RecordingMode::LocalMode,
                    });
                }
            }

            return Some(configs);
        }
    }

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
