//! Pipeline failures reported to the SyncFlow room on the `ROOM_EVENTS` data topic,
//! so dashboards can filter for them (composite-stream-plan.md §10).
//!
//! Not wired in yet: the event forwarder arrives with the switch to `CompositeStream`.
//! Payloads never include credentials.

use livekit_gstreamer::composite::FailurePhase;
use serde::{Deserialize, Serialize};

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
    pub device: DeviceDetails,
    pub outputs_affected: Vec<String>,
    pub tracks_unpublished: Vec<String>,
    pub streaming: bool,
    pub recording_continues: bool,
    pub gst: GstErrorDetails,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl RoomEventMessage {
    pub fn to_payload(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_payload_shape() {
        let message = RoomEventMessage::LocalRecorderStreamFailure(LocalRecorderStreamFailure {
            reason: "device disconnected".into(),
            occurred_at: 1,
            phase: FailurePhase::Running,
            participant: ParticipantDetails {
                identity: "host".into(),
                name: "Host".into(),
                sid: None,
            },
            session: SessionDetails {
                id: "s1".into(),
                name: "Lecture 3".into(),
            },
            device: DeviceDetails {
                role: "mic-3".into(),
                kind: "mic".into(),
                display_name: "UMC1820".into(),
                device_id: "id".into(),
                codec: "audio/x-raw".into(),
                selected_channel: Some(3),
            },
            outputs_affected: vec!["mic-3".into()],
            tracks_unpublished: vec![],
            streaming: false,
            recording_continues: true,
            gst: GstErrorDetails {
                element: "dev0:src".into(),
                domain: "gst-resource-error-quark".into(),
                code: 3,
                message: "Could not read".into(),
                debug: None,
            },
        });
        let json: serde_json::Value =
            serde_json::from_slice(&message.to_payload().unwrap()).unwrap();
        assert_eq!(json["type"], "LocalRecorderStreamFailure");
        assert_eq!(json["phase"], "running");
        assert_eq!(json["recordingContinues"], true);
        assert_eq!(json["device"]["selectedChannel"], 3);
        assert_eq!(json["gst"]["element"], "dev0:src");
    }
}
