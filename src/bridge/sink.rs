//! 协议桥接的 EventSink trait

use crate::bridge::BridgeEvent;
use crate::error::SinkError;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// 可以接收桥接事件的事件接收器 trait
///
/// EventSink 代表任何可以接收事件的协议，
/// 如 SSE、WebSocket 或 gRPC 流。
pub trait EventSink: Send + Sync {
    /// 向此接收器发送事件
    ///
    /// 如果事件发送成功返回 Ok(())，
    /// 如果接收器已关闭或发送失败则返回错误。
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>>;

    /// 优雅地关闭接收器
    ///
    /// 调用 close 后，后续的发送可能会失败。
    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// 检查接收器是否已关闭
    fn is_closed(&self) -> bool {
        false
    }
}

/// 用于类型擦除的装箱事件接收器
pub type BoxedEventSink = Box<dyn EventSink>;

/// 用于共享所有权的 Arc 包装事件接收器
pub type SharedEventSink = Arc<dyn EventSink>;

/// 事件接收器的扩展 trait
pub trait EventSinkExt: EventSink {
    /// 在发送前通过转换函数映射事件
    fn map_input<F, E>(self, f: F) -> MappedSink<Self, F>
    where
        Self: Sized,
        F: Fn(BridgeEvent) -> Result<BridgeEvent, E> + Send + Sync,
        E: Into<SinkError>,
    {
        MappedSink { sink: self, f }
    }
}

impl<T: EventSink> EventSinkExt for T {}

/// 在发送前映射事件的接收器
pub struct MappedSink<S, F> {
    sink: S,
    f: F,
}

impl<S, F, E> EventSink for MappedSink<S, F>
where
    S: EventSink,
    F: Fn(BridgeEvent) -> Result<BridgeEvent, E> + Send + Sync,
    E: Into<SinkError>,
{
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            let transformed = (self.f)(event).map_err(|e| e.into())?;
            self.sink.send(transformed).await
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        self.sink.close()
    }

    fn is_closed(&self) -> bool {
        self.sink.is_closed()
    }
}

/// 包装 channel sender 的接收器
pub struct ChannelSink {
    tx: tokio::sync::mpsc::Sender<BridgeEvent>,
}

impl ChannelSink {
    /// 使用指定的缓冲区大小创建新的 channel 接收器
    pub fn new(buffer_size: usize) -> (Self, tokio::sync::mpsc::Receiver<BridgeEvent>) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer_size);
        (Self { tx }, rx)
    }
}

impl EventSink for ChannelSink {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.tx
                .send(event)
                .await
                .map_err(|_| SinkError::Closed)
        })
    }

    fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}
