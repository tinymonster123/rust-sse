//! 桥接事件类型 - 协议桥接的统一格式

use bytes::Bytes;
use std::collections::HashMap;
use std::time::SystemTime;

/// 与桥接事件关联的元数据
#[derive(Debug, Clone, Default)]
pub struct EventMetadata {
    /// 事件接收/创建的时间戳
    pub timestamp: Option<SystemTime>,
    /// 源协议（如 "sse", "websocket", "grpc"）
    pub source_protocol: Option<String>,
    /// 自定义键值对元数据
    pub custom: HashMap<String, String>,
}

impl EventMetadata {
    /// 使用当前时间戳创建新的元数据
    pub fn now() -> Self {
        Self {
            timestamp: Some(SystemTime::now()),
            ..Default::default()
        }
    }

    /// 设置源协议
    pub fn with_source(mut self, protocol: impl Into<String>) -> Self {
        self.source_protocol = Some(protocol.into());
        self
    }

    /// 添加自定义键值对
    pub fn with_custom(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom.insert(key.into(), value.into());
        self
    }
}

/// 协议桥接的统一事件格式
///
/// 这是桥接器内部用于在不同协议（SSE、WebSocket、gRPC）之间
/// 传输事件的通用格式。
#[derive(Debug, Clone)]
pub struct BridgeEvent {
    /// 可选的事件 ID（映射到 SSE 的 id 字段）
    pub id: Option<String>,
    /// 可选的事件类型（映射到 SSE 的 event 字段）
    pub event_type: Option<String>,
    /// 事件载荷（原始字节）
    pub payload: Bytes,
    /// 事件元数据
    pub metadata: EventMetadata,
}

impl BridgeEvent {
    /// 使用给定的载荷创建新的桥接事件
    pub fn new(payload: impl Into<Bytes>) -> Self {
        Self {
            id: None,
            event_type: None,
            payload: payload.into(),
            metadata: EventMetadata::now(),
        }
    }

    /// 从字符串载荷创建桥接事件
    pub fn from_string(s: impl Into<String>) -> Self {
        Self::new(Bytes::from(s.into()))
    }

    /// 设置事件 ID
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// 设置事件类型
    pub fn with_event_type(mut self, event_type: impl Into<String>) -> Self {
        self.event_type = Some(event_type.into());
        self
    }

    /// 设置元数据
    pub fn with_metadata(mut self, metadata: EventMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// 获取载荷的 UTF-8 字符串（有损转换）
    pub fn payload_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.payload)
    }
}

impl From<crate::event::SseEvent> for BridgeEvent {
    fn from(sse: crate::event::SseEvent) -> Self {
        Self {
            id: sse.id,
            event_type: sse.event,
            payload: Bytes::from(sse.data),
            metadata: EventMetadata::now().with_source("sse"),
        }
    }
}

impl From<BridgeEvent> for crate::event::SseEvent {
    fn from(bridge: BridgeEvent) -> Self {
        Self {
            id: bridge.id,
            event: bridge.event_type,
            data: String::from_utf8_lossy(&bridge.payload).to_string(),
            retry: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_event_creation() {
        let event = BridgeEvent::from_string("hello")
            .with_id("123")
            .with_event_type("message");

        assert_eq!(event.id, Some("123".to_string()));
        assert_eq!(event.event_type, Some("message".to_string()));
        assert_eq!(event.payload_str(), "hello");
    }

    #[test]
    fn test_sse_to_bridge_conversion() {
        let sse = crate::event::SseEvent::new("test data")
            .with_id("1")
            .with_event("update");

        let bridge: BridgeEvent = sse.into();
        assert_eq!(bridge.id, Some("1".to_string()));
        assert_eq!(bridge.event_type, Some("update".to_string()));
        assert_eq!(bridge.payload_str(), "test data");
    }
}
