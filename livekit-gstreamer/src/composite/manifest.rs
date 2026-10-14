//! `manifest.json`: what a recording folder contains, which device fed which file,
//! the shared timing reference, and everything that went wrong.

use super::events::{CompositeEvent, ErrorInfo, StopReason};
use super::ids::{DeviceRef, OutputId, Role, RoleRef};
use super::naming::MANIFEST_FILE_NAME;
use super::spec::{AudioChannelMap, ResolvedSpec};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestStatus {
    Recording,
    Completed,
    CompletedWithErrors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputStatus {
    Recording,
    Finalized,
    Failed,
    /// Stopped without a clean EOS; the fragmented MP4 is still readable.
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputRecord {
    pub id: OutputId,
    pub file: String,
    pub roles: Vec<Role>,
    pub audio_channels: Option<AudioChannelMap>,
    pub status: OutputStatus,
    pub first_pts_ns: Option<u64>,
    pub last_pts_ns: Option<u64>,
    pub error: Option<ErrorInfo>,
}

/// `audiorate` counters for one mic role: a sync-quality signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioContinuity {
    pub role: Role,
    pub samples_added: u64,
    pub samples_dropped: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingManifest {
    pub status: ManifestStatus,
    /// Always `"monotonic-system"`: the pipeline runs on the system clock.
    pub clock: String,
    pub base_time_ns: Option<u64>,
    /// Wall-clock time (Unix epoch, ns) at `base_time_ns`, for cross-machine alignment.
    pub wallclock_at_base_ns: Option<i64>,
    pub devices: Vec<DeviceRef>,
    pub roles: Vec<RoleRef>,
    pub outputs: Vec<OutputRecord>,
    pub audio_continuity: Vec<AudioContinuity>,
    pub events: Vec<CompositeEvent>,
    pub context: Option<serde_json::Value>,
}

impl RecordingManifest {
    pub fn new(spec: &ResolvedSpec) -> Self {
        Self {
            status: ManifestStatus::Recording,
            clock: "monotonic-system".into(),
            base_time_ns: None,
            wallclock_at_base_ns: None,
            devices: spec.devices.iter().map(|d| d.device.clone()).collect(),
            roles: spec.roles.clone(),
            outputs: spec
                .outputs
                .iter()
                .map(|o| OutputRecord {
                    id: o.id,
                    file: o.file_name.clone(),
                    roles: o.roles.clone(),
                    audio_channels: o.audio_channels,
                    status: OutputStatus::Recording,
                    first_pts_ns: None,
                    last_pts_ns: None,
                    error: None,
                })
                .collect(),
            audio_continuity: Vec::new(),
            events: Vec::new(),
            context: spec.manifest_context.clone(),
        }
    }

    pub fn output_mut(&mut self, id: OutputId) -> Option<&mut OutputRecord> {
        self.outputs.iter_mut().find(|o| o.id == id)
    }

    /// Outputs still being written.
    pub fn live_outputs(&self) -> Vec<OutputId> {
        self.outputs
            .iter()
            .filter(|o| o.status == OutputStatus::Recording)
            .map(|o| o.id)
            .collect()
    }

    /// Marks still-recording outputs `Partial` and sets the final status: completed with
    /// errors if anything failed along the way.
    pub fn finish(&mut self) {
        for output in &mut self.outputs {
            if output.status == OutputStatus::Recording {
                output.status = OutputStatus::Partial;
            }
        }
        let failed_output = self
            .outputs
            .iter()
            .any(|o| o.status != OutputStatus::Finalized || o.error.is_some());
        let failure_event = self.events.iter().any(|e| {
            matches!(
                e,
                CompositeEvent::DeviceFailed { .. }
                    | CompositeEvent::OutputFailed { .. }
                    | CompositeEvent::SinkFailed { .. }
                    | CompositeEvent::Stopped {
                        reason: StopReason::Fatal(_) | StopReason::IsolationTimeout
                    }
            )
        });
        self.status = if failed_output || failure_event {
            ManifestStatus::CompletedWithErrors
        } else {
            ManifestStatus::Completed
        };
    }

    /// Writes `manifest.json` into `dir`, replacing any previous version. Writes to a
    /// temporary file and renames it, so a crash never leaves a torn manifest.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        let tmp = dir.join(format!("{MANIFEST_FILE_NAME}.tmp"));
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, dir.join(MANIFEST_FILE_NAME))
    }
}
