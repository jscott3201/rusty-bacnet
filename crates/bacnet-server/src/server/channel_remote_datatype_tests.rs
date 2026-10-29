//! A Channel learns the datatype of a member in another device with a
//! ReadProperty, coerces its value to it (Table 12-63) and keeps it until the
//! member is replaced or a write made with it is refused as the wrong
//! datatype. A read the device refuses leaves the value as written; one it
//! doesn't answer finds the device silent, so no write follows (#1342).
//!
//! CH-9 (channel 25) writes BO-1's Present_Value in Device 9, bound to the
//! harness peer, at once. CH-13 (channel 29) writes BO-1's and BO-2's there
//! after a minute, so their reads come long before their writes. The read and the write are answered by hand. The
//! clock is paused, and the server's APDU timeout is the default 3 seconds.
use super::channel_remote_write_tests::{
    answer_reads, error, next_reads, next_request, read_ack, reliability, sent_reads, start_with,
};
use super::channel_wire_tests::{ch, channel, settled, write_channel, write_wire};
use super::command_remote_write_tests::{ack, device, sent_writes};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{Reliability, WriteStatus};

fn bo1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap()
}

/// BO-1's Present_Value in Device 9.
fn remote_bo1() -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..BACnetDeviceObjectPropertyReference::new_local(bo1(), PV.to_raw())
    }
}

/// BO-`instance`'s Present_Value in Device 9.
fn remote_bo(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, instance).unwrap(),
            PV.to_raw(),
        )
    }
}

async fn start() -> Harness {
    start_with(|db| {
        db.add(Box::new(channel(9, 25, vec![(remote_bo1(), 0)])))
            .unwrap();
        let later = vec![(remote_bo(1), 60_000), (remote_bo(2), 60_000)];
        db.add(Box::new(channel(13, 29, later))).unwrap();
    })
    .await
}

const ACTIVE: [u8; 2] = [0x91, 0x01];
const INACTIVE: [u8; 2] = [0x91, 0x00];

/// REAL `value`, application-tagged.
fn real_octets(value: f32) -> Vec<u8> {
    let mut octets = vec![0x44];
    octets.extend_from_slice(&value.to_be_bytes());
    octets
}

/// Distribute `value` from CH-9, answer the write it makes to BO-1 with
/// `answer`, and return the value written and Write_Status.
async fn distribute(
    h: &mut Harness,
    value: f32,
    read_answer: Option<&PropertyValue>,
    answer: fn(u8) -> Apdu,
) -> (Vec<u8>, WriteStatus) {
    write_channel(h, 9, &PropertyValue::Real(value), Some(8))
        .await
        .unwrap();
    if let Some(present) = read_answer {
        answer_reads(h, 1, present).await;
    }
    let (invoke_id, written) = next_request(h, bo1()).await;
    h.respond(answer(invoke_id)).await;
    (written, settled(h, 9).await)
}

#[tokio::test(start_paused = true)]
async fn channel_coerces_a_value_to_the_datatype_a_member_in_another_device_reads_as() {
    let mut h = start().await;
    // BO-1's Present_Value reads as an ENUMERATED, so REAL 1.0 goes as
    // ACTIVE (Coercion Rule 5).
    let inactive = PropertyValue::Enumerated(0);
    let (written, status) = distribute(&mut h, 1.0, Some(&inactive), ack).await;
    assert_eq!(
        (written, status),
        (ACTIVE.to_vec(), WriteStatus::SUCCESSFUL)
    );

    // The datatype is kept: the next distribution sends no read.
    let (written, status) = distribute(&mut h, 0.0, None, ack).await;
    assert_eq!(
        (written, status),
        (INACTIVE.to_vec(), WriteStatus::SUCCESSFUL)
    );
    assert!(sent_reads(&h).is_empty());

    // Writing the member, even to the same reference, forgets it.
    let mut reference = BytesMut::new();
    encode_device_object_property_reference(&mut reference, &remote_bo1());
    let list = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    write_wire(&mut h, ch(9), list, Some(1), reference.to_vec(), None)
        .await
        .unwrap();
    let (written, status) = distribute(&mut h, 1.0, Some(&inactive), ack).await;
    assert_eq!(
        (written, status),
        (ACTIVE.to_vec(), WriteStatus::SUCCESSFUL)
    );
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn channel_sends_its_value_as_written_when_the_member_read_tells_no_datatype() {
    let mut h = start().await;
    // A refused read: the REAL goes as written, and BO-1 refuses it.
    write_channel(&mut h, 9, &PropertyValue::Real(1.0), Some(8))
        .await
        .unwrap();
    let [(invoke_id, request)]: [_; 1] = next_reads(&h, 1).await.try_into().unwrap();
    assert_eq!(request.object_identifier, bo1());
    h.respond(Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        error_class: ErrorClass::PROPERTY,
        error_code: ErrorCode::READ_ACCESS_DENIED,
        error_data: Bytes::new(),
    }))
    .await;
    let (invoke_id, written) = next_request(&h, bo1()).await;
    assert_eq!(written, real_octets(1.0));
    h.respond(error(invoke_id, ErrorCode::INVALID_DATA_TYPE))
        .await;
    assert_eq!(settled(&mut h, 9).await, WriteStatus::FAILED);
    assert_eq!(
        reliability(&mut h, 9).await,
        Reliability::CONFIGURATION_ERROR
    );

    // Nothing was kept, so the next distribution reads again. A NULL value
    // tells no datatype either, and isn't kept.
    let (written, _) = distribute(&mut h, 1.0, Some(&PropertyValue::Null), ack).await;
    assert_eq!(written, real_octets(1.0));
    write_channel(&mut h, 9, &PropertyValue::Real(1.0), Some(8))
        .await
        .unwrap();
    let [(invoke_id, request)]: [_; 1] = next_reads(&h, 1).await.try_into().unwrap();
    h.respond(read_ack(invoke_id, &request, &PropertyValue::Enumerated(1)))
        .await;
    let (invoke_id, written) = next_request(&h, bo1()).await;
    assert_eq!(written, ACTIVE);
    h.respond(ack(invoke_id)).await;
    assert_eq!(settled(&mut h, 9).await, WriteStatus::SUCCESSFUL);
}

#[tokio::test(start_paused = true)]
async fn a_member_whose_datatype_read_goes_unanswered_fails_with_no_write() {
    let mut h = start().await;
    let started = tokio::time::Instant::now();
    write_channel(&mut h, 9, &PropertyValue::Real(1.0), Some(8))
        .await
        .unwrap();
    // The read is retried as a write would be, all under one invoke ID.
    let [(first, _)]: [_; 1] = next_reads(&h, 1).await.try_into().unwrap();
    tokio::time::sleep(Duration::from_millis(11_900)).await;
    let retries = sent_reads(&h);
    assert_eq!(retries.len(), 3);
    assert!(retries.iter().all(|(id, _)| *id == first));
    // Device 9 counts as silent: the member fails as a communication failure
    // with no write sent, within the one read's retry window.
    assert_eq!(settled(&mut h, 9).await, WriteStatus::FAILED);
    let took = started.elapsed();
    assert!(
        (Duration::from_secs(12)..Duration::from_millis(12_100)).contains(&took),
        "{took:?}"
    );
    assert_eq!(
        reliability(&mut h, 9).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_kept_datatype_the_device_refuses_is_read_again() {
    let mut h = start().await;
    let inactive = PropertyValue::Enumerated(0);
    let (written, _) = distribute(&mut h, 1.0, Some(&inactive), ack).await;
    assert_eq!(written, ACTIVE);
    // BO-1 has since become something that takes no ENUMERATED: the kept
    // datatype is forgotten with the refusal.
    let refused = |id| error(id, ErrorCode::INVALID_DATA_TYPE);
    let (written, status) = distribute(&mut h, 1.0, None, refused).await;
    assert_eq!((written, status), (ACTIVE.to_vec(), WriteStatus::FAILED));
    assert_eq!(
        reliability(&mut h, 9).await,
        Reliability::CONFIGURATION_ERROR
    );
    // So the next distribution reads it again, and goes as a REAL.
    let real = PropertyValue::Real(0.0);
    let (written, status) = distribute(&mut h, 1.0, Some(&real), ack).await;
    assert_eq!(
        (written, status),
        (real_octets(1.0), WriteStatus::SUCCESSFUL)
    );

    // A refusal that says nothing of the datatype keeps it.
    let denied = |id| error(id, ErrorCode::WRITE_ACCESS_DENIED);
    distribute(&mut h, 1.0, None, denied).await;
    let (written, _) = distribute(&mut h, 1.0, None, ack).await;
    assert_eq!(written, real_octets(1.0));
    assert!(sent_reads(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_read_queued_behind_one_that_finds_the_device_silent_goes_unsent() {
    let mut h = start().await;
    write_channel(&mut h, 13, &PropertyValue::Real(1.0), Some(8))
        .await
        .unwrap();
    // BO-1's read goes four times with no answer; BO-2's, waiting its turn at
    // Device 9, is never sent, and neither is a write.
    tokio::time::sleep(Duration::from_secs(13)).await;
    let reads = sent_reads(&h);
    assert_eq!(reads.len(), 4);
    assert!(
        reads
            .iter()
            .all(|(id, request)| *id == reads[0].0 && request.object_identifier == bo1())
    );
    assert_eq!(settled(&mut h, 13).await, WriteStatus::FAILED);
    assert_eq!(
        reliability(&mut h, 13).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert!(sent_reads(&h).is_empty());
    assert!(sent_writes(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_stale_run_sends_no_read_it_was_waiting_to_send() {
    let mut h = start().await;
    write_channel(&mut h, 13, &PropertyValue::Real(1.0), Some(8))
        .await
        .unwrap();
    let [(invoke_id, request)]: [_; 1] = next_reads(&h, 1).await.try_into().unwrap();
    // CH-13 goes while BO-2's read waits its turn behind BO-1's.
    h.server
        .database()
        .write()
        .await
        .remove(&ch(13))
        .unwrap()
        .unwrap();
    h.respond(read_ack(invoke_id, &request, &PropertyValue::Enumerated(0)))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(sent_reads(&h).is_empty());
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}
