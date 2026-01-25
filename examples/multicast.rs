//! Example: One-to-many multicast with SSE
//!
//! This example demonstrates using the Broadcaster to send events
//! to multiple subscribers simultaneously.
//!
//! Run with: cargo run --example multicast --features bridge

use rust_sse::bridge::{BridgeEvent, EventSink};
use rust_sse::multicast::Broadcaster;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    println!("Starting multicast example...\n");

    // Create a broadcaster with capacity for 16 events
    let broadcaster = Broadcaster::new(16);

    // Spawn subscriber tasks
    let sub1 = broadcaster.subscribe();
    let sub2 = broadcaster.subscribe();
    let sub3 = broadcaster.subscribe();

    println!("Created {} subscribers", broadcaster.subscriber_count());

    // Spawn tasks to consume events from each subscriber
    let handle1 = tokio::spawn(subscriber_task("Subscriber 1", sub1));
    let handle2 = tokio::spawn(subscriber_task("Subscriber 2", sub2));
    let handle3 = tokio::spawn(subscriber_task("Subscriber 3", sub3));

    // Give subscribers time to start
    sleep(Duration::from_millis(100)).await;

    // Send some events
    for i in 1..=5 {
        let event = BridgeEvent::from_string(format!("Event #{}", i))
            .with_id(i.to_string())
            .with_event_type("broadcast");

        println!("Broadcasting: Event #{}", i);
        broadcaster.send(event).await.expect("Failed to send");

        sleep(Duration::from_millis(200)).await;
    }

    // Close the broadcaster
    println!("\nClosing broadcaster...");
    broadcaster.close().await.expect("Failed to close");

    // Wait for subscribers to finish
    let _ = tokio::join!(handle1, handle2, handle3);

    println!("\nMulticast example completed!");
}

async fn subscriber_task(name: &str, source: rust_sse::multicast::BroadcastSubscriber) {
    use futures_util::StreamExt;
    use rust_sse::bridge::EventSource;

    let mut stream = source.events();

    while let Some(result) = stream.next().await {
        match result {
            Ok(event) => {
                println!(
                    "  {} received: id={:?} type={:?} data={}",
                    name,
                    event.id,
                    event.event_type,
                    event.payload_str()
                );
            }
            Err(e) => {
                println!("  {} error: {:?}", name, e);
            }
        }
    }

    println!("  {} stream ended", name);
}
