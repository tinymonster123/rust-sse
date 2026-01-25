//! gRPC 事件接收器（未来实现的占位符）

use crate::bridge::{BridgeEvent, EventSink};
use crate::error::SinkError;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// gRPC 接收器配置
#[derive(Debug, Clone)]
pub struct GrpcSinkConfig {
    /// gRPC 服务器地址
    pub address: String,
    /// 服务名称
    pub service: String,
    /// 流式 RPC 的方法名称
    pub method: String,
}

impl GrpcSinkConfig {
    /// 创建新的 gRPC 接收器配置
    pub fn new(address: impl Into<String>, service: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            service: service.into(),
            method: method.into(),
        }
    }
}

/// 将 BridgeEvents 转换为 gRPC 消息的 trait
///
/// 为您的 proto 消息类型实现此 trait 以启用
/// 从桥接事件的简便转换。
pub trait FromBridgeEvent: Sized {
    /// 将 BridgeEvent 转换为此消息类型
    fn from_bridge_event(event: BridgeEvent) -> Result<Self, SinkError>;
}

/// 发送到 channel 的 gRPC 事件接收器
///
/// channel 接收端应连接到 gRPC 流。
pub struct GrpcSink<T>
where
    T: FromBridgeEvent + Send,
{
    tx: mpsc::Sender<T>,
    closed: AtomicBool,
}

impl<T> GrpcSink<T>
where
    T: FromBridgeEvent + Send,
{
    /// 使用指定的缓冲区大小创建新的 gRPC 接收器
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

impl<T> EventSink for GrpcSink<T>
where
    T: FromBridgeEvent + Send + Sync,
{
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let message = T::from_bridge_event(event)?;

            self.tx
                .send(message)
                .await
                .map_err(|_| SinkError::Closed)
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.tx.is_closed()
    }
}

/// 用于通用用途的简单字节 gRPC 消息
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
