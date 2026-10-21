//! An Address recipient naming this device's own network number is local
//! once the server knows that number (#1460), as a Notification Class
//! recipient's is (#1358): records go to its MAC with no DNET. While the
//! number is unknown it names a routed station, which target Audit does not
//! route, so it waits unresolved, reporting CONFIGURATION_ERROR as a Device
//! with no route does, and resolves from the first record after the number
//! is learned, its Reporter's health following each change of number at
//! once. When the number moves away, it no longer resolves, and a recipient
//! change goes ahead: the new recipient gets its copy, and a global
//! broadcast stands in for the old one's (Clause 12.11.66).
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};

/// The link's B/IP broadcast endpoint.
const BROADCAST: std::net::SocketAddrV4 =
    std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(10, 0, 0, 255), 0xBAC0);
/// The logger at the recipient address, and the one a change moves to.
const STATION: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];
const NEW_STATION: [u8; 6] = [10, 0, 0, 4, 0xBA, 0xC0];

fn address(network: u16, mac: &[u8]) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: network,
        mac_address: MacAddr::from_slice(mac),
    })
}

/// A started B/IP-shaped server whose Device 10 reports WRITE operations
/// to `recipient`, confirmed or not, and has not learned its number.
async fn address_logger(recipient: BACnetRecipient, confirmed: bool) -> Result<Fixture, Error> {
    let mut reporter = reporter();
    reporter
        .set_issue_confirmed_notifications(confirmed)
        .unwrap();
    try_servers_config(
        vec![reporter],
        &[10],
        Some(recipient),
        vec![],
        true,
        1476,
        bip_capture(),
    )
    .await
}

/// A B/IP-shaped capture link, with a broadcast endpoint, that records the
/// broadcasts it is handed.
fn bip_capture() -> AuditCapture {
    let mut capture = AuditCapture::default();
    capture.six_byte_mac = true;
    capture.bip_broadcast = Some(BROADCAST);
    capture.record_broadcasts = true;
    capture
}

/// The Audit records sent by global broadcast: unconfirmed, DNET 0xFFFF.
fn broadcast_records(fixture: &Fixture) -> Vec<BACnetAuditNotification> {
    let mut records = Vec::new();
    for bytes in fixture.transport.broadcasts.lock().unwrap().iter() {
        let npdu = decode_npdu(bytes.clone()).unwrap();
        let Ok(Apdu::UnconfirmedRequest(request)) = decode_apdu(npdu.payload) else {
            continue;
        };
        if request.service_choice != UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION {
            continue;
        }
        assert_eq!(npdu.destination.map(|to| to.network), Some(0xFFFF));
        let mut decoded =
            bacnet_services::audit::AuditNotificationRequest::decode(&request.service_request)
                .unwrap();
        records.append(&mut decoded.notifications);
    }
    records
}

/// Where each record went: its link MAC, and whether the NPDU had a DNET.
fn sends(fixture: &Fixture) -> Vec<(Vec<u8>, bool)> {
    let macs = fixture.transport.destinations.lock().unwrap().clone();
    let sent = fixture.transport.sent.lock().unwrap().clone();
    macs.into_iter()
        .zip(sent)
        .map(|(mac, bytes)| (mac, decode_npdu(bytes).unwrap().destination.is_some()))
        .collect()
}

/// Change Device 10's Audit_Notification_Recipient as the application.
async fn change_recipient(fixture: &Fixture, recipient: &BACnetRecipient) -> Result<(), Error> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut bytes, recipient).unwrap();
    fixture
        .server
        .write_local(
            &oid(ObjectType::DEVICE, 10),
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            PropertyValue::ApplicationData(bytes.to_vec()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

#[tokio::test(start_paused = true)]
async fn an_address_recipient_on_this_network_waits_for_the_number_then_goes_without_a_dnet() {
    let mut fixture = address_logger(address(THIS_NETWORK, &STATION), true)
        .await
        .expect("starts unresolved while the number is unknown");
    assert_eq!(
        health(&fixture.server).await,
        Reliability::CONFIGURATION_ERROR
    );
    // Unknown number: nothing goes out, and the write still succeeds.
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert!(sends(&fixture).is_empty());
    assert_eq!(
        health(&fixture.server).await,
        Reliability::CONFIGURATION_ERROR
    );
    // Known: the record goes straight to the station, and its ACK from
    // there with no SNET completes it.
    publish(&fixture, THIS_NETWORK);
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(sends(&fixture), [(STATION.to_vec(), false)]);
    let request = confirmed_notification(&fixture.transport.sent, 0);
    assert!(fixture.server.notification_transactions.admit_terminal(
        &STATION,
        None,
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        }),
    ));
    settle().await;
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    // Another number: the address is off this network again.
    publish(&fixture, REMOTE_NETWORK);
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(sends(&fixture).len(), 1, "no record off this network");
    assert_eq!(
        health(&fixture.server).await,
        Reliability::CONFIGURATION_ERROR
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_change_away_from_an_address_the_number_left_goes_to_the_new_recipient_and_by_broadcast()
{
    let mut fixture = address_logger(address(THIS_NETWORK, &STATION), false)
        .await
        .unwrap();
    // While the number is unknown, a new address on it has no route either.
    assert!(
        change_recipient(&fixture, &address(THIS_NETWORK, &NEW_STATION))
            .await
            .is_err()
    );
    publish(&fixture, THIS_NETWORK);
    publish(&fixture, REMOTE_NETWORK);
    // The old address no longer resolves; the change still goes ahead. The
    // new recipient gets its record, and the same record goes by global
    // broadcast in place of the old recipient's copy.
    let next = address(0, &NEW_STATION);
    change_recipient(&fixture, &next).await.unwrap();
    settle().await;
    assert_eq!(sends(&fixture), [(NEW_STATION.to_vec(), false)]);
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    let record = &records[0].notifications[0];
    assert_eq!(broadcast_records(&fixture), std::slice::from_ref(record));
    assert_eq!(record.operation, AuditOperation::WRITE);
    let mut old = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut old, &address(THIS_NETWORK, &STATION))
        .unwrap();
    assert_eq!(record.current_value.as_deref(), Some(&old[..]));
    // Ordinary records follow the new recipient.
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(sends(&fixture).len(), 2);
    assert_eq!(sends(&fixture)[1], (NEW_STATION.to_vec(), false));
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_change_away_from_a_device_with_no_route_is_still_refused() {
    // Only an Address the number moved away from is passed over: an old
    // Device recipient with no route still holds a change up.
    let mut fixture = try_servers_config(
        vec![reporter()],
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999))),
        vec![],
        true,
        1476,
        bip_capture(),
    )
    .await
    .unwrap();
    publish(&fixture, THIS_NETWORK);
    assert!(change_recipient(&fixture, &address(0, &NEW_STATION))
        .await
        .is_err());
    settle().await;
    assert!(sends(&fixture).is_empty());
    fixture.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_change_between_recipients_that_both_resolve_sends_no_broadcast() {
    let mut fixture = address_logger(address(THIS_NETWORK, &STATION), false)
        .await
        .unwrap();
    publish(&fixture, THIS_NETWORK);
    change_recipient(&fixture, &address(0, &NEW_STATION))
        .await
        .unwrap();
    settle().await;
    assert_eq!(
        sends(&fixture),
        [(STATION.to_vec(), false), (NEW_STATION.to_vec(), false)]
    );
    assert!(broadcast_records(&fixture).is_empty());
    fixture.server.stop().await.unwrap();
}

/// Health follows the network's number as the server's Number worker takes
/// each announcement, before any audited operation.
#[tokio::test(start_paused = true)]
async fn the_reporters_health_follows_each_change_of_number() {
    let (inbound, incoming) = mpsc::channel(8);
    let mut capture = bip_capture();
    capture.number_controls = true;
    *capture.incoming.lock().unwrap() = Some(incoming);
    let mut fixture = try_servers_config(
        vec![reporter()],
        &[10],
        Some(address(THIS_NETWORK, &STATION)),
        vec![],
        true,
        1476,
        capture,
    )
    .await
    .unwrap();
    assert_eq!(
        health(&fixture.server).await,
        Reliability::CONFIGURATION_ERROR
    );
    for (number, expected) in [
        (THIS_NETWORK, Reliability::NO_FAULT_DETECTED),
        (REMOTE_NETWORK, Reliability::CONFIGURATION_ERROR),
    ] {
        let [high, low] = number.to_be_bytes();
        inbound
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: Bytes::copy_from_slice(&[1, 0x80, 0x13, high, low, 1]),
                source_mac: MacAddr::from_slice(&NEW_STATION),
                link_layer_group: true,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
        let mut reached = false;
        for _ in 0..50 {
            settle().await;
            if health(&fixture.server).await == expected {
                reached = true;
                break;
            }
        }
        assert!(reached, "{number}: health follows the number");
        let published = fixture.server.test_network().local_network_number().get();
        assert_eq!(published, Some(number));
    }
    assert!(sends(&fixture).is_empty(), "no operation took place");
    fixture.server.stop().await.unwrap();
}

/// A registered Network Port publishes its number before target Audit
/// validates the recipient. An address on another network starts unresolved
/// too: the number may still come to name it.
#[tokio::test(start_paused = true)]
async fn an_address_off_a_number_known_at_startup_also_starts_unresolved() {
    let port_ip = [127, 0, 0, 1];
    let mut capture = bip_capture();
    capture.normal_bip = Some(std::net::SocketAddrV4::new(port_ip.into(), 0xBAC0));
    let mut device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(10, 0).expect("valid Device identity"),
        ..Default::default()
    })
    .unwrap();
    device
        .provision_audit_recipient(address(REMOTE_NETWORK, &STATION))
        .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(reporter())).unwrap();
    db.add(Box::new(
        bacnet_objects::binary::BinaryValueObject::new(1, "value").unwrap(),
    ))
    .unwrap();
    let port_config = BipPortConfig {
        network_number: THIS_NETWORK,
        ip_address: port_ip,
        ..BipPortConfig::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "Port", port_config).unwrap(),
    ))
    .unwrap();
    let mut server = BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(10, 0).unwrap(),
            audit_reporters: Some(AuditReportersConfig {
                reporters: vec![oid(ObjectType::AUDIT_REPORTER, 1)],
            }),
            registered_network_port: Some(oid(ObjectType::NETWORK_PORT, 1)),
            ..Default::default()
        },
        db,
        capture.port(),
        None,
        vec![],
    )
    .await
    .expect("starts unresolved");
    assert_eq!(
        server.test_network().local_network_number().get(),
        Some(THIS_NETWORK)
    );
    assert_eq!(health(&server).await, Reliability::CONFIGURATION_ERROR);
    assert!(matches!(
        write_value(&server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert!(capture.sent.lock().unwrap().is_empty());
    server.stop().await.unwrap();
}
