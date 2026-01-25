//! Broadcaster - 使用 tokio::broadcast 的一对多事件分发

use crate::bridge::{BridgeEvent, EventSink, EventSource};
use crate::error::{SinkError, SourceError};
use async_stream::stream;
use futures_core::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, warn};

/// 一对多事件分发的广播器
///
/// 内部使用 tokio::broadcast。事件会被克隆给所有订阅者。
/// 如果订阅者落后，它将收到 `Lagged` 错误并错过一些事件。
///
/// 这适用于以下场景：
/// - 所有订阅者都应接收所有事件
/// - 可以接受为慢订阅者丢弃事件
/// - 低延迟比保证投递更重要
pub struct Broadcaster {
    tx: broadcast::Sender<BridgeEvent>,
    capacity: usize,
    closed: AtomicBool,
}

impl Broadcaster {
    /// 使用指定的容量创建新的广播器
    ///
    /// 容量决定了在慢订阅者开始错过事件之前可以缓冲多少事件。
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            capacity,
            closed: AtomicBool::new(false),
        }
    }

    /// 获取活跃订阅者数量
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// 创建一个新的订阅者，以流的形式接收事件
    pub fn subscribe(&self) -> BroadcastSubscriber {
        BroadcastSubscriber {
            rx: Arc::new(tokio::sync::Mutex::new(self.tx.subscribe())),
        }
    }

    /// 创建一个新的订阅者接收器，转发来自此广播器的事件
    pub fn subscriber_sink(&self) -> BroadcastSubscriberSink {
        BroadcastSubscriberSink {
            rx: Arc::new(tokio::sync::Mutex::new(self.tx.subscribe())),
            closed: AtomicBool::new(false),
        }
    }

    /// 获取容量
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Clone for Broadcaster {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            capacity: self.capacity,
            closed: AtomicBool::new(self.closed.load(Ordering::SeqCst)),
        }
    }
}

impl EventSink for Broadcaster {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            // broadcast::send 只在没有接收者时失败，
            // 我们将此视为成功（事件被简单丢弃）
            match self.tx.send(event) {
                Ok(receiver_count) => {
                    debug!(receiver_count = receiver_count, "Event broadcast to receivers");
                }
                Err(_) => {
                    debug!("No receivers for broadcast event, event dropped");
                }
            }
            Ok(())
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// 广播器的订阅者
pub struct BroadcastSubscriber {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<BridgeEvent>>>,
}

impl EventSource for BroadcastSubscriber {
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>> {
        let rx = self.rx.clone();

        Box::pin(stream! {
            loop {
                let result = {
                    let mut guard = rx.lock().await;
                    guard.recv().await
                };
                match result {
                    Ok(event) => yield Ok(event),
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("Broadcast channel closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        // 记录落后事件但继续
                        warn!(lagged_events = n, "Subscriber lagged behind, missed events");
                        yield Err(SourceError::Other(format!("落后 {} 个事件", n)));
                    }
                }
            }
        })
    }
}

/// 可用作 EventSink 目标的订阅者接收器
pub struct BroadcastSubscriberSink {
    rx: Arc<tokio::sync::Mutex<broadcast::Receiver<BridgeEvent>>>,
    closed: AtomicBool,
}

impl BroadcastSubscriberSink {
    /// 接收下一个事件（用于手动轮询）
    pub async fn recv(&self) -> Result<BridgeEvent, SourceError> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Closed) => Err(SourceError::Closed),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                Err(SourceError::Other(format!("落后 {} 个事件", n)))
            }
        }
    }
}

/// 辅助函数：通过广播器运行源
pub async fn broadcast_source<S>(source: S, broadcaster: &Broadcaster) -> Result<(), SourceError>
where
    S: EventSource,
{
    use futures_util::StreamExt;

    let mut stream = source.events();

    while let Some(result) = stream.next().await {
        match result {
            Ok(event) => {
                if let Err(e) = broadcaster.send(event).await {
                    return Err(SourceError::Other(format!("广播失败: {}", e)));
                }
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn test_broadcaster_basic() {
        let broadcaster = Broadcaster::new(16);

        let sub1 = broadcaster.subscribe();
        let sub2 = broadcaster.subscribe();

        // 发送一个事件
        broadcaster
            .send(BridgeEvent::from_string("hello"))
            .await
            .unwrap();

        // 两个订阅者都应收到
        let mut stream1 = sub1.events();
        let mut stream2 = sub2.events();

        // 需要 drop 广播器来关闭 channel
        drop(broadcaster);

        let event1 = stream1.next().await.unwrap().unwrap();
        let event2 = stream2.next().await.unwrap().unwrap();

        assert_eq!(event1.payload_str(), "hello");
        assert_eq!(event2.payload_str(), "hello");
    }

    #[tokio::test]
    async fn test_broadcaster_subscriber_count() {
        let broadcaster = Broadcaster::new(16);
        assert_eq!(broadcaster.subscriber_count(), 0);

        let _sub1 = broadcaster.subscribe();
        assert_eq!(broadcaster.subscriber_count(), 1);

        let _sub2 = broadcaster.subscribe();
        assert_eq!(broadcaster.subscriber_count(), 2);
    }
}
