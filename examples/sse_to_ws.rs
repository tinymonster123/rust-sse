//! Example: SSE to WebSocket bridge
//!
//! This example demonstrates bridging from an SSE source to a WebSocket sink.
//! It shows the basic pattern of connecting different protocols.
//!
//! Run with: cargo run --example sse_to_ws --features "client,ws"

use rust_sse::bridge::{Bridge, BridgeBuilder, BridgeEvent, EventSink, EventSource};
use rust_sse::SourceError;
use rust_sse::sinks::WebSocketSink;
use std::pin::Pin;

// Mock SSE source for demonstration (replace with SseSource in real use)
struct MockSseSource {
    events: Vec<BridgeEvent>,
}

impl MockSseSource {
    fn new() -> Self {
        Self {
            events: vec![
                BridgeEvent::from_string(r#"{"message": "Hello from SSE"}"#)
                    .with_id("1")
                    .with_event_type("greeting"),
                BridgeEvent::from_string(r#"{"count": 42}"#)
                    .with_id("2")
                    .with_event_type("update"),
                BridgeEvent::from_string(r#"{"status": "complete"}"#)
                    .with_id("3")
                    .with_event_type("status"),
            ],
        }
    }
}

impl EventSource for MockSseSource {
    fn events(
        &self,
    ) -> Pin<Box<dyn futures_core::Stream<Item = Result<BridgeEvent, SourceError>> + Send + '_>>
    {
        use async_stream::stream;

        let events = self.events.clone();
        Box::pin(stream! {
            for event in events {
                // Simulate network delay
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                yield Ok(event);
            }
        })
    }
}

#[tokio::main]
async fn main() {
    println!("SSE to WebSocket Bridge Example\n");
    println!("This example demonstrates the bridge pattern.");
    println!("In a real application, you would:\n");
    println!("1. Use SseSource::get(\"https://your-sse-endpoint.com/events\")");
    println!("2. Connect the WebSocket sink to an actual WebSocket connection\n");

    // Create a mock SSE source
    let source = MockSseSource::new();

    // Create a WebSocket sink (channel-based for demo)
    let (ws_sink, mut ws_rx) = WebSocketSink::new(16);

    // Spawn a task to consume WebSocket messages
    let ws_consumer = tokio::spawn(async move {
        println!("WebSocket consumer started, waiting for messages...\n");

        while let Some(msg) = ws_rx.recv().await {
            match msg {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    println!("  WS received text: {}", text);
                }
                tokio_tungstenite::tungstenite::Message::Binary(data) => {
                    println!("  WS received binary: {} bytes", data.len());
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => {
                    println!("  WS connection closed");
                    break;
                }
                _ => {}
            }
        }

        println!("\nWebSocket consumer finished");
    });

    // Build and run the bridge
    let bridge = BridgeBuilder::new()
        .source(source)
        .sink(ws_sink)
        .build()
        .await
        .expect("Failed to build bridge");

    println!("Bridge created, starting event forwarding...\n");

    // Run the bridge
    match bridge.run().await {
        Ok(()) => println!("\nBridge completed successfully"),
        Err(e) => println!("\nBridge error: {:?}", e),
    }

    // Wait for the WebSocket consumer
    let _ = ws_consumer.await;

    println!("\nExample completed!");
}
