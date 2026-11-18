//! The node is one device (#2617, PR 2): a DeviceCommunicationControl its
//! full server accepts binds the whole node (Clause 16.1), and the Device
//! it serves states what both roles do.
use super::*;
use bacnet_client::client::{Broadcast, ClientOptions};
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{EnableDisable, ReinitializedState, ServiceSupported};
use bacnet_types::error::Error;
use std::sync::Arc;

/// A node like [`lab`]'s, built with `config`, whose full server accepts
/// DeviceCommunicationControl and ReinitializeDevice from anyone.
async fn dcc_lab(config: SessionConfig) -> Lab {
    let wire = Wire::new();
    let peer = BACnetServer::generic_builder()
        .device(DeviceIdentity::new(PEER_DEVICE, 42).unwrap())
        .database(database(PEER_DEVICE, 21.5))
        .transport(wire.port(PEER).build())
        .build()
        .await
        .unwrap();
    let other = BACnetClient::generic_builder()
        .transport(wire.port(OTHER).build())
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let port = wire.port(NODE);
    let (node_sent, node_link) = (port.sent(), port.handle());
    let mut session = EndpointSession::new(port.build(), SessionRole::Both, config)
        .unwrap()
        .device(DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
        .with_database(database(NODE_DEVICE, 42.0))
        .with_server(
            BACnetServer::generic_builder()
                .dcc_policy(bacnet_server::server::DccPolicy::LegacyPermissive)
                .on_reinitialize(|_, _| Ok(())),
        );
    session.start().await.unwrap();
    let client = session.full_client().unwrap();
    Lab {
        wire,
        session,
        node_sent,
        node_link,
        client,
        peer,
        other,
    }
}

/// The other client sends the node a DeviceCommunicationControl.
async fn control(lab: &Lab, state: EnableDisable, minutes: Option<u16>) {
    lab.other
        .device_communication_control(&Wire::mac(NODE), state, minutes, None)
        .await
        .unwrap();
}

/// The node's full client reads the peer.
async fn client_reads(client: &BACnetClient<TestTransport>) -> Result<(), Error> {
    client
        .read_property(
            &Wire::mac(PEER),
            ai1(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
        .map(drop)
}

fn disabled(result: Result<(), Error>) -> bool {
    matches!(result, Err(Error::InitiationDisabled { sent: false }))
}

/// A ConfirmedCOVNotification from the other client, for the node's client
/// to acknowledge.
fn cov_notification(invoke_id: u8) -> Bytes {
    let mut parameters = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap(),
        monitored_object_identifier: ai1(),
        time_remaining: 60,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x28, 0x00, 0x00],
            priority: None,
        }],
    }
    .encode(&mut parameters)
    .unwrap();
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        &parameters,
    )
}

/// While the node is disabled, nothing it would initiate goes: the full
/// client's requests and unconfirmed sends, the narrow client's requests
/// and the session's own I-Am all fail with the typed error, and nothing is
/// sent.
#[tokio::test(start_paused = true)]
async fn disable_initiation_stops_every_initiation_on_the_node() {
    let lab = dcc_lab(SessionConfig::default()).await;
    control(&lab, EnableDisable::DISABLE_INITIATION, None).await;
    let before = lab.node_sent.len();

    assert!(disabled(client_reads(&lab.client).await));
    assert!(disabled(lab.client.who_is(Broadcast::Global, None).await));
    assert!(disabled(lab.session.broadcast_i_am().await));
    let narrow = lab
        .session
        .client()
        .unwrap()
        .read_property(
            &Wire::mac(PEER),
            ai1(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await;
    assert!(
        matches!(narrow, Err(Error::InitiationDisabled { sent: false })),
        "{narrow:?}"
    );
    assert!(lab.server().broadcast_i_am().await.is_err());
    assert_eq!(lab.node_sent.len(), before, "the node sent nothing");

    // ENABLE lifts it for both roles.
    control(&lab, EnableDisable::ENABLE, None).await;
    client_reads(&lab.client).await.unwrap();
    lab.session.broadcast_i_am().await.unwrap();
    stop(lab).await;
}

/// What is not initiation still goes while the node is disabled: the
/// server's answers, its I-Am answer to a Who-Is, and the client's
/// acknowledgment of a COV notification (Clause 16.1).
#[tokio::test(start_paused = true)]
async fn a_disabled_node_still_answers() {
    let lab = dcc_lab(SessionConfig::default()).await;
    let (node, other) = (Wire::mac(NODE), Wire::mac(OTHER));
    control(&lab, EnableDisable::DISABLE_INITIATION, None).await;

    assert_eq!(read_node(&lab.other).await, 42.0);
    lab.other.who_is(Broadcast::Global, None).await.unwrap();
    eventually(WAIT, "the node's I-Am answer", async || {
        lab.other
            .discovered_devices()
            .await
            .iter()
            .any(|device| device.object_identifier.instance_number() == NODE_DEVICE)
    })
    .await;
    lab.wire
        .deliver(&node, wire::frame(&other, cov_notification(9), false));
    eventually(WAIT, "the client's acknowledgment", async || {
        sent_to(&lab.node_sent, &other).iter().any(|apdu| {
            matches!(apdu, Apdu::SimpleAck(ack)
                if ack.invoke_id == 9
                    && ack.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION)
        })
    })
    .await;
    stop(lab).await;
}

/// The control's duration running out, and an accepted WARMSTART, each
/// enable both roles again.
#[tokio::test(start_paused = true)]
async fn the_duration_and_a_restart_enable_the_node_again() {
    let lab = dcc_lab(SessionConfig::default()).await;
    control(&lab, EnableDisable::DISABLE_INITIATION, Some(1)).await;
    assert!(disabled(client_reads(&lab.client).await));
    tokio::time::sleep(Duration::from_secs(61)).await;
    client_reads(&lab.client).await.unwrap();

    control(&lab, EnableDisable::DISABLE_INITIATION, None).await;
    assert!(disabled(client_reads(&lab.client).await));
    lab.other
        .reinitialize_device(&Wire::mac(NODE), ReinitializedState::WARMSTART, None)
        .await
        .unwrap();
    client_reads(&lab.client).await.unwrap();
    stop(lab).await;
}

/// A client started while the node is disabled starts disabled, and one
/// its operator starts outside the node's control keeps initiating.
#[tokio::test(start_paused = true)]
async fn an_operator_override() {
    let Lab {
        mut session,
        client,
        mut peer,
        mut other,
        node_link,
        ..
    } = dcc_lab(SessionConfig::default()).await;
    other
        .device_communication_control(
            &Wire::mac(NODE),
            EnableDisable::DISABLE_INITIATION,
            None,
            None,
        )
        .await
        .unwrap();
    drop(client);
    let late = session.full_client().unwrap();
    assert!(disabled(client_reads(&late).await));
    drop(late);

    let exempt = session
        .full_client_with(ClientOptions::default().with_node_dcc_override(true))
        .unwrap();
    client_reads(&exempt).await.unwrap();
    drop(exempt);
    session.stop().await.unwrap();
    assert_eq!(node_link.stops(), 1);
    other.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// A request already waiting when the node is disabled gets no retry: it
/// ends with the typed error when its retry falls due, as the server's own
/// notifications do.
#[tokio::test(start_paused = true)]
async fn a_waiting_request_loses_its_retry() {
    let lab = dcc_lab(SessionConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 2,
        ..SessionConfig::default()
    })
    .await;
    let silent = Wire::mac(9);
    let read =
        Box::pin(
            lab.client
                .read_property(&silent, ai1(), PropertyIdentifier::PRESENT_VALUE, None),
        );
    let disable = Box::pin(async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        control(&lab, EnableDisable::DISABLE_INITIATION, None).await;
    });
    let (result, ()) = tokio::join!(read, disable);
    assert!(
        matches!(result, Err(Error::InitiationDisabled { sent: true })),
        "{result:?}"
    );
    let sent = lab
        .node_sent
        .frames()
        .into_iter()
        .filter(|frame| frame.mac == silent)
        .count();
    assert_eq!(sent, 1, "the request went once, and no retry followed");
    assert_eq!(lab.session.active_leases(), 0);
    stop(lab).await;
}

/// Whether `services`, a Protocol_Services_Supported bit string's octets,
/// lists `service`.
fn lists(services: &[u8], service: ServiceSupported) -> bool {
    let bit = usize::from(service.to_raw());
    services
        .get(bit / 8)
        .is_some_and(|octet| octet & (0x80 >> (bit % 8)) != 0)
}

/// A Device property of the node, as the other client reads it.
async fn node_device(lab: &Lab, property: PropertyIdentifier) -> PropertyValue {
    let ack = lab
        .other
        .read_property(
            &Wire::mac(NODE),
            ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap(),
            property,
            None,
        )
        .await
        .unwrap();
    bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0)
        .unwrap()
        .0
}

/// The node's Device's Protocol_Services_Supported as the other client
/// reads it by ReadRange of a Group whose one member it is.
async fn services_by_group_read_range(lab: &Lab) -> Vec<u8> {
    use bacnet_objects::group::GroupObject;
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    let mut group = GroupObject::new(7, "GRP-7").unwrap();
    group
        .add_member(ReadAccessSpecification {
            object_identifier: device,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
                property_array_index: None,
            }],
        })
        .unwrap();
    let added = lab.server().database().write().await.add(Box::new(group));
    added.unwrap();
    let ack = lab
        .other
        .read_range(
            &Wire::mac(NODE),
            ObjectIdentifier::new(ObjectType::GROUP, 7).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            None,
        )
        .await
        .unwrap();
    let rows = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack.item_data).unwrap();
    let [result] = rows.list_of_read_access_results.as_slice() else {
        panic!("{rows:?}")
    };
    assert_eq!(result.object_identifier, device);
    let value = result.list_of_results[0].property_value.as_deref().unwrap();
    match bacnet_encoding::primitives::decode_application_value(value, 0)
        .unwrap()
        .0
    {
        PropertyValue::BitString { data, .. } => data,
        other => panic!("{other:?}"),
    }
}

/// The node's Device lists what both roles execute, by ReadProperty and
/// through a Group's ReadRange alike, and states the one timing both use,
/// the session's.
#[tokio::test(start_paused = true)]
async fn the_node_device_states_both_roles() {
    let lab = dcc_lab(SessionConfig {
        apdu_timeout_ms: 2_500,
        apdu_retries: 2,
        ..SessionConfig::default()
    })
    .await;
    let PropertyValue::BitString { data, .. } =
        node_device(&lab, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED).await
    else {
        panic!("services")
    };
    let by_range = Box::pin(services_by_group_read_range(&lab)).await;
    for service in [
        ServiceSupported::I_AM,
        ServiceSupported::I_HAVE,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION,
        ServiceSupported::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
        ServiceSupported::READ_PROPERTY_MULTIPLE,
    ] {
        assert!(lists(&data, service), "{service:?}");
        assert!(lists(&by_range, service), "ReadRange: {service:?}");
    }
    assert_eq!(by_range, data, "ReadRange and ReadProperty agree");
    assert_eq!(
        node_device(&lab, PropertyIdentifier::APDU_TIMEOUT).await,
        PropertyValue::Unsigned(2_500)
    );
    assert_eq!(
        node_device(&lab, PropertyIdentifier::NUMBER_OF_APDU_RETRIES).await,
        PropertyValue::Unsigned(2)
    );
    stop(lab).await;
}

/// The node's settings have one place, the session's Device identity: a
/// session without one builds no full server, and a builder's other
/// identity is refused rather than replaced, both before the link starts.
#[tokio::test(start_paused = true)]
async fn the_node_settings_have_one_place() {
    let wire = Wire::new();
    let mut anonymous = EndpointSession::new(
        wire.port(NODE).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .with_server(BACnetServer::generic_builder());
    let error = anonymous
        .start()
        .await
        .expect_err("no identity, no full server");
    assert!(error.to_string().contains("identity"), "{error}");

    let mut session = EndpointSession::new(
        wire.port(PEER).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .device(DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
    .with_server(
        BACnetServer::generic_builder().device(DeviceIdentity::new(NODE_DEVICE + 1, 42).unwrap()),
    );
    let error = session
        .start()
        .await
        .expect_err("another identity is refused");
    assert!(error.to_string().contains("Device identity"), "{error}");
}

/// On a link that takes 480 octets, as every MS/TP node's does, a builder
/// given the session's own identity is the node's identity, not a
/// contradiction: both are compared as the link serves them, and without
/// the service list, which the full server builds itself.
#[tokio::test(start_paused = true)]
async fn the_sessions_own_identity_is_taken_on_a_small_link() {
    let wire = Wire::new();
    let identity = DeviceIdentity::new(NODE_DEVICE, 42).unwrap();
    let mut session = EndpointSession::new(
        wire.port(NODE).receive_capacity(480).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .device(identity.clone())
    .with_server(
        BACnetServer::generic_builder().device(identity.clone().with_services(&[
            ServiceSupported::READ_PROPERTY,
            ServiceSupported::WRITE_PROPERTY,
        ])),
    );
    session.start().await.expect("the session's own identity");
    let accepted = session
        .server()
        .unwrap()
        .read_local(
            &identity.device_oid(),
            PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
            None,
        )
        .await
        .unwrap();
    assert_eq!(accepted, PropertyValue::Unsigned(480));
    session.stop().await.unwrap();
}

/// A Who-Is first asks this network's number (#2572). If the node is
/// disabled while that question is going out, the Who-Is that waited on it
/// is not sent: the client checks its seat again after the question.
#[tokio::test(start_paused = true)]
async fn a_who_is_behind_the_number_question_is_held_back() {
    let wire = Wire::new();
    let other = BACnetClient::generic_builder()
        .transport(wire.port(OTHER).build())
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    // The node's link holds its What-Is-Network-Number until released, and
    // carries every other frame as the wire does.
    let asking = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let node_mac = Wire::mac(NODE);
    let port = {
        let (wire, asking, release) = (wire.clone(), Arc::clone(&asking), Arc::clone(&release));
        let source = node_mac.clone();
        wire.port(NODE).number_controls().on_send(move |sent| {
            let question = decode_npdu(sent.npdu.clone()).is_ok_and(|npdu| {
                npdu.message_type == Some(bacnet_npdu::NetworkMessageType::WHAT_IS_NETWORK_NUMBER)
            });
            let (wire, asking, release, source) = (
                wire.clone(),
                Arc::clone(&asking),
                Arc::clone(&release),
                source.clone(),
            );
            async move {
                if question {
                    asking.notify_one();
                    release.notified().await;
                }
                if sent.broadcast {
                    wire.broadcast(&source, sent.npdu);
                } else {
                    wire.deliver(&sent.mac, wire::frame(&source, sent.npdu, false));
                }
                Ok(())
            }
        })
    };
    let node_sent = port.sent();
    let mut session =
        EndpointSession::new(port.build(), SessionRole::Both, SessionConfig::default())
            .unwrap()
            .device(DeviceIdentity::new(NODE_DEVICE, 42).unwrap())
            .with_server(
                BACnetServer::generic_builder()
                    .dcc_policy(bacnet_server::server::DccPolicy::LegacyPermissive),
            );
    session.start().await.unwrap();
    let client = session.full_client().unwrap();

    let who_is = Box::pin(client.who_is(Broadcast::Global, None));
    let disable = Box::pin(async {
        asking.notified().await;
        other
            .device_communication_control(&node_mac, EnableDisable::DISABLE_INITIATION, None, None)
            .await
            .unwrap();
        release.notify_one();
    });
    let (result, ()) = tokio::join!(who_is, disable);
    assert!(
        matches!(result, Err(Error::InitiationDisabled { sent: false })),
        "{result:?}"
    );
    let who_is_sent = node_sent.frames().into_iter().any(|frame| {
        decode_npdu(frame.npdu)
            .ok()
            .and_then(|npdu| decode_apdu(npdu.payload).ok())
            .is_some_and(|apdu| {
                matches!(apdu, Apdu::UnconfirmedRequest(request)
                if request.service_choice == bacnet_types::enums::UnconfirmedServiceChoice::WHO_IS)
            })
    });
    assert!(!who_is_sent, "the Who-Is went out");
    drop(client);
    session.stop().await.unwrap();
}
