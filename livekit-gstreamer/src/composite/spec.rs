//! What a composite pipeline should contain, and the resolved graph plan derived from it.

use super::ids::{DeviceKey, DeviceKind, DeviceRef, OutputId, Role, RoleRef, SinkId};
use super::naming::output_file_name;
use crate::{AudioPublishOptions, ScreenPublishOptions, VideoPublishOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

/// Size of every streamed video track.
pub const STREAM_VIDEO_WIDTH: u32 = 640;
pub const STREAM_VIDEO_HEIGHT: u32 = 480;
/// Every streamed audio track is mono at this rate (what `audiornnoise` requires).
pub const STREAM_AUDIO_RATE: u32 = 48_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositeStreamSpec {
    pub av_mix: Option<AvMixSpec>,
    pub screen: Option<Output<ScreenPublishOptions>>,
    #[serde(default)]
    pub cameras: Vec<Output<VideoPublishOptions>>,
    #[serde(default)]
    pub mics: Vec<Output<AudioPublishOptions>>,
    /// `None` builds no file branches (stream only).
    pub output_dir: Option<PathBuf>,
    #[serde(default)]
    pub stream_audio: StreamAudioOptions,
    #[serde(default)]
    pub encoders: EncoderOptions,
    #[serde(default)]
    pub start_policy: StartPolicy,
    /// Caller context written verbatim under `context` in `manifest.json`
    /// (for example session and participant details).
    #[serde(default)]
    pub manifest_context: Option<serde_json::Value>,
}

/// Camera plus 1–2 mics recorded to `av-mix.mp4`. With two mics the audio is
/// stereo with mic-1 on the left and mic-2 on the right.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvMixSpec {
    pub camera: VideoPublishOptions,
    pub mics: Vec<AudioPublishOptions>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output<T> {
    pub options: T,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamAudioOptions {
    /// Run `audiornnoise` on every streamed audio track. Files keep raw audio.
    pub noise_suppression: bool,
    /// `audiornnoise` `voice-activity-threshold`; 0.0 disables gating.
    pub vad_threshold: f32,
}

impl Default for StreamAudioOptions {
    fn default() -> Self {
        Self {
            noise_suppression: true,
            vad_threshold: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum H264Encoder {
    /// Hardware encoder when available (`vtenc_h264_hw`, `mfh264enc`), else `x264enc`.
    #[default]
    Auto,
    X264,
    Hardware,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderOptions {
    pub h264: H264Encoder,
    /// Video bitrate per pixel per frame; each file's bitrate is
    /// `width × height × fps × video_bits_per_pixel`. 0.12 gives 8.7 Mbps at
    /// 1552×1552@30 and 3.3 Mbps at 1280×720@30.
    pub video_bits_per_pixel: f64,
    /// A fixed video bitrate for every file instead, in kbps.
    pub video_kbps: Option<u32>,
    pub aac_kbps_per_channel: u32,
}

impl Default for EncoderOptions {
    fn default() -> Self {
        Self {
            h264: H264Encoder::Auto,
            video_bits_per_pixel: 0.12,
            video_kbps: None,
            aac_kbps_per_channel: 128,
        }
    }
}

/// Lowest video bitrate used, whatever the resolution.
const MIN_VIDEO_KBPS: u32 = 500;

impl EncoderOptions {
    /// Video bitrate in kbps for one file of the given size and frame rate.
    pub fn video_kbps_for(&self, width: i32, height: i32, framerate: i32) -> u32 {
        if let Some(kbps) = self.video_kbps {
            return kbps;
        }
        let pixels_per_second =
            f64::from(width.max(0)) * f64::from(height.max(0)) * f64::from(framerate.max(0));
        let kbps = (pixels_per_second * self.video_bits_per_pixel / 1000.0).round();
        (kbps as u32).max(MIN_VIDEO_KBPS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartPolicy {
    /// Start without devices that fail to open and report them.
    #[default]
    BestEffort,
    /// Fail `start()` if any device fails to open.
    AllOrNothing,
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum SpecError {
    #[error("the spec has no devices")]
    Empty,
    #[error("the AV mix needs 1 or 2 mics, got {0}")]
    MixMicCount(usize),
    #[error("device {device_id} channel {selected_channel:?} is used more than once")]
    DuplicateSource {
        device_id: String,
        selected_channel: Option<i32>,
    },
    #[error("device {device_id} is used with conflicting formats: {reason}")]
    ConflictingDeviceOptions { device_id: String, reason: String },
    #[error(
        "device {device_id} channel {selected_channel} is out of range (device has {channels})"
    )]
    ChannelOutOfRange {
        device_id: String,
        selected_channel: i32,
        channels: i32,
    },
    #[error("{0} is neither recorded nor streamed")]
    NothingToDo(String),
    #[error("the spec has more than 255 devices or roles of one kind")]
    TooManyDevices,
}

/// Source options for one physical device. A device shared by several mic roles
/// is opened once with these options.
#[derive(Debug, Clone)]
pub enum DeviceOptions {
    Video(VideoPublishOptions),
    Audio(AudioPublishOptions),
    Screen(ScreenPublishOptions),
}

#[derive(Debug, Clone)]
pub struct ResolvedDevice {
    pub device: DeviceRef,
    pub options: DeviceOptions,
}

/// L/R placement of the AV mix mics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioChannelMap {
    #[serde(rename = "L")]
    pub left: Role,
    #[serde(rename = "R")]
    pub right: Role,
}

#[derive(Debug, Clone)]
pub struct ResolvedOutput {
    pub id: OutputId,
    pub file_name: String,
    pub roles: Vec<Role>,
    pub audio_channels: Option<AudioChannelMap>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SinkKind {
    Video { width: u32, height: u32 },
    Audio { rate: u32, channels: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SinkInfo {
    pub id: SinkId,
    pub roles: Vec<Role>,
    pub kind: SinkKind,
}

/// The whole graph plan as plain data: devices, roles, files, appsinks and the edges
/// between them. Built by [`CompositeStreamSpec::validate`]; read by the builder,
/// the bus (error attribution) and the manifest.
#[derive(Debug, Clone)]
pub struct ResolvedSpec {
    pub devices: Vec<ResolvedDevice>,
    pub roles: Vec<RoleRef>,
    pub outputs: Vec<ResolvedOutput>,
    pub sinks: Vec<SinkInfo>,
    pub output_dir: Option<PathBuf>,
    pub stream_audio: StreamAudioOptions,
    pub encoders: EncoderOptions,
    pub start_policy: StartPolicy,
    pub manifest_context: Option<serde_json::Value>,
}

impl CompositeStreamSpec {
    /// Checks the spec and assigns device keys, roles, outputs and sinks, looking up
    /// device display names (used in file names) from the system's devices.
    pub fn validate(&self) -> Result<ResolvedSpec, SpecError> {
        let names = crate::devices::get_devices_info()
            .into_iter()
            .map(|d| (d.device_path, d.display_name))
            .collect();
        self.validate_with_names(&names)
    }

    /// Like [`validate`](Self::validate), with display names keyed by device id.
    /// Devices missing from `names` are named by their id.
    ///
    /// Roles are numbered globally per kind: the AV mix takes `camera-1`, `mic-1` and
    /// `mic-2`, and additional cameras and mics continue from there. Format support
    /// is checked when the pipeline starts, not here.
    pub fn validate_with_names(
        &self,
        names: &HashMap<String, String>,
    ) -> Result<ResolvedSpec, SpecError> {
        if self.av_mix.is_none()
            && self.screen.is_none()
            && self.cameras.is_empty()
            && self.mics.is_empty()
        {
            return Err(SpecError::Empty);
        }
        let records = self.output_dir.is_some();
        let mut resolver = Resolver::new(names);
        let mut outputs = Vec::new();
        let mut sinks = Vec::new();
        let mut cameras = 0;
        let mut mics = 0;

        if let Some(mix) = &self.av_mix {
            if !(1..=2).contains(&mix.mics.len()) {
                return Err(SpecError::MixMicCount(mix.mics.len()));
            }
            if !records && !mix.stream {
                return Err(SpecError::NothingToDo(OutputId::AvMix.to_string()));
            }
            let camera = resolver.add_camera(next_index(&mut cameras)?, &mix.camera)?;
            let mix_mics = mix
                .mics
                .iter()
                .map(|options| resolver.add_mic(next_index(&mut mics)?, options))
                .collect::<Result<Vec<_>, _>>()?;
            let mic_roles: Vec<Role> = mix_mics.iter().map(|m| m.role).collect();

            if records {
                outputs.push(ResolvedOutput {
                    id: OutputId::AvMix,
                    file_name: output_file_name(OutputId::AvMix, "", None),
                    roles: std::iter::once(camera.role)
                        .chain(mic_roles.clone())
                        .collect(),
                    audio_channels: match mic_roles.as_slice() {
                        [left, right] => Some(AudioChannelMap {
                            left: *left,
                            right: *right,
                        }),
                        _ => None,
                    },
                });
                for mic in &mix_mics {
                    outputs.push(resolver.file_output(mic));
                }
            }
            if mix.stream {
                sinks.push(video_sink(SinkId::AvMixVideo, camera.role));
                sinks.push(SinkInfo {
                    id: SinkId::AvMixAudio,
                    roles: mic_roles,
                    kind: audio_sink_kind(),
                });
            }
        }

        if let Some(screen) = &self.screen {
            let role = resolver.add_screen(1, &screen.options)?;
            resolver.add_output(&role, screen.stream, records, &mut outputs, &mut sinks)?;
        }
        for camera in &self.cameras {
            let role = resolver.add_camera(next_index(&mut cameras)?, &camera.options)?;
            resolver.add_output(&role, camera.stream, records, &mut outputs, &mut sinks)?;
        }
        for mic in &self.mics {
            let role = resolver.add_mic(next_index(&mut mics)?, &mic.options)?;
            resolver.add_output(&role, mic.stream, records, &mut outputs, &mut sinks)?;
        }

        Ok(ResolvedSpec {
            devices: resolver.devices,
            roles: resolver.roles,
            outputs,
            sinks,
            output_dir: self.output_dir.clone(),
            stream_audio: self.stream_audio.clone(),
            encoders: self.encoders.clone(),
            start_policy: self.start_policy,
            manifest_context: self.manifest_context.clone(),
        })
    }
}

fn next_index(counter: &mut usize) -> Result<u8, SpecError> {
    *counter += 1;
    u8::try_from(*counter).map_err(|_| SpecError::TooManyDevices)
}

fn video_sink(id: SinkId, role: Role) -> SinkInfo {
    SinkInfo {
        id,
        roles: vec![role],
        kind: SinkKind::Video {
            width: STREAM_VIDEO_WIDTH,
            height: STREAM_VIDEO_HEIGHT,
        },
    }
}

fn audio_sink_kind() -> SinkKind {
    SinkKind::Audio {
        rate: STREAM_AUDIO_RATE,
        channels: 1,
    }
}

/// Assigns device keys and roles while checking for reused sources.
struct Resolver<'a> {
    names: &'a HashMap<String, String>,
    devices: Vec<ResolvedDevice>,
    roles: Vec<RoleRef>,
    /// (kind, device id, selected channel) of every role, to reject duplicates.
    sources: Vec<(DeviceKind, String, Option<i32>)>,
}

impl<'a> Resolver<'a> {
    fn new(names: &'a HashMap<String, String>) -> Self {
        Self {
            names,
            devices: Vec::new(),
            roles: Vec::new(),
            sources: Vec::new(),
        }
    }

    fn claim_source(
        &mut self,
        kind: DeviceKind,
        device_id: &str,
        selected_channel: Option<i32>,
    ) -> Result<(), SpecError> {
        let source = (kind, device_id.to_string(), selected_channel);
        if self.sources.contains(&source) {
            return Err(SpecError::DuplicateSource {
                device_id: device_id.to_string(),
                selected_channel,
            });
        }
        self.sources.push(source);
        Ok(())
    }

    /// Returns the key of an already-known device, or registers a new one.
    fn device_key(
        &mut self,
        kind: DeviceKind,
        device_id: &str,
        codec: &str,
        options: DeviceOptions,
    ) -> Result<DeviceKey, SpecError> {
        if let Some(existing) = self
            .devices
            .iter()
            .find(|d| d.device.kind == kind && d.device.device_id == device_id)
        {
            if let (DeviceOptions::Audio(known), DeviceOptions::Audio(new)) =
                (&existing.options, &options)
            {
                if let Some(reason) = audio_conflict(known, new) {
                    return Err(SpecError::ConflictingDeviceOptions {
                        device_id: device_id.to_string(),
                        reason,
                    });
                }
            }
            return Ok(existing.device.key);
        }
        let key =
            DeviceKey(u8::try_from(self.devices.len()).map_err(|_| SpecError::TooManyDevices)?);
        self.devices.push(ResolvedDevice {
            device: DeviceRef {
                key,
                device_id: device_id.to_string(),
                display_name: self
                    .names
                    .get(device_id)
                    .cloned()
                    .unwrap_or_else(|| device_id.to_string()),
                kind,
                codec: codec.to_string(),
            },
            options,
        });
        Ok(key)
    }

    fn push_role(
        &mut self,
        role: Role,
        device: DeviceKey,
        selected_channel: Option<u8>,
    ) -> RoleRef {
        let role_ref = RoleRef {
            role,
            device,
            selected_channel,
        };
        self.roles.push(role_ref);
        role_ref
    }

    fn add_camera(&mut self, n: u8, options: &VideoPublishOptions) -> Result<RoleRef, SpecError> {
        self.claim_source(DeviceKind::Camera, &options.device_id, None)?;
        let mut device_options = options.clone();
        device_options.local_file_save_options = None;
        let key = self.device_key(
            DeviceKind::Camera,
            &options.device_id,
            &options.codec,
            DeviceOptions::Video(device_options),
        )?;
        Ok(self.push_role(Role::Camera(n), key, None))
    }

    fn add_screen(&mut self, n: u8, options: &ScreenPublishOptions) -> Result<RoleRef, SpecError> {
        self.claim_source(DeviceKind::Screen, &options.screen_id_or_name, None)?;
        let mut device_options = options.clone();
        device_options.local_file_save_options = None;
        let key = self.device_key(
            DeviceKind::Screen,
            &options.screen_id_or_name,
            &options.codec,
            DeviceOptions::Screen(device_options),
        )?;
        Ok(self.push_role(Role::Screen(n), key, None))
    }

    fn add_mic(&mut self, n: u8, options: &AudioPublishOptions) -> Result<RoleRef, SpecError> {
        let selected_channel = match options.selected_channel {
            None => None,
            Some(channel) => Some(
                u8::try_from(channel)
                    .ok()
                    .filter(|c| *c >= 1 && i32::from(*c) <= options.channels)
                    .ok_or_else(|| SpecError::ChannelOutOfRange {
                        device_id: options.device_id.clone(),
                        selected_channel: channel,
                        channels: options.channels,
                    })?,
            ),
        };
        self.claim_source(
            DeviceKind::Mic,
            &options.device_id,
            options.selected_channel,
        )?;
        // The device is opened once with all its channels; roles pick their channel.
        let mut device_options = options.clone();
        device_options.selected_channel = None;
        device_options.local_file_save_options = None;
        let key = self.device_key(
            DeviceKind::Mic,
            &options.device_id,
            &options.codec,
            DeviceOptions::Audio(device_options),
        )?;
        Ok(self.push_role(Role::Mic(n), key, selected_channel))
    }

    fn display_name(&self, key: DeviceKey) -> &str {
        self.devices
            .iter()
            .find(|d| d.device.key == key)
            .map_or("", |d| d.device.display_name.as_str())
    }

    /// The file of a single-role output (screen, camera or mic).
    fn file_output(&self, role: &RoleRef) -> ResolvedOutput {
        let id = match role.role {
            Role::Camera(n) => OutputId::Camera(n),
            Role::Mic(n) => OutputId::Mic(n),
            Role::Screen(n) => OutputId::Screen(n),
        };
        ResolvedOutput {
            id,
            file_name: output_file_name(id, self.display_name(role.device), role.selected_channel),
            roles: vec![role.role],
            audio_channels: None,
        }
    }

    /// Adds the file and optional stream of a single-role output.
    fn add_output(
        &self,
        role: &RoleRef,
        stream: bool,
        records: bool,
        outputs: &mut Vec<ResolvedOutput>,
        sinks: &mut Vec<SinkInfo>,
    ) -> Result<(), SpecError> {
        if !records && !stream {
            return Err(SpecError::NothingToDo(role.role.to_string()));
        }
        if records {
            outputs.push(self.file_output(role));
        }
        if stream {
            sinks.push(match role.role {
                Role::Camera(n) => video_sink(SinkId::Camera(n), role.role),
                Role::Screen(n) => video_sink(SinkId::Screen(n), role.role),
                Role::Mic(n) => SinkInfo {
                    id: SinkId::Mic(n),
                    roles: vec![role.role],
                    kind: audio_sink_kind(),
                },
            });
        }
        Ok(())
    }
}

/// Why two roles can't share one opened audio device, if they can't.
fn audio_conflict(known: &AudioPublishOptions, new: &AudioPublishOptions) -> Option<String> {
    if known.codec != new.codec {
        Some(format!("codec {} vs {}", known.codec, new.codec))
    } else if known.framerate != new.framerate {
        Some(format!("rate {} vs {}", known.framerate, new.framerate))
    } else if known.channels != new.channels {
        Some(format!("channels {} vs {}", known.channels, new.channels))
    } else {
        None
    }
}

impl ResolvedSpec {
    pub fn device(&self, key: DeviceKey) -> Option<&ResolvedDevice> {
        self.devices.iter().find(|d| d.device.key == key)
    }

    pub fn device_of(&self, role: Role) -> Option<DeviceKey> {
        self.roles.iter().find(|r| r.role == role).map(|r| r.device)
    }

    /// The L/R placement of the AV mix mics, when the AV mix has two (recorded or
    /// streamed).
    pub fn lr_mix(&self) -> Option<AudioChannelMap> {
        self.outputs
            .iter()
            .find(|o| o.id == OutputId::AvMix)
            .and_then(|o| o.audio_channels)
            .or_else(|| {
                self.sinks
                    .iter()
                    .find(|s| s.id == SinkId::AvMixAudio)
                    .and_then(|s| match s.roles.as_slice() {
                        [left, right] => Some(AudioChannelMap {
                            left: *left,
                            right: *right,
                        }),
                        _ => None,
                    })
            })
    }

    /// Whether `role` is one of the AV mix mics (recorded or streamed).
    pub fn is_mix_role(&self, role: Role) -> bool {
        self.outputs
            .iter()
            .filter(|o| o.id == OutputId::AvMix)
            .flat_map(|o| &o.roles)
            .chain(
                self.sinks
                    .iter()
                    .filter(|s| s.id == SinkId::AvMixAudio)
                    .flat_map(|s| &s.roles),
            )
            .any(|r| *r == role)
    }

    pub fn roles_of(&self, key: DeviceKey) -> Vec<Role> {
        self.roles
            .iter()
            .filter(|r| r.device == key)
            .map(|r| r.role)
            .collect()
    }

    pub fn outputs_fed_by(&self, key: DeviceKey) -> Vec<OutputId> {
        let roles = self.roles_of(key);
        self.outputs
            .iter()
            .filter(|o| o.roles.iter().any(|r| roles.contains(r)))
            .map(|o| o.id)
            .collect()
    }

    pub fn sinks_fed_by(&self, key: DeviceKey) -> Vec<SinkId> {
        let roles = self.roles_of(key);
        self.sinks
            .iter()
            .filter(|s| s.roles.iter().any(|r| roles.contains(r)))
            .map(|s| s.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(id: &str) -> VideoPublishOptions {
        VideoPublishOptions {
            codec: "image/jpeg".into(),
            device_id: id.into(),
            width: 1280,
            height: 720,
            framerate: 30,
            local_file_save_options: None,
        }
    }

    fn mic(id: &str, channels: i32, selected_channel: Option<i32>) -> AudioPublishOptions {
        AudioPublishOptions {
            codec: "audio/x-raw".into(),
            device_id: id.into(),
            framerate: 48000,
            channels,
            selected_channel,
            local_file_save_options: None,
        }
    }

    fn screen(id: &str) -> ScreenPublishOptions {
        ScreenPublishOptions {
            codec: "video/x-raw".into(),
            screen_id_or_name: id.into(),
            width: 1920,
            height: 1080,
            framerate: 15,
            local_file_save_options: None,
        }
    }

    fn out<T>(options: T, stream: bool) -> Output<T> {
        Output { options, stream }
    }

    fn names() -> HashMap<String, String> {
        [
            ("cam", "FaceTime HD Camera"),
            ("c920-a", "Logitech C920"),
            ("c920-b", "Logitech C920"),
            ("mic-a", "USB Mic A"),
            ("mic-b", "USB Mic B"),
            ("umc", "UMC1820"),
            ("display", "Display 1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn recording() -> CompositeStreamSpec {
        CompositeStreamSpec {
            output_dir: Some(PathBuf::from("/tmp/recording")),
            ..Default::default()
        }
    }

    fn av_mix(mics: Vec<AudioPublishOptions>, stream: bool) -> Option<AvMixSpec> {
        Some(AvMixSpec {
            camera: camera("cam"),
            mics,
            stream,
        })
    }

    fn resolve(spec: &CompositeStreamSpec) -> ResolvedSpec {
        spec.validate_with_names(&names()).unwrap()
    }

    fn files(spec: &ResolvedSpec) -> Vec<&str> {
        spec.outputs.iter().map(|o| o.file_name.as_str()).collect()
    }

    fn sink_ids(spec: &ResolvedSpec) -> Vec<SinkId> {
        spec.sinks.iter().map(|s| s.id).collect()
    }

    fn output(spec: &ResolvedSpec, id: OutputId) -> &ResolvedOutput {
        spec.outputs.iter().find(|o| o.id == id).unwrap()
    }

    #[test]
    fn video_bitrate_scales_with_resolution() {
        let encoders = EncoderOptions::default();
        assert_eq!(encoders.video_kbps_for(1552, 1552, 30), 8671);
        assert_eq!(encoders.video_kbps_for(1280, 720, 30), 3318);
        assert_eq!(encoders.video_kbps_for(1512, 982, 15), 2673);
        assert_eq!(encoders.video_kbps_for(160, 120, 5), MIN_VIDEO_KBPS);
        let fixed = EncoderOptions {
            video_kbps: Some(2000),
            ..Default::default()
        };
        assert_eq!(fixed.video_kbps_for(1920, 1080, 30), 2000);
    }

    #[test]
    fn c1_av_mix_one_mic() {
        let resolved = resolve(&CompositeStreamSpec {
            av_mix: av_mix(vec![mic("mic-a", 1, None)], true),
            ..recording()
        });
        assert_eq!(resolved.devices.len(), 2);
        assert_eq!(files(&resolved), ["av-mix.mp4", "mic-1_USB-Mic-A.m4a"]);
        let mix = output(&resolved, OutputId::AvMix);
        assert_eq!(mix.roles, [Role::Camera(1), Role::Mic(1)]);
        assert_eq!(mix.audio_channels, None);
        assert_eq!(
            sink_ids(&resolved),
            [SinkId::AvMixVideo, SinkId::AvMixAudio]
        );
        assert_eq!(
            resolved.sinks[1].kind,
            SinkKind::Audio {
                rate: 48000,
                channels: 1
            }
        );
    }

    #[test]
    fn c2_av_mix_two_mics_left_right() {
        let resolved = resolve(&CompositeStreamSpec {
            av_mix: av_mix(vec![mic("mic-a", 1, None), mic("mic-b", 1, None)], true),
            ..recording()
        });
        assert_eq!(
            files(&resolved),
            ["av-mix.mp4", "mic-1_USB-Mic-A.m4a", "mic-2_USB-Mic-B.m4a"]
        );
        assert_eq!(
            output(&resolved, OutputId::AvMix).audio_channels,
            Some(AudioChannelMap {
                left: Role::Mic(1),
                right: Role::Mic(2)
            })
        );
        // Streaming only sees the mix, never the individual mics.
        assert_eq!(
            sink_ids(&resolved),
            [SinkId::AvMixVideo, SinkId::AvMixAudio]
        );
        assert_eq!(resolved.sinks[1].roles, [Role::Mic(1), Role::Mic(2)]);
    }

    #[test]
    fn c3_av_mix_with_screen() {
        let resolved = resolve(&CompositeStreamSpec {
            av_mix: av_mix(vec![mic("mic-a", 1, None), mic("mic-b", 1, None)], true),
            screen: Some(out(screen("display"), true)),
            ..recording()
        });
        assert!(files(&resolved).contains(&"screen-1_Display-1.mp4"));
        assert!(sink_ids(&resolved).contains(&SinkId::Screen(1)));
    }

    #[test]
    fn c4_extras_continue_numbering_with_own_stream_toggles() {
        let resolved = resolve(&CompositeStreamSpec {
            av_mix: av_mix(vec![mic("mic-a", 1, None), mic("mic-b", 1, None)], false),
            cameras: vec![out(camera("c920-a"), true)],
            mics: vec![out(mic("umc", 4, Some(2)), false)],
            ..recording()
        });
        assert_eq!(
            files(&resolved),
            [
                "av-mix.mp4",
                "mic-1_USB-Mic-A.m4a",
                "mic-2_USB-Mic-B.m4a",
                "camera-2_Logitech-C920.mp4",
                "mic-3_UMC1820-ch2.m4a",
            ]
        );
        assert_eq!(sink_ids(&resolved), [SinkId::Camera(2)]);
    }

    #[test]
    fn c5_extras_only_start_at_one() {
        let resolved = resolve(&CompositeStreamSpec {
            cameras: vec![out(camera("c920-a"), true)],
            mics: vec![out(mic("mic-a", 1, None), true)],
            ..recording()
        });
        assert_eq!(
            files(&resolved),
            ["camera-1_Logitech-C920.mp4", "mic-1_USB-Mic-A.m4a"]
        );
        assert!(resolved.outputs.iter().all(|o| o.id != OutputId::AvMix));
        assert_eq!(sink_ids(&resolved), [SinkId::Camera(1), SinkId::Mic(1)]);
    }

    #[test]
    fn c6_shared_multichannel_device_opens_once() {
        let resolved = resolve(&CompositeStreamSpec {
            av_mix: av_mix(vec![mic("umc", 4, Some(1)), mic("umc", 4, Some(2))], false),
            mics: vec![out(mic("umc", 4, Some(3)), false)],
            ..recording()
        });
        assert_eq!(resolved.devices.len(), 2);
        let umc = resolved.devices[1].device.key;
        assert_eq!(
            resolved.roles_of(umc),
            [Role::Mic(1), Role::Mic(2), Role::Mic(3)]
        );
        assert_eq!(
            resolved.outputs_fed_by(umc),
            [
                OutputId::AvMix,
                OutputId::Mic(1),
                OutputId::Mic(2),
                OutputId::Mic(3)
            ]
        );
        match &resolved.devices[1].options {
            DeviceOptions::Audio(options) => {
                assert_eq!(options.channels, 4);
                assert_eq!(options.selected_channel, None);
            }
            other => panic!("expected audio options, got {other:?}"),
        }
        assert!(files(&resolved).contains(&"mic-3_UMC1820-ch3.m4a"));
    }

    #[test]
    fn c7_record_only_has_no_sinks() {
        let resolved = resolve(&CompositeStreamSpec {
            av_mix: av_mix(vec![mic("mic-a", 1, None)], false),
            screen: Some(out(screen("display"), false)),
            ..recording()
        });
        assert!(resolved.sinks.is_empty());
        assert_eq!(resolved.outputs.len(), 3);
    }

    #[test]
    fn c8_stream_only_has_no_files() {
        let resolved = resolve(&CompositeStreamSpec {
            av_mix: av_mix(vec![mic("mic-a", 1, None)], true),
            mics: vec![out(mic("mic-b", 1, None), true)],
            ..Default::default()
        });
        assert!(resolved.outputs.is_empty());
        assert_eq!(
            sink_ids(&resolved),
            [SinkId::AvMixVideo, SinkId::AvMixAudio, SinkId::Mic(2)]
        );
    }

    #[test]
    fn c9_identical_models_get_unique_names() {
        let resolved = resolve(&CompositeStreamSpec {
            cameras: vec![out(camera("c920-a"), false), out(camera("c920-b"), false)],
            ..recording()
        });
        assert_eq!(resolved.devices.len(), 2);
        assert_eq!(
            files(&resolved),
            ["camera-1_Logitech-C920.mp4", "camera-2_Logitech-C920.mp4"]
        );
    }

    #[test]
    fn unknown_devices_are_named_by_id() {
        let resolved = resolve(&CompositeStreamSpec {
            mics: vec![out(mic("0x1420 unknown", 1, None), false)],
            ..recording()
        });
        assert_eq!(files(&resolved), ["mic-1_0x1420-unknown.m4a"]);
    }

    fn rejects(spec: CompositeStreamSpec) -> SpecError {
        spec.validate_with_names(&names()).unwrap_err()
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(rejects(recording()), SpecError::Empty);
    }

    #[test]
    fn rejects_mix_mic_count() {
        assert_eq!(
            rejects(CompositeStreamSpec {
                av_mix: av_mix(vec![], true),
                ..recording()
            }),
            SpecError::MixMicCount(0)
        );
        let three = vec![
            mic("mic-a", 1, None),
            mic("mic-b", 1, None),
            mic("umc", 4, Some(1)),
        ];
        assert_eq!(
            rejects(CompositeStreamSpec {
                av_mix: av_mix(three, true),
                ..recording()
            }),
            SpecError::MixMicCount(3)
        );
    }

    #[test]
    fn rejects_reused_camera() {
        assert_eq!(
            rejects(CompositeStreamSpec {
                av_mix: av_mix(vec![mic("mic-a", 1, None)], true),
                cameras: vec![out(camera("cam"), true)],
                ..recording()
            }),
            SpecError::DuplicateSource {
                device_id: "cam".into(),
                selected_channel: None
            }
        );
    }

    #[test]
    fn rejects_reused_mic_channel() {
        assert_eq!(
            rejects(CompositeStreamSpec {
                av_mix: av_mix(vec![mic("umc", 4, Some(1))], true),
                mics: vec![out(mic("umc", 4, Some(1)), true)],
                ..recording()
            }),
            SpecError::DuplicateSource {
                device_id: "umc".into(),
                selected_channel: Some(1)
            }
        );
    }

    #[test]
    fn rejects_conflicting_shared_device_formats() {
        let mut other_rate = mic("umc", 4, Some(2));
        other_rate.framerate = 44100;
        assert!(matches!(
            rejects(CompositeStreamSpec {
                mics: vec![out(mic("umc", 4, Some(1)), false), out(other_rate, false)],
                ..recording()
            }),
            SpecError::ConflictingDeviceOptions { .. }
        ));
    }

    #[test]
    fn rejects_channel_out_of_range() {
        for channel in [0, 5, -1] {
            assert_eq!(
                rejects(CompositeStreamSpec {
                    mics: vec![out(mic("umc", 4, Some(channel)), false)],
                    ..recording()
                }),
                SpecError::ChannelOutOfRange {
                    device_id: "umc".into(),
                    selected_channel: channel,
                    channels: 4
                }
            );
        }
    }

    #[test]
    fn rejects_outputs_that_do_nothing() {
        assert_eq!(
            rejects(CompositeStreamSpec {
                av_mix: av_mix(vec![mic("mic-a", 1, None)], false),
                ..Default::default()
            }),
            SpecError::NothingToDo("av-mix".into())
        );
        assert_eq!(
            rejects(CompositeStreamSpec {
                cameras: vec![out(camera("c920-a"), true), out(camera("c920-b"), false)],
                ..Default::default()
            }),
            SpecError::NothingToDo("camera-2".into())
        );
    }
}
