use std::{path::PathBuf, sync::Arc, vec};

use crate::{errors::SyncFlowPublisherError, models::DeviceRecordingAndStreamingConfig};
use livekit::{Room, RoomOptions};
use livekit_gstreamer::{lk_participant, GstMediaStream, LocalFileSaveOptions, PublishOptions};
use serde::{Deserialize, Serialize};
use syncflow_shared::{
    device_models::NewSessionMessage,
    livekit_models::{TokenRequest, TokenResponse, VideoGrantsWrapper},
};
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "kind")]
pub enum PublicationNotifcation {
    Failure {
        reason: String,
    },
    Success {
        session_id: String,
        session_name: String,
        started_at: String,
        devices: Vec<String>,
    },
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

pub async fn record_publish_to_syncflow(
    participant_name: String,
    session_details: NewSessionMessage,
    configs: Vec<DeviceRecordingAndStreamingConfig>,
    event_emitter: tauri::AppHandle,
    project_client: &syncflow_client::ProjectClient,
    out_dir: &PathBuf,
) {
    let mut streams_and_recording_config: Vec<(GstMediaStream, bool)> = configs
        .into_iter()
        .map(|config| {
            let mut cloned_publish_options = config.publish_options.clone();
            let output_dir = out_dir.join(format!(
                "{}-{}",
                session_details.session_id, session_details.session_name
            ));
            let local_file_save_options = Some(LocalFileSaveOptions {
                output_dir: output_dir.to_string_lossy().to_string(),
            });

            match &mut cloned_publish_options {
                PublishOptions::Video(video_publish_options) => {
                    video_publish_options.local_file_save_options = local_file_save_options;
                }
                PublishOptions::Audio(audio_publish_options) => {
                    audio_publish_options.local_file_save_options = local_file_save_options;
                }
                PublishOptions::Screen(screen_publish_options) => {
                    screen_publish_options.local_file_save_options = local_file_save_options;
                }
            }

            let stream = GstMediaStream::new(cloned_publish_options);
            (stream, config.enable_streaming)
        })
        .collect();

    let token_result =
        generate_session_token(project_client, participant_name.clone(), &session_details).await;

    if let Err(e) = token_result {
        let _ = event_emitter.emit(
            "publication-notification",
            PublicationNotifcation::Failure {
                reason: e.to_string(),
            },
        );
        return;
    }

    let token = token_result.unwrap();

    let room_result = Room::connect(
        &token.livekit_server_url.unwrap(),
        &token.token,
        RoomOptions::default(),
    )
    .await;

    if let Err(e) = room_result {
        let _ = event_emitter.emit(
            "publication-notification",
            PublicationNotifcation::Failure {
                reason: e.to_string(),
            },
        );
        return;
    }

    let (room, mut room_rx) = room_result.unwrap();

    let room_arc = Arc::new(room);

    let mut participant = lk_participant::LKParticipant::new(room_arc.clone());

    let mut all_failures = vec![];

    for (stream, enable_streaming) in streams_and_recording_config.iter_mut() {
        stream.start().await.unwrap();
        if *enable_streaming {
            let result = participant.publish_stream(stream, None).await;
            if let Err(e) = result {
                all_failures.push(e.to_string());
            }
        }
    }

    while let Some(msg) = room_rx.recv().await {
        match msg {
            livekit::RoomEvent::Disconnected { reason } => {
                println!("Disconnected from room: {:?}", reason);
                for stream in streams_and_recording_config.iter_mut() {
                    stream.0.stop().await.unwrap();
                }
                room_arc.close().await.unwrap();
                break;
            }
            _ => {
                println!("Received room event: {:?}", msg);
            }
        }
    }
}
