//! 集成测试：使用 axum 模拟 SSE 服务器进行端到端测试

use axum::{
    routing::get,
    Router,
    response::sse::{Event, Sse},
};
use futures_util::{stream, StreamExt};
use rust_sse::{SseClient, SseRequest, SseRetry};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::timeout;

/// 创建一个测试用的 SSE 服务器
async fn create_test_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/events", get(sse_handler))
        .route("/events-with-id", get(sse_with_id_handler))
        .route("/events-limited", get(sse_limited_handler))
        .route("/events-slow", get(sse_slow_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // 给服务器一点启动时间
    tokio::time::sleep(Duration::from_millis(50)).await;

    (addr, handle)
}

/// 发送 3 个事件然后关闭
async fn sse_handler() -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::iter(vec![
        Ok(Event::default().data("event1")),
        Ok(Event::default().data("event2")),
        Ok(Event::default().data("event3")),
    ]);

    Sse::new(stream)
}

/// 发送带 ID 的事件
async fn sse_with_id_handler() -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::iter(vec![
        Ok(Event::default().id("1").event("message").data("first")),
        Ok(Event::default().id("2").event("message").data("second")),
        Ok(Event::default().id("3").event("update").data("third")),
    ]);

    Sse::new(stream)
}

/// 发送有限数量的事件（用于测试重连）
async fn sse_limited_handler() -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::iter(vec![
        Ok(Event::default().data("limited1")),
        Ok(Event::default().data("limited2")),
    ]);

    Sse::new(stream)
}

/// 发送事件之间有延迟（用于测试慢连接）
async fn sse_slow_handler() -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let stream = async_stream::stream! {
        yield Ok(Event::default().data("slow1"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        yield Ok(Event::default().data("slow2"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        yield Ok(Event::default().data("slow3"));
    };

    Sse::new(stream)
}

#[tokio::test]
async fn test_basic_sse_stream() {
    let (addr, _handle) = create_test_server().await;

    let client = SseClient::new();
    let request = SseRequest::get(format!("http://{}/events", addr))
        .retry(SseRetry {
            max_retries: Some(0),
            ..Default::default()
        });

    let stream = client.stream(request);
    tokio::pin!(stream);

    let mut events = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(event) => {
                events.push(event.data);
            }
            Err(_) => break,
        }
    }

    assert_eq!(events.len(), 3);
    assert_eq!(events[0], "event1");
    assert_eq!(events[1], "event2");
    assert_eq!(events[2], "event3");
}

#[tokio::test]
async fn test_sse_with_event_ids() {
    let (addr, _handle) = create_test_server().await;

    let client = SseClient::new();
    let request = SseRequest::get(format!("http://{}/events-with-id", addr))
        .retry(SseRetry {
            max_retries: Some(0),
            ..Default::default()
        });

    let stream = client.stream(request);
    tokio::pin!(stream);

    let mut events = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(event) => {
                events.push((event.id, event.event, event.data));
            }
            Err(_) => break,
        }
    }

    assert_eq!(events.len(), 3);
    assert_eq!(events[0], (Some("1".to_string()), Some("message".to_string()), "first".to_string()));
    assert_eq!(events[1], (Some("2".to_string()), Some("message".to_string()), "second".to_string()));
    assert_eq!(events[2], (Some("3".to_string()), Some("update".to_string()), "third".to_string()));
}

#[tokio::test]
async fn test_max_retries() {
    let (addr, _handle) = create_test_server().await;

    let client = SseClient::new();
    let request = SseRequest::get(format!("http://{}/events-limited", addr))
        .retry(SseRetry {
            base_delay: Duration::from_millis(10),
            max_retries: Some(2),
            ..Default::default()
        });

    let stream = client.stream(request);
    tokio::pin!(stream);

    // 使用 timeout 防止测试挂起
    let result = timeout(Duration::from_secs(2), async {
        let mut count = 0;
        while let Some(result) = stream.next().await {
            if result.is_ok() {
                count += 1;
            }
        }
        count
    }).await;

    // 应该收到一些事件（每次重连 2 个，最多 3 次尝试 = 最多 6 个事件）
    assert!(result.is_ok());
    let count = result.unwrap();
    assert!(count >= 2 && count <= 6, "Expected 2-6 events, got {}", count);
}

#[tokio::test]
async fn test_exponential_backoff() {
    // 测试指数退避配置是否正确应用
    let retry = SseRetry::with_exponential_backoff(Duration::from_millis(100))
        .max_retries(3)
        .backoff_factor(2.0)
        .max_delay(Duration::from_secs(1));

    assert!(retry.exponential_backoff);
    assert_eq!(retry.backoff_factor, 2.0);
    assert_eq!(retry.max_delay, Duration::from_secs(1));
    assert_eq!(retry.max_retries, Some(3));
}

#[tokio::test]
async fn test_slow_stream() {
    let (addr, _handle) = create_test_server().await;

    let client = SseClient::new();
    let request = SseRequest::get(format!("http://{}/events-slow", addr))
        .connect_timeout(Duration::from_secs(5))
        .retry(SseRetry {
            max_retries: Some(0),
            ..Default::default()
        });

    let stream = client.stream(request);
    tokio::pin!(stream);

    let result = timeout(Duration::from_secs(2), async {
        let mut events = Vec::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(event) => events.push(event.data),
                Err(_) => break,
            }
        }
        events
    }).await;

    assert!(result.is_ok());
    let events = result.unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events, vec!["slow1", "slow2", "slow3"]);
}

#[tokio::test]
async fn test_invalid_url() {
    let client = SseClient::new();
    let request = SseRequest::get("not-a-valid-url");

    let stream = client.stream(request);
    tokio::pin!(stream);

    let result = stream.next().await;
    assert!(result.is_some());
    assert!(result.unwrap().is_err());
}

#[tokio::test]
async fn test_connection_refused() {
    let client = SseClient::new();
    // 使用一个不存在的端口
    let request = SseRequest::get("http://127.0.0.1:59999/events")
        .connect_timeout(Duration::from_millis(100))
        .retry(SseRetry {
            max_retries: Some(0),
            ..Default::default()
        });

    let stream = client.stream(request);
    tokio::pin!(stream);

    let result = timeout(Duration::from_secs(2), async {
        stream.next().await
    }).await;

    assert!(result.is_ok());
    let event_result = result.unwrap();
    assert!(event_result.is_some());
    assert!(event_result.unwrap().is_err());
}
