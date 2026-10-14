//! One GStreamer pipeline driving every configured device: an AV mix (camera plus
//! up to two mics recorded L/R), an optional screenshare, and additional cameras and
//! mics. Each part has its own file branch and an optional, identified stream branch.
//!
//! Design and scenarios: `composite-stream-plan.md` at the repository root.

pub mod builder;
pub mod bus;
pub mod events;
pub mod ids;
pub mod manifest;
pub mod naming;
pub mod spec;
pub mod stream;
pub mod tags;

pub use events::{
    CompositeError, CompositeEvent, DeviceStartFailure, ErrorInfo, FailurePhase, StopReason,
};
pub use ids::{DeviceKey, DeviceKind, DeviceRef, OutputId, Role, RoleRef, SinkId};
pub use manifest::{ManifestStatus, OutputRecord, OutputStatus, RecordingManifest};
pub use spec::{
    AvMixSpec, CompositeStreamSpec, EncoderOptions, H264Encoder, Output, SinkInfo, SinkKind,
    SpecError, StartPolicy, StreamAudioOptions,
};
pub use stream::{CompositeStream, SinkSubscription, StartReport};
