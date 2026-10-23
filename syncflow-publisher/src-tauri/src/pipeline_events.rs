//! Forwards composite pipeline events to the UI (`publication-notification`) and, in
//! session mode, to the SyncFlow room on the `ROOM_EVENTS` data topic, so dashboards can
//! filter for recorder failures (composite-stream-plan.md §10).
//!
//! Payloads never include credentials.

use std::sync::Arc;

use livekit::{DataPacket, Room};
use livekit_gstreamer::composite::{
    CompositeEvent, DeviceKind, DeviceRef, ErrorInfo, FailurePhase, OutputId, RecordingManifest,
    Role, SinkId, StopReason,
};
use livekit_gstreamer::utils::system_time_nanos;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::broadcast;

use crate::syncflow_publisher::{
    DeviceFailureData, FailureData, OutputFailureData, PipelineWarningData,
    PublicationNotification, StreamFailureData,
};

pub const ROOM_EVENTS_TOPIC: &str = "ROOM_EVENTS";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RoomEventMessage {
    LocalRecorderStreamFailure(LocalRecorderStreamFailure),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalRecorderStreamFailure {
    pub reason: String,
    /// Unix epoch, nanoseconds.
    pub occurred_at: i64,
    pub phase: FailurePhase,
    pub participant: ParticipantDetails,
    pub session: SessionDetails,
    /// The device that failed; `None` for pipeline-wide failures.
    pub device: Option<DeviceDetails>,
    pub outputs_affected: Vec<String>,
    pub tracks_unpublished: Vec<String>,
    pub streaming: bool,
    pub recording_continues: bool,
    pub gst: Option<GstErrorDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantDetails {
    pub identity: String,
    pub name: String,
    pub sid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetails {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDetails {
    /// e.g. `mic-3`.
    pub role: String,
    /// `camera`, `mic` or `screen`.
    pub kind: String,
    pub display_name: String,
    pub device_id: String,
    pub codec: String,
    pub selected_channel: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GstErrorDetails {
    pub element: String,
    pub domain: String,
    pub code: i32,
    pub message: String,
    pub debug: Option<String>,
}

impl From<&ErrorInfo> for GstErrorDetails {
    fn from(error: &ErrorInfo) -> Self {
        Self {
            element: error.element.clone(),
            domain: error.domain.clone(),
            code: error.code,
            message: error.message.clone(),
            debug: error.debug.clone(),
        }
    }
}

impl RoomEventMessage {
    pub fn to_payload(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }
}

/// Who and where: identifies the recording in every notification and room message.
#[derive(Debug, Clone)]
pub struct EventContext {
    pub session_id: String,
    pub session_name: String,
    pub participant_identity: String,
    pub participant_name: String,
    /// Whether anything of this recording is streamed.
    pub streaming: bool,
}

fn device_details(
    manifest: &RecordingManifest,
    device: &DeviceRef,
    role: Option<Role>,
) -> DeviceDetails {
    DeviceDetails {
        role: role.map(|r| r.to_string()).unwrap_or_default(),
        kind: match device.kind {
            DeviceKind::Camera => "camera",
            DeviceKind::Mic => "mic",
            DeviceKind::Screen => "screen",
        }
        .into(),
        display_name: device.display_name.clone(),
        device_id: device.device_id.clone(),
        codec: device.codec.clone(),
        selected_channel: role.and_then(|role| {
            manifest
                .roles
                .iter()
                .find(|r| r.role == role)
                .and_then(|r| r.selected_channel)
        }),
    }
}

/// The device behind `role`, per the manifest.
fn role_device(manifest: &RecordingManifest, role: Role) -> Option<DeviceDetails> {
    let key = manifest.roles.iter().find(|r| r.role == role)?.device;
    let device = manifest.devices.iter().find(|d| d.key == key)?;
    Some(device_details(manifest, device, Some(role)))
}

/// The first device feeding `output`.
fn output_device(manifest: &RecordingManifest, output: OutputId) -> Option<DeviceDetails> {
    let record = manifest.outputs.iter().find(|o| o.id == output)?;
    role_device(manifest, *record.roles.first()?)
}

/// The first device feeding `sink`.
fn sink_device(manifest: &RecordingManifest, sink: SinkId) -> Option<DeviceDetails> {
    let av_mix_role = |index: usize| {
        manifest
            .outputs
            .iter()
            .find(|o| o.id == OutputId::AvMix)
            .and_then(|o| o.roles.get(index).copied())
    };
    let role = match sink {
        SinkId::AvMixVideo => av_mix_role(0)?,
        SinkId::AvMixAudio => av_mix_role(1)?,
        SinkId::Camera(n) => Role::Camera(n),
        SinkId::Mic(n) => Role::Mic(n),
        SinkId::Screen(n) => Role::Screen(n),
    };
    role_device(manifest, role)
}

fn strings<T: ToString>(items: &[T]) -> Vec<String> {
    items.iter().map(T::to_string).collect()
}

/// What the UI is told about `event`, if anything.
pub fn notification_for(
    event: &CompositeEvent,
    ctx: &EventContext,
    manifest: &RecordingManifest,
) -> Option<PublicationNotification> {
    let session_id = ctx.session_id.clone();
    Some(match event {
        CompositeEvent::DeviceFailed {
            device,
            roles,
            outputs_affected,
            phase,
            error,
            recording_continues,
            ..
        } => PublicationNotification::DeviceFailure(DeviceFailureData {
            session_id,
            device: device_details(manifest, device, roles.first().copied()),
            roles: strings(roles),
            outputs: strings(outputs_affected),
            reason: error.message.clone(),
            phase: *phase,
            recording_continues: *recording_continues,
        }),
        CompositeEvent::OutputFailed {
            output,
            error,
            recording_continues,
        } => PublicationNotification::OutputFailure(OutputFailureData {
            session_id,
            output: output.to_string(),
            reason: error.message.clone(),
            recording_continues: *recording_continues,
        }),
        CompositeEvent::SinkFailed { sink, error } => {
            PublicationNotification::StreamFailure(StreamFailureData {
                session_id,
                sink: sink.to_string(),
                reason: error.message.clone(),
            })
        }
        CompositeEvent::Warning { scope, message } => {
            PublicationNotification::PipelineWarning(PipelineWarningData {
                session_id,
                scope: scope.clone(),
                message: message.clone(),
            })
        }
        CompositeEvent::Stopped { reason } => PublicationNotification::Failure(FailureData {
            session_id,
            reason: stop_reason(reason)?,
        }),
        CompositeEvent::OutputFinalized { .. } => return None,
    })
}

/// Why the pipeline stopped, for stops that are failures.
fn stop_reason(reason: &StopReason) -> Option<String> {
    match reason {
        StopReason::Fatal(error) => Some(format!("Recording stopped: {}", error.message)),
        StopReason::IsolationTimeout => {
            Some("Recording stopped: a failed device could not be isolated in time".into())
        }
        StopReason::AllOutputsDone => Some("Recording stopped: every device failed".into()),
        StopReason::Requested => None,
    }
}

/// The `ROOM_EVENTS` message for `event`, if it is a failure.
pub fn room_message_for(
    event: &CompositeEvent,
    ctx: &EventContext,
    manifest: &RecordingManifest,
    participant_sid: Option<String>,
) -> Option<RoomEventMessage> {
    let failure = |reason: String,
                   phase: FailurePhase,
                   device: Option<DeviceDetails>,
                   outputs_affected: Vec<String>,
                   tracks_unpublished: Vec<String>,
                   recording_continues: bool,
                   gst: Option<GstErrorDetails>| {
        RoomEventMessage::LocalRecorderStreamFailure(LocalRecorderStreamFailure {
            reason,
            occurred_at: system_time_nanos(),
            phase,
            participant: ParticipantDetails {
                identity: ctx.participant_identity.clone(),
                name: ctx.participant_name.clone(),
                sid: participant_sid.clone(),
            },
            session: SessionDetails {
                id: ctx.session_id.clone(),
                name: ctx.session_name.clone(),
            },
            device,
            outputs_affected,
            tracks_unpublished,
            streaming: ctx.streaming,
            recording_continues,
            gst,
        })
    };
    match event {
        CompositeEvent::DeviceFailed {
            device,
            roles,
            outputs_affected,
            sinks_closed,
            phase,
            error,
            recording_continues,
        } => Some(failure(
            error.message.clone(),
            *phase,
            Some(device_details(manifest, device, roles.first().copied())),
            strings(outputs_affected),
            strings(sinks_closed),
            *recording_continues,
            Some(error.into()),
        )),
        CompositeEvent::OutputFailed {
            output,
            error,
            recording_continues,
        } => Some(failure(
            error.message.clone(),
            FailurePhase::Running,
            output_device(manifest, *output),
            vec![output.to_string()],
            Vec::new(),
            *recording_continues,
            Some(error.into()),
        )),
        CompositeEvent::SinkFailed { sink, error } => Some(failure(
            error.message.clone(),
            FailurePhase::Running,
            sink_device(manifest, *sink),
            Vec::new(),
            vec![sink.to_string()],
            true,
            Some(error.into()),
        )),
        CompositeEvent::Stopped { reason } => {
            let gst = match reason {
                StopReason::Fatal(error) => Some(error.into()),
                _ => None,
            };
            Some(failure(
                stop_reason(reason)?,
                FailurePhase::Running,
                None,
                Vec::new(),
                Vec::new(),
                false,
                gst,
            ))
        }
        CompositeEvent::Warning { .. } | CompositeEvent::OutputFinalized { .. } => None,
    }
}

async fn publish(room: &Room, message: &RoomEventMessage) -> Result<(), String> {
    let payload = message.to_payload().map_err(|e| e.to_string())?;
    room.local_participant()
        .publish_data(DataPacket {
            payload,
            topic: Some(ROOM_EVENTS_TOPIC.into()),
            reliable: true,
            ..Default::default()
        })
        .await
        .map_err(|e| e.to_string())
}

/// Forwards `events` until the stream is dropped. `manifest` (taken after start) names the
/// devices behind each file and stream. Without a `room` (local mode), only the UI is told.
pub fn spawn_event_forwarder(
    mut events: broadcast::Receiver<CompositeEvent>,
    ctx: EventContext,
    manifest: RecordingManifest,
    room: Option<Arc<Room>>,
    app: tauri::AppHandle,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            if let Some(notification) = notification_for(&event, &ctx, &manifest) {
                let _ = app.emit("publication-notification", notification);
            }
            let Some(room) = &room else {
                continue;
            };
            let sid = Some(room.local_participant().sid().to_string());
            if let Some(message) = room_message_for(&event, &ctx, &manifest, sid) {
                if let Err(e) = publish(room, &message).await {
                    let _ = app.emit(
                        "publication-notification",
                        PublicationNotification::PipelineWarning(PipelineWarningData {
                            session_id: ctx.session_id.clone(),
                            scope: None,
                            message: format!("Failed to report to {ROOM_EVENTS_TOPIC}: {e}"),
                        }),
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use livekit_gstreamer::composite::{AvMixSpec, CompositeStreamSpec, Output};
    use livekit_gstreamer::{AudioPublishOptions, VideoPublishOptions};
    use std::collections::HashMap;

    fn mic(id: &str, selected_channel: Option<i32>) -> AudioPublishOptions {
        AudioPublishOptions {
            codec: "audio/x-raw".into(),
            device_id: id.into(),
            framerate: 48000,
            channels: 4,
            selected_channel,
            exclusive: false,
        }
    }

    /// AV mix (camera, mic-1, mic-2) and an extra mic-3 on channel 3 of an interface.
    fn manifest() -> RecordingManifest {
        let spec = CompositeStreamSpec {
            av_mix: Some(AvMixSpec {
                camera: VideoPublishOptions {
                    codec: "video/x-raw".into(),
                    device_id: "cam".into(),
                    width: 1280,
                    height: 720,
                    framerate: 30,
                },
                mics: vec![mic("usb-a", None), mic("usb-b", None)],
                stream: true,
            }),
            mics: vec![Output {
                options: mic("umc", Some(3)),
                stream: true,
            }],
            output_dir: Some("/tmp".into()),
            ..Default::default()
        };
        let names = HashMap::from([("umc".to_string(), "UMC1820".to_string())]);
        RecordingManifest::new(&spec.validate_with_names(&names).unwrap())
    }

    fn ctx() -> EventContext {
        EventContext {
            session_id: "s1".into(),
            session_name: "Lecture 3".into(),
            participant_identity: "host".into(),
            participant_name: "Host".into(),
            streaming: true,
        }
    }

    fn error() -> ErrorInfo {
        ErrorInfo {
            element: "dev3:src".into(),
            domain: "gst-resource-error-quark".into(),
            code: 3,
            message: "Could not read from resource.".into(),
            debug: None,
        }
    }

    fn json(message: &RoomEventMessage) -> serde_json::Value {
        serde_json::from_slice(&message.to_payload().unwrap()).unwrap()
    }

    fn mic3_failed(manifest: &RecordingManifest) -> CompositeEvent {
        CompositeEvent::DeviceFailed {
            device: manifest
                .devices
                .iter()
                .find(|d| d.device_id == "umc")
                .unwrap()
                .clone(),
            roles: vec![Role::Mic(3)],
            outputs_affected: vec![OutputId::Mic(3)],
            sinks_closed: vec![SinkId::Mic(3)],
            phase: FailurePhase::Running,
            error: error(),
            recording_continues: true,
        }
    }

    #[test]
    fn device_failure_reaches_ui_and_room() {
        let manifest = manifest();
        let event = mic3_failed(&manifest);

        let Some(PublicationNotification::DeviceFailure(data)) =
            notification_for(&event, &ctx(), &manifest)
        else {
            panic!("expected a device failure notification");
        };
        assert_eq!(data.device.display_name, "UMC1820");
        assert_eq!(data.device.selected_channel, Some(3));
        assert_eq!(data.outputs, ["mic-3"]);
        assert!(data.recording_continues);

        let message = room_message_for(&event, &ctx(), &manifest, Some("PA_1".into())).unwrap();
        let json = json(&message);
        assert_eq!(json["type"], "LocalRecorderStreamFailure");
        assert_eq!(json["phase"], "running");
        assert_eq!(json["participant"]["identity"], "host");
        assert_eq!(json["participant"]["sid"], "PA_1");
        assert_eq!(json["session"]["name"], "Lecture 3");
        assert_eq!(json["device"]["role"], "mic-3");
        assert_eq!(json["device"]["kind"], "mic");
        assert_eq!(json["device"]["selectedChannel"], 3);
        assert_eq!(json["outputsAffected"][0], "mic-3");
        assert_eq!(json["tracksUnpublished"][0], "mic-3");
        assert_eq!(json["recordingContinues"], true);
        assert_eq!(json["gst"]["element"], "dev3:src");
    }

    #[test]
    fn output_and_stream_failures_name_their_device() {
        let manifest = manifest();
        let output = CompositeEvent::OutputFailed {
            output: OutputId::AvMix,
            error: error(),
            recording_continues: true,
        };
        let json_output = json(&room_message_for(&output, &ctx(), &manifest, None).unwrap());
        assert_eq!(json_output["device"]["kind"], "camera");
        assert_eq!(json_output["outputsAffected"][0], "av-mix");

        let sink = CompositeEvent::SinkFailed {
            sink: SinkId::AvMixAudio,
            error: error(),
        };
        let json_sink = json(&room_message_for(&sink, &ctx(), &manifest, None).unwrap());
        assert_eq!(json_sink["device"]["role"], "mic-1");
        assert_eq!(json_sink["tracksUnpublished"][0], "av-mix-audio");
        assert!(matches!(
            notification_for(&sink, &ctx(), &manifest),
            Some(PublicationNotification::StreamFailure(_))
        ));
    }

    #[test]
    fn pipeline_stops_are_failures_without_a_device() {
        let manifest = manifest();
        let fatal = CompositeEvent::Stopped {
            reason: StopReason::Fatal(error()),
        };
        let json = json(&room_message_for(&fatal, &ctx(), &manifest, None).unwrap());
        assert!(json["device"].is_null());
        assert_eq!(json["recordingContinues"], false);
        assert!(matches!(
            notification_for(&fatal, &ctx(), &manifest),
            Some(PublicationNotification::Failure(_))
        ));

        let requested = CompositeEvent::Stopped {
            reason: StopReason::Requested,
        };
        assert!(room_message_for(&requested, &ctx(), &manifest, None).is_none());
        assert!(notification_for(&requested, &ctx(), &manifest).is_none());
    }

    #[test]
    fn warnings_reach_the_ui_only() {
        let manifest = manifest();
        let warning = CompositeEvent::Warning {
            scope: Some("sink.mic-3".into()),
            message: "audiornnoise is not installed".into(),
        };
        assert!(matches!(
            notification_for(&warning, &ctx(), &manifest),
            Some(PublicationNotification::PipelineWarning(_))
        ));
        assert!(room_message_for(&warning, &ctx(), &manifest, None).is_none());
    }
}
