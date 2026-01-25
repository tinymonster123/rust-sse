//! 协议桥接的 EventSource trait

use crate::bridge::BridgeEvent;
use crate::error::SourceError;
use futures_core::Stream;
use std::pin::Pin;

/// 可以产生桥接事件的事件源 trait
///
/// EventSource 代表任何可以产生事件的协议，
/// 如 SSE、WebSocket 或 gRPC 流。
pub trait EventSource: Send + Sync {
    /// 从此源获取事件流
    ///
    /// 返回的流会持续产生事件，直到源耗尽或发生错误。
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>>;
}

/// 用于类型擦除的装箱事件源
pub type BoxedEventSource = Box<dyn EventSource>;

/// 事件源的扩展 trait
pub trait EventSourceExt: EventSource {
    /// 通过转换函数映射事件
    fn map_events<F, E>(self, f: F) -> MappedSource<Self, F>
    where
        Self: Sized,
        F: Fn(BridgeEvent) -> Result<BridgeEvent, E> + Send + Sync,
        E: Into<SourceError>,
    {
        MappedSource { source: self, f }
    }

    /// 根据谓词过滤事件
    fn filter_events<F>(self, predicate: F) -> FilteredSource<Self, F>
    where
        Self: Sized,
        F: Fn(&BridgeEvent) -> bool + Send + Sync,
    {
        FilteredSource {
            source: self,
            predicate,
        }
    }
}

impl<T: EventSource> EventSourceExt for T {}

/// 通过转换映射事件的源
pub struct MappedSource<S, F> {
    source: S,
    f: F,
}

impl<S, F, E> EventSource for MappedSource<S, F>
where
    S: EventSource,
    F: Fn(BridgeEvent) -> Result<BridgeEvent, E> + Send + Sync,
    E: Into<SourceError>,
{
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>> {
        use futures_util::StreamExt;

        let stream = self.source.events();
        let f = &self.f;

        Box::pin(stream.map(move |result| {
            result.and_then(|event| (f)(event).map_err(|e| e.into()))
        }))
    }
}

/// 过滤事件的源
pub struct FilteredSource<S, F> {
    source: S,
    predicate: F,
}

impl<S, F> EventSource for FilteredSource<S, F>
where
    S: EventSource,
    F: Fn(&BridgeEvent) -> bool + Send + Sync,
{
    fn events(&self) -> Pin<Box<dyn Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>> {
        use futures_util::StreamExt;

        let stream = self.source.events();
        let predicate = &self.predicate;

        Box::pin(stream.filter(move |result| {
            std::future::ready(match result {
                Ok(event) => predicate(event),
                Err(_) => true, // 始终传递错误
            })
        }))
    }
}
