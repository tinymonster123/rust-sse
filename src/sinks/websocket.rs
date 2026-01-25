//! WebSocket 事件接收器

use crate::bridge::{BridgeEvent, EventSink};
use crate::error::SinkError;
use futures_sink::Sink;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::warn;

/// WebSocket 消息格式
#[derive(Debug, Clone, Copy, Default)]
pub enum WebSocketFormat {
    /// 作为文本消息发送（默认）
    #[default]
    Text,
    /// 作为二进制消息发送
    Binary,
    /// 作为 JSON 序列化文本发送
    Json,
}

/// WebSocket 接收器配置
#[derive(Debug, Clone)]
pub struct WebSocketSinkConfig {
    /// 消息格式
    pub format: WebSocketFormat,
    /// 是否在 JSON 格式中包含事件元数据
    pub include_metadata: bool,
}

impl Default for WebSocketSinkConfig {
    fn default() -> Self {
        Self {
            format: WebSocketFormat::Text,
            include_metadata: false,
        }
    }
}

/// 发送到 channel 的 WebSocket 事件接收器
///
/// channel 接收端应连接到 WebSocket 写入器。
pub struct WebSocketSink {
    tx: mpsc::Sender<Message>,
    config: WebSocketSinkConfig,
    closed: AtomicBool,
}

impl WebSocketSink {
    /// 使用指定的缓冲区大小创建新的 WebSocket 接收器
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<Message>) {
        Self::with_config(buffer_size, WebSocketSinkConfig::default())
    }

    /// 使用自定义配置创建新的 WebSocket 接收器
    pub fn with_config(buffer_size: usize, config: WebSocketSinkConfig) -> (Self, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        (
            Self {
                tx,
                config,
                closed: AtomicBool::new(false),
            },
            rx,
        )
    }

    fn format_message(&self, event: &BridgeEvent) -> Message {
        match self.config.format {
            WebSocketFormat::Text => {
                Message::Text(String::from_utf8_lossy(&event.payload).to_string().into())
            }
            WebSocketFormat::Binary => Message::Binary(event.payload.to_vec().into()),
            WebSocketFormat::Json => {
                // 简单 JSON 格式
                let json = if self.config.include_metadata {
                    format!(
                        r#"{{"id":{},"type":{},"data":{},"source":{}}}"#,
                        event.id.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or_else(|| "null".to_string()),
                        event.event_type.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or_else(|| "null".to_string()),
                        serde_json_escape(&String::from_utf8_lossy(&event.payload)),
                        event.metadata.source_protocol.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or_else(|| "null".to_string()),
                    )
                } else {
                    format!(
                        r#"{{"id":{},"type":{},"data":{}}}"#,
                        event.id.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or_else(|| "null".to_string()),
                        event.event_type.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or_else(|| "null".to_string()),
                        serde_json_escape(&String::from_utf8_lossy(&event.payload)),
                    )
                };
                Message::Text(json.into())
            }
        }
    }
}

/// 简单的 JSON 字符串转义（不引入 serde_json）
fn serde_json_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

impl EventSink for WebSocketSink {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let message = self.format_message(&event);

            self.tx
                .send(message)
                .await
                .map_err(|_| SinkError::Closed)
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            // 发送关闭帧
            if let Err(e) = self.tx.send(Message::Close(None)).await {
                warn!(error = %e, "Failed to send WebSocket close frame");
            }
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.tx.is_closed()
    }
}

/// 直接写入 WebSocket 流的接收器
pub struct DirectWebSocketSink<S>
where
    S: Sink<Message> + Send + Unpin,
{
    stream: Arc<Mutex<S>>,
    config: WebSocketSinkConfig,
    closed: AtomicBool,
}

impl<S> DirectWebSocketSink<S>
where
    S: Sink<Message> + Send + Unpin,
{
    /// 包装给定流创建新的直接 WebSocket 接收器
    pub fn new(stream: S) -> Self {
        Self {
            stream: Arc::new(Mutex::new(stream)),
            config: WebSocketSinkConfig::default(),
            closed: AtomicBool::new(false),
        }
    }

    /// 使用自定义配置创建
    pub fn with_config(stream: S, config: WebSocketSinkConfig) -> Self {
        Self {
            stream: Arc::new(Mutex::new(stream)),
            config,
            closed: AtomicBool::new(false),
        }
    }

    fn format_message(&self, event: &BridgeEvent) -> Message {
        match self.config.format {
            WebSocketFormat::Text => {
                Message::Text(String::from_utf8_lossy(&event.payload).to_string().into())
            }
            WebSocketFormat::Binary => Message::Binary(event.payload.to_vec().into()),
            WebSocketFormat::Json => {
                let json = format!(
                    r#"{{"id":{},"type":{},"data":{}}}"#,
                    event.id.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or_else(|| "null".to_string()),
                    event.event_type.as_ref().map(|s| format!("\"{}\"", s)).unwrap_or_else(|| "null".to_string()),
                    serde_json_escape(&String::from_utf8_lossy(&event.payload)),
                );
                Message::Text(json.into())
            }
        }
    }
}

impl<S> EventSink for DirectWebSocketSink<S>
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Send + Sync + Unpin,
{
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            use futures_util::SinkExt;

            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let message = self.format_message(&event);
            let mut stream = self.stream.lock().await;

            stream
                .send(message)
                .await
                .map_err(SinkError::WebSocket)
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            use futures_util::SinkExt;

            self.closed.store(true, Ordering::SeqCst);

            let mut stream = self.stream.lock().await;
            stream
                .send(Message::Close(None))
                .await
                .map_err(SinkError::WebSocket)?;
            stream.close().await.map_err(SinkError::WebSocket)
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_websocket_sink_text() {
        let (sink, mut rx) = WebSocketSink::new(10);

        let event = BridgeEvent::from_string("hello");
        sink.send(event).await.unwrap();

        let msg = rx.recv().await.unwrap();
        match msg {
            Message::Text(text) => assert_eq!(text.as_str(), "hello"),
            _ => panic!("期望文本消息"),
        }
    }

    #[tokio::test]
    async fn test_websocket_sink_json() {
        let config = WebSocketSinkConfig {
            format: WebSocketFormat::Json,
            include_metadata: false,
        };
        let (sink, mut rx) = WebSocketSink::with_config(10, config);

        let event = BridgeEvent::from_string("test")
            .with_id("1")
            .with_event_type("message");
        sink.send(event).await.unwrap();

        let msg = rx.recv().await.unwrap();
        match msg {
            Message::Text(json) => {
                assert!(json.contains("\"id\":\"1\""));
                assert!(json.contains("\"type\":\"message\""));
                assert!(json.contains("\"data\":\"test\""));
            }
            _ => panic!("期望文本消息"),
        }
    }
}
