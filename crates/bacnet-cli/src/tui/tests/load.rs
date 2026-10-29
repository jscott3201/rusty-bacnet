//! Load: a 1,000 events/s burst against a stalled UI, and the time to draw
//! a full device table.

use std::time::{Duration, Instant};

use bacnet_client::client::{DeviceEvent, DeviceEventKind};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::{broadcast, mpsc};

use super::*;
use crate::tui::worker::{DeviceFeed, EVENT_CHANNEL_CAPACITY, EventSink};

/// Distinct instances the burst cycles through.
const INSTANCES: u32 = 500;
/// Events per second, and seconds of burst.
const RATE: u32 = 1_000;
const SECONDS: u32 = 3;

fn device(instance: u32) -> DiscoveredDevice {
    let [a, b] = (instance as u16).to_be_bytes();
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        mac_address: MacAddr::from_slice(&[10, 1, a, b, 0xBA, 0xC0]),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 260,
        last_seen: std::time::Instant::now(),
        source_network: None,
        source_address: None,
    }
}

#[tokio::test(start_paused = true)]
async fn a_1000_per_second_burst_stays_bounded_and_counts_every_drop() {
    // The client's device broadcast, folded by the worker's feed into the
    // bounded UI channel: the same path `serve` runs.
    let (device_tx, mut device_rx) = broadcast::channel::<DeviceEvent>(256);
    let (sink, mut events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let counter = sink.dropped_counter();
    let forwarder = tokio::spawn(async move {
        let mut feed = DeviceFeed::new(AddressStyle::Bip);
        while feed.devices_open {
            let received = device_rx.recv().await;
            feed.on_device(&sink, received);
        }
        feed.needs_resync
    });
    let producer = tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(1) / RATE);
        for i in 0..RATE * SECONDS {
            every.tick().await;
            let kind = if i < INSTANCES {
                DeviceEventKind::Discovered
            } else {
                DeviceEventKind::Updated
            };
            let _ = device_tx.send(DeviceEvent {
                kind,
                device: device(i % INSTANCES),
            });
        }
    });

    // The UI is stuck in a slow draw for two seconds, then drains a frame's
    // budget at 20 fps, as the event loop does.
    let mut app = connected_app();
    let mut delivered = 0u64;
    let mut max_queued = 0usize;
    tokio::time::sleep(Duration::from_secs(2)).await;
    loop {
        max_queued = max_queued.max(events.len());
        assert!(events.len() <= EVENT_CHANNEL_CAPACITY);
        let mut closed = false;
        for _ in 0..256 {
            match events.try_recv() {
                Ok(event) => {
                    worker(&mut app, event);
                    delivered += 1;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        let now = tokio::time::Instant::now().into_std();
        let dropped = counter.load(std::sync::atomic::Ordering::Relaxed);
        let log_generation = app.log.generation();
        update(
            &mut app,
            Action::Tick {
                now,
                dropped,
                log_generation,
            },
        );
        if closed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    producer.await.unwrap();
    assert!(
        forwarder.await.unwrap(),
        "drops ask the worker for a resync"
    );

    let produced = u64::from(RATE * SECONDS);
    let dropped = counter.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(max_queued, EVENT_CHANNEL_CAPACITY, "the channel filled");
    assert!(dropped > 0, "the overflow was counted");
    assert_eq!(
        delivered + dropped,
        produced,
        "every event delivered or counted"
    );
    assert_eq!(app.dropped, dropped);
    assert!(
        app.devices.len() <= INSTANCES as usize,
        "rows bounded by instances"
    );
    let text = screen(&mut app, 80, 24);
    assert!(text.contains(&format!("drop {dropped}")), "{text}");
}

#[test]
fn drawing_a_full_device_table_at_120x40_fits_the_frame_budget() {
    // The client's discovery table holds at most 4,096 devices.
    let mut app = connected_app();
    for instance in 0..4_096u32 {
        let address = format!(
            "10.{}.{}.{}:47808",
            instance >> 16,
            (instance >> 8) & 0xFF,
            instance & 0xFF
        );
        worker(
            &mut app,
            WorkerEvent::Discovered(row(instance, &address, 260, 0)),
        );
    }
    app.settle();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| crate::tui::view::draw(f, &app)).unwrap();
    const FRAMES: u32 = 50;
    let start = Instant::now();
    for _ in 0..FRAMES {
        terminal.draw(|f| crate::tui::view::draw(f, &app)).unwrap();
    }
    let per_frame = start.elapsed() / FRAMES;
    println!("device table at 120x40 with 4,096 rows: {per_frame:?} per frame");
    // The 5 ms budget is for release builds; debug only guards against a
    // draw that scales with the row count.
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(250)
    } else {
        Duration::from_millis(5)
    };
    assert!(
        per_frame < budget,
        "{per_frame:?} per frame, budget {budget:?}"
    );
}

/// A full watch list with full sparklines, after a flood of samples, draws
/// within the same budget.
#[test]
fn drawing_a_full_watch_list_at_120x40_fits_the_frame_budget() {
    use crate::tui::app::watch::{HISTORY, Screen};
    use crate::tui::watched::{Sample, WatchEvent};
    use crate::tui::worker::watch::MAX_ROWS;
    use ratatui::crossterm::event::KeyCode;

    let mut app = connected_app();
    let mut commands = Vec::new();
    for instance in 1..=MAX_ROWS {
        press(&mut app, KeyCode::Char(':'));
        type_text(&mut app, &format!("watch 100 av:{instance}"));
        commands.extend(press(&mut app, KeyCode::Enter));
    }
    assert_eq!(commands.len(), MAX_ROWS);
    assert_eq!(app.watch.rows().len(), MAX_ROWS);
    let ids: Vec<_> = app.watch.rows().iter().map(|row| row.id).collect();
    // Ten sparklines' worth of samples for every row, a tick's batch each.
    for round in 0..HISTORY * 10 {
        let value = (round % 17) as f64;
        let samples = ids
            .iter()
            .map(|id| {
                let sample = Sample {
                    value: Ok(value.to_string()),
                    number: Some(value),
                    flags: None,
                    at: t0(),
                };
                (*id, sample)
            })
            .collect();
        worker(&mut app, WorkerEvent::Watch(WatchEvent::Samples(samples)));
    }
    assert!(
        app.watch
            .rows()
            .iter()
            .all(|row| row.history.len() == HISTORY)
    );
    app.screen = Screen::Watch;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    const FRAMES: u32 = 50;
    let start = Instant::now();
    for _ in 0..FRAMES {
        terminal.draw(|f| crate::tui::view::draw(f, &app)).unwrap();
    }
    let per_frame = start.elapsed() / FRAMES;
    println!("watch list at 120x40 with {MAX_ROWS} rows: {per_frame:?} per frame");
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(250)
    } else {
        Duration::from_millis(5)
    };
    assert!(
        per_frame < budget,
        "{per_frame:?} per frame, budget {budget:?}"
    );
}
