use std::error::Error;
use std::path::PathBuf;
use std::{env, sync::Arc};

use amqprs::{
    channel::{BasicConsumeArguments, QueueBindArguments, QueueDeclareArguments},
    connection::{Connection, OpenConnectionArguments},
    tls::TlsAdaptor,
};
use chrono::format;
use livekit::{Room, RoomEvent, RoomOptions};
use livekit_gstreamer::{
    initialize_gstreamer, AudioPublishOptions, GstMediaStream, LKParticipant, LocalFileSaveOptions,
    PublishOptions, VideoPublishOptions,
};
use rusoto_s3::S3Client;
use syncflow_client::{ProjectClient, ProjectClientError};
use syncflow_shared::{
    device_models::{DeviceRegisterRequest, DeviceResponse, NewSessionMessage},
    livekit_models::{TokenRequest, VideoGrantsWrapper},
};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[path = "./helper/s3_uploader.rs"]
mod s3_uploader;

pub fn get_s3_client(
    s3_region: &str,
    s3_endpoint: &str,
    s3_access_key: &str,
    s3_secret_key: &str,
) -> S3Client {
     let client = rusoto_core::HttpClient::new().expect("Failed to create request dispatcher");
    let region = rusoto_core::Region::Custom {
            name: s3_region.to_string(),
            endpoint: s3_endpoint.to_string(),
        };
        let credentials_provider = rusoto_credential::StaticProvider::new_minimal(
            s3_access_key.to_string(),
            s3_secret_key.to_string(),
        );

        let s3_client = rusoto_s3::S3Client::new_with(client, credentials_provider, region);

        s3_client
}

async fn register_device(
    project_client: &ProjectClient,
) -> Result<DeviceResponse, ProjectClientError> {
    let device_request = DeviceRegisterRequest {
        comments: Some("UMC1820 Publisher Registration".into()),
        group: "GEMSTEP-TEST".into(),
        name: "UMC1820Publisher".into(),
    };

    project_client.register_device(&device_request).await
}

async fn broadcast_new_session(
    rabbitmq_host: &str,
    rabbitmq_port: u16,
    rabbitmq_username: &str,
    rabbitmq_password: &str,
    rabbitmq_vhost: &str,
    exchange_name: &str,
    binding_key: &str,
    tx: tokio::sync::mpsc::Sender<NewSessionMessage>,
) -> Result<(), Box<dyn Error>> {
    let args = OpenConnectionArguments::new(
        rabbitmq_host,
        rabbitmq_port,
        rabbitmq_username,
        rabbitmq_password,
    )
    .virtual_host(rabbitmq_vhost)
    .tls_adaptor(TlsAdaptor::without_client_auth(None, rabbitmq_host.to_string()).unwrap())
    .finish();

    let connection = Connection::open(&args).await?;

    let channel = connection.open_channel(None).await?;

    let queue_declare_args = QueueDeclareArguments::default()
        .exclusive(true)
        .auto_delete(true)
        .finish();

    let (queue_name, _, _) = channel
        .queue_declare(queue_declare_args)
        .await?
        .ok_or_else(|| "Failed to declare queue")?;

    let queue_bind_args = QueueBindArguments::new(&queue_name, &exchange_name, &binding_key);
    channel.queue_bind(queue_bind_args).await?;

    let consume_args = BasicConsumeArguments::new(&queue_name, "lk-example-consumer");

    let result = channel.basic_consume_rx(consume_args).await;

    let (_, mut rx) = result?;

    while let Some(message) = rx.recv().await {
        if let Ok(new_session_message) =
            serde_json::from_slice::<NewSessionMessage>(&message.content.unwrap())
        {
            if tx.send(new_session_message).await.is_err() {
                log::error!("Failed to send new session message to channel");
            }
        } else {
            log::warn!("Received invalid message format");
        }
    }

    Ok(())
}

async fn publish_streams(
    project_client: &ProjectClient,
    session_message: &NewSessionMessage,
    cancel_rx: &mut tokio::sync::broadcast::Receiver<()>,
    s3_client: Option<&S3Client>,
    device_id: &str,
    selected_channels: Vec<u32>,
) -> Result<(), Box<dyn Error>> {
    let tk_request_channel_1 = TokenRequest {
        identity: "umc1820-channel-1".to_string(),
        name: Some("UMC1820 Publisher Channel 1".to_string()),
        video_grants: VideoGrantsWrapper {
            room: session_message.session_name.clone(),
            can_publish: true,
            room_join: true,
            room_create: false,
            ..Default::default()
        },
    };

    let tk_request_channel_rest = TokenRequest {
        identity: "umc1820-multi-channel".to_string(),
        name: Some("UMC1820 Publisher Multi Channel".to_string()),
        video_grants: VideoGrantsWrapper {
            room: session_message.session_name.clone(),
            can_publish: true,
            room_join: true,
            room_create: false,
            ..Default::default()
        },
    };

    let token_channel_1 = project_client
        .generate_session_token(&session_message.session_id, &tk_request_channel_1)
        .await?;

    let token_channel_rest = project_client
        .generate_session_token(&session_message.session_id, &tk_request_channel_rest)
        .await?;

    let (room_channel_1, mut room_channel_1_rx) = Room::connect(
        &token_channel_1.livekit_server_url.unwrap(),
        &token_channel_1.token,
        RoomOptions::default(),
    )
    .await
    .unwrap();

    let (room_rest, mut room_rest_rx) = Room::connect(
        &token_channel_rest.livekit_server_url.unwrap(),
        &token_channel_rest.token,
        RoomOptions::default(),
    ).await.unwrap();

    let new_room_channel_1 = Arc::new(room_channel_1);
    let mut participant_channel_1 = LKParticipant::new(new_room_channel_1.clone());
    let session_id: &str = &session_message.session_id;
    let session_name = new_room_channel_1.name();
    let op_dir = format!("recordings-umc-1820/{}-{}", session_name, session_id);

    let new_room_rest = Arc::new(room_rest);
    let mut participant_rest = LKParticipant::new(new_room_rest.clone());


    let stream_closure = |hw_id: &str, channel| {
        GstMediaStream::new(PublishOptions::Audio(AudioPublishOptions {
            codec: "audio/x-raw".to_string(),
            framerate: 96000,
            channels: 10,
            selected_channel: Some(channel),
            device_id: hw_id.to_string(),
            local_file_save_options: Some(LocalFileSaveOptions {
                output_dir: op_dir.clone().to_string(),
            }),
        }))
    };

    let mut streams = selected_channels
        .iter()
        .map(|&ch| stream_closure(device_id, ch as i32))
        .collect::<Vec<GstMediaStream>>();

    for (stream, ch) in streams.iter_mut().zip(selected_channels.iter()) {
        stream.start().await?;
        if (*ch == 1) {
            participant_channel_1
                .publish_stream(stream, Some("UMC1820-Channel1".into()))
                .await?;
            continue;
        } else {
            participant_rest
                .publish_stream(stream, Some(format!("UMC1820-Channel{}", ch)))
                .await?;
        }
    }

    log::info!(
        "Connected to room: {} - {}",
        new_room_channel_1.name(),
        String::from(new_room_channel_1.sid().await)
    );

    loop {
        tokio::select! {
            msg = room_channel_1_rx.recv() => {
                match msg {
                    Some(RoomEvent::Disconnected { reason }) => {
                        log::info!("Disconnected from room: {:?}", reason);
                        for stream in &mut streams {
                            stream.stop().await?;
                        }
                        break;
                    }
                    Some(other_event) => {
                        log::info!("Received room event: {:?}", other_event);
                    }
                    None => {
                        log::info!("Room event channel closed");
                        // stream.stop().await?;
                        for stream in &mut streams {
                            stream.stop().await?;
                        }
                        break;
                    }
                }
            }
            _ = cancel_rx.recv() => {
                log::info!("Received Ctrl+C, stopping stream and disconnecting");
                // stream.stop().await?;
                for stream in &mut streams {
                    stream.stop().await?;
                }
                new_room_channel_1.close().await?;
                new_room_rest.close().await?;
                log::info!("Disconnected from room");
                break;
            }
        }
    }

    if let Some(s3_client) = s3_client {
        let project_details = project_client.get_project_details().await?;
        let bucket = "gemstep-bucket";
        let key = format!(
            "{}-{}/{}/local-recordings/{}",
            project_details.name,
            project_details.id,
            session_name,
            "umc1820".to_string()
        );

        s3_uploader::upload_to_s3(
            &PathBuf::from(&op_dir),
            bucket,
            &key,
            s3_client,
            None
        ).await;
    }

    Ok(())
}

struct TaskHandle {
    task: JoinHandle<()>,
    close_tx: broadcast::Sender<()>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    env::set_var("RUST_LOG", "info");
    env_logger::init();
    initialize_gstreamer();

    let server_url = env::var("SYNCFLOW_SERVER_URL").expect("SYNCFLOW_SERVER_URL is not set");
    let api_key = env::var("SYNCFLOW_API_KEY").expect("SYNCFLOW_API_KEY is not set");
    let api_secret = env::var("SYNCFLOW_API_SECRET").expect("SYNCFLOW_API_SECRET is not set");
    let project_id = env::var("SYNCFLOW_PROJECT_ID").expect("SYNCFLOW_PROJECT_ID is not set");

    let rabbitmq_host = env::var("RABBITMQ_HOST").expect("RABBITMQ_HOST is not set");
    let rabbitmq_port: u16 = env::var("RABBITMQ_PORT")
        .expect("RABBITMQ_PORT is not set")
        .parse()
        .expect("RABBITMQ_PORT must be a valid u16");

    let rabbitmq_username = env::var("RABBITMQ_USERNAME").expect("RABBITMQ_USERNAME is not set");
    let rabbitmq_password = env::var("RABBITMQ_PASSWORD").expect("RABBITMQ_PASSWORD is not set");
    let rabbitmq_vhost = env::var("RABBITMQ_VHOST").expect("RABBITMQ_VHOST is not set");
    let s3_region = env::var("S3_REGION").expect("S3_REGION is not set");
    let s3_endpoint = env::var("S3_ENDPOINT").expect("S3_ENDPOINT is not set");
    let s3_access_key = env::var("S3_ACCESS_KEY").expect("S3_ACCESS_KEY is not set");
    let s3_secret_key = env::var("S3_SECRET_KEY").expect("S3_SECRET_KEY is not set");
    let hw_id = env::var("AUDIO_HW_ID").expect("AUDIO_HW_ID is not set");
    let selected_channels = env::var("AUDIO_SELECTED_CHANNELS").expect("AUDIO_SELECTED_CHANNELS is not set");
    let selected_channels: Vec<u32> = selected_channels
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();

    let project_client = ProjectClient::new(&server_url, &project_id, &api_key, &api_secret);

    let registration_response = register_device(&project_client).await?;

    log::info!(
        "Device registered successfully: {:#?}",
        registration_response
    );

    let exchange_name = registration_response
        .session_notification_exchange_name
        .expect("Exchange name not found");
    let binding_key = registration_response
        .session_notification_binding_key
        .expect("Binding key not found");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<NewSessionMessage>(1);
    let mut task_handle: Option<TaskHandle> = None;
    let (sender, _) = broadcast::channel::<()>(1);

    tokio::spawn(async move {
        let _ = broadcast_new_session(
            &rabbitmq_host,
            rabbitmq_port,
            &rabbitmq_username,
            &rabbitmq_password,
            &rabbitmq_vhost,
            &exchange_name,
            &binding_key,
            tx.clone(),
        )
        .await;
    });


    loop {
        tokio::select! {
            Some(new_session_message) = rx.recv() => {
                let s3_client = get_s3_client(&s3_region, &s3_endpoint, &s3_access_key, &s3_secret_key);
                log::info!("Received new session message: {:?}", new_session_message);
                let pc = project_client.clone();
                let session_message = new_session_message;
                let mut close_rx = sender.subscribe();
                let close_tx = sender.clone();
                let hw_id = hw_id.clone();
                let selected_channels = selected_channels.clone();

                let task = tokio::spawn(async move {
                    let _ = publish_streams(
                        &pc,
                        &session_message,
                        &mut close_rx,
                        Some(&s3_client),
                        &hw_id,
                        selected_channels.clone(),
                    ).await;
                });

                task_handle = Some(TaskHandle {
                    task,
                    close_tx,
                });
            }
            _ = tokio::signal::ctrl_c() => {
                log::info!("Received Ctrl+C, shutting down...");
                if let Some(handle) = task_handle {
                    let _ = handle.close_tx.send(());
                    let _ = handle.task.await;
                }
                break;
            }
        }
    }

    let deletion_response = project_client
        .delete_device(&registration_response.id)
        .await?;

    log::info!("Device deleted successfully: {:#?}", deletion_response);

    Ok(())
}
