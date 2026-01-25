//! gRPC 事件接收器

use crate::bridge::{BridgeEvent, EventSink};
use crate::error::SinkError;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, info};

/// 将 BridgeEvents 转换为 gRPC 消息的 trait
///
/// 为您的 proto 消息类型实现此 trait 以启用从桥接事件的简便转换。
pub trait FromBridgeEvent: Sized {
    /// 将 BridgeEvent 转换为此消息类型
    fn from_bridge_event(event: BridgeEvent) -> Result<Self, SinkError>;
}

/// gRPC 接收器配置
#[derive(Debug, Clone)]
pub struct GrpcSinkConfig {
    /// gRPC 服务器地址
    pub address: String,
    /// 缓冲区大小
    pub buffer_size: usize,
}

impl GrpcSinkConfig {
    /// 创建新的 gRPC 接收器配置
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            buffer_size: 16,
        }
    }

    /// 设置缓冲区大小
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }
}

/// 发送到 channel 的 gRPC 事件接收器
///
/// 这是一个基于 channel 的接收器，可以与自定义的 gRPC 流集成。
/// 您需要从接收端获取消息并通过您自己的 gRPC 客户端发送。
///
/// # Example
///
/// ```ignore
/// use rust_sse::sinks::{GrpcChannelSink, FromBridgeEvent};
///
/// // 定义您的 proto 消息类型并实现 FromBridgeEvent
/// impl FromBridgeEvent for MyProtoEvent {
///     fn from_bridge_event(event: BridgeEvent) -> Result<Self, SinkError> {
///         Ok(MyProtoEvent {
///             id: event.id,
///             data: event.payload.to_vec(),
///         })
///     }
/// }
///
/// // 创建 channel sink
/// let (sink, mut rx) = GrpcChannelSink::<MyProtoEvent>::new(16);
///
/// // 在另一个任务中消费消息并发送到 gRPC
/// tokio::spawn(async move {
///     while let Some(event) = rx.recv().await {
///         client.send(event).await?;
///     }
/// });
///
/// // 使用 sink
/// sink.send(bridge_event).await?;
/// ```
pub struct GrpcChannelSink<T>
where
    T: FromBridgeEvent + Send,
{
    tx: mpsc::Sender<T>,
    closed: AtomicBool,
}

impl<T> GrpcChannelSink<T>
where
    T: FromBridgeEvent + Send,
{
    /// 使用指定的缓冲区大小创建新的 gRPC channel 接收器
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<T>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        (
            Self {
                tx,
                closed: AtomicBool::new(false),
            },
            rx,
        )
    }
}

impl<T> EventSink for GrpcChannelSink<T>
where
    T: FromBridgeEvent + Send + Sync,
{
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let message = T::from_bridge_event(event)?;
            debug!("Sending event to gRPC channel");

            self.tx
                .send(message)
                .await
                .map_err(|_| SinkError::Closed)
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            info!("gRPC channel sink closed");
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.tx.is_closed()
    }
}

/// 简单的 gRPC 接收器（占位实现）
///
/// 如果您需要一个自动连接的 gRPC 接收器，您需要：
/// 1. 使用 protoc 生成您的 gRPC 客户端代码
/// 2. 创建客户端连接
/// 3. 使用 `GrpcChannelSink` 并自行处理消息发送
///
/// 或者您可以实现自己的 EventSink trait。
pub struct GrpcSink {
    config: GrpcSinkConfig,
    closed: AtomicBool,
}

impl GrpcSink {
    /// 创建新的 gRPC 接收器
    ///
    /// 注意：这是一个占位实现。要使用完整功能，
    /// 请使用 `GrpcChannelSink` 并自行管理 gRPC 连接。
    pub fn new(config: GrpcSinkConfig) -> Self {
        Self {
            config,
            closed: AtomicBool::new(false),
        }
    }

    /// 获取配置
    pub fn config(&self) -> &GrpcSinkConfig {
        &self.config
    }
}

impl EventSink for GrpcSink {
    fn send(&self, _event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            Err(SinkError::Send(
                "GrpcSink 是一个占位实现。请使用 GrpcChannelSink \
                 并自行管理 gRPC 连接，或者实现自定义的 EventSink。".to_string()
            ))
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// 用于通用用途的简单字节消息
#[derive(Debug, Clone)]
pub struct BytesMessage {
    /// 事件 ID
    pub id: Option<String>,
    /// 事件类型
    pub event_type: Option<String>,
    /// 载荷字节
    pub payload: bytes::Bytes,
}

impl FromBridgeEvent for BytesMessage {
    fn from_bridge_event(event: BridgeEvent) -> Result<Self, SinkError> {
        Ok(Self {
            id: event.id,
            event_type: event.event_type,
            payload: event.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_sink_config() {
        let config = GrpcSinkConfig::new("http://localhost:50051")
            .buffer_size(32);

        assert_eq!(config.address, "http://localhost:50051");
        assert_eq!(config.buffer_size, 32);
    }

    #[tokio::test]
    async fn test_grpc_channel_sink() {
        let (sink, mut rx) = GrpcChannelSink::<BytesMessage>::new(16);

        let event = BridgeEvent::from_string("test message");
        sink.send(event).await.unwrap();

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.payload.as_ref(), b"test message");
    }
}
