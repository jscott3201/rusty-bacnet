//! Notification Forwarder object tests (#1225).

use super::*;
use bacnet_encoding::constructed::{
    encode_destination_list, encode_event_notification_subscription_list, encode_port_permission,
};
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::Time;

#[cfg(feature = "std")]
mod fold_tests;
#[cfg(feature = "std")]
mod persistence_tests;
mod properties;
mod recipient_list_restart_tests;
mod selection_tests;
#[cfg(feature = "std")]
mod staging_tests;
mod storage;

const MINUTE: Duration = Duration::from_secs(60);

fn address(network_number: u16, mac: &[u8]) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number,
        mac_address: MacAddr::from_slice(mac),
    })
}

fn device(instance: u32) -> BACnetRecipient {
    BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap())
}

fn subscription(
    recipient: BACnetRecipient,
    process_identifier: u32,
    minutes: u32,
) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient,
        process_identifier,
        issue_confirmed_notifications: false,
        time_remaining: minutes,
    }
}

fn framed_subscriptions(subscriptions: &[BACnetEventNotificationSubscription]) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    PropertyValue::ApplicationData(buf.to_vec())
}

/// A destination open on every day, at every time, for every transition.
fn destination(
    recipient: BACnetRecipient,
    process_identifier: u32,
    confirmed: bool,
) -> BACnetDestination {
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient,
        process_identifier,
        issue_confirmed_notifications: confirmed,
        transitions: EventTransitionBits::all(),
    }
}

fn framed_destinations(destinations: &[BACnetDestination]) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, destinations).unwrap();
    PropertyValue::ApplicationData(buf.to_vec())
}

fn port(port_id: u8, enabled: bool) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_port_permission(&mut buf, &BACnetPortPermission { port_id, enabled });
    PropertyValue::ApplicationData(buf.to_vec())
}

fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    match result {
        Err(
            Error::Remote(RemoteError::Error {
                class: got_class,
                code: got_code,
            })
            | Error::Structured {
                class: got_class,
                code: got_code,
                ..
            },
        ) => assert_eq!(
            (got_class, got_code),
            (class, code),
            "expected {class:?} / {code:?}"
        ),
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

/// A monotonic clock the test moves by hand.
fn manual_clock() -> (Arc<MonotonicClock>, impl Fn(Duration)) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let nanos = Arc::new(AtomicU64::new(0));
    let read = Arc::clone(&nanos);
    let clock: Arc<MonotonicClock> =
        Arc::new(move || Duration::from_nanos(read.load(Ordering::SeqCst)));
    let set = move |at: Duration| {
        nanos.store(u64::try_from(at.as_nanos()).unwrap(), Ordering::SeqCst);
    };
    (clock, set)
}
