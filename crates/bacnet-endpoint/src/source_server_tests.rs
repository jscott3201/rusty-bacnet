//! Source Audit beside the full server the session builds (#2617): the
//! session's records go through the server's pool of notifications, whose
//! dispatch completes their acknowledgments; the server reports the
//! ReinitializeDevice requests it accepts; a peer's write of the Device's
//! Audit_Notification_Recipient is recorded with its requester; and the
//! node's DeviceCommunicationControl, which the server holds, reaches the
//! audited full client.
use super::*;
use bacnet_client::client::ClientOptions;
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_server::mutation::MutationPolicy;
use bacnet_server::server::{AuditReportersConfig, BACnetServer, DccPolicy, ServerBuilder};
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{EnableDisable, ReinitializedState};
use std::sync::atomic::{AtomicBool, Ordering};

fn config() -> SessionConfig {
    SessionConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..SessionConfig::default()
    }
}

/// An audited session with both roles and a full server from `builder`,
/// on Device 123, whose Reporter records to Device 999 at `sink`.
fn server_session(
    db: ObjectDatabase,
    sink: &NetworkLayer<TestTransport>,
    builder: ServerBuilder<TestTransport>,
    config: SessionConfig,
) -> EndpointSession<TestTransport> {
    server_session_on(bus_port(), db, sink, builder, config)
}

/// [`server_session`] on `port`.
fn server_session_on(
    port: TestTransport,
    mut db: ObjectDatabase,
    sink: &NetworkLayer<TestTransport>,
    builder: ServerBuilder<TestTransport>,
    config: SessionConfig,
) -> EndpointSession<TestTransport> {
    db.get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)))
        .unwrap();
    db.set_clock_reader(None);
    let mut session = EndpointSession::new(port, SessionRole::Both, config)
        .unwrap()
        .device(crate::DeviceIdentity::new(123, 42).unwrap())
        .with_database(db)
        .with_source_audit_reporter(selected())
        .with_server(builder);
    session
        .source_audit_bindings
        .push((oid(ObjectType::DEVICE, 999), address(sink)));
    session
}

/// Set the Reporter in `db` to record `operations`, confirmed or not.
fn reporting(db: &mut ObjectDatabase, operations: &[AuditOperation], confirmed: bool) {
    let mut flags = AuditOperationFlags::empty();
    for operation in operations {
        flags.insert(*operation);
    }
    db.get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::AUDIT_ALL,
            flags,
            confirmed,
            None,
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
}

/// The session's own address, from a frame it sends `peer`.
async fn session_mac(
    session: &EndpointSession<TestTransport>,
    peer: &NetworkLayer<TestTransport>,
    replies: &mut bacnet_network::layer::ApduReceiver,
) -> MacAddr {
    session
        .egress
        .as_ref()
        .unwrap()
        .send_apdu(
            vec![0x10, 8],
            bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(peer.local_mac()),
            },
            false,
            NetworkPriority::Normal,
            Vec::new(),
        )
        .await
        .unwrap();
    receive(replies).await.source_mac
}

/// `service` with `data` under `invoke_id`.
fn confirmed(service: ConfirmedServiceChoice, data: BytesMut, invoke_id: u8) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: data.freeze(),
    })
}

/// `service` with `data` from `peer` to `session`, and its answer.
async fn ask(
    peer: &NetworkLayer<TestTransport>,
    session: &[u8],
    replies: &mut bacnet_network::layer::ApduReceiver,
    service: ConfirmedServiceChoice,
    data: BytesMut,
    invoke_id: u8,
) -> Apdu {
    send(peer, session, confirmed(service, data, invoke_id)).await;
    decode_apdu(receive(replies).await.apdu).unwrap()
}

/// A WARMSTART ReinitializeDevice's parameters.
fn warmstart() -> BytesMut {
    let mut data = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::WARMSTART,
        password: None,
    }
    .encode(&mut data)
    .unwrap();
    data
}

fn requester(peer: &NetworkLayer<TestTransport>) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(peer.local_mac()),
    })
}

/// A ReinitializeDevice the server refuses is not recorded; one it
/// accepts is, with its requester as the source, as the narrow role's
/// was (#1564).
#[tokio::test(start_paused = true)]
async fn a_reset_the_server_accepts_is_recorded_with_its_requester() {
    let (sink, mut records) = network().await;
    let (peer, mut replies) = network().await;
    let mut db = database(false);
    reporting(&mut db, &[AuditOperation::DEVICE_RESET], false);
    let refuse = Arc::new(AtomicBool::new(true));
    let answer = Arc::clone(&refuse);
    let builder = allowing().on_reinitialize(move |_, _| {
        if answer.load(Ordering::SeqCst) {
            Err(Error::protocol(
                ErrorClass::DEVICE,
                ErrorCode::CONFIGURATION_IN_PROGRESS,
            ))
        } else {
            Ok(())
        }
    });
    let mut session = server_session(db, &sink, builder, config());
    session.start().await.unwrap();
    let mac = session_mac(&session, &peer, &mut replies).await;
    let service = ConfirmedServiceChoice::REINITIALIZE_DEVICE;
    let reply = ask(&peer, &mac, &mut replies, service, warmstart(), 1).await;
    assert!(matches!(reply, Apdu::Error(_)), "{reply:?}");
    refuse.store(false, Ordering::SeqCst);
    let reply = ask(&peer, &mac, &mut replies, service, warmstart(), 2).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.operation, AuditOperation::DEVICE_RESET);
    assert_eq!(record.source_device, requester(&peer));
    assert_eq!(record.invoke_id, Some(2));
    assert_eq!(
        record.target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 123))
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(records.try_recv().is_err());
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// A ReinitializeDevice already on the link when the session starts is
/// recorded, with its requester: the server takes nothing from its seat
/// until the session's source Audit is in place, however long the start
/// takes after the server is built. A pause there gives the server's
/// dispatch its turn; without the hold it would accept the reset before
/// the source Audit could record it.
#[tokio::test(start_paused = true)]
async fn a_reset_waiting_as_the_session_starts_is_recorded() {
    let (sink, mut records) = network().await;
    let (peer, mut replies) = network().await;
    let mut db = database(false);
    reporting(&mut db, &[AuditOperation::DEVICE_RESET], false);
    let port = bus_port();
    let mac = port.local_mac().to_vec();
    let builder = allowing().on_reinitialize(|_, _| Ok(()));
    let mut session = server_session_on(port, db, &sink, builder, config());
    let service = ConfirmedServiceChoice::REINITIALIZE_DEVICE;
    send(&peer, &mac, confirmed(service, warmstart(), 3)).await;
    let pauses = crate::session::server_seat::StartPauses {
        after_build: Duration::from_millis(20),
        ..Default::default()
    };
    let start = crate::session::server_seat::START_PAUSES.scope(pauses, session.start());
    Box::pin(start).await.unwrap();
    let reply = decode_apdu(receive(&mut replies).await.apdu).unwrap();
    assert!(
        matches!(&reply, Apdu::SimpleAck(ack) if ack.invoke_id == 3),
        "{reply:?}"
    );
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.operation, AuditOperation::DEVICE_RESET);
    assert_eq!(record.source_device, requester(&peer));
    assert_eq!(record.invoke_id, Some(3));
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// A peer's write of the Device's Audit_Notification_Recipient, which the
/// server's policy allows, is recorded with its requester as the source,
/// to the old recipient and the new. Both acknowledge their confirmed
/// records, and each acknowledgment completes its record through the
/// server's pool: no record goes again, and the Reporter counts no
/// failure.
#[tokio::test(start_paused = true)]
async fn a_peer_write_of_the_recipient_reaches_both_and_their_acks_complete() {
    let (old, mut old_records) = network().await;
    let (new, mut new_records) = network().await;
    let (peer, mut replies) = network().await;
    let mut db = database(true);
    reporting(&mut db, &[], true);
    let mut session = server_session(db, &old, allowing(), config());
    session
        .source_audit_bindings
        .push((oid(ObjectType::DEVICE, 1000), address(&new)));
    session.start().await.unwrap();
    let mac = session_mac(&session, &peer, &mut replies).await;
    let next = BACnetRecipient::Device(oid(ObjectType::DEVICE, 1000));
    let mut value = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut value, &next).unwrap();
    let mut data = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::DEVICE, 123),
        property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut data)
    .unwrap();
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let reply = ask(&peer, &mac, &mut replies, service, data, 71).await;
    assert!(
        matches!(&reply, Apdu::SimpleAck(ack) if ack.invoke_id == 71),
        "{reply:?}"
    );
    let (first, old_invoke) = notification(&receive(&mut old_records).await, true);
    let (second, new_invoke) = notification(&receive(&mut new_records).await, true);
    assert_eq!(first, second);
    assert_eq!(first.operation, AuditOperation::WRITE);
    assert_eq!(first.source_device, requester(&peer));
    assert_eq!(first.invoke_id, Some(71));
    assert_eq!(first.target_value, Some(value.to_vec()));
    for (sink, invoke) in [(&old, old_invoke), (&new, new_invoke)] {
        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id: invoke.unwrap(),
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        });
        send(sink, &mac, ack).await;
    }
    eventually(WAIT, "the records' invoke IDs come back", async || {
        session.active_leases() == 0
    })
    .await;
    // Past every retry the session would make: nothing went again.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        old_records.try_recv().is_err(),
        "the old recipient's record went again"
    );
    assert!(
        new_records.try_recv().is_err(),
        "the new recipient's record went again"
    );
    assert_eq!(
        reliability(&session).await,
        Reliability::NO_FAULT_DETECTED.to_raw(),
        "a record counted as failed"
    );
    session.stop().await.unwrap();
    for network in [old, new, peer] {
        network.stop().await.unwrap();
    }
}

/// Beside a source Audit Reporter the session's server needs a policy
/// that decides mutations, so no peer rewrites the audited Device's
/// recipient by default, and no Audit Reporters of its own, since the
/// Device has one recipient: refused at start otherwise, before the link
/// starts. `MutationPolicy::DenyAll` decides them all.
#[tokio::test(start_paused = true)]
async fn source_audit_beside_a_server_needs_a_policy_and_no_reporters_of_its_own() {
    let (sink, _records) = network().await;
    let refusals: [(ServerBuilder<TestTransport>, &str); 2] = [
        (BACnetServer::generic_builder(), "decides mutations"),
        (
            allowing().audit_reporters(AuditReportersConfig {
                reporters: vec![selected()],
            }),
            "Audit Reporters",
        ),
    ];
    for (builder, why) in refusals {
        let mut session = server_session(database(false), &sink, builder, config());
        let error = session.start().await.expect_err("refused at start");
        assert!(error.to_string().contains(why), "{why}: {error}");
        assert!(!session.is_running());
    }
    let denying = BACnetServer::generic_builder().mutation_policy(MutationPolicy::DenyAll);
    let mut session = server_session(database(false), &sink, denying, config());
    session.start().await.unwrap();
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A full client read of `peer`'s AV-7, in a task of its own.
fn read(
    client: &Arc<bacnet_client::client::BACnetClient<TestTransport>>,
    peer: &NetworkLayer<TestTransport>,
) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
    let client = Arc::clone(client);
    let mac = peer.local_mac().to_vec();
    tokio::spawn(async move {
        client
            .read_property(&mac[..], target(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
    })
}

/// The node's DeviceCommunicationControl reaches an audited session
/// through its real server: a DISABLE_INITIATION the server accepts from
/// a peer withholds the retry of a read already sent, which is recorded
/// as failed (SERVICES, COMMUNICATION_DISABLED), and refuses a new read
/// unsent and unrecorded; a client its operator started outside the
/// node's control reads and is recorded as usual.
#[tokio::test(start_paused = true)]
async fn the_servers_dcc_reaches_the_audited_client() {
    let (sink, mut records) = network().await;
    let (peer, mut requests) = network().await;
    let controller = bacnet_client::client::BACnetClient::generic_builder()
        .transport(bus_port())
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let mut db = database(false);
    reporting(&mut db, &[AuditOperation::READ], false);
    let mut session = server_session(
        db,
        &sink,
        allowing().dcc_policy(DccPolicy::LegacyPermissive),
        SessionConfig {
            apdu_timeout_ms: 1_000,
            apdu_retries: 2,
            ..SessionConfig::default()
        },
    );
    session.start().await.unwrap();
    let client = Arc::new(session.full_client().unwrap());
    let sent = read(&client, &peer);
    let request = receive(&mut requests).await;
    let (invoke, _) = read_request(&request);
    let node = request.source_mac.clone();
    controller
        .device_communication_control(&node[..], EnableDisable::DISABLE_INITIATION, None, None)
        .await
        .unwrap();
    assert!(matches!(
        timeout(WAIT, sent).await.unwrap().unwrap(),
        Err(Error::InitiationDisabled { sent: true })
    ));
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(
        record.result,
        Some((ErrorClass::SERVICES, ErrorCode::COMMUNICATION_DISABLED))
    );
    assert!(requests.try_recv().is_err(), "no retry went");

    let refused = read(&client, &peer);
    assert!(matches!(
        timeout(WAIT, refused).await.unwrap().unwrap(),
        Err(Error::InitiationDisabled { sent: false })
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(requests.try_recv().is_err(), "the refused read went");
    assert!(records.try_recv().is_err(), "the refused read was recorded");

    drop(client);
    let exempt = Arc::new(
        session
            .full_client_with(ClientOptions::default().with_node_dcc_override(true))
            .unwrap(),
    );
    let exempt_read = read(&exempt, &peer);
    let request = receive(&mut requests).await;
    let (invoke, rp) = read_request(&request);
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, exempt_read).await.unwrap().unwrap().is_ok());
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    drop(exempt);
    session.stop().await.unwrap();
    let mut controller = controller;
    controller.stop().await.unwrap();
    sink.stop().await.unwrap();
    peer.stop().await.unwrap();
}
