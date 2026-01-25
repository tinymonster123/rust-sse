//! Axum 集成，用于 SSE 端点

use crate::bridge::{BridgeEvent, EventSource, SharedEventSink};
use crate::multicast::{Broadcaster, Fanout, FanoutHandle};
use crate::sinks::SseSink;
use axum::body::Body;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Axum 的 SSE 响应包装器
///
/// 包装 SSE 事件流并设置适当的 headers。
pub struct SseResponse {
    body: Body,
}

impl SseResponse {
    /// 从字符串流（已格式化为 SSE）创建 SSE 响应
    pub fn from_string_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = String> + Send + 'static,
    {
        let byte_stream = stream.map(|s| Ok::<_, Infallible>(Bytes::from(s)));
        Self {
            body: Body::from_stream(byte_stream),
        }
    }

    /// 从字节流创建 SSE 响应
    pub fn from_bytes_stream<S>(stream: S) -> Self
    where
        S: Stream<Item = Bytes> + Send + 'static,
    {
        let mapped = stream.map(|b| Ok::<_, Infallible>(b));
        Self {
            body: Body::from_stream(mapped),
        }
    }

    /// 从 EventSource 创建 SSE 响应
    ///
    /// 此方法使用 Arc 包装 source 以确保生命周期正确。
    pub fn from_source<S>(source: Arc<S>) -> Self
    where
        S: EventSource + 'static,
    {
        let stream = async_stream::stream! {
            let event_stream = source.events();
            tokio::pin!(event_stream);
            while let Some(result) = event_stream.next().await {
                match result {
                    Ok(event) => {
                        yield SseSink::format_event(&event);
                    }
                    Err(_) => {} // 跳过错误
                }
            }
        };

        Self::from_string_stream(stream)
    }

    /// 从 mpsc 接收器创建 SSE 响应
    pub fn from_receiver(rx: mpsc::Receiver<String>) -> Self {
        let stream = ReceiverStream::new(rx);
        Self::from_string_stream(stream)
    }

    /// 从 BridgeEvent 接收器创建 SSE 响应
    pub fn from_event_receiver(rx: mpsc::Receiver<BridgeEvent>) -> Self {
        let stream = ReceiverStream::new(rx).map(|event| SseSink::format_event(&event));
        Self::from_string_stream(stream)
    }
}

impl IntoResponse for SseResponse {
    fn into_response(self) -> Response {
        Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .header("X-Accel-Buffering", "no") // 禁用 nginx 缓冲
            .body(self.body)
            .unwrap()
    }
}

/// 用于广播场景的 SSE 端点状态
///
/// 可在 Axum handlers 之间共享以创建简单的发布/订阅系统。
#[derive(Clone)]
pub struct SseBroadcastState {
    broadcaster: Arc<Broadcaster>,
}

impl SseBroadcastState {
    /// 使用指定的容量创建新的广播状态
    pub fn new(capacity: usize) -> Self {
        Self {
            broadcaster: Arc::new(Broadcaster::new(capacity)),
        }
    }

    /// 获取广播器的引用以发送事件
    pub fn broadcaster(&self) -> &Broadcaster {
        &self.broadcaster
    }

    /// 向所有订阅者发送事件
    pub async fn send(&self, event: BridgeEvent) -> Result<(), crate::error::SinkError> {
        use crate::bridge::EventSink;
        self.broadcaster.send(event).await
    }

    /// 为新订阅者创建 SSE 响应
    pub fn subscribe(&self) -> SseResponse {
        let subscriber = self.broadcaster.subscribe();
        SseResponse::from_source(Arc::new(subscriber))
    }

    /// 获取活跃订阅者数量
    pub fn subscriber_count(&self) -> usize {
        self.broadcaster.subscriber_count()
    }
}

/// 用于带每订阅者背压的 fanout 场景的 SSE 端点状态
#[derive(Clone)]
pub struct SseFanoutState {
    fanout: Arc<Fanout>,
}

impl SseFanoutState {
    /// 使用指定的每订阅者缓冲区大小创建新的 fanout 状态
    pub fn new(buffer_size: usize) -> Self {
        Self {
            fanout: Arc::new(Fanout::new(buffer_size)),
        }
    }

    /// 获取 fanout 的引用以发送事件
    pub fn fanout(&self) -> &Fanout {
        &self.fanout
    }

    /// 向所有订阅者发送事件
    pub async fn send(&self, event: BridgeEvent) -> Result<(), crate::error::SinkError> {
        use crate::bridge::EventSink;
        self.fanout.send(event).await
    }

    /// 为新订阅者创建 SSE 响应
    pub async fn subscribe(&self) -> SseResponse {
        let handle = self.fanout.subscribe().await;
        SseResponse::from_fanout_handle(handle)
    }

    /// 获取活跃订阅者数量
    pub async fn subscriber_count(&self) -> usize {
        self.fanout.subscriber_count().await
    }
}

impl SseResponse {
    /// 从 FanoutHandle 创建 SSE 响应
    fn from_fanout_handle(mut handle: FanoutHandle) -> Self {
        let stream = async_stream::stream! {
            while let Some(event) = handle.recv().await {
                yield SseSink::format_event(&event);
            }
        };

        Self::from_string_stream(stream)
    }
}

/// 用于创建将事件转发到 SSE 端点的桥接的辅助结构
pub struct SseBridge {
    sink: SharedEventSink,
    rx: mpsc::Receiver<String>,
}

impl SseBridge {
    /// 使用指定的缓冲区大小创建新的 SSE 桥接
    pub fn new(buffer_size: usize) -> Self {
        let (sink, rx) = SseSink::new(buffer_size);
        Self {
            sink: Arc::new(sink),
            rx,
        }
    }

    /// 获取接收器的共享引用以用于 Bridge
    pub fn sink(&self) -> SharedEventSink {
        self.sink.clone()
    }

    /// 将此桥接转换为 SSE 响应
    pub fn into_response(self) -> SseResponse {
        SseResponse::from_receiver(self.rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::EventSink;

    #[tokio::test]
    async fn test_broadcast_state() {
        let state = SseBroadcastState::new(16);

        // 创建一个订阅者（但不消费 - 只测试状态）
        let _response = state.subscribe();

        assert_eq!(state.subscriber_count(), 1);

        // 发送一个事件
        state.send(BridgeEvent::from_string("test")).await.unwrap();
    }
}
