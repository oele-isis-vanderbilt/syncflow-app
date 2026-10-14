use std::{path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

use crate::{
    errors::SyncFlowPublisherError,
    models::{CompositeConfig, RecordingMode},
    pipeline_events::{spawn_event_forwarder, DeviceDetails, EventContext},
    s3_uploader::upload_to_s3,
    utils::{recording_folder_name, short_session_id, unique_recording_dir},
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
use tauri::Emitter;
use tokio::sync::{broadcast, mpsc::channel};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PublicationNotification {
    Failure(FailureData),
    StreamingSuccess(StreamingSuccessData),
    UploadProgress(UploadProgressData),
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
pub struct UploadProgressData {
    pub session_id: String,
    pub progress: f32,
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

/// Builds and starts the composite pipeline for one recording in a new, readably named
/// folder under `out_dir`. Returns the stream, its folder, and an events receiver taken
/// before start (so it also sees devices that failed to open).
async fn start_recording(
    config: &CompositeConfig,
    out_dir: &std::path::Path,
    folder: &str,
    manifest_context: serde_json::Value,
) -> Result<
    (
        CompositeStream,
        PathBuf,
        broadcast::Receiver<CompositeEvent>,
    ),
    SyncFlowPublisherError,
> {
    let output_dir = unique_recording_dir(out_dir, folder);
    let mut stream =
        CompositeStream::new(config.to_stream_spec(output_dir.clone(), manifest_context));
    let events = stream.events();
    stream.start().await?;
    Ok((stream, output_dir, events))
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

/// Zips `output_dir` next to itself and uploads it under `key` (`.zip` is appended),
/// reporting progress and failures to the UI.
async fn upload_recording(
    app: &tauri::AppHandle,
    session_id: &str,
    output_dir: PathBuf,
    bucket_name: String,
    key: String,
    s3_client: rusoto_s3::S3Client,
) {
    let (tx, mut rx) = channel::<f32>(100);
    let failure_app = app.clone();
    let failure_session = session_id.to_string();
    tauri::async_runtime::spawn(async move {
        let result = upload_to_s3(&output_dir, &bucket_name, &key, &s3_client, Some(tx)).await;
        if let Err(e) = result {
            notify_failure(&failure_app, &failure_session, e);
        }
    });
    while let Some(progress) = rx.recv().await {
        notify(
            app,
            PublicationNotification::UploadProgress(UploadProgressData {
                progress,
                session_id: session_id.to_string(),
            }),
        );
    }
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

    let folder = recording_folder_name(
        chrono::Local::now(),
        &session_name,
        &short_session_id(&session_id),
    );
    let context = serde_json::json!({
        "session": { "id": session_id, "name": session_name },
        "participant": { "identity": participant_name, "name": participant_name },
        "recordingMode": RecordingMode::SessionMode,
        "appVersion": env!("CARGO_PKG_VERSION"),
    });
    let (mut stream, output_dir, forwarder_events) =
        match start_recording(&config, out_dir, &folder, context).await {
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
    // The S3 key format is relied on downstream; keep it unchanged.
    let key = format!(
        "{}-{}/{}/local-recordings/{}",
        project.name,
        project.id,
        session_name,
        participant_name.replace(" ", "-").replace(".", "-"),
    );
    upload_recording(app, &session_id, output_dir, bucket_name, key, s3_client).await;
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

    let folder = recording_folder_name(chrono::Local::now(), &session_name, "local");
    let context = serde_json::json!({
        "session": { "id": session_id, "name": session_name },
        "participant": { "identity": participant_name, "name": participant_name },
        "recordingMode": RecordingMode::LocalMode,
        "appVersion": env!("CARGO_PKG_VERSION"),
    });
    let (mut stream, output_dir, forwarder_events) =
        match start_recording(&config, out_dir, &folder, context).await {
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
            // The S3 key format is relied on downstream; keep it unchanged.
            let key = format!(
                "recordings/{}-{}-{}.zip",
                session_id,
                session_name.replace(" ", "-"),
                participant_name
            );
            upload_recording(app, &session_id, output_dir, bucket_name, key, s3_client).await;
        }
    }

    notify(
        app,
        PublicationNotification::SessionEnded(SessionEndedData {
            session_id: session_id.clone(),
        }),
    );
}
