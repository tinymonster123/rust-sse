//! SSE 事件源 - 将 SseClient 包装为 EventSource

use crate::bridge::{BridgeEvent, EventMetadata, EventSource};
use crate::client::{SseClient, SseRequest};
use crate::error::SourceError;
use async_stream::stream;
use futures_core::Stream;
use futures_util::StreamExt;
use std::pin::Pin;

/// 包装 SseClient 的 SSE 事件源
///
/// 此适配器允许将 SSE 流用作桥接事件源。
pub struct SseSource {
    client: SseClient,
    request: SseRequest,
}

impl SseSource {
    /// 使用给定的客户端和请求创建新的 SSE 源
    pub fn new(client: SseClient, request: SseRequest) -> Self {
        Self { client, request }
    }

    /// 使用 GET 请求到给定 URL 创建新的 SSE 源
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            client: SseClient::new(),
            request: SseRequest::get(url),
        }
    }

    /// 获取请求配置的引用
    pub fn request(&self) -> &SseRequest {
        &self.request
    }

    /// 获取请求配置的可变引用
    pub fn request_mut(&mut self) -> &mut SseRequest {
        &mut self.request
    }
}

impl EventSource for SseSource {
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>> {
        let client = self.client.clone();
        let request = self.request.clone();

        Box::pin(stream! {
            let sse_stream = client.stream(request);
            tokio::pin!(sse_stream);

            while let Some(result) = sse_stream.next().await {
                match result {
                    Ok(sse_event) => {
                        let bridge_event = BridgeEvent {
                            id: sse_event.id,
                            event_type: sse_event.event,
                            payload: bytes::Bytes::from(sse_event.data),
                            metadata: EventMetadata::now().with_source("sse"),
                        };
                        yield Ok(bridge_event);
                    }
                    Err(e) => {
                        yield Err(SourceError::Sse(e));
                    }
                }
            }
        })
    }
}
