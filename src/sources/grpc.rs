//! gRPC 事件源

use crate::bridge::{BridgeEvent, EventSource};
use crate::error::SourceError;
use async_stream::stream;
use futures_core::Stream;
use std::pin::Pin;
use tracing::{debug, error, info};

/// 将 gRPC 消息转换为 BridgeEvents 的辅助 trait
///
/// 为您的 proto 消息类型实现此 trait 以启用到桥接事件的简便转换。
pub trait IntoBridgeEvent {
    /// 将此消息转换为 BridgeEvent
    fn into_bridge_event(self) -> BridgeEvent;
}

/// gRPC 事件源配置
#[derive(Debug, Clone)]
pub struct GrpcSourceConfig {
    /// gRPC 服务器地址（如 "http://localhost:50051"）
    pub address: String,
    /// 要订阅的事件类型（空表示订阅所有）
    pub event_types: Vec<String>,
    /// 起始事件 ID（用于断点续传）
    pub last_event_id: Option<String>,
}

impl GrpcSourceConfig {
    /// 创建新的 gRPC 源配置
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            event_types: Vec::new(),
            last_event_id: None,
        }
    }

    /// 设置要订阅的事件类型
    pub fn event_types(mut self, types: Vec<String>) -> Self {
        self.event_types = types;
        self
    }

    /// 设置起始事件 ID
    pub fn last_event_id(mut self, id: impl Into<String>) -> Self {
        self.last_event_id = Some(id.into());
        self
    }
}

/// 从 tonic Streaming 创建的事件源
///
/// 这是使用 gRPC 作为事件源的推荐方式。
/// 您需要使用自己的 proto 定义生成 gRPC 客户端代码，
/// 然后将 streaming response 传入此结构。
///
/// # Example
///
/// ```ignore
/// use rust_sse::sources::GrpcStreamSource;
///
/// // 假设您有一个生成的 gRPC 客户端
/// let mut client = MyEventServiceClient::connect("http://localhost:50051").await?;
/// let response = client.subscribe(request).await?;
/// let stream = response.into_inner();
///
/// // 创建事件源
/// let source = GrpcStreamSource::from_streaming(stream);
/// let events = source.into_events();
/// ```
pub struct GrpcStreamSource<T>
where
    T: Send,
{
    inner: GrpcStreamInner<T>,
}

enum GrpcStreamInner<T> {
    Streaming(tonic::Streaming<T>),
    Consumed,
}

impl<T> GrpcStreamSource<T>
where
    T: Send,
{
    /// 从 tonic Streaming 创建新的 gRPC 流源
    pub fn from_streaming(stream: tonic::Streaming<T>) -> Self {
        Self {
            inner: GrpcStreamInner::Streaming(stream),
        }
    }
}

impl<T> GrpcStreamSource<T>
where
    T: IntoBridgeEvent + Send + 'static,
{
    /// 消费自身并返回事件流
    ///
    /// 注意：此方法只能调用一次，因为 tonic::Streaming 不可 Clone。
    pub fn into_events(mut self) -> impl Stream<Item = Result<BridgeEvent, SourceError>> + Send {
        stream! {
            let stream = match std::mem::replace(&mut self.inner, GrpcStreamInner::Consumed) {
                GrpcStreamInner::Streaming(s) => s,
                GrpcStreamInner::Consumed => {
                    yield Err(SourceError::Other("Stream already consumed".to_string()));
                    return;
                }
            };

            let mut stream = stream;
            info!("Starting to consume gRPC stream");

            loop {
                match stream.message().await {
                    Ok(Some(event)) => {
                        debug!("Received gRPC event");
                        yield Ok(event.into_bridge_event());
                    }
                    Ok(None) => {
                        info!("gRPC stream ended");
                        break;
                    }
                    Err(e) => {
                        error!(error = %e, "gRPC stream error");
                        yield Err(SourceError::Grpc(e));
                        break;
                    }
                }
            }
        }
    }
}

/// 通用 gRPC 事件源（占位实现）
///
/// 如果您需要一个自动连接的 gRPC 源，您需要：
/// 1. 使用 protoc 生成您的 gRPC 客户端代码
/// 2. 创建客户端连接并调用 streaming RPC
/// 3. 使用 `GrpcStreamSource::from_streaming()` 包装响应流
///
/// 或者您可以实现自己的 EventSource trait。
pub struct GrpcSource {
    config: GrpcSourceConfig,
}

impl GrpcSource {
    /// 使用给定配置创建新的 gRPC 源
    ///
    /// 注意：这是一个占位实现。要使用完整功能，
    /// 请使用 `GrpcStreamSource` 并提供您自己的 gRPC 流。
    pub fn new(config: GrpcSourceConfig) -> Self {
        Self { config }
    }

    /// 获取配置
    pub fn config(&self) -> &GrpcSourceConfig {
        &self.config
    }
}

impl EventSource for GrpcSource {
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>> {
        let address = self.config.address.clone();

        Box::pin(stream! {
            error!(address = %address, "GrpcSource requires a custom implementation");
            yield Err(SourceError::Other(
                "GrpcSource 是一个占位实现。请使用 GrpcStreamSource::from_streaming() \
                 并提供您自己生成的 gRPC 流，或者实现自定义的 EventSource。".to_string()
            ));
        })
    }
}

/// 便捷函数：从任意实现了 IntoBridgeEvent 的 tonic::Streaming 创建事件流
pub fn stream_from_grpc<T>(
    stream: tonic::Streaming<T>,
) -> impl Stream<Item = Result<BridgeEvent, SourceError>> + Send
where
    T: IntoBridgeEvent + Send + 'static,
{
    GrpcStreamSource::from_streaming(stream).into_events()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_source_config() {
        let config = GrpcSourceConfig::new("http://localhost:50051")
            .event_types(vec!["message".to_string(), "update".to_string()])
            .last_event_id("123");

        assert_eq!(config.address, "http://localhost:50051");
        assert_eq!(config.event_types, vec!["message", "update"]);
        assert_eq!(config.last_event_id, Some("123".to_string()));
    }
}
