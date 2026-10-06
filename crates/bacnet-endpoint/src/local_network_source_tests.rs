//! Source Audit records follow the same rule (#1403). Once the session knows
//! its network's number, an Audit_Notification_Recipient address naming that
//! number is a station on this link: records go to its MAC with no DNET, and
//! an audited read routed to that network is audited as the direct read it
//! is. An address on another network, or one naming this network while the
//! number is unknown, still has no route.
use super::*;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::{BACnetAddress, BACnetAuditNotification, BACnetRecipient};
use bacnet_types::enums::{AuditLevel, AuditOperation, UnconfirmedServiceChoice};

/// Where the bound Device recipient the session starts with is, and where
/// the tests move the recipient to.
const SINK: u8 = 5;
const NEW_SINK: u8 = 6;

fn address(network: u16, station: u8) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: network,
        mac_address: mac(station),
    })
}

/// A started source session of `role` that reports READ operations,
/// unconfirmed, to Device 999 bound at `SINK`, and has learned
/// `THIS_NETWORK` or no number at all.
async fn source(role: SessionRole, learned: bool) -> Endpoint {
    source_reporting_to(
        role,
        learned,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)),
    )
    .await
}

/// [`source`], with `recipient` provisioned as the one it starts with.
async fn source_reporting_to(
    role: SessionRole,
    learned: bool,
    recipient: BACnetRecipient,
) -> Endpoint {
    let mut endpoint = unstarted_source(role, recipient, None);
    endpoint.start(learned).await;
    endpoint
}

/// The source session [`source_reporting_to`] starts. With `port_number`,
/// it registers a B/IP Network Port configured with that network number,
/// which is the session's number from startup.
fn unstarted_source(
    role: SessionRole,
    recipient: BACnetRecipient,
    port_number: Option<u16>,
) -> Endpoint {
    let device = oid(ObjectType::DEVICE, 123);
    let mut identity = crate::DeviceIdentity::new(123, 42).unwrap();
    if let Some(number) = port_number {
        identity = identity
            .with_bip_port(2, number.into(), *host(SELF).ip(), PORT)
            .unwrap();
    }
    let mut db = identity.build_database().unwrap();
    db.get_mut(&device)
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(recipient)
        .unwrap();
    let mut reporter = AuditReporterObject::new(1, "Source").unwrap();
    reporter
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(1).unwrap(),
            false,
            None,
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
    db.add(Box::new(reporter)).unwrap();
    let mut endpoint = Endpoint::new(role);
    let mut session = endpoint
        .session
        .with_database(db)
        .with_source_audit_reporter(oid(ObjectType::AUDIT_REPORTER, 1));
    if role == SessionRole::Both {
        session = session.with_device_writes(Arc::new(|_| true));
    }
    if port_number.is_some() {
        session = session
            .with_identity(identity)
            .with_registered_network_port(oid(ObjectType::NETWORK_PORT, 2));
    }
    session
        .source_audit_bindings
        .push((oid(ObjectType::DEVICE, 999), host(SINK)));
    endpoint.session = session;
    endpoint
}

/// The unconfirmed Audit record in `sent`, checked to have gone straight to
/// `station` with no DNET.
fn record(sent: Sent, station: u8) -> BACnetAuditNotification {
    assert_eq!(sent.link, mac(station), "sent to the recipient itself");
    let npdu = decode_npdu(sent.npdu).unwrap();
    assert_eq!(npdu.destination, None, "sent with no DNET");
    let Apdu::UnconfirmedRequest(pdu) = decode_apdu(npdu.payload).unwrap() else {
        panic!("an unconfirmed notification")
    };
    assert_eq!(
        pdu.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
    );
    let mut request = AuditNotificationRequest::decode(&pdu.service_request).unwrap();
    assert_eq!(request.notifications.len(), 1);
    request.notifications.remove(0)
}

/// Complete the audited read the session sends next, sent straight to the
/// peer, and return the record it gave `station`.
async fn audited_read(
    endpoint: &mut Endpoint,
    destination: EndpointApduDestination,
    station: u8,
) -> BACnetAuditNotification {
    let read = endpoint.read(destination);
    let (route, request) = endpoint.request().await;
    assert_eq!(route, local(PEER));
    endpoint
        .deliver_apdu(ack(&request, 5), mac(PEER), None)
        .await;
    let answer = bounded(read).await.unwrap().unwrap();
    assert_eq!(answer.property_value, [0x21, 5]);
    let audit = record(endpoint.next().await, station);
    assert_eq!(audit.operation, AuditOperation::READ);
    assert_eq!(audit.invoke_id, Some(request.invoke_id));
    assert_eq!(audit.target_device, address(0, PEER));
    audit
}

#[tokio::test]
async fn source_records_for_an_address_on_this_network_go_without_a_dnet() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let mut endpoint = source(role, true).await;
        let next = address(THIS_NETWORK, NEW_SINK);
        bounded(endpoint.session.write_audit_recipient(Some(next)))
            .await
            .unwrap();
        // The change is reported to the old recipient and to the new one.
        let mut pair = [endpoint.next().await, endpoint.next().await];
        pair.sort_by_key(|sent| sent.link.to_vec());
        let [old, new] = pair;
        assert_eq!(record(old, SINK), record(new, NEW_SINK), "{role:?}");
        // A read routed to this network goes to the peer itself and is
        // audited to the new recipient like any direct read.
        audited_read(&mut endpoint, routed_read(THIS_NETWORK), NEW_SINK).await;
        endpoint.stop().await;
    }
}

#[tokio::test]
async fn source_routes_refuse_another_network_and_an_unknown_number() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for learned in [true, false] {
            let mut endpoint = source(role, learned).await;
            let network = if learned {
                REMOTE_NETWORK
            } else {
                THIS_NETWORK
            };
            // Such an address has no route, so the change is refused unsent.
            let change = endpoint
                .session
                .write_audit_recipient(Some(address(network, NEW_SINK)));
            assert!(
                bounded(change).await.is_err(),
                "{role:?}, learned {learned}"
            );
            // An audited read routed there is not a direct one, so it is
            // refused before any send.
            let read = endpoint.read(routed_read(network));
            assert!(bounded(read).await.unwrap().is_err());
            // Nothing went out: the next frame is a direct read's request,
            // and its record still goes to the bound Device.
            let direct = EndpointApduDestination::Direct {
                destination_mac: mac(PEER),
            };
            audited_read(&mut endpoint, direct, SINK).await;
            endpoint.stop().await;
        }
    }
}

#[path = "local_network_recipient_tests.rs"]
mod recipient;

#[path = "durable_drop_tests.rs"]
mod durable_drop;
