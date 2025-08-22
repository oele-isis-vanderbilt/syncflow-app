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
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tokio::sync::Mutex as AsyncMutex;

use devices::{delete_streaming_config, get_devices, get_streaming_config, set_streaming_config};
use register::{delete_registration, register_to_syncflow};
use session_listener::SessionListener;
use tauri::{Listener, Manager};

use crate::{
    devices::initialize_streaming_config,
    errors::SyncFlowPublisherError,
    models::S3Config,
    register::{get_credentials, get_device_details, RegistrationResponse},
    session_listener::ClonableNewSessionMessage,
    syncflow_publisher::record_publish_to_syncflow,
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
            let _id = app.listen("new-session", move |event| {
                let payload = event.payload();
                let handle = app_handle.clone();
                let recordings_dir_cloned = recordings_dir.clone();

                if let Ok(new_session_details) =
                    serde_json::from_str::<ClonableNewSessionMessage>(payload)
                {
                    let new_session = new_session_details.clone();
                    let devices_and_streaming_config = {
                        let app_state = app_handle.state::<models::AppState>();
                        let config_guard = app_state.recording_and_streaming_config.lock().unwrap();
                        config_guard.clone()
                    };
                    let registration = {
                        let app_state = app_handle.state::<models::AppState>();
                        let registration_guard = app_state.registration.lock().unwrap();
                        registration_guard.clone()
                    };
                    let project_client = {
                        let app_state = app_handle.state::<models::AppState>();
                        let client_guard = app_state.client.lock().unwrap();
                        client_guard.clone()
                    };

                    let (s3_client, bucket_name) = (s3_client.clone(), bucket_name.clone());
                    tauri::async_runtime::spawn(async move {
                        if let (
                            Some(devices_config),
                            Some(registration_details),
                            Some(project_client),
                        ) = (devices_and_streaming_config, registration, project_client)
                        {
                            record_publish_to_syncflow(
                                format!(
                                    "{}-{}({})",
                                    registration_details.device_name.replace(" ", "-"),
                                    registration_details.device_id[..8].to_string(),
                                    registration_details.device_group,
                                ),
                                new_session.into(),
                                devices_config,
                                handle,
                                &project_client,
                                &recordings_dir_cloned,
                                s3_client,
                                bucket_name,
                            )
                            .await;
                        }
                    });
                }
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
                let app_state = models::AppState {
                    client: Arc::new(Mutex::new(client)),
                    app_dir,
                    registration: Arc::new(Mutex::new(registration)),
                    recording_and_streaming_config: Arc::new(Mutex::new(streaming_config)),
                    session_listener: Arc::new(AsyncMutex::new(session_listener)),
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
