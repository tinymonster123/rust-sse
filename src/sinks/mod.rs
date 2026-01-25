//! 不同协议的事件接收器

mod sse;

#[cfg(feature = "ws")]
mod websocket;

#[cfg(feature = "grpc")]
mod grpc;

pub use sse::SseSink;

#[cfg(feature = "ws")]
pub use websocket::WebSocketSink;

#[cfg(feature = "grpc")]
pub use grpc::GrpcSink;
