// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod devices;
mod errors;
mod models;
mod register;
mod s3_uploader;
mod session_listener;
mod syncflow_publisher;
mod utils;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use livekit_gstreamer::utils::system_time_nanos;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use devices::{delete_streaming_config, get_devices, get_streaming_config, set_streaming_config};
use register::{delete_registration, register_to_syncflow};
use session_listener::SessionListener;
use tauri::{Emitter, Listener, Manager};

use crate::{
    devices::initialize_streaming_config,
    errors::SyncFlowPublisherError,
    models::{ActiveSession, S3Config},
    register::{get_credentials, get_device_details, RegistrationResponse},
    session_listener::ClonableNewSessionMessage,
    syncflow_publisher::{
        local_recording_only, record_publish_to_syncflow,
        record_publish_to_syncflow_with_cancellation,
    },
    utils::load_json,
};

fn create_app_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home_dir = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not find home directory",
        )
    })?;
    let app_dir = home_dir.join(".syncflow-publisher");
    std::fs::create_dir_all(&app_dir)?;
    Ok(app_dir)
}

#[tauri::command]
fn get_registration(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<RegistrationResponse, SyncFlowPublisherError> {
    let registration_guard = app_state.registration.lock().unwrap();
    if let Some(registration) = &*registration_guard {
        Ok(registration.clone())
    } else {
        Err(SyncFlowPublisherError::NotIntialized(
            "Registration not found".to_string(),
        ))
    }
}

#[tauri::command]
async fn exit_session(
    session_id: String,
    app_state: tauri::State<'_, models::AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), SyncFlowPublisherError> {
    let mut active_sessions = app_state.active_sessions.lock().await;

    if let Some(session) = active_sessions.get(&session_id) {
        session.cancel_token.cancel();
        println!("Cancelled session: {}", session_id);
    }

    // Keep session in active_sessions for potential rejoining
    // Only remove from active_sessions when task completes naturally

    // Clear currently joined session if this was the joined one
    let mut currently_joined = app_state.currently_joined_session.lock().await;
    if let Some(ref joined_id) = *currently_joined {
        if joined_id == &session_id {
            *currently_joined = None;
        }
    }
    drop(currently_joined);

    drop(active_sessions);

    // Emit session update event
    let _ = app_handle.emit("session-update", ());
    Ok(())
}

#[tauri::command]
async fn rejoin_session(
    session_id: String,
    session_name: String,
    app_state: tauri::State<'_, models::AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), SyncFlowPublisherError> {
    // Get required data from app state
    let devices_and_streaming_config = {
        let config_guard = app_state.recording_and_streaming_config.lock().unwrap();
        config_guard.clone()
    };

    let registration = {
        let registration_guard = app_state.registration.lock().unwrap();
        registration_guard.clone()
    };

    let project_client = {
        let client_guard = app_state.client.lock().unwrap();
        client_guard.clone()
    };

    // Exit currently joined session before joining new one
    exit_currently_joined_session(&app_state).await;

    if let (Some(devices_config), Some(registration_details), Some(project_client)) =
        (devices_and_streaming_config, registration, project_client)
    {
        // Load S3 config if available
        let s3_config_file = app_state.app_dir.join("s3_credentials.json");
        let (s3_client, bucket_name): (Option<rusoto_s3::S3Client>, Option<String>) =
            if s3_config_file.exists() {
                match load_json::<S3Config>(&s3_config_file) {
                    Ok(config) => {
                        let bucket_name = config.s3_bucket.clone();
                        let s3_client = rusoto_s3::S3Client::from(config);
                        (Some(s3_client), Some(bucket_name))
                    }
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            };

        let recordings_dir = app_state.app_dir.join("recordings");
        let active_sessions = app_state.active_sessions.clone();
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();
        let session_id_clone = session_id.clone();
        let session_name_clone = session_name.clone();

        let new_session = syncflow_shared::device_models::NewSessionMessage {
            session_id: session_id.clone(),
            session_name: session_name.clone(),
        };
        let cloned_app_handle = app_handle.clone();
        let task = tauri::async_runtime::spawn(async move {
            record_publish_to_syncflow_with_cancellation(
                format!(
                    "{}-{}({})",
                    registration_details.device_name.replace(" ", "-"),
                    registration_details.device_id[..8].to_string(),
                    registration_details.device_group,
                ),
                new_session,
                devices_config,
                cloned_app_handle,
                &project_client,
                &recordings_dir,
                s3_client,
                bucket_name,
                cancel_token_clone,
            )
            .await;

            // Remove from active sessions when task completes naturally
            let mut active_sessions_guard = active_sessions.lock().await;
            active_sessions_guard.remove(&session_id_clone);
        });

        // Add to active sessions
        let mut active_sessions_guard = app_state.active_sessions.lock().await;
        active_sessions_guard.insert(
            session_id.clone(),
            ActiveSession {
                session_id: session_id.clone(),
                session_name: session_name_clone,
                cancel_token,
                task_handle: task,
            },
        );
        drop(active_sessions_guard);

        // Set as currently joined session
        let mut currently_joined = app_state.currently_joined_session.lock().await;
        *currently_joined = Some(session_id.clone());
        drop(currently_joined);

        // Emit session update event
        let _ = app_handle.emit("session-update", ());

        Ok(())
    } else {
        Err(SyncFlowPublisherError::NotIntialized(
            "Required configuration not found".to_string(),
        ))
    }
}

#[tauri::command]
async fn get_active_sessions(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<Vec<(String, String)>, SyncFlowPublisherError> {
    let active_sessions = app_state.active_sessions.lock().await;
    let sessions: Vec<(String, String)> = active_sessions
        .values()
        .map(|session| (session.session_id.clone(), session.session_name.clone()))
        .collect();
    Ok(sessions)
}

#[tauri::command]
async fn get_all_sessions(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<Vec<(String, String, bool)>, SyncFlowPublisherError> {
    // Get active sessions from memory
    let active_sessions = app_state.active_sessions.lock().await;
    let active_session_ids: std::collections::HashSet<String> =
        active_sessions.keys().cloned().collect();

    // Get all sessions from syncflow client
    let all_sessions = if let Some(client) = {
        let client_guard = app_state.client.lock().unwrap();
        client_guard.clone()
    } {
        client.get_sessions().await.unwrap_or_default()
    } else {
        Vec::new()
    };

    // Check which session is currently joined (actually running)
    let currently_joined = app_state.currently_joined_session.lock().await;
    let joined_session_id = currently_joined.clone();
    drop(currently_joined);

    // Combine and mark active status - only mark as active if it's the currently joined session
    let sessions: Vec<(String, String, bool)> = all_sessions
        .into_iter()
        .filter(|session| session.status == "Started")
        .map(|session| {
            let is_active = joined_session_id.as_ref() == Some(&session.id);
            (session.id, session.name, is_active)
        })
        .collect();

    Ok(sessions)
}

async fn exit_currently_joined_session(app_state: &models::AppState) {
    let mut currently_joined = app_state.currently_joined_session.lock().await;

    if let Some(session_id) = currently_joined.take() {
        let active_sessions = app_state.active_sessions.lock().await;
        if let Some(session) = active_sessions.get(&session_id) {
            session.cancel_token.cancel();
            println!("Exited currently joined session: {}", session_id);
        }
    }
}

#[tauri::command]
async fn get_currently_joined_session(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<Option<String>, SyncFlowPublisherError> {
    let currently_joined = app_state.currently_joined_session.lock().await;
    Ok(currently_joined.clone())
}

#[tauri::command]
async fn start_local_recording(
    session_name: String,
    app_state: tauri::State<'_, models::AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), SyncFlowPublisherError> {
    // Get required data from app state
    let configs = {
        let config_guard = app_state.recording_and_streaming_config.lock().unwrap();
        config_guard.clone()
    };

    let configs = configs.ok_or_else(|| {
        SyncFlowPublisherError::NotIntialized("Recording config not found".to_string())
    })?;

    // Check if already recording
    let mut local_recording_guard = app_state.local_recording_session.lock().await;
    if local_recording_guard.is_some() {
        return Err(SyncFlowPublisherError::NotIntialized(
            "Local recording already in progress".to_string(),
        ));
    }

    // Load S3 config if available
    let s3_config_file = app_state.app_dir.join("s3_credentials.json");
    let (s3_client, bucket_name): (Option<rusoto_s3::S3Client>, Option<String>) =
        if s3_config_file.exists() {
            match load_json::<S3Config>(&s3_config_file) {
                Ok(config) => {
                    let bucket_name = config.s3_bucket.clone();
                    let s3_client = rusoto_s3::S3Client::from(config);
                    (Some(s3_client), Some(bucket_name))
                }
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        };

    let recordings_dir = app_state.app_dir.join("recordings");
    let cancel_token = CancellationToken::new();
    let cancel_token_clone = cancel_token.clone();
    let session_name_clone = session_name.clone();
    let session_id = format!("local-{}", system_time_nanos());

    let task = tauri::async_runtime::spawn(async move {
        local_recording_only(
            session_name_clone,
            configs,
            app_handle,
            &recordings_dir,
            s3_client,
            bucket_name,
            cancel_token_clone,
        )
        .await;
    });

    let active_session = ActiveSession {
        session_id: session_id.clone(),
        session_name: session_name,
        cancel_token,
        task_handle: task,
    };

    *local_recording_guard = Some(active_session);

    Ok(())
}

#[tauri::command]
async fn stop_local_recording(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<(), SyncFlowPublisherError> {
    let mut local_recording_guard = app_state.local_recording_session.lock().await;

    if let Some(session) = local_recording_guard.take() {
        session.cancel_token.cancel();
        println!("Stopped local recording session: {}", session.session_id);
        Ok(())
    } else {
        Err(SyncFlowPublisherError::NotIntialized(
            "No local recording session to stop".to_string(),
        ))
    }
}

#[tauri::command]
fn set_recording_mode(
    recording_mode: models::RecordingMode,
    app_state: tauri::State<'_, models::AppState>,
) -> Result<(), SyncFlowPublisherError> {
    let recording_mode_file = app_state.app_dir.join("recording_mode.json");
    utils::save_json(&recording_mode, &recording_mode_file)?;
    Ok(())
}

#[tauri::command]
fn get_recording_mode(
    app_state: tauri::State<'_, models::AppState>,
) -> Result<models::RecordingMode, SyncFlowPublisherError> {
    let recording_mode_file = app_state.app_dir.join("recording_mode.json");
    if recording_mode_file.exists() {
        utils::load_json(&recording_mode_file)
    } else {
        // Default to session mode if no preference is stored
        Ok(models::RecordingMode::SessionMode)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            livekit_gstreamer::initialize_gstreamer();
            let app_dir = create_app_dir().expect("Failed to create app directory");
            let recordings_dir = app_dir.clone().join("recordings");
            let s3_config_file = app_dir.join("s3_credentials.json");

            let (s3_client, bucket_name): (Option<rusoto_s3::S3Client>, Option<String>) =
                if s3_config_file.exists() {
                    match load_json::<S3Config>(&s3_config_file) {
                        Ok(config) => {
                            println!("S3 config loaded successfully");
                            let bucket_name = config.s3_bucket.clone();
                            let s3_client = rusoto_s3::S3Client::from(config);
                            (Some(s3_client), Some(bucket_name))
                        }
                        Err(_) => (None, None),
                    }
                } else {
                    (None, None)
                };

            let app_handle = app.handle().clone();
            let recordings_dir_for_listener = recordings_dir.clone();
            let s3_client_for_listener = s3_client.clone();
            let bucket_name_for_listener = bucket_name.clone();
            let app_handle_for_listener = app_handle.clone();
            let _id = app.listen("new-session", move |event| {
                let recordings_dir_cloned = recordings_dir_for_listener.clone();
                let s3_client_cloned = s3_client_for_listener.clone();
                let bucket_name_cloned = bucket_name_for_listener.clone();
                let app_handle_cloned = app_handle_for_listener.clone();
                tauri::async_runtime::spawn(async move {
                    let payload = event.payload();
                    let handle = app_handle_cloned.clone();

                    if let Ok(new_session_details) =
                        serde_json::from_str::<ClonableNewSessionMessage>(payload)
                    {
                        let new_session = new_session_details.clone();
                        let devices_and_streaming_config = {
                            let app_state = handle.state::<models::AppState>();
                            let config_guard =
                                app_state.recording_and_streaming_config.lock().unwrap();
                            config_guard.clone()
                        };
                        let registration = {
                            let app_state = handle.state::<models::AppState>();
                            let registration_guard = app_state.registration.lock().unwrap();
                            registration_guard.clone()
                        };
                        let project_client = {
                            let app_state = handle.state::<models::AppState>();
                            let client_guard = app_state.client.lock().unwrap();
                            client_guard.clone()
                        };

                        let (s3_client, bucket_name) =
                            (s3_client_cloned.clone(), bucket_name_cloned.clone());
                        let app_state = handle.state::<models::AppState>();
                        let active_sessions = app_state.active_sessions.clone();

                        // Exit currently joined session before joining new one
                        exit_currently_joined_session(&app_state).await;

                        let session_id = new_session.session_id.clone();
                        let session_name = new_session.session_name.clone();
                        let cancel_token = CancellationToken::new();
                        let cancel_token_clone = cancel_token.clone();
                        let active_sessions_for_cleanup = active_sessions.clone();
                        let session_id_for_cleanup = session_id.clone();
                        let handle_for_task = handle.clone();

                        let task = tauri::async_runtime::spawn(async move {
                            if let (
                                Some(devices_config),
                                Some(registration_details),
                                Some(project_client),
                            ) = (devices_and_streaming_config, registration, project_client)
                            {
                                record_publish_to_syncflow_with_cancellation(
                                    format!(
                                        "{}-{}({})",
                                        registration_details.device_name.replace(" ", "-"),
                                        registration_details.device_id[..8].to_string(),
                                        registration_details.device_group,
                                    ),
                                    new_session.into(),
                                    devices_config,
                                    handle_for_task,
                                    &project_client,
                                    &recordings_dir_cloned,
                                    s3_client,
                                    bucket_name,
                                    cancel_token_clone,
                                )
                                .await;
                            }

                            // Remove from active sessions when task completes naturally
                            let mut active_sessions_guard =
                                active_sessions_for_cleanup.lock().await;
                            active_sessions_guard.remove(&session_id_for_cleanup);
                        });

                        // Add to active sessions immediately
                        let mut active_sessions_guard = active_sessions.lock().await;
                        active_sessions_guard.insert(
                            session_id.clone(),
                            ActiveSession {
                                session_id: session_id.clone(),
                                session_name: session_name.clone(),
                                cancel_token,
                                task_handle: task,
                            },
                        );
                        drop(active_sessions_guard);

                        // Set as currently joined session
                        let mut currently_joined = app_state.currently_joined_session.lock().await;
                        *currently_joined = Some(session_id.clone());
                        drop(currently_joined);

                        // Emit unified session update event instead of just new-session
                        let _ = handle.emit("session-update", ());
                    }
                });
            });
            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(async {
                let client = register::intialize_client(&app_dir).await;
                let credentials = get_credentials(&app_dir).await;
                let device_registration_details = get_device_details(&app_dir).await;
                let registration = if let Some(c) = client.as_ref() {
                    register::register_if_needed(c, &app_dir).await
                } else {
                    None
                };

                let streaming_config = initialize_streaming_config(&app_dir);
                // Fetch active sessions from syncflow client on startup
                let startup_active_sessions = if let Some(ref project_client) = client {
                    let sessions = project_client.get_sessions().await.unwrap_or_default();
                    sessions
                        .into_iter()
                        .filter(|session| session.status == "Started")
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };

                let session_listener = if let (Some(reg_details), Some(credentials)) =
                    (device_registration_details.as_ref(), credentials)
                {
                    let mut listener = SessionListener::new(
                        &credentials.rabbitmq_host,
                        credentials.rabbitmq_port,
                        &credentials.rabbitmq_username,
                        &credentials.rabbitmq_password,
                        &credentials.rabbitmq_vhost,
                        &reg_details
                            .session_notification_exchange_name
                            .clone()
                            .unwrap(),
                        &reg_details
                            .session_notification_binding_key
                            .clone()
                            .unwrap(),
                    );
                    let _ = listener.start().await;
                    let _ = listener.start_frontend_notifications(app_handle).await;
                    Some(listener)
                } else {
                    None
                };
                // Create initial active sessions HashMap from startup sessions
                let mut initial_active_sessions = HashMap::new();
                for session in startup_active_sessions {
                    initial_active_sessions.insert(
                        session.id.clone(),
                        ActiveSession {
                            session_id: session.id.clone(),
                            session_name: session.name.clone(),
                            cancel_token: CancellationToken::new(),
                            task_handle: tauri::async_runtime::spawn(async {}),
                        },
                    );
                }

                let app_state = models::AppState {
                    client: Arc::new(Mutex::new(client)),
                    app_dir,
                    registration: Arc::new(Mutex::new(registration)),
                    recording_and_streaming_config: Arc::new(Mutex::new(streaming_config)),
                    session_listener: Arc::new(AsyncMutex::new(session_listener)),
                    active_sessions: Arc::new(AsyncMutex::new(initial_active_sessions)),
                    currently_joined_session: Arc::new(AsyncMutex::new(None)),
                    local_recording_session: Arc::new(AsyncMutex::new(None)),
                };
                app.manage(app_state);
            });

            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_devices,
            set_streaming_config,
            get_streaming_config,
            delete_streaming_config,
            get_registration,
            register_to_syncflow,
            delete_registration,
            exit_session,
            rejoin_session,
            get_active_sessions,
            get_all_sessions,
            get_currently_joined_session,
            start_local_recording,
            stop_local_recording,
            set_recording_mode,
            get_recording_mode,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                tauri::async_runtime::block_on(async {
                    let app_state = app_handle.state::<models::AppState>();
                    let _ = register::deregister_from_syncflow(&app_state).await;

                    let mut session_listener_guard = app_state.session_listener.lock().await;

                    if let Some(ref mut listener) = *session_listener_guard {
                        println!("Stopping session listener...");
                        if let Err(e) = listener.stop().await {
                            eprintln!("Error stopping session listener: {}", e);
                        }
                    }
                });
            }
        })
}
