//! A Command action naming another device goes out as a confirmed
//! WriteProperty, and the device's answer decides its write-successful flag
//! (#1180, Clause 12.10.8).
//!
//! Device 9 is bound to the harness peer. CMD-1's list 1 writes AO-1 in
//! Device 9 to 80.0 at priority 8, then the local AO-2 to 80.0. List 2 is the
//! same with the remote write quitting on failure. Requests and the reads of
//! In_Process and All_Writes_Successful go over the wire; the peer's answers
//! are delivered by hand. The clock is paused, and the server's APDU timeout
//! is the default 3 seconds.
use super::command_action_wire_tests::{
    ao, cmd, idle, outputs, read_db, slot8, state, write, write_pv,
};
use super::command_run_stop_tests::{db_flags, db_state};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_objects::command::CommandObject;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};

pub(super) fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn objects(db: &mut ObjectDatabase) {
    outputs(db);
    let remote = BACnetActionCommand {
        device_identifier: Some(device(9)),
        ..write(ao(1), PropertyValue::Real(80.0), 8)
    };
    let quitting = BACnetActionCommand {
        quit_on_failure: true,
        ..remote.clone()
    };
    let local = write(ao(2), PropertyValue::Real(80.0), 8);
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![
            BACnetActionList {
                commands: vec![remote, local.clone()],
            },
            BACnetActionList {
                commands: vec![quitting, local],
            },
        ])
        .unwrap();
    db.add(Box::new(command)).unwrap();
}

/// A harness whose server has no binding for Device 9.
pub(super) async fn start_unbound() -> Harness {
    Harness::start_with(ServerConfig::default(), objects).await
}

/// A harness whose server reaches Device 9 through `binding`.
async fn start(binding: DeviceBinding) -> Harness {
    let h = start_unbound().await;
    h.server
        .device_bindings
        .write()
        .await
        .insert_configured(binding, |_| false)
        .unwrap();
    h
}

pub(super) async fn start_local() -> Harness {
    start(DeviceBinding::local(device(9), PEER).unwrap()).await
}

/// Take every WriteProperty request the server has sent.
pub(super) fn sent_writes(h: &Harness) -> Vec<(u8, WritePropertyRequest)> {
    let mut frames = h.frames.lock().unwrap();
    let mut taken = Vec::new();
    frames.retain(|apdu| match apdu {
        Apdu::ConfirmedRequest(request)
            if request.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY =>
        {
            let decoded = WritePropertyRequest::decode(&request.service_request).unwrap();
            taken.push((request.invoke_id, decoded));
            false
        }
        _ => true,
    });
    taken
}

/// Wait for the one WriteProperty request the run sends, and check it carries
/// the command.
pub(super) async fn remote_write(h: &Harness) -> u8 {
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
    .expect("a WriteProperty to Device 9");
    assert_eq!(sent.len(), 1);
    let (invoke_id, request) = sent.into_iter().next().unwrap();
    assert_eq!(
        request,
        WritePropertyRequest {
            object_identifier: ao(1),
            property_identifier: PV,
            property_array_index: None,
            property_value: real(80.0),
            priority: Some(8),
        }
    );
    invoke_id
}

/// Deliver `answer` as `source_mac` sent it, from `source` when routed.
pub(super) async fn deliver(
    h: &Harness,
    answer: &Apdu,
    source_mac: &[u8],
    source: Option<NpduAddress>,
) {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, answer).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source,
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    h.tx.send(ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(source_mac),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    })
    .await
    .unwrap();
}

/// DeviceCommunicationControl's DISABLE_INITIATION: requests are still
/// answered, nothing is initiated.
pub(super) fn disable_initiation(h: &Harness) {
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
}

pub(super) fn ack(invoke_id: u8) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    })
}

/// #1493: a binding at a group address, the device's own or its router's,
/// names no device. The request path refuses it, so no WriteProperty goes
/// out and the remote write fails as for an unbound device, while the local
/// write still runs.
#[tokio::test(start_paused = true)]
async fn command_write_to_a_device_bound_at_a_group_address_is_never_sent() {
    for binding in [
        DeviceBinding::local(device(9), GROUP),
        DeviceBinding::routed(device(9), 700, [0x33], GROUP),
    ] {
        let mut h = start(binding.unwrap()).await;
        write_pv(&mut h, 1, 1).await.unwrap();
        idle(&h, 1).await;
        assert!(sent_writes(&h).is_empty());
        assert_eq!(db_flags(&h, 1).await, [false, true]);
        assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
        assert_eq!(state(&mut h, 1).await, (false, false));
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn command_writes_another_device_and_its_ack_marks_the_command_successful() {
    let mut h = start_local().await;
    write_pv(&mut h, 1, 1).await.unwrap();
    let invoke_id = remote_write(&h).await;
    // The run waits for the answer without holding the database.
    assert_eq!(state(&mut h, 1).await, (true, false));
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
    assert_eq!(h.server.notification_transactions.active_count(), 1);

    // An ACK for another service under the same invoke ID, a late answer to
    // a notification that once held it, leaves the write outstanding.
    h.respond(Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
    }))
    .await;
    h.settle().await;
    assert_eq!(state(&mut h, 1).await, (true, false));
    assert_eq!(h.server.notification_transactions.active_count(), 1);

    h.respond(ack(invoke_id)).await;
    idle(&h, 1).await;
    assert_eq!(db_flags(&h, 1).await, [true, true]);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(state(&mut h, 1).await, (false, true));
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn command_write_another_device_refuses_busy_fails_at_once_and_quits() {
    let mut h = start_local().await;
    write_pv(&mut h, 1, 2).await.unwrap();
    let invoke_id = remote_write(&h).await;
    h.respond(Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::BUSY,
        error_data: Bytes::new(),
    }))
    .await;
    idle(&h, 1).await;
    // BUSY is final: no retry follows, and the quitting failure leaves the
    // local write unmade.
    assert_eq!(db_flags(&h, 2).await, [false, false]);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
    assert_eq!(state(&mut h, 1).await, (false, false));
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert!(sent_writes(&h).is_empty());
}

#[tokio::test(start_paused = true)]
async fn command_write_another_device_never_answers_is_retried_then_fails() {
    let mut h = start_local().await;
    write_pv(&mut h, 1, 1).await.unwrap();
    let invoke_id = remote_write(&h).await;
    // Each silent attempt waits 3 seconds; three retries follow the first,
    // all under the one invoke ID.
    tokio::time::sleep(Duration::from_millis(11_900)).await;
    let retries = sent_writes(&h);
    assert_eq!(retries.len(), 3);
    assert!(retries.iter().all(|(id, _)| *id == invoke_id));
    assert_eq!(state(&mut h, 1).await, (true, false));

    idle(&h, 1).await;
    assert_eq!(db_flags(&h, 1).await, [false, true]);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(state(&mut h, 1).await, (false, false));
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn command_write_another_device_fails_unsent_while_dcc_restricts_initiation() {
    let mut h = start_local().await;
    disable_initiation(&h);
    write_pv(&mut h, 1, 1).await.unwrap();
    idle(&h, 1).await;
    assert!(sent_writes(&h).is_empty());
    assert_eq!(db_flags(&h, 1).await, [false, true]);
    assert_eq!(state(&mut h, 1).await, (false, false));

    h.server.comm_state.set_for_test(DccState::Enable);
    write_pv(&mut h, 1, 1).await.unwrap();
    let invoke_id = remote_write(&h).await;
    h.respond(ack(invoke_id)).await;
    idle(&h, 1).await;
    assert_eq!(db_flags(&h, 1).await, [true, true]);
    assert_eq!(state(&mut h, 1).await, (false, true));
}

#[tokio::test(start_paused = true)]
async fn command_write_another_device_ends_at_the_first_retry_dcc_blocks() {
    let mut h = start_local().await;
    write_pv(&mut h, 1, 1).await.unwrap();
    remote_write(&h).await;
    let sent = tokio::time::Instant::now();
    disable_initiation(&h);
    // The first retry, 3 seconds on, is where DCC is checked: the write ends
    // there, freeing its invoke ID, rather than sitting out the other two.
    while read_db(&h, cmd(1), PropertyIdentifier::IN_PROCESS, None).await
        == PropertyValue::Boolean(true)
    {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let ended = sent.elapsed();
    assert!(
        (Duration::from_millis(2_900)..Duration::from_millis(3_050)).contains(&ended),
        "{ended:?}"
    );
    assert_eq!(h.server.notification_transactions.active_count(), 0);
    assert!(sent_writes(&h).is_empty());
    assert_eq!(db_flags(&h, 1).await, [false, true]);
}

#[tokio::test(start_paused = true)]
async fn command_write_through_a_routed_binding_completes_on_the_routed_ack() {
    // Device 9 sits at address 0x09 on network 5, behind the harness peer.
    let mut h = start(DeviceBinding::routed(device(9), 5, [0x09], PEER).unwrap()).await;
    write_pv(&mut h, 1, 1).await.unwrap();
    let invoke_id = remote_write(&h).await;
    let source = NpduAddress {
        network: 5,
        mac_address: MacAddr::from_slice(&[0x09]),
    };
    deliver(&h, &ack(invoke_id), &PEER, Some(source)).await;
    idle(&h, 1).await;
    assert_eq!(db_flags(&h, 1).await, [true, true]);
    assert_eq!(state(&mut h, 1).await, (false, true));
}

#[tokio::test(start_paused = true)]
async fn stop_during_an_outstanding_remote_write_ends_the_run_and_frees_its_invoke_id() {
    let mut h = start_local().await;
    write_pv(&mut h, 1, 1).await.unwrap();
    remote_write(&h).await;
    assert_eq!(h.server.notification_transactions.active_count(), 1);

    h.server.stop().await.unwrap();
    assert_eq!(db_state(&h, 1).await, (false, false));
    assert_eq!(db_flags(&h, 1).await, [false, false]);
    assert_eq!(h.server.notification_transactions.active_count(), 0);
}
