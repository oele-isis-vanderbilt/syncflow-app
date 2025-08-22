use std::path::PathBuf;

use amqprs::{
    channel::{BasicConsumeArguments, QueueBindArguments, QueueDeclareArguments},
    connection::{Connection, OpenConnectionArguments},
    tls::TlsAdaptor,
};
use syncflow_shared::device_models::{DeviceResponse, NewSessionMessage};
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::{errors::SyncFlowPublisherError, register::RegisterCredentials, utils::load_json};
use rand::distr::Alphanumeric;
use rand::Rng;

fn random_string(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

#[derive(Debug)]
enum ConsumerExit {
    CancelledByUser,
    StreamEnded,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClonableNewSessionMessage {
    pub session_id: String,
    pub session_name: String,
}

impl From<NewSessionMessage> for ClonableNewSessionMessage {
    fn from(msg: NewSessionMessage) -> Self {
        Self {
            session_id: msg.session_id,
            session_name: msg.session_name,
        }
    }
}

impl From<ClonableNewSessionMessage> for NewSessionMessage {
    fn from(msg: ClonableNewSessionMessage) -> Self {
        Self {
            session_id: msg.session_id,
            session_name: msg.session_name,
        }
    }
}

pub struct RMQTaskHandle {
    pub task: tokio::task::JoinHandle<()>,
    pub session_tx: tokio::sync::broadcast::Sender<ClonableNewSessionMessage>,
    pub cancel: CancellationToken,
}

#[derive(Debug)]
pub enum SessionListenerStatus {
    Idle,
    Active,
}

pub async fn initialize_session_listener(
    app_dir: &PathBuf,
    app_handle: tauri::AppHandle,
) -> Option<SessionListener> {
    let credentials = load_json::<RegisterCredentials>(&app_dir.join("credentials.json")).ok()?;

    let device_registration =
        load_json::<DeviceResponse>(&app_dir.join("registration.json")).ok()?;

    let mut listener = SessionListener::new(
        &credentials.rabbitmq_host,
        credentials.rabbitmq_port,
        &credentials.rabbitmq_username,
        &credentials.rabbitmq_password,
        &credentials.rabbitmq_vhost,
        &device_registration
            .session_notification_exchange_name
            .clone()
            .unwrap(),
        &device_registration
            .session_notification_binding_key
            .clone()
            .unwrap(),
    );
    let _ = listener.start().await;
    let _ = listener.start_frontend_notifications(app_handle).await;
    Some(listener)
}

pub(crate) struct SessionListener {
    pub rabbitmq_host: String,
    pub rabbitmq_port: u16,
    pub rabbitmq_username: String,
    pub rabbitmq_password: String,
    pub rabbitmq_vhost: String,
    pub exchange_name: String,
    pub binding_key: String,
    pub status: SessionListenerStatus,
    pub rmq_task_handle: Option<RMQTaskHandle>,
}

impl SessionListener {
    pub fn new(
        rabbitmq_host: &str,
        rabbitmq_port: u16,
        rabbitmq_username: &str,
        rabbitmq_password: &str,
        rabbitmq_vhost: &str,
        exchange_name: &str,
        binding_key: &str,
    ) -> Self {
        SessionListener {
            rabbitmq_host: rabbitmq_host.to_string(),
            rabbitmq_port,
            rabbitmq_username: rabbitmq_username.to_string(),
            rabbitmq_password: rabbitmq_password.to_string(),
            exchange_name: exchange_name.to_string(),
            rabbitmq_vhost: rabbitmq_vhost.to_string(),
            binding_key: binding_key.to_string(),
            status: SessionListenerStatus::Idle,
            rmq_task_handle: None,
        }
    }

    pub async fn start(&mut self) -> Result<(), SyncFlowPublisherError> {
        if matches!(self.status, SessionListenerStatus::Active) {
            return Err(SyncFlowPublisherError::NotIntialized(
                "SessionListener is already active".into(),
            ));
        }
        let cancel = CancellationToken::new();
        let cancel_child = cancel.child_token();
        let (tx, _) = tokio::sync::broadcast::channel::<ClonableNewSessionMessage>(100);
        let tx_clone = tx.clone();

        let rabbitmq_host = self.rabbitmq_host.clone();
        let rabbitmq_port = self.rabbitmq_port;
        let rabbitmq_username = self.rabbitmq_username.clone();
        let rabbitmq_password = self.rabbitmq_password.clone();
        let rabbitmq_vhost = self.rabbitmq_vhost.clone();
        let exchange_name = self.exchange_name.clone();
        let binding_key = self.binding_key.clone();

        let task = tokio::spawn(async move {
            Self::run_forever(
                rabbitmq_host,
                rabbitmq_port,
                rabbitmq_username,
                rabbitmq_password,
                rabbitmq_vhost,
                exchange_name,
                binding_key,
                tx_clone,
                cancel_child,
            )
            .await;
        });

        self.rmq_task_handle = Some(RMQTaskHandle {
            task,
            session_tx: tx,
            cancel: cancel,
        });
        self.status = SessionListenerStatus::Active;

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), SyncFlowPublisherError> {
        if let Some(handle) = self.rmq_task_handle.take() {
            handle.cancel.cancel();

            match tokio::time::timeout(std::time::Duration::from_secs(5), handle.task).await {
                Ok(Ok(_)) => println!("SessionListener task stopped cleanly"),
                Ok(Err(e)) => eprintln!("SessionListener task panicked: {}", e),
                Err(_) => {
                    println!(
                        "SessionListener task did not stop within timeout, it will be aborted"
                    );
                }
            }
        }

        self.status = SessionListenerStatus::Idle;
        println!("SessionListener is now idle");
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, SessionListenerStatus::Active)
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.status, SessionListenerStatus::Idle)
    }

    pub fn subscribe(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<ClonableNewSessionMessage>, SyncFlowPublisherError>
    {
        if let Some(handle) = &self.rmq_task_handle {
            Ok(handle.session_tx.subscribe())
        } else {
            Err(SyncFlowPublisherError::NotIntialized(
                "SessionListener is not initialized".into(),
            ))
        }
    }

    pub async fn start_frontend_notifications(
        &self,
        app_handle: tauri::AppHandle,
    ) -> Result<(), SyncFlowPublisherError> {
        let mut receiver = self.subscribe()?;

        tokio::spawn(async move {
            println!("Frontend notification task started");

            loop {
                match receiver.recv().await {
                    Ok(message) => {
                        println!("Emitting session to frontend: {:?}", message);
                        if let Err(e) = app_handle.emit("new-session", message) {
                            eprintln!("Failed to emit new session event: {}", e);
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        println!(
                            "Session listener channel closed, stopping frontend notifications"
                        );
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        eprintln!("Frontend notifications lagged, missed {} messages", missed);
                        continue;
                    }
                }
            }

            println!("Frontend notification task ended");
        });

        Ok(())
    }

    pub fn receiver_count(&self) -> usize {
        if let Some(handle) = &self.rmq_task_handle {
            handle.session_tx.receiver_count()
        } else {
            0
        }
    }

    pub fn is_task_running(&self) -> bool {
        if let Some(handle) = &self.rmq_task_handle {
            !handle.task.is_finished()
        } else {
            false
        }
    }
}

impl SessionListener {
    async fn create_connection_with_heartbeat(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        vhost: &str,
        heartbeat_secs: u16,
    ) -> Result<Connection, SyncFlowPublisherError> {
        let tls = TlsAdaptor::without_client_auth(None, host.to_string())?;
        let args = OpenConnectionArguments::new(host, port, username, password)
            .virtual_host(vhost)
            .tls_adaptor(tls)
            .heartbeat(heartbeat_secs)
            .finish();

        Ok(Connection::open(&args).await?)
    }
}

impl SessionListener {
    async fn run_forever(
        rabbitmq_host: String,
        rabbitmq_port: u16,
        rabbitmq_username: String,
        rabbitmq_password: String,
        rabbitmq_vhost: String,
        exchange_name: String,
        binding_key: String,
        tx_clone: tokio::sync::broadcast::Sender<ClonableNewSessionMessage>,
        cancel: CancellationToken,
    ) {
        let mut backoff = std::time::Duration::from_millis(500);

        loop {
            if cancel.is_cancelled() {
                break;
            }

            match Self::run_once(
                &rabbitmq_host,
                rabbitmq_port,
                &rabbitmq_username,
                &rabbitmq_password,
                &rabbitmq_vhost,
                &exchange_name,
                &binding_key,
                tx_clone.clone(),
                cancel.clone(),
            )
            .await
            {
                Ok(ConsumerExit::CancelledByUser) => {
                    println!("Consumer stopped by cancel signal.");
                    break;
                }
                Ok(ConsumerExit::StreamEnded) => {
                    println!(
                        "Consumer stream ended (server cancel / channel closed). Reconnecting…"
                    );
                }
                Err(e) => {
                    eprintln!("Consumer error: {e}. Reconnecting…");
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = std::cmp::min(backoff * 2, std::time::Duration::from_secs(30));
        }
    }

    async fn run_once(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        vhost: &str,
        exchange_name: &str,
        binding_key: &str,
        tx_clone: tokio::sync::broadcast::Sender<ClonableNewSessionMessage>,
        cancel_rx: CancellationToken,
    ) -> Result<ConsumerExit, SyncFlowPublisherError> {
        let connection =
            Self::create_connection_with_heartbeat(host, port, username, password, vhost, 30)
                .await?;

        let channel = connection.open_channel(None).await?;

        let queue_declare_args = QueueDeclareArguments::default()
            .exclusive(true)
            .auto_delete(true)
            .finish();

        let (queue_name, _, _) = channel
            .queue_declare(queue_declare_args)
            .await?
            .ok_or_else(|| {
                SyncFlowPublisherError::NotIntialized("queue_declare returned None".into())
            })?;

        channel
            .queue_bind(QueueBindArguments::new(
                &queue_name,
                exchange_name,
                binding_key,
            ))
            .await?;

        let consume_args = BasicConsumeArguments::new(
            &queue_name,
            &format!("session_listener_{}", random_string(8)),
        )
        .manual_ack(false)
        .finish();

        let (_ctag, mut rx) = channel.basic_consume_rx(consume_args).await?;

        println!("Session listener started; waiting for messages on queue '{queue_name}' …");

        loop {
            tokio::select! {
                biased;

                _ = cancel_rx.cancelled() => {
                    println!("Cancel signal received; closing channel/connection.");
                    let _ = channel.close().await;
                    let _ = connection.close().await;
                    return Ok(ConsumerExit::CancelledByUser);
                }

                maybe_msg = rx.recv() => {
                    match maybe_msg {
                        Some(delivery) => {
                            if let Some(body) = delivery.content {
                                match serde_json::from_slice::<NewSessionMessage>(&body) {
                                    Ok(ns) => {
                                        let _ = tx_clone.send(ns.into());
                                    }
                                    Err(e) => eprintln!("Failed to parse message: {e}"),
                                }
                            } else {
                                eprintln!("Received a delivery with no content.");
                            }
                        }
                        None => {
                            let _ = channel.close().await;
                            let _ = connection.close().await;
                            return Ok(ConsumerExit::StreamEnded);
                        }
                    }
                }
            }
        }
    }
}
