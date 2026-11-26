//! An unconfirmed notification the transport refuses to send moves
//! `unconfirmed_send_failed` once per destination, and the transition's other
//! destinations are still served (#1196).

use super::*;
use crate::server::test_transport::SentFrame;

const PEER: [u8; 6] = [127, 0, 0, 1, 0xBA, 0xC1];
const FAILING_A: [u8; 6] = [127, 0, 0, 2, 0xBA, 0xC1];
const FAILING_B: [u8; 6] = [127, 0, 0, 3, 0xBA, 0xC1];

/// Every send fails when `fails` accepts its frame; failed sends are still
/// logged, so the log shows each destination was attempted.
async fn distribute_failing(
    destinations: Vec<BACnetDestination>,
    fails: fn(&SentFrame) -> bool,
) -> (Vec<Bytes>, Vec<UnicastFrame>, EventNotificationCounters) {
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(LITERAL_BROADCAST_MAC)
        .on_send(move |frame| async move {
            if fails(&frame) {
                Err(bacnet_types::error::Error::Transport(
                    std::io::Error::other("injected send failure"),
                ))
            } else {
                Ok(())
            }
        })
        .build();
    let mut db = clocked_test_database();
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    for destination in destinations {
        nc.add_destination(destination).unwrap();
    }
    db.add(Box::new(nc)).unwrap();
    let bindings = Arc::new(RwLock::new(
        super::super::device_bindings::DeviceBindingTable::new(),
    ));
    distribute_counted_on(transport, db, bindings, DccState::Enable).await
}

#[tokio::test]
async fn failed_unicasts_count_once_each_and_the_rest_are_still_sent() {
    let (broadcasts, unicasts, counters) = distribute_failing(
        vec![
            destination_for(address_recipient(0, &FAILING_A), false),
            destination_for(address_recipient(0, &PEER), false),
            destination_for(address_recipient(0, &FAILING_B), false),
            destination_for(address_recipient(0, &[]), false),
        ],
        |frame| !frame.broadcast && frame.mac.as_slice() != PEER,
    )
    .await;
    assert_eq!(
        counters,
        EventNotificationCounters {
            unconfirmed_send_failed: 2,
            ..Default::default()
        }
    );
    // The destinations after the first failure were all attempted.
    let macs: Vec<_> = unicasts.iter().map(|(mac, _)| mac.as_slice()).collect();
    assert_eq!(macs, [&FAILING_A[..], &PEER[..], &FAILING_B[..]]);
    assert_eq!(broadcasts.len(), 1);
}

#[tokio::test]
async fn failed_broadcast_forms_count_once_each() {
    let (broadcasts, unicasts, counters) = distribute_failing(
        vec![
            destination_for(address_recipient(0, &[]), false),
            destination_for(address_recipient(1000, &[]), false),
            destination_for(address_recipient(65535, &[]), false),
            destination_for(address_recipient(0, &PEER), false),
        ],
        |frame| frame.broadcast,
    )
    .await;
    assert_eq!(
        counters,
        EventNotificationCounters {
            unconfirmed_send_failed: 3,
            ..Default::default()
        }
    );
    assert_eq!(broadcasts.len(), 3);
    assert_eq!(unicasts.len(), 1);
}
