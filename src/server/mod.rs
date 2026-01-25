//! 用于服务 SSE 端点的服务器集成

#[cfg(feature = "server-axum")]
mod axum;

#[cfg(feature = "server-axum")]
pub use self::axum::*;
