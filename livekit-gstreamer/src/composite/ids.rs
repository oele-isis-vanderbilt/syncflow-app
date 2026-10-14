//! Stable identities for devices, roles, files and appsinks in a composite pipeline.
//!
//! Numbering is global per kind: the AV mix takes `camera-1`, `mic-1` and `mic-2`,
//! and additional devices continue from `camera-2`, `mic-3`, and so on.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("invalid {kind} id: {value:?}")]
pub struct IdParseError {
    pub kind: &'static str,
    pub value: String,
}

impl IdParseError {
    fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
        }
    }
}

/// A source role: one camera, one microphone channel selection, or one screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Camera(u8),
    Mic(u8),
    Screen(u8),
}

/// One recorded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OutputId {
    AvMix,
    Camera(u8),
    Mic(u8),
    Screen(u8),
}

/// One appsink, published as one LiveKit track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SinkId {
    AvMixVideo,
    AvMixAudio,
    Camera(u8),
    Mic(u8),
    Screen(u8),
}

/// One physical device. A multichannel interface feeding several roles has one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceKey(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Camera,
    Mic,
    Screen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRef {
    pub key: DeviceKey,
    pub device_id: String,
    pub display_name: String,
    pub kind: DeviceKind,
    pub codec: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRef {
    pub role: Role,
    pub device: DeviceKey,
    /// 1-based channel picked from a multichannel device.
    pub selected_channel: Option<u8>,
}

fn parse_indexed(value: &str, prefix: &str) -> Option<u8> {
    value.strip_prefix(prefix)?.strip_prefix('-')?.parse().ok()
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Camera(n) => write!(f, "camera-{n}"),
            Role::Mic(n) => write!(f, "mic-{n}"),
            Role::Screen(n) => write!(f, "screen-{n}"),
        }
    }
}

impl FromStr for Role {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(n) = parse_indexed(s, "camera") {
            Ok(Role::Camera(n))
        } else if let Some(n) = parse_indexed(s, "mic") {
            Ok(Role::Mic(n))
        } else if let Some(n) = parse_indexed(s, "screen") {
            Ok(Role::Screen(n))
        } else {
            Err(IdParseError::new("role", s))
        }
    }
}

impl fmt::Display for OutputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutputId::AvMix => write!(f, "av-mix"),
            OutputId::Camera(n) => write!(f, "camera-{n}"),
            OutputId::Mic(n) => write!(f, "mic-{n}"),
            OutputId::Screen(n) => write!(f, "screen-{n}"),
        }
    }
}

impl FromStr for OutputId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "av-mix" {
            return Ok(OutputId::AvMix);
        }
        match s.parse::<Role>() {
            Ok(Role::Camera(n)) => Ok(OutputId::Camera(n)),
            Ok(Role::Mic(n)) => Ok(OutputId::Mic(n)),
            Ok(Role::Screen(n)) => Ok(OutputId::Screen(n)),
            Err(_) => Err(IdParseError::new("output", s)),
        }
    }
}

impl fmt::Display for SinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SinkId::AvMixVideo => write!(f, "av-mix-video"),
            SinkId::AvMixAudio => write!(f, "av-mix-audio"),
            SinkId::Camera(n) => write!(f, "camera-{n}"),
            SinkId::Mic(n) => write!(f, "mic-{n}"),
            SinkId::Screen(n) => write!(f, "screen-{n}"),
        }
    }
}

impl FromStr for SinkId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "av-mix-video" => Ok(SinkId::AvMixVideo),
            "av-mix-audio" => Ok(SinkId::AvMixAudio),
            _ => match s.parse::<Role>() {
                Ok(Role::Camera(n)) => Ok(SinkId::Camera(n)),
                Ok(Role::Mic(n)) => Ok(SinkId::Mic(n)),
                Ok(Role::Screen(n)) => Ok(SinkId::Screen(n)),
                Err(_) => Err(IdParseError::new("sink", s)),
            },
        }
    }
}

impl fmt::Display for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dev{}", self.0)
    }
}

impl FromStr for DeviceKey {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.strip_prefix("dev")
            .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse().ok())
            .map(DeviceKey)
            .ok_or_else(|| IdParseError::new("device", s))
    }
}

/// Serializes the ids as their display strings, e.g. `"mic-3"`.
macro_rules! string_serde {
    ($($ty:ty),*) => {$(
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }
    )*};
}

string_serde!(Role, OutputId, SinkId, DeviceKey);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for role in [Role::Camera(1), Role::Mic(3), Role::Screen(1)] {
            assert_eq!(role.to_string().parse::<Role>(), Ok(role));
        }
        for output in [
            OutputId::AvMix,
            OutputId::Camera(2),
            OutputId::Mic(1),
            OutputId::Screen(1),
        ] {
            assert_eq!(output.to_string().parse::<OutputId>(), Ok(output));
        }
        for sink in [
            SinkId::AvMixVideo,
            SinkId::AvMixAudio,
            SinkId::Camera(2),
            SinkId::Mic(4),
            SinkId::Screen(1),
        ] {
            assert_eq!(sink.to_string().parse::<SinkId>(), Ok(sink));
        }
        assert_eq!("dev12".parse::<DeviceKey>(), Ok(DeviceKey(12)));
    }

    #[test]
    fn ids_display() {
        assert_eq!(Role::Mic(3).to_string(), "mic-3");
        assert_eq!(OutputId::AvMix.to_string(), "av-mix");
        assert_eq!(SinkId::AvMixAudio.to_string(), "av-mix-audio");
        assert_eq!(DeviceKey(0).to_string(), "dev0");
    }

    #[test]
    fn ids_reject_garbage() {
        assert!("mic".parse::<Role>().is_err());
        assert!("mic-x".parse::<Role>().is_err());
        assert!("speaker-1".parse::<Role>().is_err());
        assert!("av-mix-video".parse::<OutputId>().is_err());
        assert!("av-mix".parse::<SinkId>().is_err());
        assert!("dev".parse::<DeviceKey>().is_err());
        assert!("dev-1".parse::<DeviceKey>().is_err());
    }

    #[test]
    fn ids_serde_as_strings() {
        let json = serde_json::to_string(&(Role::Mic(2), SinkId::AvMixVideo)).unwrap();
        assert_eq!(json, r#"["mic-2","av-mix-video"]"#);
        let back: (Role, SinkId) = serde_json::from_str(&json).unwrap();
        assert_eq!(back, (Role::Mic(2), SinkId::AvMixVideo));
    }
}
