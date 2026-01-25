//! Example: Axum SSE server with broadcast
//!
//! This example demonstrates creating an SSE server endpoint with Axum
//! that broadcasts events to all connected clients.
//!
//! Run with: cargo run --example axum_sse_server --features "server-axum"
//!
//! Then open http://localhost:3000/events in multiple browser tabs
//! and POST to http://localhost:3000/send to broadcast messages.

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use rust_sse::bridge::BridgeEvent;
use rust_sse::server::{SseBroadcastState, SseResponse};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
struct AppState {
    sse: SseBroadcastState,
}

#[tokio::main]
async fn main() {
    println!("Starting Axum SSE server...");
    println!("Endpoints:");
    println!("  GET  /events  - SSE event stream");
    println!("  POST /send    - Broadcast a message");
    println!("  GET  /stats   - Get subscriber count\n");

    // Create shared state
    let state = AppState {
        sse: SseBroadcastState::new(256),
    };

    // Spawn a background task to send periodic events
    let sse_clone = state.sse.clone();
    tokio::spawn(async move {
        let mut counter = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            counter += 1;

            let event = BridgeEvent::from_string(format!(
                r#"{{"type":"heartbeat","count":{}}}"#,
                counter
            ))
            .with_event_type("heartbeat");

            let _ = sse_clone.send(event).await;
        }
    });

    // Build the router
    let app = Router::new()
        .route("/events", get(sse_handler))
        .route("/send", post(send_handler))
        .route("/stats", get(stats_handler))
        .with_state(Arc::new(state));

    // Start the server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Server listening on http://localhost:3000");

    axum::serve(listener, app).await.unwrap();
}

// SSE event stream endpoint
async fn sse_handler(State(state): State<Arc<AppState>>) -> SseResponse {
    println!("New SSE subscriber connected (total: {})", state.sse.subscriber_count() + 1);
    state.sse.subscribe()
}

#[derive(Deserialize)]
struct SendPayload {
    message: String,
    #[serde(default)]
    event_type: Option<String>,
}

// Endpoint to broadcast a message
async fn send_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SendPayload>,
) -> &'static str {
    let event = BridgeEvent::from_string(payload.message)
        .with_event_type(payload.event_type.unwrap_or_else(|| "message".to_string()));

    match state.sse.send(event).await {
        Ok(_) => "Message sent",
        Err(_) => "Failed to send",
    }
}

// Stats endpoint
async fn stats_handler(State(state): State<Arc<AppState>>) -> String {
    format!(
        r#"{{"subscribers":{}}}"#,
        state.sse.subscriber_count()
    )
}
