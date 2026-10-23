use display_info::DisplayInfo;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gstreamer::{prelude::*, Device, DeviceMonitor};

use crate::{AudioCapability, MediaCapability, MediaDeviceInfo, ScreenCapability, VideoCapability};

static GLOBAL_DEVICE_MONITOR: Lazy<Arc<Mutex<DeviceMonitor>>> = Lazy::new(|| {
    let monitor = DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    monitor.add_filter(Some("Audio/Source"), None);
    monitor.add_filter(Some("Source/Video"), None);
    monitor.add_filter(Some("Source/Audio"), None);
    if let Err(err) = monitor.start() {
        eprintln!("Failed to start global device monitor: {:?}", err);
    }
    Arc::new(Mutex::new(monitor))
});

const SUPPORTED_APIS: [&str; 4] = ["wasapi2", "mediafoundation", "directshow", "dshow"];

pub fn get_gst_device(path: &str) -> Option<Device> {
    let device_monitor = GLOBAL_DEVICE_MONITOR.clone();
    let device_monitor = device_monitor.lock().unwrap();
    let devices = device_monitor.devices();

    devices.into_iter().find(|d| {
        let props = d.properties();

        match props {
            Some(props) => {
                // Try matching against multiple possible properties
                let candidates = [
                    props.get::<Option<String>>("object.path"),
                    props.get::<Option<String>>("device.path"),
                    props.get::<Option<String>>("device.id"),
                ];

                // Return true if any property matches the given path
                candidates.iter().any(|res| {
                    res.clone()
                        .is_ok_and(|opt| opt.as_ref().is_some_and(|v| v.contains(path)))
                })
            }
            None => false,
        }
    })
}

pub fn get_device_capabilities(device: &Device) -> Vec<MediaCapability> {
    let caps = match device.caps() {
        Some(c) => c,
        None => return vec![],
    };

    if device.device_class() == "Video/Source" || device.device_class() == "Source/Video" {
        #[allow(clippy::unnecessary_filter_map)]
        caps.iter()
            .filter_map(|structure| {
                let width = structure.get::<i32>("width").ok();
                let height = structure.get::<i32>("height").ok();

                let mut framerates = vec![];
                if let Ok(framerate_list) = structure.get::<gstreamer::List>("framerate") {
                    for val in framerate_list.iter() {
                        if let Ok(frac) = val.get::<gstreamer::Fraction>() {
                            framerates.push(frac.numer() as f64 / frac.denom() as f64);
                        }
                    }
                } else if let Ok(framerate) = structure.get::<gstreamer::Fraction>("framerate") {
                    framerates.push(framerate.numer() as f64 / framerate.denom() as f64);
                }

                let codec = structure.name().to_string();

                Some(MediaCapability::Video(VideoCapability {
                    width: width.unwrap_or(0),
                    height: height.unwrap_or(0),
                    framerates: framerates.iter().map(|&f| f as i32).collect(),
                    codec,
                }))
            })
            .collect()
    } else {
        let mut capabilities = audio_capabilities(&caps, false);
        let exclusive = exclusive_capabilities(device)
            .into_iter()
            .filter(|native| !capabilities.iter().any(|shared| covers(shared, native)))
            .collect::<Vec<_>>();
        capabilities.extend(exclusive);
        capabilities
    }
}

/// Audio capabilities of `caps`, one per structure.
fn audio_capabilities(caps: &gstreamer::CapsRef, exclusive: bool) -> Vec<MediaCapability> {
    caps.iter()
        .map(|structure| {
            let channels = structure.get::<i32>("channels").unwrap_or(0);

            let mut rates = vec![];
            // Try to get a list of rates
            if let Ok(rate_list) = structure.get::<gstreamer::List>("rate") {
                for val in rate_list.iter() {
                    if let Ok(rate) = val.get::<i32>() {
                        rates.push(rate);
                    }
                }
            } else if let Ok(rate) = structure.get::<i32>("rate") {
                rates.push(rate);
            } else if let Ok(rate_range) = structure.get::<gstreamer::IntRange<i32>>("rate") {
                rates.push(rate_range.min());
                rates.push(rate_range.max());
            }

            MediaCapability::Audio(AudioCapability {
                channels,
                framerates: (
                    rates.iter().copied().min().unwrap_or(0),
                    rates.iter().copied().max().unwrap_or(0),
                ),
                codec: structure.name().to_string(),
                exclusive,
            })
        })
        .collect()
}

/// Whether the shared-mode capability `shared` already offers the format of `native`.
fn covers(shared: &MediaCapability, native: &MediaCapability) -> bool {
    match (shared, native) {
        (MediaCapability::Audio(s), MediaCapability::Audio(n)) => {
            s.codec == n.codec
                && s.channels == n.channels
                && s.framerates.0 <= n.framerates.0
                && s.framerates.1 >= n.framerates.1
        }
        _ => false,
    }
}

/// Exclusive-mode formats found so far, by device id. Probing opens the device, which
/// fails while another app (or a running pipeline) holds it, so only successes are kept.
static EXCLUSIVE_CAPABILITIES: Lazy<Mutex<HashMap<String, Vec<MediaCapability>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// The formats `device` offers in WASAPI exclusive mode: the device's own format (the
/// Sound panel's "Default Format"), which can differ from the shared-mode mix format that
/// the device monitor reports. For example, with Intel's USB audio offload a 48 kHz USB
/// mic is mixed at 44.1 kHz. Empty for anything but `wasapi2` capture devices.
fn exclusive_capabilities(device: &Device) -> Vec<MediaCapability> {
    let Some(props) = device.properties() else {
        return vec![];
    };
    if props.get::<String>("device.api").ok().as_deref() != Some("wasapi2")
        || props
            .get::<bool>("wasapi2.device.loopback")
            .unwrap_or(false)
    {
        return vec![];
    }
    let Ok(id) = props.get::<String>("device.id") else {
        return vec![];
    };
    if let Some(known) = EXCLUSIVE_CAPABILITIES.lock().unwrap().get(&id) {
        return known.clone();
    }
    let Some(caps) = probe_exclusive_caps(device) else {
        return vec![];
    };
    let capabilities = audio_capabilities(&caps, true);
    EXCLUSIVE_CAPABILITIES
        .lock()
        .unwrap()
        .insert(id, capabilities.clone());
    capabilities
}

/// Opens `device` in exclusive mode and reads the caps its source then offers. Before the
/// device is open the source reports only its template (any rate), which is rejected.
fn probe_exclusive_caps(device: &Device) -> Option<gstreamer::Caps> {
    let source = device.create_element(None).ok()?;
    source.find_property("exclusive")?;
    source.set_property("exclusive", true);
    let caps = source
        .set_state(gstreamer::State::Ready)
        .ok()
        .and_then(|_| source.static_pad("src"))
        .map(|pad| pad.query_caps(None));
    let _ = source.set_state(gstreamer::State::Null);
    let caps = caps?;
    let fixed_rates = !caps.is_empty()
        && caps
            .iter()
            .all(|s| s.get::<i32>("rate").is_ok() && s.get::<i32>("channels").is_ok());
    fixed_rates.then_some(caps)
}

fn get_device_path(device: &Device) -> Option<String> {
    let props = device.properties()?;
    let path = props
        .get("device.path")
        .unwrap_or_else(|_| props.get("device.id").ok().flatten());

    #[allow(clippy::manual_unwrap_or_default)]
    path.or_else(|| match props.get::<Option<String>>("device.path") {
        Ok(path) => path,
        Err(_) => None,
    })
}

fn get_device_class(device: &Device) -> String {
    match device.device_class().as_str() {
        "Video/Source" | "Source/Video" => "Video/Source".to_string(),
        "Audio/Source" | "Source/Audio" => "Audio/Source".to_string(),
        _ => device.device_class().to_string(),
    }
}

fn confirm_supported_api(device: &Device) -> Option<bool> {
    let api = device
        .properties()
        .and_then(|props| props.get::<String>("device.api").ok())
        .unwrap_or_default();

    SUPPORTED_APIS.contains(&api.as_str()).then_some(true)
}

pub fn parse_monitors_windows() -> Vec<MediaDeviceInfo> {
    let mut all_monitors = DisplayInfo::all().unwrap_or_else(|_| vec![]);
    all_monitors.sort_by_key(|m| (m.x, m.y));

    all_monitors
        .into_iter()
        .map(MediaDeviceInfo::from)
        .collect()
}

pub fn get_monitor(id_or_name: &str) -> Option<(MediaDeviceInfo, i32)> {
    let mut all_monitors = DisplayInfo::all().unwrap_or_else(|_| vec![]);

    all_monitors.sort_by_key(|m| (m.x, m.y));
    all_monitors
        .into_iter()
        .enumerate()
        .find(|(_, m)| m.id.to_string() == id_or_name || m.name == id_or_name)
        .map(|(idx, m)| (MediaDeviceInfo::from(m), idx as i32))
}

fn get_frame_rates(display_info: &DisplayInfo) -> Vec<i32> {
    let rate = display_info.frequency;
    let mut rates = vec![rate as i32];
    if rate > 30.0 {
        rates.push(30);
    }

    rates
}

impl From<DisplayInfo> for MediaDeviceInfo {
    fn from(display_info: DisplayInfo) -> Self {
        use std::vec;
        println!("Display Info: {:?}", display_info);

        let startx = display_info.x;
        let starty = display_info.y;

        let endx = startx + display_info.width as i32;
        let endy = starty + display_info.height as i32;

        let actual_width = display_info.width as i32;
        let actual_height = display_info.height as i32;

        MediaDeviceInfo {
            device_path: display_info.id.clone().to_string(),
            display_name: display_info.friendly_name.clone(),
            capabilities: vec![MediaCapability::Screen(ScreenCapability {
                width: actual_width,
                height: actual_height,
                framerates: get_frame_rates(&display_info),
                codec: "video/x-raw".to_string(),
                startx,
                starty,
                endx,
                endy,
            })],
            device_class: "Screen/Source".to_string(),
        }
    }
}

pub fn get_devices_info() -> Vec<MediaDeviceInfo> {
    let device_monitor = GLOBAL_DEVICE_MONITOR.clone();
    let device_monitor = device_monitor.lock().unwrap();
    let devices = device_monitor.devices();
    let mut devices = devices
        .into_iter()
        .filter_map(|d| {
            confirm_supported_api(&d)?;
            let path = get_device_path(&d)?;
            let caps = get_device_capabilities(&d);
            let display_name = d.display_name().into();
            let class = get_device_class(&d);
            Some(MediaDeviceInfo {
                device_path: path,
                display_name,
                capabilities: caps,
                device_class: class,
            })
        })
        .collect::<Vec<MediaDeviceInfo>>();

    devices.extend(parse_monitors_windows());

    devices
}
