//! The aggregator on its own: notifications into rows, the ring's bound,
//! and every loss counted.

use bacnet_client::client::COVNotificationDelivery;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use tokio::sync::broadcast;

use super::*;
use crate::tui::worker::EVENT_CHANNEL_CAPACITY;

const DEVICE: u32 = 100;

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn notification(row: RowId, object: ObjectIdentifier, value: f32) -> ReceivedCOVNotification {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &PropertyValue::Real(value)).unwrap();
    ReceivedCOVNotification {
        notification: COVNotificationRequest {
            subscriber_process_identifier: row,
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, DEVICE)
                .unwrap(),
            monitored_object_identifier: object,
            time_remaining: 300,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: buf.to_vec(),
                priority: None,
            }],
        },
        source_mac: MacAddr::from_slice(&[10, 0, 0, 100, 0xBA, 0xC0]),
        source_network: None,
        source_address: None,
        delivery: COVNotificationDelivery::Unconfirmed,
    }
}

/// A state with row 1 watching AI:1, as if its start job were running.
fn state_with_row<'a>(sink: &'a EventSink, options: &'a WatchOptions) -> State<'a> {
    let mut state = State::new(sink, options);
    state.command(WatchCommand::Add {
        row: 1,
        point: Point {
            device: DEVICE,
            network: None,
            object: ai(1),
            property: PropertyIdentifier::PRESENT_VALUE,
        },
    });
    state.rows.get_mut(&1).unwrap().mode = Mode::Starting;
    state
}

/// A notification lands in its own row only, and the next tick hands it
/// to the UI.
#[tokio::test(start_paused = true)]
async fn notifications_fold_into_their_row() {
    let (sink, mut events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let options = WatchOptions::default();
    let mut state = state_with_row(&sink, &options);
    assert!(state.notification(Ok(notification(1, ai(1), 72.5))));
    // Another object, or another row's process id, is not this row's.
    state.notification(Ok(notification(1, ai(2), 1.0)));
    state.notification(Ok(notification(9, ai(1), 2.0)));
    state.tick(&RateCap::new(0));
    let Some(WorkerEvent::Watch(WatchEvent::Samples(samples))) = events.recv().await else {
        panic!("expected samples");
    };
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].0, 1);
    assert_eq!(samples[0].1.value.as_deref(), Ok("72.5"));
}

/// Flooding past the client's notification channel: the receiver lags,
/// and every notification it missed is counted as dropped, as is every
/// sample the row's ring pushes out before a tick.
#[tokio::test(start_paused = true)]
async fn a_flood_past_capacity_counts_every_lost_notification() {
    let (sink, _events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let dropped = sink.dropped_counter();
    let options = WatchOptions::default();
    let mut state = state_with_row(&sink, &options);
    let (tx, mut rx) = broadcast::channel(8);
    for n in 0..50 {
        tx.send(notification(1, ai(1), n as f32)).unwrap();
    }
    let mut received = 0;
    loop {
        match rx.try_recv() {
            Ok(note) => {
                received += 1;
                state.notification(Ok(note));
            }
            Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                state.notification(Err(RecvError::Lagged(missed)));
            }
            Err(_) => break,
        }
    }
    assert_eq!(received, 8);
    assert_eq!(dropped.load(std::sync::atomic::Ordering::Relaxed), 42);

    // The ring keeps the newest PENDING_PER_ROW between ticks.
    for n in 0..PENDING_PER_ROW + 3 {
        state.notification(Ok(notification(1, ai(1), n as f32)));
    }
    assert_eq!(
        dropped.load(std::sync::atomic::Ordering::Relaxed),
        42 + 8 + 3,
        "the 8 pending from the flood and 3 more fell out of the ring"
    );
}

/// The log keeps the device a plain instance; a routed one's network has
/// its own column, last, empty on this network.
#[test]
fn a_log_line_puts_a_routed_device_network_in_its_own_column() {
    let sample = Sample {
        value: Ok("72.5 °F".into()),
        number: Some(72.5),
        flags: None,
        at: std::time::Instant::now(),
    };
    let mut point = Point {
        device: 2001,
        network: Some(2),
        object: ai(1),
        property: PropertyIdentifier::PRESENT_VALUE,
    };
    let line = csv_line(&point, &sample, &Ok("72.5".into()), 7, "polled");
    assert_eq!(line, "7,2001,analog-input 1,present-value,72.5,,polled,2");
    assert_eq!(
        csv::HEADER.split(',').count(),
        line.split(',').count(),
        "a field per header column"
    );
    point.network = None;
    let line = csv_line(&point, &sample, &Ok("72.5".into()), 7, "cov");
    assert_eq!(line, "7,2001,analog-input 1,present-value,72.5,,cov,");
}
