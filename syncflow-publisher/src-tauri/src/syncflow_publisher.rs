use std::{path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

use crate::{
    errors::SyncFlowPublisherError,
    models::{AppState, CompositeConfig, RecordingMode},
    pipeline_events::{spawn_event_forwarder, DeviceDetails, EventContext},
    s3_uploader::{upload_join, FileStatus, UploadEvent, UploadState},
    utils::{claim_join_dir, session_folder_name},
};
use livekit::{Room, RoomOptions};
use livekit_gstreamer::composite::{
    CompositeEvent, CompositeStream, FailurePhase, OutputStatus, RecordingManifest, StopReason,
};
use livekit_gstreamer::utils::system_time_nanos;
use livekit_gstreamer::LKParticipant;
use serde::{Deserialize, Serialize};
use syncflow_shared::{
    device_models::NewSessionMessage,
    livekit_models::{TokenRequest, TokenResponse, VideoGrantsWrapper},
};
use tauri::{Emitter, Manager};
use tokio::sync::{broadcast, mpsc::channel};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicationNotification {
    Failure(FailureData),
    StreamingSuccess(StreamingSuccessData),
    /// One file of a join's upload changed status or sent a part.
    FileUploadProgress(FileUploadProgressData),
    /// A join's upload stopped: everything is in S3, or some files failed.
    UploadFinished(UploadFinishedData),
    SessionEnded(SessionEndedData),
    /// A device failed; what it fed was finalized or closed.
    DeviceFailure(DeviceFailureData),
    /// One file failed (encoder, muxer or disk); the device keeps feeding the rest.
    OutputFailure(OutputFailureData),
    /// One stream failed; files are unaffected.
    StreamFailure(StreamFailureData),
    PipelineWarning(PipelineWarningData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureData {
    pub session_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSuccessData {
    pub session_id: String,
    pub session_name: String,
    pub started_at: String,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadProgressData {
    pub session_id: String,
    pub join: u32,
    pub file: String,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
    pub status: FileStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadFinishedData {
    pub session_id: String,
    pub join: u32,
    pub complete: bool,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEndedData {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFailureData {
    pub session_id: String,
    pub device: DeviceDetails,
    pub roles: Vec<String>,
    pub outputs: Vec<String>,
    pub reason: String,
    pub phase: FailurePhase,
    pub recording_continues: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFailureData {
    pub session_id: String,
    pub output: String,
    pub reason: String,
    pub recording_continues: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamFailureData {
    pub session_id: String,
    pub sink: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineWarningData {
    pub session_id: String,
    pub scope: Option<String>,
    pub message: String,
}

fn notify(app: &tauri::AppHandle, notification: PublicationNotification) {
    let _ = app.emit("publication-notification", notification);
}

fn notify_failure(app: &tauri::AppHandle, session_id: &str, reason: impl ToString) {
    notify(
        app,
        PublicationNotification::Failure(FailureData {
            session_id: session_id.to_string(),
            reason: reason.to_string(),
        }),
    );
}

async fn generate_session_token(
    pc: &syncflow_client::ProjectClient,
    participant_name: String,
    session_details: &NewSessionMessage,
) -> Result<TokenResponse, SyncFlowPublisherError> {
    let token_response = pc
        .generate_session_token(
            &session_details.session_id,
            &TokenRequest {
                identity: participant_name.clone(),
                name: Some(participant_name),
                video_grants: VideoGrantsWrapper {
                    room: session_details.session_name.clone(),
                    can_publish: true,
                    // Pipeline failures are reported on the ROOM_EVENTS data topic.
                    can_publish_data: true,
                    room_join: true,
                    room_create: false,
                    ..Default::default()
                },
            },
        )
        .await
        .map_err(SyncFlowPublisherError::ProjectClientError)?;

    Ok(token_response)
}

/// A started recording: one join of a session.
struct Recording {
    stream: CompositeStream,
    /// `<out_dir>/<session folder>/<join>`.
    dir: PathBuf,
    join: u32,
    /// Taken before start, so it also sees devices that failed to open.
    events: broadcast::Receiver<CompositeEvent>,
}

/// Builds and starts the composite pipeline for one join of a session, in the next
/// numbered folder (`1`, `2`, …) of the session's folder under `out_dir`. The join
/// number is added to the manifest context.
async fn start_recording(
    config: &CompositeConfig,
    out_dir: &std::path::Path,
    session_name: &str,
    session_id: &str,
    mut manifest_context: serde_json::Value,
) -> Result<Recording, SyncFlowPublisherError> {
    let session_dir = out_dir.join(session_folder_name(session_name, session_id));
    let (join, dir) = claim_join_dir(&session_dir)?;
    manifest_context["join"] = join.into();
    let mut stream = CompositeStream::new(config.to_stream_spec(dir.clone(), manifest_context));
    let events = stream.events();
    stream.start().await?;
    Ok(Recording {
        stream,
        dir,
        join,
        events,
    })
}

/// Labels of the files being written, e.g. `mic-3 (mic-3_UMC1820-ch3.m4a)`.
fn output_labels(stream: &CompositeStream) -> Vec<String> {
    stream
        .manifest()
        .map(|m| {
            m.outputs
                .iter()
                .filter(|o| o.status == OutputStatus::Recording)
                .map(|o| format!("{} ({})", o.id, o.file))
                .collect()
        })
        .unwrap_or_default()
}

/// Waits for an unrequested stop (every device failed, or a fatal error).
async fn pipeline_stopped(events: &mut broadcast::Receiver<CompositeEvent>) {
    loop {
        match events.recv().await {
            Ok(CompositeEvent::Stopped { reason }) if reason != StopReason::Requested => return,
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => std::future::pending().await,
        }
    }
}

fn any_finalized(manifest: &RecordingManifest) -> bool {
    manifest
        .outputs
        .iter()
        .any(|o| o.status == OutputStatus::Finalized)
}

/// Marks a join folder as uploading in `AppState::uploads_in_progress` until dropped.
struct UploadGuard {
    app: tauri::AppHandle,
    dir: PathBuf,
}

impl UploadGuard {
    fn claim(app: &tauri::AppHandle, dir: &std::path::Path) -> Option<Self> {
        let state = app.state::<AppState>();
        let mut uploading = state.uploads_in_progress.lock().unwrap();
        uploading.insert(dir.to_path_buf()).then(|| Self {
            app: app.clone(),
            dir: dir.to_path_buf(),
        })
    }
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        state.uploads_in_progress.lock().unwrap().remove(&self.dir);
    }
}

/// Uploads the join folder `dir` file by file to `state.prefix`, reporting each file's
/// progress and the outcome to the UI. Also used to retry a failed upload: files already
/// uploaded are skipped.
pub async fn upload_recording(
    app: &tauri::AppHandle,
    dir: PathBuf,
    state: UploadState,
    s3_client: rusoto_s3::S3Client,
) {
    let session_id = state.session_id.clone();
    let join = state.join;
    let Some(_guard) = UploadGuard::claim(app, &dir) else {
        return notify_failure(
            app,
            &session_id,
            format!("Join {join} is already uploading"),
        );
    };
    let (tx, mut rx) = channel::<UploadEvent>(100);
    let forward = async {
        while let Some(event) = rx.recv().await {
            notify(
                app,
                PublicationNotification::FileUploadProgress(FileUploadProgressData {
                    session_id: session_id.clone(),
                    join,
                    file: event.file,
                    uploaded_bytes: event.uploaded_bytes,
                    total_bytes: event.total_bytes,
                    status: event.status,
                    error: event.error,
                }),
            );
        }
    };
    let (result, ()) = tokio::join!(upload_join(&dir, state, &s3_client, Some(tx)), forward);
    let (complete, failed) = match result {
        Ok(state) => (
            state.is_complete(),
            state
                .files
                .iter()
                .filter(|(_, f)| f.status == FileStatus::Failed)
                .map(|(name, _)| name.clone())
                .collect(),
        ),
        Err(e) => {
            notify_failure(app, &session_id, e);
            (false, Vec::new())
        }
    };
    // Sent even after an error, so the UI knows this upload is no longer running.
    notify(
        app,
        PublicationNotification::UploadFinished(UploadFinishedData {
            session_id,
            join,
            complete,
            failed,
        }),
    );
}

/// Records (and streams) one SyncFlow session until it is cancelled or the room
/// disconnects, then uploads the recording.
///
/// Joins the room first: if that fails, the session is not recorded. Once recording,
/// device failures are isolated by the pipeline and reported to the UI and to the room
/// on `ROOM_EVENTS`; the session keeps going while any file is still being written.
#[allow(clippy::too_many_arguments)]
pub async fn record_publish_to_syncflow_with_cancellation(
    participant_name: String,
    session_details: NewSessionMessage,
    config: CompositeConfig,
    event_emitter: tauri::AppHandle,
    project_client: &syncflow_client::ProjectClient,
    out_dir: &std::path::Path,
    s3_client: Option<rusoto_s3::S3Client>,
    bucket_name: Option<String>,
    cancel_token: CancellationToken,
) {
    let app = &event_emitter;
    let participant_name = participant_name.replace(".", "-").replace(" ", "-");
    let session_id = session_details.session_id.clone();
    let session_name = session_details.session_name.clone();

    let token =
        match generate_session_token(project_client, participant_name.clone(), &session_details)
            .await
        {
            Ok(token) => token,
            Err(e) => return notify_failure(app, &session_id, e),
        };
    let Some(server_url) = token.livekit_server_url.clone() else {
        return notify_failure(app, &session_id, "SyncFlow returned no LiveKit server URL");
    };
    let (room, mut room_rx) =
        match Room::connect(&server_url, &token.token, RoomOptions::default()).await {
            Ok(connected) => connected,
            Err(e) => return notify_failure(app, &session_id, e),
        };
    let room = Arc::new(room);

    let context = serde_json::json!({
        "session": { "id": session_id, "name": session_name },
        "participant": { "identity": participant_name, "name": participant_name },
        "recordingMode": RecordingMode::SessionMode,
        "appVersion": env!("CARGO_PKG_VERSION"),
    });
    let Recording {
        mut stream,
        dir,
        join,
        events: forwarder_events,
    } = match start_recording(&config, out_dir, &session_name, &session_id, context).await {
        Ok(started) => started,
        Err(e) => return notify_failure(app, &session_id, e),
    };
    let mut stop_events = stream.events();
    spawn_event_forwarder(
        forwarder_events,
        EventContext {
            session_id: session_id.clone(),
            session_name: session_name.clone(),
            participant_identity: participant_name.clone(),
            participant_name: participant_name.clone(),
            streaming: config.streams_anything(),
        },
        stream
            .manifest()
            .unwrap_or_else(|| unreachable!("the stream has started")),
        Some(room.clone()),
        event_emitter.clone(),
    );

    let mut participant = LKParticipant::new(room.clone());
    if let Err(e) = participant.publish_composite(&stream).await {
        notify_failure(
            app,
            &session_id,
            format!("Streaming failed, still recording: {e}"),
        );
    }
    notify(
        app,
        PublicationNotification::StreamingSuccess(StreamingSuccessData {
            session_id: session_id.clone(),
            session_name: session_name.clone(),
            started_at: system_time_nanos().to_string(),
            devices: output_labels(&stream),
        }),
    );

    loop {
        tokio::select! {
            biased;
            _ = cancel_token.cancelled() => break,
            _ = pipeline_stopped(&mut stop_events) => break,
            event = room_rx.recv() => match event {
                Some(livekit::RoomEvent::Disconnected { .. }) | None => break,
                Some(_) => {}
            },
        }
    }

    let manifest = match stream.stop().await {
        Ok(manifest) => Some(manifest),
        Err(e) => {
            notify_failure(app, &session_id, e);
            stream.manifest()
        }
    };
    notify(
        app,
        PublicationNotification::SessionEnded(SessionEndedData {
            session_id: session_id.clone(),
        }),
    );

    let (Some(s3_client), Some(bucket_name)) = (s3_client, bucket_name) else {
        return;
    };
    if !manifest.as_ref().is_some_and(any_finalized) {
        return;
    }
    let project = match project_client.get_project_details().await {
        Ok(project) => project,
        Err(e) => return notify_failure(app, &session_id, e),
    };
    // Downstream analysis reads this layout: `<prefix>/<join>/<file>`, with
    // `manifest.json` uploaded last. Ask before changing it.
    let prefix = format!(
        "{}-{}/{}/local-recordings/{}/{join}",
        project.name,
        project.id,
        session_name,
        participant_name.replace(" ", "-").replace(".", "-"),
    );
    let state = UploadState::new(&session_id, &session_name, join, &bucket_name, &prefix);
    upload_recording(app, dir, state, s3_client).await;
}

/// Records locally until cancelled, then uploads the recording if S3 is configured and
/// any output is set to stream (the existing local-mode rule).
#[allow(clippy::too_many_arguments)]
pub async fn local_recording_only(
    session_id: String,
    session_name: String,
    config: CompositeConfig,
    event_emitter: tauri::AppHandle,
    out_dir: &std::path::Path,
    s3_client: Option<rusoto_s3::S3Client>,
    bucket_name: Option<String>,
    cancel_token: CancellationToken,
) {
    let app = &event_emitter;
    let participant_name = "local-recorder";

    let context = serde_json::json!({
        "session": { "id": session_id, "name": session_name },
        "participant": { "identity": participant_name, "name": participant_name },
        "recordingMode": RecordingMode::LocalMode,
        "appVersion": env!("CARGO_PKG_VERSION"),
    });
    let Recording {
        mut stream,
        dir,
        join,
        events: forwarder_events,
    } = match start_recording(&config, out_dir, &session_name, &session_id, context).await {
        Ok(started) => started,
        Err(e) => return notify_failure(app, &session_id, e),
    };
    let mut stop_events = stream.events();
    spawn_event_forwarder(
        forwarder_events,
        EventContext {
            session_id: session_id.clone(),
            session_name: session_name.clone(),
            participant_identity: participant_name.into(),
            participant_name: participant_name.into(),
            streaming: false,
        },
        stream
            .manifest()
            .unwrap_or_else(|| unreachable!("the stream has started")),
        None,
        event_emitter.clone(),
    );
    notify(
        app,
        PublicationNotification::StreamingSuccess(StreamingSuccessData {
            session_id: session_id.clone(),
            session_name: session_name.clone(),
            started_at: system_time_nanos().to_string(),
            devices: output_labels(&stream),
        }),
    );

    tokio::select! {
        _ = cancel_token.cancelled() => {}
        _ = pipeline_stopped(&mut stop_events) => {}
    }

    let manifest = match stream.stop().await {
        Ok(manifest) => Some(manifest),
        Err(e) => {
            notify_failure(app, &session_id, e);
            stream.manifest()
        }
    };

    if let (Some(s3_client), Some(bucket_name)) = (s3_client, bucket_name) {
        if config.streams_anything() && manifest.as_ref().is_some_and(any_finalized) {
            // Downstream analysis reads this layout: `<prefix>/<join>/<file>`, with
            // `manifest.json` uploaded last. Ask before changing it.
            let prefix = format!(
                "recordings/{}-{}-{}/{join}",
                session_id,
                session_name.replace(" ", "-"),
                participant_name
            );
            let state = UploadState::new(&session_id, &session_name, join, &bucket_name, &prefix);
            upload_recording(app, dir, state, s3_client).await;
        }
    }

    notify(
        app,
        PublicationNotification::SessionEnded(SessionEndedData {
            session_id: session_id.clone(),
        }),
    );
}
