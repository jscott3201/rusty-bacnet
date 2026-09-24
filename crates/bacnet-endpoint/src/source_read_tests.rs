use super::*;
use crate::bip::BipEndpointBuilder;
use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, AbortPdu, ComplexAck, ErrorPdu, RejectPdu, SimpleAck,
};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bvll::{decode_bip_mac, encode_bip_mac};
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::{BACnetAuditNotification, BACnetObjectSelector, BACnetRecipient};
use bacnet_types::enums::{
    AbortReason, AuditLevel, AuditOperation, ConfirmedServiceChoice, ErrorClass, ErrorCode,
    NetworkPriority, RejectReason, Reliability, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::BACnetTimeStamp;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::time::{timeout, Duration};

const WAIT: Duration = Duration::from_secs(2);
fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}
fn selected() -> ObjectIdentifier {
    oid(ObjectType::AUDIT_REPORTER, 1)
}
fn target() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 7)
}
fn database(confirmed: bool) -> ObjectDatabase {
    let mut db = crate::DeviceIdentity::new(123, 42)
        .unwrap()
        .build_database()
        .unwrap();
    let mut reporter = AuditReporterObject::new(1, "Source").unwrap();
    reporter
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(1).unwrap(),
            confirmed,
            None,
            BACnetPriorityFilter::empty(),
        )
        .unwrap();
    db.add(Box::new(reporter)).unwrap();
    db
}
async fn network() -> (NetworkLayer<BipTransport>, mpsc::Receiver<ReceivedApdu>) {
    let mut network = NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    ));
    let receiver = network.start().await.unwrap();
    (network, receiver)
}
fn address(network: &NetworkLayer<BipTransport>) -> SocketAddrV4 {
    let (ip, port) = decode_bip_mac(network.local_mac()).unwrap();
    SocketAddrV4::new(ip.into(), port)
}
fn session(
    mut db: ObjectDatabase,
    role: SessionRole,
    sink: &NetworkLayer<BipTransport>,
) -> EndpointSession<BipTransport> {
    db.get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)))
        .unwrap();
    let session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(role)
        .database(db)
        .client_timers(100, 0)
        .source_audit_device_binding(oid(ObjectType::DEVICE, 999), address(sink))
        .build_session()
        .unwrap()
        .with_source_audit_reporter(selected());
    if role == SessionRole::Both {
        session.with_device_writes(Arc::new(|_| true))
    } else {
        session
    }
}
async fn receive(receiver: &mut mpsc::Receiver<ReceivedApdu>) -> ReceivedApdu {
    timeout(WAIT, receiver.recv()).await.unwrap().unwrap()
}
async fn send(network: &NetworkLayer<BipTransport>, mac: &[u8], pdu: Apdu) {
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, &pdu).unwrap();
    network
        .send_apdu(&bytes, mac, false, NetworkPriority::NORMAL)
        .await
        .unwrap();
}
fn ack(invoke: u8, request: &ReadPropertyRequest) -> Apdu {
    let mut service = BytesMut::new();
    ReadPropertyACK {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        property_value: vec![0x21, 42],
    }
    .encode(&mut service);
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: invoke,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: service.freeze(),
    })
}
fn read_request(envelope: &ReceivedApdu) -> (u8, ReadPropertyRequest) {
    let Apdu::ConfirmedRequest(request) = decode_apdu(envelope.apdu.clone()).unwrap() else {
        panic!("RP request")
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::READ_PROPERTY
    );
    (
        request.invoke_id,
        ReadPropertyRequest::decode(&request.service_request).unwrap(),
    )
}
fn notification(envelope: &ReceivedApdu, confirmed: bool) -> (BACnetAuditNotification, Option<u8>) {
    let (bytes, invoke) = match decode_apdu(envelope.apdu.clone()).unwrap() {
        Apdu::ConfirmedRequest(pdu) if confirmed => {
            assert_eq!(
                pdu.service_choice,
                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
            );
            (pdu.service_request, Some(pdu.invoke_id))
        }
        Apdu::UnconfirmedRequest(pdu) if !confirmed => {
            assert_eq!(
                pdu.service_choice,
                UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
            );
            (pdu.service_request, None)
        }
        other => panic!("unexpected notification {other:?}"),
    };
    let mut request = AuditNotificationRequest::decode(&bytes).unwrap();
    assert_eq!(request.notifications.len(), 1);
    (request.notifications.remove(0), invoke)
}
async fn reliability(session: &EndpointSession<BipTransport>) -> u32 {
    let db = session.database.as_ref().unwrap().read().await;
    let PropertyValue::Enumerated(value) = db
        .get(&selected())
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
    else {
        panic!()
    };
    value
}
fn start_read(
    session: &EndpointSession<BipTransport>,
    target_mac: &[u8],
    property: PropertyIdentifier,
    index: Option<u32>,
) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
    let handle = session.cloned_client_handle().unwrap();
    let mac = target_mac.to_vec();
    tokio::spawn(async move { handle.read_property(&mac, target(), property, index).await })
}

#[tokio::test]
async fn source_read_wire_fields_both_roles_modes_and_result_before_delivery() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for confirmed in [false, true] {
            let mut session = session(database(confirmed), role, &sink);
            session.start().await.unwrap();
            assert!(records.try_recv().is_err());
            // Reserve a competing request ID to prove record IDs are not guessed.
            let lease = session
                .coordinator
                .reserve(
                    bacnet_endpoint_core::coordinator::LeaseMetadata::notification(
                        bacnet_endpoint_core::coordinator::CanonicalPeer::direct(peer.local_mac()),
                        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                    ),
                )
                .unwrap();
            let read = start_read(
                &session,
                peer.local_mac(),
                PropertyIdentifier::OBJECT_NAME,
                Some(2),
            );
            let request = receive(&mut requests).await;
            let (invoke, rp) = read_request(&request);
            assert_ne!(invoke, lease.invoke_id());
            send(&peer, &request.source_mac, ack(invoke, &rp)).await;
            assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
            // Confirmed audit ACK deliberately withheld until after caller result.
            let envelope = receive(&mut records).await;
            let (record, notification_invoke) = notification(&envelope, confirmed);
            assert_eq!(
                record.source_timestamp,
                Some(BACnetTimeStamp::SequenceNumber(0))
            );
            assert_eq!(
                record.source_device,
                BACnetRecipient::Device(oid(ObjectType::DEVICE, 123))
            );
            let BACnetRecipient::Address(address) = record.target_device else {
                panic!("no invented target Device")
            };
            assert_eq!(address.network_number, 0);
            assert_eq!(address.mac_address.as_slice(), peer.local_mac());
            assert_eq!(record.target_object, Some(target()));
            assert_eq!(record.invoke_id, Some(invoke));
            assert_eq!(record.operation, AuditOperation::READ);
            let property = record.target_property.unwrap();
            assert_eq!(
                property.property_identifier,
                PropertyIdentifier::OBJECT_NAME
            );
            assert_eq!(property.property_array_index, Some(2));
            assert_eq!(record.result, None);
            assert!(
                record.target_timestamp.is_none()
                    && record.source_object.is_none()
                    && record.source_comment.is_none()
                    && record.target_comment.is_none()
                    && record.source_user_id.is_none()
                    && record.source_user_role.is_none()
                    && record.target_priority.is_none()
                    && record.target_value.is_none()
                    && record.current_value.is_none()
            );
            if let Some(invoke_id) = notification_invoke {
                assert_ne!(invoke_id, lease.invoke_id());
                send(
                    &sink,
                    &envelope.source_mac,
                    Apdu::SimpleAck(SimpleAck {
                        invoke_id,
                        service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                    }),
                )
                .await;
            }
            session.coordinator.cancel(lease).unwrap();
            session.stop().await.unwrap();
            assert_eq!(session.coordinator.active_count().unwrap(), 0);
        }
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_peer_and_local_failure_outcomes_are_not_success() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    for case in 0..7 {
        let read = start_read(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let envelope = receive(&mut requests).await;
        let (invoke_id, mut rp) = read_request(&envelope);
        let (reply, expected) = match case {
            0 => (
                Some(Apdu::Error(ErrorPdu {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    error_class: ErrorClass::PROPERTY,
                    error_code: ErrorCode::UNKNOWN_PROPERTY,
                    error_data: Bytes::new(),
                })),
                (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            ),
            1 => (
                Some(Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::INVALID_PARAMETER_DATA_TYPE,
                })),
                (
                    ErrorClass::COMMUNICATION,
                    ErrorCode::REJECT_INVALID_PARAMETER_DATA_TYPE,
                ),
            ),
            2 => (
                Some(Apdu::Abort(AbortPdu {
                    invoke_id,
                    sent_by_server: true,
                    abort_reason: AbortReason::OUT_OF_RESOURCES,
                })),
                (ErrorClass::COMMUNICATION, ErrorCode::ABORT_OUT_OF_RESOURCES),
            ),
            3 => (None, (ErrorClass::COMMUNICATION, ErrorCode::TIMEOUT)),
            4 => {
                rp.property_array_index = Some(9);
                (
                    Some(ack(invoke_id, &rp)),
                    (ErrorClass::COMMUNICATION, ErrorCode::OTHER),
                )
            }
            5 => (
                Some(Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    service_ack: Bytes::from_static(&[0]),
                })),
                (ErrorClass::COMMUNICATION, ErrorCode::OTHER),
            ),
            _ => (
                Some(Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::from_raw(200),
                })),
                (ErrorClass::COMMUNICATION, ErrorCode::REJECT_PROPRIETARY),
            ),
        };
        if let Some(reply) = reply {
            send(&peer, &envelope.source_mac, reply).await;
        }
        assert!(read.await.unwrap().is_err());
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.result, Some(expected));
        assert_eq!(record.invoke_id, Some(invoke_id));
        assert_eq!(
            record.source_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(case))
        );
    }
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_dropped_waiter_retries_once_per_logical_request() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::Both, &sink);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let read = start_read(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first = receive(&mut requests).await;
    let (invoke, rp) = read_request(&first);
    read.abort();
    let _ = read.await;
    let second = receive(&mut requests).await;
    assert_eq!(read_request(&second).0, invoke);
    send(&peer, &second.source_mac, ack(invoke, &rp)).await;
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    assert_eq!(
        record.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    assert!(timeout(Duration::from_millis(30), records.recv())
        .await
        .is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_selector_preflight_is_atomic_and_adapter_rejects_mutation() {
    let (mut sink, _records) = network().await;
    for selectors in [vec![], vec![BACnetObjectSelector::None]] {
        let mut db = database(false);
        db.get_mut(&selected())
            .unwrap()
            .configure_audit_reporter_internal(
                AuditLevel::AUDIT_ALL,
                AuditOperationFlags::from_bits(1).unwrap(),
                false,
                Some(selectors),
                BACnetPriorityFilter::all(),
            )
            .unwrap();
        let mut session = session(db, SessionRole::ClientOnly, &sink);
        assert!(session.start().await.is_err());
        assert_eq!(
            session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Ready as u8
        );
        let db = Arc::get_mut(session.database.as_mut().unwrap())
            .unwrap()
            .get_mut();
        let object = db.get_mut(&selected()).unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        object
            .configure_audit_reporter_internal(
                AuditLevel::AUDIT_ALL,
                AuditOperationFlags::from_bits(1).unwrap(),
                false,
                None,
                BACnetPriorityFilter::all(),
            )
            .unwrap();
        session.start().await.unwrap();
        {
            let mut db = session.database.as_ref().unwrap().write().await;
            let object = db.get_mut(&selected()).unwrap();
            assert!(object
                .configure_audit_reporter_internal(
                    AuditLevel::NONE,
                    AuditOperationFlags::empty(),
                    true,
                    Some(vec![]),
                    BACnetPriorityFilter::empty()
                )
                .is_err());
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::AUDIT_LEVEL, None)
                    .unwrap(),
                PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw())
            );
        }
        session.stop().await.unwrap();
    }
    sink.stop().await.unwrap();
}

#[path = "source_read_lifecycle_tests.rs"]
mod lifecycle;

#[path = "source_read_queue_tests.rs"]
mod queued;

#[path = "source_read_failure_tests.rs"]
mod failures;

#[path = "source_recipient_tests.rs"]
mod recipient_changes;

#[path = "source_range_tests.rs"]
mod range;

#[path = "source_property_identity_tests.rs"]
mod property_identity;

#[path = "source_rpm_tests.rs"]
mod rpm;
