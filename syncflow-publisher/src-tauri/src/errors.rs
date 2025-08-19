#[derive(Debug, thiserror::Error)]
pub enum SyncFlowPublisherError {
    #[error("{0}")]
    ProjectClientError(#[from] syncflow_client::ProjectClientError),

    #[error("IoError: {0}")]
    IoError(#[from] std::io::Error),

    #[error("{0}")]
    JsonError(#[from] serde_json::Error),

    #[error("ConfigError: {0}")]
    ConfigError(String),

    #[error("Failed to read file: {0}")]
    NotIntialized(String),

    #[error("GStreamer error: {0}")]
    GStreamerError(#[from] livekit_gstreamer::GStreamerError),

    #[error("Amqp error: {0}")]
    AmqpError(#[from] amqprs::error::Error),

    #[error("Failed to initialize: {0}")]
    InitializationError(String),
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    Io(String),
    Json(String),
    ProjectClient(String),
    Config(String),
    GStreamer(String),
    Amqp(String),
    Initialize(String),
}

impl serde::Serialize for SyncFlowPublisherError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let error_message = self.to_string();
        let error_kind = match self {
            Self::IoError(_) => ErrorKind::Io(error_message),
            Self::JsonError(_) => ErrorKind::Json(error_message),
            Self::ProjectClientError(_) => ErrorKind::ProjectClient(error_message),
            Self::NotIntialized(_) => ErrorKind::Io(error_message),
            Self::ConfigError(_) => ErrorKind::Config(error_message),
            Self::GStreamerError(_) => ErrorKind::GStreamer(error_message),
            Self::AmqpError(_) => ErrorKind::Amqp(error_message),
            Self::InitializationError(_) => ErrorKind::Initialize(error_message),
        };
        error_kind.serialize(serializer)
    }
}
