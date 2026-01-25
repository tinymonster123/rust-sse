//! rust-sse 统一错误类型

use thiserror::Error;

/// SSE 客户端操作中可能发生的错误
#[derive(Debug, Error)]
pub enum SseError {
    #[cfg(feature = "client")]
    #[error("HTTP 错误: {0}")]
    Http(#[from] reqwest::Error),

    #[error("无效的 content-type: 期望 text/event-stream, 收到 {0:?}")]
    InvalidContentType(Option<String>),

    #[error("无效的 header 值")]
    InvalidHeaderValue,

    #[error("URL 解析错误: {0}")]
    Url(String),
}

/// 事件源的错误
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("连接已关闭")]
    Closed,

    #[error("连接错误: {0}")]
    Connection(String),

    #[error("解析错误: {0}")]
    Parse(String),

    #[error("SSE 错误: {0}")]
    Sse(#[from] SseError),

    #[cfg(feature = "ws")]
    #[error("WebSocket 错误: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[cfg(feature = "grpc")]
    #[error("gRPC 错误: {0}")]
    Grpc(#[from] tonic::Status),

    #[error("其他错误: {0}")]
    Other(String),
}

/// 事件接收器的错误
#[derive(Debug, Error)]
pub enum SinkError {
    #[error("接收器已关闭")]
    Closed,

    #[error("发送错误: {0}")]
    Send(String),

    #[error("序列化错误: {0}")]
    Serialization(String),

    #[cfg(feature = "ws")]
    #[error("WebSocket 错误: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[cfg(feature = "grpc")]
    #[error("gRPC 错误: {0}")]
    Grpc(#[from] tonic::Status),

    #[error("其他错误: {0}")]
    Other(String),
}

/// 桥接操作的错误
#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("源错误: {0}")]
    Source(#[from] SourceError),

    #[error("接收器错误: {0}")]
    Sink(#[from] SinkError),

    #[error("没有注册接收器")]
    NoSinks,

    #[error("桥接已停止")]
    Stopped,
}
