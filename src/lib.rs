//! rust-sse: SSE 解析器、客户端和协议桥接器
//!
//! 本库提供:
//! - SSE (Server-Sent Events) 解析器
//! - 带自动重连的 SSE 客户端
//! - SSE ↔ WebSocket ↔ gRPC 协议桥接器
//! - 支持一对多事件分发的多播/广播功能
//!
//! # 功能特性
//!
//! - `client` (默认): 使用 reqwest 的 SSE 客户端
//! - `bridge`: 核心桥接抽象 (EventSource, EventSink, BridgeEvent)
//! - `ws`: 通过 tokio-tungstenite 支持 WebSocket
//! - `grpc`: 通过 tonic 支持 gRPC
//! - `server-axum`: Axum 集成，用于 SSE 端点
//! - `full`: 所有功能
//!
//! # 快速开始
//!
//! ## SSE 客户端
//!
//! ```rust,no_run
//! use rust_sse::{SseClient, SseRequest};
//! use futures_util::StreamExt;
//!
//! # async fn example() {
//! let client = SseClient::new();
//! let request = SseRequest::get("https://example.com/events");
//!
//! let stream = client.stream(request);
//! tokio::pin!(stream);
//! while let Some(event) = stream.next().await {
//!     match event {
//!         Ok(sse) => println!("事件: {:?}", sse),
//!         Err(e) => eprintln!("错误: {:?}", e),
//!     }
//! }
//! # }
//! ```
//!
//! ## 协议桥接
//!
//! ```rust,ignore
//! use rust_sse::bridge::{Bridge, BridgeBuilder};
//! use rust_sse::sources::SseSource;
//! use rust_sse::sinks::SseSink;
//!
//! # async fn example() {
//! // 创建 SSE 到 SSE 的桥接
//! let source = SseSource::get("https://example.com/events");
//! let (sink, rx) = SseSink::new(16);
//!
//! let bridge = BridgeBuilder::new()
//!     .source(source)
//!     .sink(sink)
//!     .build()
//!     .await
//!     .unwrap();
//!
//! bridge.run().await.unwrap();
//! # }
//! ```

// 核心模块（始终可用）
mod error;
mod event;
mod parser;

// 核心类型的重导出
pub use error::{SseError, SourceError, SinkError, BridgeError};
pub use event::SseEvent;
pub use parser::SseParser;

// 客户端模块（可选）
#[cfg(feature = "client")]
mod client;

#[cfg(feature = "client")]
pub use client::{SseClient, SseRequest, SseRetry};

// 桥接模块（可选，由 bridge 特性或任何协议特性启用）
#[cfg(any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum"))]
pub mod bridge;

// 事件源模块（需要 bridge）
#[cfg(any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum"))]
pub mod sources;

// 事件接收器模块（需要 bridge）
#[cfg(any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum"))]
pub mod sinks;

// 多播模块（需要 bridge）
#[cfg(any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum"))]
pub mod multicast;

// 服务器模块（可选）
#[cfg(feature = "server-axum")]
pub mod server;

// 启用桥接时的便捷重导出
#[cfg(any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum"))]
pub use bridge::{BridgeEvent, EventSource, EventSink, Bridge, BridgeBuilder};

#[cfg(any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum"))]
pub use multicast::{Broadcaster, Fanout};

// 重导出 sink 类型
#[cfg(any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum"))]
pub use sinks::SseSink;

#[cfg(feature = "ws")]
pub use sinks::WebSocketSink;

// 重导出 source 类型
#[cfg(all(feature = "client", any(feature = "bridge", feature = "ws", feature = "grpc", feature = "server-axum")))]
pub use sources::SseSource;

#[cfg(feature = "ws")]
pub use sources::WebSocketSource;

// 服务器重导出
#[cfg(feature = "server-axum")]
pub use server::{SseResponse, SseBroadcastState, SseFanoutState};
