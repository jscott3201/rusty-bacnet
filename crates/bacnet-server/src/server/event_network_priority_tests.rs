//! Table 13-6's event-priority → NPDU-priority projection (issue #187).
//!
//! Clause 13.2.5.4: "the Network Priority as defined in Clause 6.2.2 shall be
//! set as a function of the alarm and event priority as defined in Table
//! 13-6." The assertion target is the encoded NPDU control octet, not the
//! notification's APDU `priority` field — the APDU field was always right;
//! the NPDU bits were always Normal.
//!
//! Split from `event_notifications_tests.rs`, which is near the 700-LOC cap.

use super::event_recipient_route::network_priority_for_event;
use super::event_recipient_routing_tests::{
    address_recipient, destination_for, distribute_with_priority,
};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_types::enums::NetworkPriority;
use bytes::Bytes;

/// Every band edge of Table 13-6, both ends.
#[test]
fn table_13_6_band_boundaries() {
    for (event_priority, expected) in [
        (0u8, NetworkPriority::LIFE_SAFETY),
        (63, NetworkPriority::LIFE_SAFETY),
        (64, NetworkPriority::CRITICAL_EQUIPMENT),
        (127, NetworkPriority::CRITICAL_EQUIPMENT),
        (128, NetworkPriority::URGENT),
        (191, NetworkPriority::URGENT),
        (192, NetworkPriority::NORMAL),
        (255, NetworkPriority::NORMAL),
    ] {
        assert_eq!(
            network_priority_for_event(event_priority),
            expected,
            "event priority {event_priority}"
        );
    }
}

fn npdu_priority(frame: &Bytes) -> NetworkPriority {
    decode_npdu(frame.clone()).expect("decode NPDU").priority
}

/// An unconfirmed notification's NPDU carries the projected priority, at
/// both edges of every Table 13-6 band.
#[tokio::test]
async fn unconfirmed_notification_npdu_carries_projected_priority() {
    for (event_priority, expected) in [
        (0u8, NetworkPriority::LIFE_SAFETY),
        (63, NetworkPriority::LIFE_SAFETY),
        (64, NetworkPriority::CRITICAL_EQUIPMENT),
        (127, NetworkPriority::CRITICAL_EQUIPMENT),
        (128, NetworkPriority::URGENT),
        (191, NetworkPriority::URGENT),
        (192, NetworkPriority::NORMAL),
        (255, NetworkPriority::NORMAL),
    ] {
        let (broadcasts, unicasts) = distribute_with_priority(
            [event_priority; 3],
            vec![destination_for(address_recipient(0, &[]), false)],
        )
        .await;

        assert_eq!(broadcasts.len(), 1, "event priority {event_priority}");
        assert!(unicasts.is_empty());
        assert_eq!(
            npdu_priority(&broadcasts[0]),
            expected,
            "event priority {event_priority}"
        );
    }
}

/// The confirmed path — including its retry loop's send site — carries the
/// projected priority too.
#[tokio::test]
async fn confirmed_notification_npdu_carries_projected_priority() {
    let mac = [0x0A, 0x00, 0x00, 0x64, 0xBA, 0xC0];
    let (broadcasts, unicasts) = distribute_with_priority(
        [0; 3],
        vec![destination_for(address_recipient(0, &mac), true)],
    )
    .await;

    assert!(broadcasts.is_empty());
    assert_eq!(unicasts.len(), 1, "confirmed unicast goes out");
    assert_eq!(unicasts[0].0, mac);
    assert_eq!(npdu_priority(&unicasts[0].1), NetworkPriority::LIFE_SAFETY);
}

/// Every unconfirmed route shape carries the projected priority — each send
/// site is a separate call, and any one of them can regress alone.
#[tokio::test]
async fn every_unconfirmed_route_carries_projected_priority() {
    let remote_mac = [0x0A, 0x00, 0x00, 0x64, 0xBA, 0xC0];
    let recipients = [
        address_recipient(0, &[]),
        address_recipient(1000, &[]),
        address_recipient(65535, &[]),
        address_recipient(1000, &remote_mac),
    ];
    for recipient in recipients {
        let (broadcasts, unicasts) =
            distribute_with_priority([0; 3], vec![destination_for(recipient.clone(), false)]).await;

        assert_eq!(broadcasts.len(), 1, "route {recipient:?} goes out");
        assert!(unicasts.is_empty());
        assert_eq!(
            npdu_priority(&broadcasts[0]),
            NetworkPriority::LIFE_SAFETY,
            "route {recipient:?}"
        );
    }

    // The local-unicast site sends on the unicast path instead.
    let local_mac = [0x0A, 0x00, 0x00, 0x07, 0xBA, 0xC0];
    let (broadcasts, unicasts) = distribute_with_priority(
        [0; 3],
        vec![destination_for(address_recipient(0, &local_mac), false)],
    )
    .await;
    assert!(broadcasts.is_empty());
    assert_eq!(unicasts.len(), 1);
    assert_eq!(npdu_priority(&unicasts[0].1), NetworkPriority::LIFE_SAFETY);
}
