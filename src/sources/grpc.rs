//! gRPC 事件源（未来实现的占位符）

use crate::bridge::{BridgeEvent, EventMetadata, EventSource};
use crate::error::SourceError;
use async_stream::stream;
use futures_core::Stream;
use std::pin::Pin;

/// gRPC 事件源配置
#[derive(Debug, Clone)]
pub struct GrpcSourceConfig {
    /// gRPC 服务器地址
    pub address: String,
    /// 服务名称
    pub service: String,
    /// 流式 RPC 的方法名称
    pub method: String,
}

impl GrpcSourceConfig {
    /// 创建新的 gRPC 源配置
    pub fn new(address: impl Into<String>, service: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            service: service.into(),
            method: method.into(),
        }
    }
}

/// gRPC 流式事件源
///
/// 连接到 gRPC 服务器并从流式 RPC 产生事件。
///
/// 注意：这是一个占位实现。完整实现需要
/// 针对您的用例生成的 proto 客户端。
pub struct GrpcSource {
    config: GrpcSourceConfig,
}

impl GrpcSource {
    /// 使用给定配置创建新的 gRPC 源
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
        let _config = self.config.clone();

        Box::pin(stream! {
            // 占位符：在实际实现中，您需要：
            // 1. 连接到 gRPC 服务器
            // 2. 调用流式 RPC 方法
            // 3. 将 gRPC 消息转换为 BridgeEvents

            yield Err(SourceError::Other(
                "gRPC 源需要使用您的 proto 定义进行自定义实现".to_string()
            ));
        })
    }
}

/// 将 gRPC 消息转换为 BridgeEvents 的辅助 trait
///
/// 为您的 proto 消息类型实现此 trait 以启用
/// 到桥接事件的简便转换。
pub trait IntoBridgeEvent {
    /// 将此消息转换为 BridgeEvent
    fn into_bridge_event(self) -> BridgeEvent;
}

/// 适用于任何流式响应类型的通用 gRPC 源
pub struct GenericGrpcSource<S>
where
    S: futures_core::Stream + Send + Unpin,
{
    stream: S,
}

impl<S, T> GenericGrpcSource<S>
where
    S: futures_core::Stream<Item = Result<T, tonic::Status>> + Send + Unpin,
    T: IntoBridgeEvent,
{
    /// 从 tonic 流式响应创建新的通用 gRPC 源
    pub fn from_stream(stream: S) -> Self {
        Self { stream }
    }
}

impl<S, T> EventSource for GenericGrpcSource<S>
where
    S: futures_core::Stream<Item = Result<T, tonic::Status>> + Send + Sync + Unpin + Clone,
    T: IntoBridgeEvent + Send,
{
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>> {
        use futures_util::StreamExt;

        let stream = self.stream.clone();
        Box::pin(stream.map(|result| {
            result
                .map(|msg| msg.into_bridge_event())
                .map_err(SourceError::Grpc)
        }))
    }
}
