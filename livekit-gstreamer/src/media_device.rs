//! Devices as GStreamer reports them: lookup by path, capabilities, format checks, and
//! the per-OS screen capture source.

use gstreamer::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::get_device_capabilities;
use crate::utils::random_string;
use crate::{get_gst_device, get_monitor};

/// A struct representing a GStreamer device
/// This implementation assumes that GStreamer is initialized elsewhere
#[derive(Debug, Clone)]
pub struct GstMediaDevice {
    pub display_name: String,
    #[allow(dead_code)]
    pub device_class: String,
    pub device_path: String,
}

impl GstMediaDevice {
    pub fn from_device_path(path: &str) -> Result<Self, GStreamerError> {
        let device = get_gst_device(path);
        let device =
            device.ok_or_else(|| GStreamerError::DeviceError("No device found".to_string()))?;
        let display_name: String = device.display_name().into();

        let device = GstMediaDevice {
            display_name,
            device_class: device.device_class().into(),
            device_path: path.into(),
        };
        Ok(device)
    }

    pub fn from_screen_id_or_name(screen_id_or_name: &str) -> Result<Self, GStreamerError> {
        let monitor = {
            #[cfg(target_os = "windows")]
            {
                let (monitor, _) = get_monitor(screen_id_or_name)
                    .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?;
                monitor
            }
            #[cfg(target_os = "linux")]
            {
                get_monitor(screen_id_or_name)
                    .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?
            }
            #[cfg(target_os = "macos")]
            {
                get_monitor(screen_id_or_name).ok_or_else(|| {
                    GStreamerError::DeviceError(format!(
                        "Screen with ID or name '{}' not found",
                        screen_id_or_name
                    ))
                })?
            }
        };

        let device = GstMediaDevice {
            display_name: monitor.display_name,
            device_class: "Screen/Source".to_string(),
            device_path: monitor.device_path,
        };

        Ok(device)
    }

    pub fn capabilities(&self) -> Vec<MediaCapability> {
        if self.device_class == "Screen/Source" {
            #[cfg(target_os = "windows")]
            {
                if let Some((monitor, _)) = get_monitor(&self.device_path) {
                    return monitor.capabilities;
                } else {
                    return vec![];
                }
            }

            #[cfg(target_os = "linux")]
            {
                return get_monitor(&self.device_path).map_or(vec![], |m| m.capabilities);
            }

            #[cfg(target_os = "macos")]
            {
                return get_monitor(&self.device_path).map_or(vec![], |m| m.capabilities);
            }
        }
        let device = get_gst_device(&self.device_path).unwrap();
        get_device_capabilities(&device)
    }

    pub fn supports_video(&self, codec: &str, width: i32, height: i32, framerate: i32) -> bool {
        let caps = self.capabilities();
        if self.device_class == "Audio/Source" {
            return false;
        }
        let caps = caps
            .iter()
            .filter_map(|c| match c {
                MediaCapability::Video(c) => Some(c),
                _ => None,
            })
            .collect::<Vec<_>>();

        caps.iter().any(|c| {
            c.codec == codec
                && c.width == width
                && c.height == height
                && c.framerates.contains(&framerate)
        })
    }

    pub fn supports_audio(
        &self,
        codec: &str,
        channels: i32,
        framerate: i32,
        exclusive: bool,
    ) -> bool {
        let caps = self.capabilities();
        if self.device_class == "Video/Source" {
            return false;
        }
        let caps = caps
            .iter()
            .filter_map(|c| match c {
                MediaCapability::Audio(c) => Some(c),
                _ => None,
            })
            .collect::<Vec<_>>();

        caps.iter().any(|c| {
            c.codec == codec
                && c.channels == channels
                && c.exclusive == exclusive
                && c.framerates.0 <= framerate
                && c.framerates.1 >= framerate
        })
    }

    pub fn supports_screen_share(
        &self,
        codec: &str,
        width: i32,
        height: i32,
        framerate: i32,
    ) -> bool {
        if self.device_class != "Screen/Source" {
            return false;
        }
        let caps = self.capabilities();
        let caps = caps
            .iter()
            .filter_map(|c| match c {
                MediaCapability::Screen(c) => Some(c),
                _ => None,
            })
            .collect::<Vec<_>>();
        caps.iter().any(|c| {
            c.codec == codec
                && c.width >= width
                && c.height >= height
                && c.framerates.contains(&framerate)
        })
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn get_screen_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let (_, idx) = get_monitor(&self.device_path)
            .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?;

        if gstreamer::ElementFactory::find("dx9screencapsrc").is_some() {
            let element = gstreamer::ElementFactory::make("dx9screencapsrc")
                .name(random_string("screen-source"))
                .build()
                .map_err(|_| {
                    GStreamerError::PipelineError("Failed to create dxgiscreencapsrc".to_string())
                })?;

            element.set_property("monitor", idx);
            element.set_property("cursor", true);

            Ok(element)
        } else {
            Err(GStreamerError::PipelineError(
                "dx9screencapsrc not found".to_string(),
            ))
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn get_screen_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let monitor = get_monitor(&self.device_path).ok_or_else(|| {
            GStreamerError::DeviceError(format!("No screen found {}", self.device_path))
        })?;

        let element = gstreamer::ElementFactory::make("avfvideosrc")
            .name(random_string("screen-source"))
            .build()
            .map_err(|_| {
                GStreamerError::PipelineError("Failed to create osxvideosrc".to_string())
            })?;

        if let Some(MediaCapability::Screen(cap)) = monitor.capabilities.first() {
            element.set_property("capture-screen", true);
            element.set_property("capture-screen-cursor", true);
            element.set_property("screen-crop-x", cap.startx as u32);
            element.set_property("screen-crop-y", cap.starty as u32);
            element.set_property("screen-crop-width", cap.endx as u32 - cap.startx as u32);
            element.set_property("screen-crop-height", cap.endy as u32 - cap.starty as u32);
        } else {
            return Err(GStreamerError::PipelineError(
                "No screen capability found".to_string(),
            ));
        }

        Ok(element)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn get_screen_element(&self) -> Result<gstreamer::Element, GStreamerError> {
        let monitor = get_monitor(&self.device_path)
            .ok_or_else(|| GStreamerError::DeviceError("No screen found".to_string()))?;

        let element = gstreamer::ElementFactory::make("ximagesrc")
            .name(random_string("screen-source"))
            .build()
            .map_err(|_| GStreamerError::PipelineError("Failed to create ximagesrc".to_string()))?;

        if let Some(MediaCapability::Screen(cap)) = monitor.capabilities.first() {
            element.set_property("use-damage", false);
            element.set_property("show-pointer", true);
            element.set_property("startx", cap.startx as u32);
            element.set_property("starty", cap.starty as u32);
            element.set_property("endx", cap.endx as u32 - 1);
            element.set_property("endy", cap.endy as u32 - 1);
        } else {
            return Err(GStreamerError::PipelineError(
                "No screen capability found".to_string(),
            ));
        }

        Ok(element)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCapability {
    pub width: i32,
    pub height: i32,
    pub framerates: Vec<i32>,
    pub codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCapability {
    pub channels: i32,
    pub framerates: (i32, i32),
    pub codec: String,
    /// Offered only when the device is opened in exclusive mode (Windows WASAPI): the
    /// device's own format, which can differ from the shared-mode mix format.
    #[serde(default)]
    pub exclusive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCapability {
    pub width: i32,
    pub height: i32,
    pub framerates: Vec<i32>,
    pub codec: String,
    pub startx: i32,
    pub starty: i32,
    pub endx: i32,
    pub endy: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDeviceInfo {
    pub device_path: String,
    pub display_name: String,
    pub capabilities: Vec<MediaCapability>,
    pub device_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum MediaCapability {
    Video(VideoCapability),
    Audio(AudioCapability),
    Screen(ScreenCapability), // For screen capture capabilities
}

#[derive(Debug, Clone, Error)]
pub enum GStreamerError {
    #[error("Failed to create pipeline: {0}")]
    PipelineError(String),
    #[error("Devices: {0}")]
    DeviceError(String),
}
