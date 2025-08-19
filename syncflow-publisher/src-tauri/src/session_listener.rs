use std::path::PathBuf;

use amqprs::{
    channel::{BasicConsumeArguments, QueueBindArguments, QueueDeclareArguments},
    connection::{Connection, OpenConnectionArguments},
    tls::TlsAdaptor,
};
use syncflow_shared::device_models::{DeviceResponse, NewSessionMessage};
use tauri::Emitter;

use crate::{errors::SyncFlowPublisherError, register::RegisterCredentials, utils::load_json};

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
    pub cancel_tx: tokio::sync::oneshot::Sender<()>,
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

        let (tx, _) = tokio::sync::broadcast::channel::<ClonableNewSessionMessage>(100);
        let tx_clone = tx.clone();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let rabbitmq_host = self.rabbitmq_host.clone();
        let rabbitmq_port = self.rabbitmq_port;
        let rabbitmq_username = self.rabbitmq_username.clone();
        let rabbitmq_password = self.rabbitmq_password.clone();
        let rabbitmq_vhost = self.rabbitmq_vhost.clone();
        let exchange_name = self.exchange_name.clone();
        let binding_key = self.binding_key.clone();

        let task = tokio::spawn(async move {
            let mut cancel_rx = cancel_rx;

            let connection = match Self::create_connection(
                &rabbitmq_host,
                rabbitmq_port,
                &rabbitmq_username,
                &rabbitmq_password,
                &rabbitmq_vhost,
            )
            .await
            {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("Failed to create RabbitMQ connection: {}", e);
                    return;
                }
            };

            let channel = match connection.open_channel(None).await {
                Ok(ch) => ch,
                Err(e) => {
                    eprintln!("Failed to open RabbitMQ channel: {}", e);
                    let _ = connection.close().await;
                    return;
                }
            };

            let queue_declare_args = QueueDeclareArguments::default()
                .exclusive(true)
                .auto_delete(true)
                .finish();

            let (queue_name, _, _) = match channel.queue_declare(queue_declare_args).await {
                Ok(Some(result)) => result,
                Ok(None) => {
                    let _ = connection.close().await;
                    return;
                }
                Err(e) => {
                    let _ = connection.close().await;
                    return;
                }
            };

            let queue_bind_args =
                QueueBindArguments::new(&queue_name, &exchange_name, &binding_key);
            if let Err(e) = channel.queue_bind(queue_bind_args).await {
                eprintln!("Failed to bind queue: {}", e);
                let _ = connection.close().await;
                return;
            }

            let consume_args =
                BasicConsumeArguments::new(&queue_name, "syncflow-publisher-consumer");
            let (_, mut rx) = match channel.basic_consume_rx(consume_args).await {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("Failed to start consuming: {}", e);
                    let _ = connection.close().await;
                    return;
                }
            };

            println!("Session listener started, waiting for messages...");

            loop {
                tokio::select! {
                    message = rx.recv() => {
                        match message {
                            Some(message) => {
                                match message.content {
                                    Some(content) => {
                                        match serde_json::from_slice::<NewSessionMessage>(&content) {
                                            Ok(new_session_message) => {
                                                println!("New session message received: {:?}", new_session_message);
                                                if let Err(_) = tx_clone.send(new_session_message.into()) {
                                                    eprintln!("Failed to send new session message to channel (no receivers)");
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("Failed to parse message: {}", e);
                                            }
                                        }
                                    }
                                    None => {
                                        eprintln!("Message has no content");
                                    }
                                }
                            }
                            None => {
                                println!("📪 No more messages, ending loop");
                                break;
                            }
                        }
                    }
                    _ = &mut cancel_rx => {
                        println!("Received cancel signal, stopping session listener");
                        break;
                    }
                }
            }

            if let Err(e) = connection.close().await {
                eprintln!("Error closing RabbitMQ connection: {}", e);
            } else {
                println!("RabbitMQ connection closed successfully");
            }
        });

        self.rmq_task_handle = Some(RMQTaskHandle {
            task,
            session_tx: tx,
            cancel_tx,
        });
        self.status = SessionListenerStatus::Active;

        Ok(())
    }

    // Helper function to create connection
    async fn create_connection(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        vhost: &str,
    ) -> Result<Connection, SyncFlowPublisherError> {
        let args = OpenConnectionArguments::new(host, port, username, password)
            .virtual_host(vhost)
            .tls_adaptor(TlsAdaptor::without_client_auth(None, host.to_string()).unwrap())
            .finish();

        let connection = Connection::open(&args).await?;
        Ok(connection)
    }

    pub async fn stop(&mut self) -> Result<(), SyncFlowPublisherError> {
        if let Some(handle) = self.rmq_task_handle.take() {
            if let Err(_) = handle.cancel_tx.send(()) {
                println!("Cancel signal receiver already dropped");
            }

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
