//! WebSocket 事件源

use crate::bridge::{BridgeEvent, EventMetadata, EventSource};
use crate::error::SourceError;
use async_stream::stream;
use futures_core::Stream;
use futures_util::StreamExt;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;

/// WebSocket 事件源配置
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    /// 连接 URL（ws:// 或 wss://）
    pub url: String,
    /// 是否在断开时自动重连
    pub auto_reconnect: bool,
    /// 重连延迟（毫秒）
    pub reconnect_delay_ms: u64,
    /// 最大重连次数（None 表示无限）
    pub max_reconnects: Option<usize>,
}

impl WebSocketConfig {
    /// 使用给定 URL 创建新的 WebSocket 配置
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            auto_reconnect: true,
            reconnect_delay_ms: 1000,
            max_reconnects: None,
        }
    }

    /// 禁用自动重连
    pub fn no_reconnect(mut self) -> Self {
        self.auto_reconnect = false;
        self
    }
}

/// WebSocket 事件源
///
/// 连接到 WebSocket 服务器并从接收到的消息产生事件。
pub struct WebSocketSource {
    config: WebSocketConfig,
    /// 跟踪连接状态的共享状态
    connected: Arc<RwLock<bool>>,
}

impl WebSocketSource {
    /// 使用给定 URL 创建新的 WebSocket 源
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            config: WebSocketConfig::new(url),
            connected: Arc::new(RwLock::new(false)),
        }
    }

    /// 使用自定义配置创建新的 WebSocket 源
    pub fn with_config(config: WebSocketConfig) -> Self {
        Self {
            config,
            connected: Arc::new(RwLock::new(false)),
        }
    }

    /// 检查当前是否已连接
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }
}

impl EventSource for WebSocketSource {
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>> {
        let config = self.config.clone();
        let connected = self.connected.clone();

        Box::pin(stream! {
            let mut reconnect_count = 0usize;

            loop {
                // 尝试连接
                let connect_result = tokio_tungstenite::connect_async(&config.url).await;

                match connect_result {
                    Ok((ws_stream, _response)) => {
                        *connected.write().await = true;
                        reconnect_count = 0;

                        let (_write, mut read) = ws_stream.split();

                        while let Some(msg_result) = read.next().await {
                            match msg_result {
                                Ok(msg) => {
                                    let bridge_event = match msg {
                                        Message::Text(text) => {
                                            Some(BridgeEvent {
                                                id: None,
                                                event_type: Some("message".to_string()),
                                                payload: bytes::Bytes::from(text),
                                                metadata: EventMetadata::now().with_source("websocket"),
                                            })
                                        }
                                        Message::Binary(data) => {
                                            Some(BridgeEvent {
                                                id: None,
                                                event_type: Some("binary".to_string()),
                                                payload: bytes::Bytes::from(data),
                                                metadata: EventMetadata::now().with_source("websocket"),
                                            })
                                        }
                                        Message::Ping(_) | Message::Pong(_) => None,
                                        Message::Close(_) => {
                                            *connected.write().await = false;
                                            break;
                                        }
                                        Message::Frame(_) => None,
                                    };

                                    if let Some(event) = bridge_event {
                                        yield Ok(event);
                                    }
                                }
                                Err(e) => {
                                    *connected.write().await = false;
                                    yield Err(SourceError::WebSocket(e));
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        *connected.write().await = false;
                        yield Err(SourceError::WebSocket(e));
                    }
                }

                // 处理重连
                if !config.auto_reconnect {
                    break;
                }

                reconnect_count += 1;
                if let Some(max) = config.max_reconnects {
                    if reconnect_count > max {
                        break;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(config.reconnect_delay_ms)).await;
            }
        })
    }
}
