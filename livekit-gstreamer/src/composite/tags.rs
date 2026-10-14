//! Element naming: every element in a composite pipeline is named `<scope>:<purpose>`,
//! so a bus message can be traced back to the device, role, file or appsink it came from.

use super::ids::{DeviceKey, OutputId, Role, SinkId};
use gstreamer::prelude::*;
use std::fmt;

const MIX_SCOPE: &str = "mix.lr";
const FILE_PREFIX: &str = "file.";
const SINK_PREFIX: &str = "sink.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Shared source chain of one physical device, e.g. `dev0`.
    Device(DeviceKey),
    /// Per-role chain, e.g. `mic-1`.
    Role(Role),
    /// The L/R mix of the AV mix mics, `mix.lr`.
    Mix,
    /// A file branch, e.g. `file.av-mix`.
    File(OutputId),
    /// A stream branch, e.g. `sink.av-mix-audio`.
    Sink(SinkId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementTag {
    pub scope: Scope,
    pub purpose: String,
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Device(key) => write!(f, "{key}"),
            Scope::Role(role) => write!(f, "{role}"),
            Scope::Mix => f.write_str(MIX_SCOPE),
            Scope::File(output) => write!(f, "{FILE_PREFIX}{output}"),
            Scope::Sink(sink) => write!(f, "{SINK_PREFIX}{sink}"),
        }
    }
}

impl Scope {
    pub fn parse(value: &str) -> Option<Self> {
        if value == MIX_SCOPE {
            Some(Scope::Mix)
        } else if let Some(output) = value.strip_prefix(FILE_PREFIX) {
            output.parse().ok().map(Scope::File)
        } else if let Some(sink) = value.strip_prefix(SINK_PREFIX) {
            sink.parse().ok().map(Scope::Sink)
        } else if let Ok(key) = value.parse() {
            Some(Scope::Device(key))
        } else {
            value.parse().ok().map(Scope::Role)
        }
    }
}

impl fmt::Display for ElementTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.scope, self.purpose)
    }
}

impl ElementTag {
    pub fn new(scope: Scope, purpose: impl Into<String>) -> Self {
        Self {
            scope,
            purpose: purpose.into(),
        }
    }

    /// Parses an element name such as `sink.av-mix-audio:rnnoise`.
    pub fn parse(name: &str) -> Option<Self> {
        let (scope, purpose) = name.split_once(':')?;
        if purpose.is_empty() {
            return None;
        }
        Some(Self::new(Scope::parse(scope)?, purpose))
    }

    /// Finds the tag of `object` or of its nearest tagged ancestor.
    pub fn find(object: &gstreamer::Object) -> Option<Self> {
        let mut current = Some(object.clone());
        while let Some(obj) = current {
            if let Some(tag) = Self::parse(&obj.name()) {
                return Some(tag);
            }
            current = obj.parent();
        }
        None
    }
}

/// Builds the element name for `scope` and `purpose`, e.g. `dev0:src`.
pub fn element_name(scope: Scope, purpose: &str) -> String {
    ElementTag::new(scope, purpose).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip() {
        let scopes = [
            Scope::Device(DeviceKey(0)),
            Scope::Role(Role::Mic(1)),
            Scope::Mix,
            Scope::File(OutputId::AvMix),
            Scope::File(OutputId::Mic(3)),
            Scope::Sink(SinkId::AvMixAudio),
            Scope::Sink(SinkId::Camera(2)),
        ];
        for scope in scopes {
            let name = element_name(scope, "q");
            assert_eq!(ElementTag::parse(&name), Some(ElementTag::new(scope, "q")));
        }
    }

    #[test]
    fn tags_examples() {
        assert_eq!(element_name(Scope::Device(DeviceKey(0)), "src"), "dev0:src");
        assert_eq!(
            element_name(Scope::Sink(SinkId::AvMixAudio), "rnnoise"),
            "sink.av-mix-audio:rnnoise"
        );
        assert_eq!(
            ElementTag::parse("file.mic-3:sink").map(|t| t.scope),
            Some(Scope::File(OutputId::Mic(3)))
        );
    }

    #[test]
    fn tags_reject_untagged() {
        for name in [
            "pipeline0",
            "osxaudiosrc3",
            "dev0:",
            "nope:src",
            "file.av:mux",
        ] {
            assert_eq!(ElementTag::parse(name), None, "{name}");
        }
    }

    #[test]
    fn tags_find_walks_parents() {
        gstreamer::init().unwrap();
        let bin = gstreamer::Bin::with_name("mic-2:bin");
        let inner = gstreamer::ElementFactory::make("identity")
            .name("untagged")
            .build()
            .unwrap();
        bin.add(&inner).unwrap();
        let tag = ElementTag::find(inner.upcast_ref()).unwrap();
        assert_eq!(tag.scope, Scope::Role(Role::Mic(2)));
    }
}
