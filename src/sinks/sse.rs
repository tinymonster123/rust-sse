//! SSE 事件接收器 - 将事件格式化为 SSE 文本

use crate::bridge::{BridgeEvent, EventSink};
use crate::error::SinkError;
use crate::event::SseEvent;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// 将事件格式化为 SSE 文本的接收器
///
/// 此接收器将 BridgeEvents 转换为 SSE 格式并通过 channel 发送。
/// 接收端可用于写入 HTTP 响应体。
pub struct SseSink {
    tx: mpsc::Sender<String>,
    closed: AtomicBool,
}

impl SseSink {
    /// 使用指定的缓冲区大小创建新的 SSE 接收器
    ///
    /// 返回接收器和一个产生 SSE 格式字符串的接收端。
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        (
            Self {
                tx,
                closed: AtomicBool::new(false),
            },
            rx,
        )
    }

    /// 使用默认缓冲区大小（16）创建新的 SSE 接收器
    pub fn default_buffered() -> (Self, mpsc::Receiver<String>) {
        Self::new(16)
    }

    /// 将 BridgeEvent 转换为 SSE 格式字符串
    pub fn format_event(event: &BridgeEvent) -> String {
        let sse_event = SseEvent {
            id: event.id.clone(),
            event: event.event_type.clone(),
            data: String::from_utf8_lossy(&event.payload).to_string(),
            retry: None,
        };
        sse_event.to_sse_string()
    }
}

impl EventSink for SseSink {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let sse_text = Self::format_event(&event);

            self.tx
                .send(sse_text)
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

/// 直接写入字节发送器的 SSE 接收器
///
/// 这对于与使用基于字节的 body 的 axum 或其他框架集成很有用。
pub struct SseBytesSink {
    tx: mpsc::Sender<bytes::Bytes>,
    closed: AtomicBool,
}

impl SseBytesSink {
    /// 使用指定的缓冲区大小创建新的 SSE 字节接收器
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<bytes::Bytes>) {
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

impl EventSink for SseBytesSink {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let sse_text = SseSink::format_event(&event);
            let bytes = bytes::Bytes::from(sse_text);

            self.tx
                .send(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sse_sink_format() {
        let event = BridgeEvent::from_string("hello world")
            .with_id("123")
            .with_event_type("message");

        let formatted = SseSink::format_event(&event);

        assert!(formatted.contains("id: 123\n"));
        assert!(formatted.contains("event: message\n"));
        assert!(formatted.contains("data: hello world\n"));
        assert!(formatted.ends_with("\n\n"));
    }

    #[tokio::test]
    async fn test_sse_sink_send() {
        let (sink, mut rx) = SseSink::new(10);

        let event = BridgeEvent::from_string("test");
        sink.send(event).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert!(received.contains("data: test\n"));
    }
}
