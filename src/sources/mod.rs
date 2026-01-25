//! 不同协议的事件源

#[cfg(feature = "client")]
mod sse;

#[cfg(feature = "ws")]
mod websocket;

#[cfg(feature = "grpc")]
mod grpc;

#[cfg(feature = "client")]
pub use sse::SseSource;

#[cfg(feature = "ws")]
pub use websocket::WebSocketSource;

#[cfg(feature = "grpc")]
pub use grpc::GrpcSource;
