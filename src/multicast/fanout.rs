//! Fanout - 带独立背压的一对多事件分发

use crate::bridge::{BridgeEvent, EventSink, SharedEventSink};
use crate::error::SinkError;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, warn};

/// fanout 订阅者的唯一标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FanoutId(usize);

static NEXT_FANOUT_ID: AtomicUsize = AtomicUsize::new(0);

impl FanoutId {
    fn new() -> Self {
        Self(NEXT_FANOUT_ID.fetch_add(1, Ordering::SeqCst))
    }
}

/// fanout 订阅者的句柄
///
/// 当被 drop 时，订阅者会自动注销。
pub struct FanoutHandle {
    id: FanoutId,
    rx: mpsc::Receiver<BridgeEvent>,
    fanout: Arc<FanoutInner>,
}

impl FanoutHandle {
    /// 获取订阅者 ID
    pub fn id(&self) -> FanoutId {
        self.id
    }

    /// 接收下一个事件
    pub async fn recv(&mut self) -> Option<BridgeEvent> {
        self.rx.recv().await
    }

    /// 尝试非阻塞地接收事件
    pub fn try_recv(&mut self) -> Result<BridgeEvent, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }
}

impl Drop for FanoutHandle {
    fn drop(&mut self) {
        // 从 fanout 移除此订阅者
        let fanout = self.fanout.clone();
        let id = self.id;
        tokio::spawn(async move {
            fanout.remove_subscriber(id).await;
        });
    }
}

struct Subscriber {
    id: FanoutId,
    tx: mpsc::Sender<BridgeEvent>,
}

struct FanoutInner {
    subscribers: RwLock<Vec<Subscriber>>,
    buffer_size: usize,
    closed: AtomicBool,
}

impl FanoutInner {
    async fn remove_subscriber(&self, id: FanoutId) {
        let mut subs = self.subscribers.write().await;
        subs.retain(|s| s.id != id);
    }
}

/// 带独立背压的一对多事件分发 Fanout
///
/// 与 Broadcaster 不同，Fanout 为每个订阅者使用独立的 mpsc channel。
/// 这意味着：
/// - 每个订阅者有自己的缓冲区
/// - 慢订阅者不会影响快订阅者
/// - 可以动态添加/移除订阅者
/// - 事件会被克隆给每个订阅者
///
/// 这适用于以下场景：
/// - 保证投递很重要
/// - 订阅者有不同的处理速度
/// - 您需要每个订阅者独立的背压
pub struct Fanout {
    inner: Arc<FanoutInner>,
}

impl Fanout {
    /// 使用指定的每订阅者缓冲区大小创建新的 Fanout
    pub fn new(buffer_size: usize) -> Self {
        Self {
            inner: Arc::new(FanoutInner {
                subscribers: RwLock::new(Vec::new()),
                buffer_size,
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// 创建新的订阅者并返回接收事件的句柄
    pub async fn subscribe(&self) -> FanoutHandle {
        let id = FanoutId::new();
        let (tx, rx) = mpsc::channel(self.inner.buffer_size);

        let mut subs = self.inner.subscribers.write().await;
        subs.push(Subscriber { id, tx });

        FanoutHandle {
            id,
            rx,
            fanout: self.inner.clone(),
        }
    }

    /// 获取活跃订阅者数量
    pub async fn subscriber_count(&self) -> usize {
        self.inner.subscribers.read().await.len()
    }

    /// 移除已关闭/断开的订阅者
    pub async fn remove_closed(&self) {
        let mut subs = self.inner.subscribers.write().await;
        subs.retain(|s| !s.tx.is_closed());
    }

    /// 通过 ID 发送给特定订阅者
    pub async fn send_to(&self, id: FanoutId, event: BridgeEvent) -> Result<(), SinkError> {
        let subs = self.inner.subscribers.read().await;
        for sub in subs.iter() {
            if sub.id == id {
                return sub.tx.send(event).await.map_err(|_| SinkError::Closed);
            }
        }
        Err(SinkError::Send(format!("未找到订阅者 {:?}", id)))
    }

    /// 创建用于 Bridge 的接收器包装器
    pub fn as_sink(self: Arc<Self>) -> FanoutSink {
        FanoutSink { fanout: self }
    }
}

impl Clone for Fanout {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl EventSink for Fanout {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.inner.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let subs = self.inner.subscribers.read().await;
            let mut failed_ids = Vec::new();

            for sub in subs.iter() {
                if sub.tx.is_closed() {
                    debug!(subscriber_id = sub.id.0, "Subscriber channel closed, marking for removal");
                    failed_ids.push(sub.id);
                    continue;
                }

                // 使用 try_send 避免阻塞慢订阅者
                // 如果缓冲区满，该订阅者的事件将被丢弃
                if let Err(e) = sub.tx.try_send(event.clone()) {
                    match e {
                        mpsc::error::TrySendError::Full(_) => {
                            warn!(subscriber_id = sub.id.0, "Subscriber buffer full, event dropped");
                        }
                        mpsc::error::TrySendError::Closed(_) => {
                            debug!(subscriber_id = sub.id.0, "Subscriber channel closed during send");
                            failed_ids.push(sub.id);
                        }
                    }
                }
            }

            drop(subs);

            // 移除失败的订阅者
            if !failed_ids.is_empty() {
                let mut subs = self.inner.subscribers.write().await;
                let before_count = subs.len();
                subs.retain(|s| !failed_ids.contains(&s.id));
                debug!(removed = before_count - subs.len(), remaining = subs.len(), "Removed closed subscribers");
            }

            Ok(())
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.inner.closed.store(true, Ordering::SeqCst);
            // 清除所有订阅者
            let mut subs = self.inner.subscribers.write().await;
            subs.clear();
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }
}

/// 用于作为 SharedEventSink 的 Fanout 接收器包装器
pub struct FanoutSink {
    fanout: Arc<Fanout>,
}

impl EventSink for FanoutSink {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        self.fanout.send(event)
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        self.fanout.close()
    }

    fn is_closed(&self) -> bool {
        self.fanout.is_closed()
    }
}

/// 带阻塞发送的 Fanout（等待所有订阅者）
///
/// 此变体会阻塞直到所有订阅者都收到事件，
/// 以延迟为代价提供保证投递。
pub struct BlockingFanout {
    inner: Arc<FanoutInner>,
}

impl BlockingFanout {
    /// 创建新的阻塞 Fanout
    pub fn new(buffer_size: usize) -> Self {
        Self {
            inner: Arc::new(FanoutInner {
                subscribers: RwLock::new(Vec::new()),
                buffer_size,
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// 创建新的订阅者
    pub async fn subscribe(&self) -> FanoutHandle {
        let id = FanoutId::new();
        let (tx, rx) = mpsc::channel(self.inner.buffer_size);

        let mut subs = self.inner.subscribers.write().await;
        subs.push(Subscriber { id, tx });

        FanoutHandle {
            id,
            rx,
            fanout: self.inner.clone(),
        }
    }

    /// 获取订阅者数量
    pub async fn subscriber_count(&self) -> usize {
        self.inner.subscribers.read().await.len()
    }
}

impl EventSink for BlockingFanout {
    fn send(&self, event: BridgeEvent) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            if self.inner.closed.load(Ordering::SeqCst) {
                return Err(SinkError::Closed);
            }

            let subs = self.inner.subscribers.read().await;

            // 发送给所有订阅者，等待每个
            for sub in subs.iter() {
                if !sub.tx.is_closed() {
                    // 这会在缓冲区满时等待
                    if let Err(e) = sub.tx.send(event.clone()).await {
                        // 订阅者断开连接，稍后会被清理
                        debug!(subscriber_id = sub.id.0, error = %e, "Subscriber disconnected during blocking send");
                    }
                }
            }

            Ok(())
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + '_>> {
        Box::pin(async move {
            self.inner.closed.store(true, Ordering::SeqCst);
            let mut subs = self.inner.subscribers.write().await;
            subs.clear();
            Ok(())
        })
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fanout_basic() {
        let fanout = Fanout::new(16);

        let mut handle1 = fanout.subscribe().await;
        let mut handle2 = fanout.subscribe().await;

        assert_eq!(fanout.subscriber_count().await, 2);

        // 发送一个事件
        fanout.send(BridgeEvent::from_string("hello")).await.unwrap();

        // 两个都应收到
        let event1 = handle1.recv().await.unwrap();
        let event2 = handle2.recv().await.unwrap();

        assert_eq!(event1.payload_str(), "hello");
        assert_eq!(event2.payload_str(), "hello");
    }

    #[tokio::test]
    async fn test_fanout_independent_backpressure() {
        let fanout = Fanout::new(2); // 小缓冲区

        let mut fast_handle = fanout.subscribe().await;
        let _slow_handle = fanout.subscribe().await; // 不消费

        // 发送多个事件
        for i in 0..5 {
            fanout
                .send(BridgeEvent::from_string(format!("event{}", i)))
                .await
                .unwrap();
        }

        // 快订阅者仍应收到（部分）事件
        let event = fast_handle.recv().await.unwrap();
        assert!(event.payload_str().starts_with("event"));
    }

    #[tokio::test]
    async fn test_fanout_handle_drop() {
        let fanout = Fanout::new(16);

        {
            let _handle = fanout.subscribe().await;
            assert_eq!(fanout.subscriber_count().await, 1);
        }

        // 给 drop 一些时间执行
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_eq!(fanout.subscriber_count().await, 0);
    }
}
