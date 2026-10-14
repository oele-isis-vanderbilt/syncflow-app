//! Events and errors reported by a composite pipeline.

use super::ids::{DeviceRef, OutputId, Role, SinkId};
use super::spec::SpecError;
use crate::media_device::GStreamerError;
use gstreamer::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// A GStreamer error message, kept with the element that posted it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorInfo {
    pub element: String,
    pub domain: String,
    pub code: i32,
    pub message: String,
    pub debug: Option<String>,
}

impl ErrorInfo {
    pub fn from_error_message(message: &gstreamer::message::Error) -> Self {
        use gstreamer::glib::translate::IntoGlib;
        let error = message.error();
        let code = error
            .kind::<gstreamer::ResourceError>()
            .map(IntoGlib::into_glib)
            .or_else(|| {
                error
                    .kind::<gstreamer::StreamError>()
                    .map(IntoGlib::into_glib)
            })
            .or_else(|| {
                error
                    .kind::<gstreamer::CoreError>()
                    .map(IntoGlib::into_glib)
            })
            .or_else(|| {
                error
                    .kind::<gstreamer::LibraryError>()
                    .map(IntoGlib::into_glib)
            })
            .unwrap_or(0);
        Self {
            element: message
                .src()
                .map(|src| src.name().to_string())
                .unwrap_or_default(),
            domain: error.domain().as_str().to_string(),
            code,
            message: error.message().to_string(),
            debug: message.debug().map(|d| d.to_string()),
        }
    }

    /// An error that did not come from the bus, such as a device failing to open.
    pub fn from_composite_error(element: impl Into<String>, error: &CompositeError) -> Self {
        Self {
            element: element.into(),
            domain: "composite".into(),
            code: 0,
            message: error.to_string(),
            debug: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailurePhase {
    Start,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "reason", content = "error")]
pub enum StopReason {
    Requested,
    AllOutputsDone,
    Fatal(ErrorInfo),
    IsolationTimeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum CompositeEvent {
    /// A device failed. Everything it fed has been finalized or closed.
    DeviceFailed {
        device: DeviceRef,
        roles: Vec<Role>,
        outputs_affected: Vec<OutputId>,
        sinks_closed: Vec<SinkId>,
        phase: FailurePhase,
        error: ErrorInfo,
        recording_continues: bool,
    },
    /// An encoder, muxer or file sink failed. Only that file is affected.
    OutputFailed {
        output: OutputId,
        error: ErrorInfo,
        recording_continues: bool,
    },
    /// A stream branch failed. Files are unaffected.
    SinkFailed {
        sink: SinkId,
        error: ErrorInfo,
    },
    OutputFinalized {
        output: OutputId,
        path: PathBuf,
    },
    Warning {
        scope: Option<String>,
        message: String,
    },
    Stopped {
        reason: StopReason,
    },
}

/// Boxed inside [`CompositeError`] to keep the error small.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStartFailure {
    pub device: DeviceRef,
    pub error: ErrorInfo,
}

#[derive(Error, Debug)]
pub enum CompositeError {
    #[error("invalid spec: {0}")]
    Spec(#[from] SpecError),
    #[error(transparent)]
    GStreamer(#[from] GStreamerError),
    #[error("device {} ({}) failed to start: {}", .0.device.display_name, .0.device.key, .0.error.message)]
    DeviceStart(Box<DeviceStartFailure>),
    #[error("the composite stream has not been started")]
    NotStarted,
    #[error("the composite stream is already running")]
    AlreadyStarted,
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
