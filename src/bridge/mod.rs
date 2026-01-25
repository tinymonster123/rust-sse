//! 协议桥接模块
//!
//! 本模块提供在不同流式协议（SSE、WebSocket、gRPC）之间
//! 进行桥接的核心抽象。

mod event;
mod sink;
mod source;

pub use event::{BridgeEvent, EventMetadata};
pub use sink::{BoxedEventSink, ChannelSink, EventSink, EventSinkExt, MappedSink, SharedEventSink};
pub use source::{BoxedEventSource, EventSource, EventSourceExt, FilteredSource, MappedSource};

use crate::error::{BridgeError, SinkError, SourceError};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// 连接源和多个接收器的桥接控制器
///
/// Bridge 从单个源读取事件并转发到一个或多个接收器。支持:
/// - 多个接收器，各自独立的背压
/// - 优雅关闭
/// - 单个接收器失败的错误处理
pub struct Bridge {
    source: BoxedEventSource,
    sinks: RwLock<Vec<SharedEventSink>>,
    running: AtomicBool,
}

impl Bridge {
    /// 使用给定的源创建新的桥接
    pub fn new(source: impl EventSource + 'static) -> Self {
        Self {
            source: Box::new(source),
            sinks: RwLock::new(Vec::new()),
            running: AtomicBool::new(false),
        }
    }

    /// 向桥接添加接收器
    pub async fn add_sink(&self, sink: impl EventSink + 'static) {
        let mut sinks = self.sinks.write().await;
        sinks.push(Arc::new(sink));
    }

    /// 向桥接添加共享接收器
    pub async fn add_shared_sink(&self, sink: SharedEventSink) {
        let mut sinks = self.sinks.write().await;
        sinks.push(sink);
    }

    /// 从桥接移除已关闭的接收器
    pub async fn remove_closed_sinks(&self) {
        let mut sinks = self.sinks.write().await;
        sinks.retain(|sink| !sink.is_closed());
    }

    /// 获取活跃接收器数量
    pub async fn sink_count(&self) -> usize {
        self.sinks.read().await.len()
    }

    /// 检查桥接是否正在运行
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 停止桥接
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// 运行桥接，将事件从源转发到所有接收器
    ///
    /// 此方法会一直运行，直到:
    /// - 源耗尽
    /// - 通过 `stop()` 停止桥接
    /// - 发生致命错误
    ///
    /// 如果源正常耗尽返回 Ok(())，
    /// 否则返回描述错误原因的错误。
    pub async fn run(&self) -> Result<(), BridgeError> {
        let sink_count = self.sinks.read().await.len();
        if sink_count == 0 {
            warn!("Bridge started with no sinks");
            return Err(BridgeError::NoSinks);
        }

        info!(sink_count = sink_count, "Bridge starting");
        self.running.store(true, Ordering::SeqCst);

        let mut stream = self.source.events();
        let mut events_forwarded: u64 = 0;

        while self.is_running() {
            let event = match stream.next().await {
                Some(Ok(event)) => event,
                Some(Err(e)) => {
                    // 记录源错误但尽可能继续
                    match e {
                        SourceError::Closed => {
                            info!("Source closed, stopping bridge");
                            break;
                        }
                        _ => {
                            error!(error = %e, "Source error, stopping bridge");
                            return Err(BridgeError::Source(e));
                        }
                    }
                }
                None => {
                    info!("Source exhausted, stopping bridge");
                    break;
                }
            };

            events_forwarded += 1;
            debug!(events_forwarded = events_forwarded, "Forwarding event to sinks");

            // 发送到所有接收器
            let sinks = self.sinks.read().await;
            let mut failed_indices = Vec::new();

            for (i, sink) in sinks.iter().enumerate() {
                if sink.is_closed() {
                    debug!(sink_index = i, "Sink already closed, marking for removal");
                    failed_indices.push(i);
                    continue;
                }

                if let Err(e) = sink.send(event.clone()).await {
                    // 标记为待移除，但不使整个桥接失败
                    warn!(sink_index = i, error = %e, "Sink send failed, marking for removal");
                    failed_indices.push(i);
                }
            }

            drop(sinks);

            // 移除失败的接收器
            if !failed_indices.is_empty() {
                let mut sinks = self.sinks.write().await;
                for i in failed_indices.into_iter().rev() {
                    if i < sinks.len() {
                        sinks.remove(i);
                    }
                }

                let remaining = sinks.len();
                debug!(remaining_sinks = remaining, "Removed failed sinks");

                // 如果所有接收器都没了，停止
                if remaining == 0 {
                    warn!("All sinks removed, stopping bridge");
                    break;
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);

        // 关闭所有剩余的接收器
        let sinks = self.sinks.read().await;
        for (i, sink) in sinks.iter().enumerate() {
            if let Err(e) = sink.close().await {
                warn!(sink_index = i, error = %e, "Failed to close sink");
            }
        }

        info!(events_forwarded = events_forwarded, "Bridge stopped");
        Ok(())
    }
}

/// 使用流式 API 创建桥接的构建器
pub struct BridgeBuilder {
    source: Option<BoxedEventSource>,
    sinks: Vec<SharedEventSink>,
}

impl BridgeBuilder {
    /// 创建新的桥接构建器
    pub fn new() -> Self {
        Self {
            source: None,
            sinks: Vec::new(),
        }
    }

    /// 设置桥接的源
    pub fn source(mut self, source: impl EventSource + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// 向桥接添加接收器
    pub fn sink(mut self, sink: impl EventSink + 'static) -> Self {
        self.sinks.push(Arc::new(sink));
        self
    }

    /// 向桥接添加共享接收器
    pub fn shared_sink(mut self, sink: SharedEventSink) -> Self {
        self.sinks.push(sink);
        self
    }

    /// 构建桥接
    ///
    /// 如果没有设置源则返回 None
    pub async fn build(self) -> Option<Bridge> {
        let source = self.source?;
        let bridge = Bridge {
            source,
            sinks: RwLock::new(self.sinks),
            running: AtomicBool::new(false),
        };
        Some(bridge)
    }
}

impl Default for BridgeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_stream::stream;
    use std::sync::atomic::AtomicUsize;

    struct TestSource {
        events: Vec<BridgeEvent>,
    }

    impl EventSource for TestSource {
        fn events(
            &self,
        ) -> std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>>
        {
            let events = self.events.clone();
            Box::pin(stream! {
                for event in events {
                    yield Ok(event);
                }
            })
        }
    }

    struct CountingSink {
        count: AtomicUsize,
    }

    impl EventSink for CountingSink {
        fn send(
            &self,
            _event: BridgeEvent,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), SinkError>> + Send + '_>>
        {
            self.count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn test_bridge_basic() {
        let source = TestSource {
            events: vec![
                BridgeEvent::from_string("event1"),
                BridgeEvent::from_string("event2"),
                BridgeEvent::from_string("event3"),
            ],
        };

        let sink = Arc::new(CountingSink {
            count: AtomicUsize::new(0),
        });

        let bridge = BridgeBuilder::new()
            .source(source)
            .shared_sink(sink.clone())
            .build()
            .await
            .unwrap();

        bridge.run().await.unwrap();

        assert_eq!(sink.count.load(Ordering::SeqCst), 3);
    }
}
