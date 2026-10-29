//! A Channel member in another device is written there as a confirmed
//! WriteProperty, and how that write ends decides Write_Status and
//! Reliability (#1264, Clauses 12.53.7, 12.53.9 and 12.53.11).
//!
//! Devices 9 and 11 are bound to the harness peer; Device 10 has no binding,
//! so a request there looks for it with a Who-Is first (#1322). CH-5 (channel
//! 21, group 27) writes AO-1 in Device 9 at once and the local AO-2 100 ms
//! after the distribution starts. CH-6 writes AO-1 in Device 10. CH-7 writes,
//! in this order and all at once, AO-1 and AO-3 in Device 9, AO-4 in Device
//! 11 and the local AO-2. CH-8 writes, all at once, AO-1 and AO-3 in Device
//! 10 and the local AO-2. A REAL distributed to a member in another device is
//! preceded by a ReadProperty that learns the member's datatype (#1342);
//! these tests answer it with a REAL, so the value goes on as one. A device
//! gets one request from the server's runs at a time (`remote_slots`).
//! Requests are taken from the frames the server sends and answered by hand,
//! as are Who-Is requests with an I-Am. The clock is paused, and the server's
//! APDU timeout is the default 3 seconds.
use super::channel_wire_tests::{ch, channel, member, settled, write_channel, write_status};
use super::command_action_wire_tests::{ao, outputs, read_db, read_wire, slot8};
use super::command_remote_write_tests::{
    ack, deliver, device, disable_initiation, remote_write, sent_writes,
};
use super::cov_wire_test_support::*;
use super::remote_write_discovery_tests::{everywhere, i_am, next_who_is, targeted, who_is_sent};
use super::*;
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{Reliability, WriteStatus};

/// AO-1's Present_Value in Device `instance`.
fn remote_member(instance: u32) -> BACnetDeviceObjectPropertyReference {
    remote_output(instance, 1)
}

/// AO-`output`'s Present_Value in Device `instance`.
pub(super) fn remote_output(instance: u32, output: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(instance)),
        ..member(ao(output), PV)
    }
}

async fn start() -> Harness {
    start_with(|_| {}).await
}

/// The harness with CH-5 to CH-8, the Channels `more` adds, and Devices 9
/// and 11 bound to the peer.
pub(super) async fn start_with(more: impl FnOnce(&mut ObjectDatabase)) -> Harness {
    let h = Harness::start_with(ServerConfig::default(), |db| {
        outputs(db);
        let mut five = channel(5, 21, vec![(remote_member(9), 0), (member(ao(2), PV), 100)]);
        five.set_control_groups(vec![27]).unwrap();
        db.add(Box::new(five)).unwrap();
        db.add(Box::new(channel(6, 22, vec![(remote_member(10), 0)])))
            .unwrap();
        let seven = vec![
            (remote_output(9, 1), 0),
            (remote_output(9, 3), 0),
            (remote_output(11, 4), 0),
            (member(ao(2), PV), 0),
        ];
        db.add(Box::new(channel(7, 23, seven))).unwrap();
        let eight = vec![
            (remote_output(10, 1), 0),
            (remote_output(10, 3), 0),
            (member(ao(2), PV), 0),
        ];
        db.add(Box::new(channel(8, 24, eight))).unwrap();
        more(db);
    })
    .await;
    for instance in [9, 11] {
        let binding = DeviceBinding::local(device(instance), PEER).unwrap();
        h.server
            .device_bindings
            .write()
            .await
            .insert_configured(binding, |_| false)
            .unwrap();
    }
    h
}

/// CH-`instance`'s Reliability, read over the wire.
pub(super) async fn reliability(h: &mut Harness, instance: u32) -> Reliability {
    match read_wire(h, ch(instance), PropertyIdentifier::RELIABILITY, None)
        .await
        .unwrap()[..]
    {
        [0x91, raw] => Reliability::from_raw(raw.into()),
        ref other => panic!("Reliability read {other:?}"),
    }
}

/// Whether CH-`instance`'s Status_Flags show FAULT, read over the wire.
async fn fault(h: &mut Harness, instance: u32) -> bool {
    match read_wire(h, ch(instance), SF, None).await.unwrap()[..] {
        [0x82, 0x04, flags] => flags & 0x40 != 0,
        ref other => panic!("Status_Flags read {other:?}"),
    }
}

/// Wait for the one WriteProperty sent next, check it writes `object`, and
/// return its invoke ID and value.
pub(super) async fn next_request(h: &Harness, object: ObjectIdentifier) -> (u8, Vec<u8>) {
    let sent = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let sent = sent_writes(h);
            if !sent.is_empty() {
                return sent;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("a WriteProperty to another device");
    let [(invoke_id, request)]: [(u8, WritePropertyRequest); 1] = sent.try_into().unwrap();
    assert_eq!(request.object_identifier, object);
    (invoke_id, request.property_value)
}

/// Take every ReadProperty request the server has sent.
pub(super) fn sent_reads(h: &Harness) -> Vec<(u8, ReadPropertyRequest)> {
    let mut frames = h.frames.lock().unwrap();
    let mut taken = Vec::new();
    frames.retain(|apdu| match apdu {
        Apdu::ConfirmedRequest(request)
            if request.service_choice == ConfirmedServiceChoice::READ_PROPERTY =>
        {
            let decoded = ReadPropertyRequest::decode(&request.service_request).unwrap();
            taken.push((request.invoke_id, decoded));
            false
        }
        _ => true,
    });
    taken
}

/// Wait until `count` ReadProperty requests have gone out, and return them.
pub(super) async fn next_reads(h: &Harness, count: usize) -> Vec<(u8, ReadPropertyRequest)> {
    let mut reads = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), async {
        while reads.len() < count {
            reads.extend(sent_reads(h));
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the ReadProperty requests");
    assert_eq!(reads.len(), count);
    reads
}

/// The ComplexAck a device answers read `request`, invoke ID `invoke_id`,
/// with: the property holds `value`.
pub(super) fn read_ack(
    invoke_id: u8,
    request: &ReadPropertyRequest,
    value: &PropertyValue,
) -> Apdu {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, value).unwrap();
    let mut service_ack = BytesMut::new();
    ReadPropertyACK {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        property_value: encoded.to_vec(),
    }
    .encode(&mut service_ack);
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: service_ack.freeze(),
    })
}

/// Answer each ReadProperty request as it goes out, until `count` have: the
/// property holds `value`. A device gets one at a time, so the next one there
/// waits for the answer to this one.
pub(super) async fn answer_reads(h: &Harness, count: usize, value: &PropertyValue) {
    let mut answered = 0;
    tokio::time::timeout(Duration::from_secs(1), async {
        while answered < count {
            for (invoke_id, request) in sent_reads(h) {
                assert_eq!(request.property_identifier, PV);
                h.respond(read_ack(invoke_id, &request, value)).await;
                answered += 1;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the ReadProperty requests");
    assert_eq!(answered, count);
}

/// Hand the server a WriteGroup for group 27 at priority 8 giving channel
/// 21 `value`, as a local broadcast.
async fn write_group(h: &Harness, value: &PropertyValue) {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, value).unwrap();
    let mut service = BytesMut::new();
    WriteGroupRequest {
        group_number: std::num::NonZeroU32::new(27).unwrap(),
        write_priority: 8,
        change_list: vec![GroupChannelValue {
            channel: 21,
            override_priority: None,
            value: encoded.to_vec(),
        }],
        inhibit_delay: None,
    }
    .encode(&mut service)
    .unwrap();
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::WRITE_GROUP,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    let payload = apdu.freeze();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload,
            ..Npdu::default()
        },
    )
    .unwrap();
    h.tx.send(ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&PEER),
        link_layer_group: true,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    })
    .await
    .unwrap();
}

/// A device's answer to the request with this invoke ID.
type Answer = fn(u8) -> Apdu;

pub(super) fn error(invoke_id: u8, error_code: ErrorCode) -> Apdu {
    Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        error_class: ErrorClass::PROPERTY,
        error_code,
        error_data: Bytes::new(),
    })
}

fn reject(invoke_id: u8, reject_reason: RejectReason) -> Apdu {
    Apdu::Reject(RejectPdu {
        invoke_id,
        reject_reason,
    })
}

#[tokio::test(start_paused = true)]
async fn channel_writes_a_member_in_another_device_and_waits_for_its_answer() {
    let mut h = start().await;
    write_channel(&mut h, 5, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    // Device 9 gets the value as a REAL, the datatype its read returns, at
    // the written priority.
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    let invoke_id = remote_write(&h).await;
    assert_eq!(write_status(&mut h, 5).await, WriteStatus::IN_PROGRESS);
    let busy = write_channel(&mut h, 5, &PropertyValue::Real(1.0), Some(8)).await;
    assert_eq!(busy.unwrap_err().error_code, ErrorCode::BUSY);

    // AO-2's 100 ms pass while the write is outstanding, and it is written
    // on time (#1343); the distribution waits for the answer.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(write_status(&mut h, 5).await, WriteStatus::IN_PROGRESS);
    assert_eq!(h.server.notification_transactions.active_count(), 1);
    h.respond(ack(invoke_id)).await;
    assert_eq!(settled(&mut h, 5).await, WriteStatus::SUCCESSFUL);
    assert_eq!(reliability(&mut h, 5).await, Reliability::NO_FAULT_DETECTED);
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn channel_member_in_another_device_that_refuses_fails_the_distribution() {
    let mut h = start().await;
    let answers: [(Answer, Reliability); 4] = [
        (
            |id| error(id, ErrorCode::WRITE_ACCESS_DENIED),
            Reliability::PROCESS_ERROR,
        ),
        (
            |id| {
                Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id: id,
                    abort_reason: AbortReason::OTHER,
                })
            },
            Reliability::PROCESS_ERROR,
        ),
        // The answer says the member names nothing there, or a property that
        // takes no REAL: the member list is at fault.
        (
            |id| error(id, ErrorCode::UNKNOWN_OBJECT),
            Reliability::CONFIGURATION_ERROR,
        ),
        (
            |id| reject(id, RejectReason::INVALID_PARAMETER_DATA_TYPE),
            Reliability::CONFIGURATION_ERROR,
        ),
    ];
    // The first distribution reads; the datatype is kept until a refusal
    // says the member's configuration is at fault.
    let mut kept = false;
    for (answer, expected) in answers {
        write_channel(&mut h, 5, &PropertyValue::Real(80.0), Some(8))
            .await
            .unwrap();
        if !kept {
            answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
        }
        kept = expected != Reliability::CONFIGURATION_ERROR;
        let invoke_id = remote_write(&h).await;
        h.respond(answer(invoke_id)).await;
        // The refusal is final: no retry, and the local member still goes.
        assert_eq!(settled(&mut h, 5).await, WriteStatus::FAILED);
        assert_eq!(reliability(&mut h, 5).await, expected);
        assert!(fault(&mut h, 5).await);
        assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
        assert!(sent_writes(&h).is_empty());
        assert!(sent_reads(&h).is_empty());
    }

    // The next distribution that succeeds clears the fault.
    write_channel(&mut h, 5, &PropertyValue::Real(70.0), Some(8))
        .await
        .unwrap();
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    let (invoke_id, _) = next_request(&h, ao(1)).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(settled(&mut h, 5).await, WriteStatus::SUCCESSFUL);
    assert_eq!(reliability(&mut h, 5).await, Reliability::NO_FAULT_DETECTED);
    assert!(!fault(&mut h, 5).await);
}

#[tokio::test(start_paused = true)]
async fn channel_null_a_member_in_another_device_refuses_as_the_wrong_datatype_succeeds() {
    let mut h = start().await;
    // Clause 12.53.7 names an Error of INVALID_DATATYPE and a Reject of
    // INVALID_PARAMETER_DATA_TYPE; both arrive with their payload (#1323).
    let answers: [fn(u8) -> Apdu; 2] = [
        |id| error(id, ErrorCode::INVALID_DATA_TYPE),
        |id| reject(id, RejectReason::INVALID_PARAMETER_DATA_TYPE),
    ];
    for answer in answers {
        write_channel(&mut h, 5, &PropertyValue::Null, Some(8))
            .await
            .unwrap();
        let (invoke_id, value) = next_request(&h, ao(1)).await;
        assert_eq!(value, [0x00]);
        h.respond(answer(invoke_id)).await;
        assert_eq!(settled(&mut h, 5).await, WriteStatus::SUCCESSFUL);
        assert_eq!(reliability(&mut h, 5).await, Reliability::NO_FAULT_DETECTED);
    }
    // A NULL goes to every datatype, so nothing was read for it.
    assert!(sent_reads(&h).is_empty());

    // Any other refusal of a NULL still fails.
    write_channel(&mut h, 5, &PropertyValue::Null, Some(8))
        .await
        .unwrap();
    let (invoke_id, _) = next_request(&h, ao(1)).await;
    h.respond(error(invoke_id, ErrorCode::WRITE_ACCESS_DENIED))
        .await;
    assert_eq!(settled(&mut h, 5).await, WriteStatus::FAILED);
    assert_eq!(reliability(&mut h, 5).await, Reliability::PROCESS_ERROR);
}

#[tokio::test(start_paused = true)]
async fn channel_member_in_another_device_that_never_answers_is_a_communication_failure() {
    let mut h = start().await;
    write_channel(&mut h, 5, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    let invoke_id = remote_write(&h).await;
    // Each silent attempt waits 3 seconds; three retries follow the first,
    // all under the one invoke ID.
    tokio::time::sleep(Duration::from_millis(11_900)).await;
    let retries = sent_writes(&h);
    assert_eq!(retries.len(), 3);
    assert!(retries.iter().all(|(id, _)| *id == invoke_id));
    assert_eq!(write_status(&mut h, 5).await, WriteStatus::IN_PROGRESS);

    // The local member went on time, and its success doesn't hide the
    // failure.
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(settled(&mut h, 5).await, WriteStatus::FAILED);
    assert_eq!(
        reliability(&mut h, 5).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn channel_skips_the_rest_of_a_silent_device_for_the_distribution() {
    let mut h = start().await;
    let started = tokio::time::Instant::now();
    write_channel(&mut h, 7, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    // AO-1, AO-3 and AO-4 are read; Device 9 takes its two reads in turn.
    answer_reads(&h, 3, &PropertyValue::Real(0.0)).await;
    // AO-1's write to Device 9 goes four times under one invoke ID, with no
    // answer; Device 11 answers AO-4's at once, and the local AO-2 goes.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let first = sent_writes(&h);
    let (silent, answered): (Vec<_>, Vec<_>) = first
        .into_iter()
        .partition(|(_, request)| request.object_identifier == ao(1));
    let [(ao4, _)]: [(u8, WritePropertyRequest); 1] = answered.try_into().unwrap();
    h.respond(ack(ao4)).await;
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    tokio::time::sleep(Duration::from_millis(11_900)).await;
    let mut attempts = silent;
    attempts.extend(sent_writes(&h));
    assert_eq!(attempts.len(), 4);
    assert!(
        attempts
            .iter()
            .all(|(id, request)| *id == attempts[0].0 && request.object_identifier == ao(1))
    );

    // AO-3's turn at Device 9 comes once it has gone silent, so it fails
    // unsent: the distribution waits out one write's retries.
    assert_eq!(settled(&mut h, 7).await, WriteStatus::FAILED);
    let took = started.elapsed();
    assert!(
        (Duration::from_secs(12)..Duration::from_millis(12_100)).contains(&took),
        "{took:?}"
    );
    assert_eq!(
        reliability(&mut h, 7).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert!(sent_writes(&h).is_empty());

    // The next distribution tries Device 9 again, with nothing to read: its
    // two writes go one after the other.
    write_channel(&mut h, 7, &PropertyValue::Real(70.0), Some(8))
        .await
        .unwrap();
    let mut written = Vec::new();
    while written.len() < 3 {
        tokio::time::sleep(Duration::from_millis(1)).await;
        for (invoke_id, request) in sent_writes(&h) {
            written.push(request.object_identifier.instance_number());
            h.respond(ack(invoke_id)).await;
        }
    }
    written.sort_unstable();
    assert_eq!(written, [1, 3, 4]);
    assert_eq!(settled(&mut h, 7).await, WriteStatus::SUCCESSFUL);
    assert_eq!(reliability(&mut h, 7).await, Reliability::NO_FAULT_DETECTED);
    assert!(sent_reads(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn channel_member_in_another_device_sends_nothing_while_dcc_restricts_initiation() {
    let mut h = start().await;
    // Bound Device 9 gets no ReadProperty or WriteProperty, and unbound
    // Device 10 no Who-Is.
    disable_initiation(&h);
    for instance in [5, 6] {
        write_channel(&mut h, instance, &PropertyValue::Real(80.0), Some(8))
            .await
            .unwrap();
        assert_eq!(settled(&mut h, instance).await, WriteStatus::FAILED);
        assert_eq!(
            reliability(&mut h, instance).await,
            Reliability::COMMUNICATION_FAILURE
        );
    }
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert!(sent_writes(&h).is_empty());
    assert!(sent_reads(&h).is_empty());
    assert!(who_is_sent(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn channel_member_in_an_unbound_device_is_written_once_its_who_is_is_answered() {
    let mut h = start().await;
    write_channel(&mut h, 6, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(10)));
    assert_eq!(write_status(&mut h, 6).await, WriteStatus::IN_PROGRESS);
    assert!(sent_reads(&h).is_empty());
    deliver(&h, &i_am(10), &PEER, None).await;
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    let (invoke_id, _) = next_request(&h, ao(1)).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(settled(&mut h, 6).await, WriteStatus::SUCCESSFUL);
    assert_eq!(reliability(&mut h, 6).await, Reliability::NO_FAULT_DETECTED);
}

#[tokio::test(start_paused = true)]
async fn channel_skips_the_rest_of_an_unbound_device_its_who_is_finds_silent() {
    let mut h = start().await;
    let started = tokio::time::Instant::now();
    write_channel(&mut h, 8, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    // Both members' reads look for Device 10 with the one Who-Is.
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(10)));
    // No I-Am comes. Neither member is sent, there is no second Who-Is, and
    // the local AO-2 is still written: the distribution waits out one Who-Is
    // in all.
    assert_eq!(settled(&mut h, 8).await, WriteStatus::FAILED);
    let took = started.elapsed();
    let wait = Duration::from_secs(3);
    assert!(
        (wait..wait + Duration::from_millis(100)).contains(&took),
        "{took:?}"
    );
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(
        reliability(&mut h, 8).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert!(sent_writes(&h).is_empty());
    assert!(sent_reads(&h).is_empty());
    assert!(who_is_sent(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn stop_during_an_outstanding_member_write_ends_the_distribution_failed() {
    let mut h = start().await;
    write_channel(&mut h, 5, &PropertyValue::Real(80.0), Some(8))
        .await
        .unwrap();
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    remote_write(&h).await;
    assert_eq!(h.server.notification_transactions.active_count(), 1);

    h.server.stop().await.unwrap();
    let status = read_db(&h, ch(5), PropertyIdentifier::WRITE_STATUS, None).await;
    assert_eq!(
        status,
        PropertyValue::Enumerated(WriteStatus::FAILED.to_raw())
    );
    let reliability = read_db(&h, ch(5), PropertyIdentifier::RELIABILITY, None).await;
    assert_eq!(
        reliability,
        PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw())
    );
    // The invoke ID is free, and the member not yet due was never written.
    assert_eq!(h.server.notification_transactions.active_count(), 0);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn write_group_distribution_writes_a_member_in_another_device() {
    let mut h = start().await;
    write_group(&h, &PropertyValue::Real(80.0)).await;
    answer_reads(&h, 1, &PropertyValue::Real(0.0)).await;
    let invoke_id = remote_write(&h).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(settled(&mut h, 5).await, WriteStatus::SUCCESSFUL);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(reliability(&mut h, 5).await, Reliability::NO_FAULT_DETECTED);
}
