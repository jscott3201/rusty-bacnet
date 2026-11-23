//! Target Audit sends no record to a group address of the link (#1493). A
//! Device route whose next hop the started link reports as one is pruned at
//! startup, as one at its broadcast MAC is, and an Address recipient at one,
//! such as the B/IP broadcast IP at another port, has no route: it is refused
//! at startup and as a change.
use super::*;
use bacnet_types::error::RemoteError;

/// A MAC the generic link reports as a group destination once started.
const GROUP: &[u8] = &[0x43];
/// The B/IP link's broadcast endpoint, and its broadcast IP at another
/// port: a group destination that isn't the link's broadcast.
const BROADCAST: std::net::SocketAddrV4 =
    std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(10, 0, 0, 255), 0xBAC0);
const BROADCAST_IP_ELSEWHERE: [u8; 6] = [10, 0, 0, 255, 0xBA, 0xC1];
const STATION: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];

fn address(mac: &[u8]) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(mac),
    })
}

#[tokio::test(start_paused = true)]
async fn a_device_route_to_a_learned_group_address_is_pruned() {
    for routed in [false, true] {
        let recipient = oid(ObjectType::DEVICE, 20);
        let binding = if routed {
            DeviceBinding::routed(recipient, 200, [9], GROUP)
        } else {
            DeviceBinding::local(recipient, GROUP)
        };
        let mut capture = AuditCapture::default();
        capture.learned_group = Some(MacAddr::from_slice(GROUP));
        let mut fixture = try_servers_config(
            vec![reporter()],
            &[10],
            Some(BACnetRecipient::Device(recipient)),
            vec![binding.unwrap()],
            true,
            1476,
            capture,
        )
        .await
        .expect("an unresolved Device recipient still starts");
        fixture
            .transport
            .reject_route_callbacks
            .store(true, Ordering::Release);
        assert_eq!(
            health(&fixture.server).await,
            Reliability::CONFIGURATION_ERROR,
            "routed {routed}"
        );
        assert!(matches!(
            write_value(&fixture.server, None).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        fixture.server.stop().await.unwrap();
    }
}

/// A B/IP-shaped link whose broadcast endpoint is [`BROADCAST`] and whose
/// group rule takes in [`BROADCAST_IP_ELSEWHERE`].
fn bip_capture() -> AuditCapture {
    let mut capture = AuditCapture::default();
    capture.six_byte_mac = true;
    capture.bip_broadcast = Some(BROADCAST);
    capture.group_macs = vec![MacAddr::from_slice(&BROADCAST_IP_ELSEWHERE)];
    capture
}

async fn address_logger(mac: &[u8]) -> Result<Fixture, Error> {
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    try_servers_config(
        vec![reporter],
        &[10],
        Some(address(mac)),
        vec![],
        true,
        1476,
        bip_capture(),
    )
    .await
}

#[tokio::test(start_paused = true)]
async fn an_address_recipient_at_a_group_address_is_refused() {
    let Err(error) = address_logger(&BROADCAST_IP_ELSEWHERE).await else {
        panic!("a server started with an Address recipient at a group address");
    };
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { .. })),
        "{error}"
    );

    let mut fixture = address_logger(&STATION).await.expect("a station starts");
    let change = |mac: &[u8]| {
        let mut bytes = BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(&mut bytes, &address(mac)).unwrap();
        PropertyValue::ApplicationData(bytes.to_vec())
    };
    let device = oid(ObjectType::DEVICE, 10);
    let recipient = PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT;
    let source = || crate::LocalCommandSource::ServerDevice;
    let server = &fixture.server;
    assert!(
        server
            .write_local(
                &device,
                recipient,
                None,
                change(&BROADCAST_IP_ELSEWHERE),
                None,
                source()
            )
            .await
            .is_err()
    );
    let mut station = STATION;
    station[3] = 4;
    server
        .write_local(&device, recipient, None, change(&station), None, source())
        .await
        .expect("a change to another station goes ahead");
    settle().await;
    let sent = fixture.transport.destinations.lock().unwrap().clone();
    assert!(
        sent.iter().all(|mac| mac[..] != BROADCAST_IP_ELSEWHERE),
        "{sent:?}"
    );
    fixture.server.stop().await.unwrap();
}

/// #2732: a Device recipient bound through a router at a broadcast or group
/// form on its network keeps its route for an unconfirmed reporter, whose
/// record still goes there through the router. A confirmed reporter has no
/// route to it: it reads a configuration error from the start, as for a
/// binding at a group address of the link (#1493), and sends nothing.
#[tokio::test(start_paused = true)]
async fn a_device_bound_at_a_remote_broadcast_form_gets_unconfirmed_records_only() {
    let recipient = oid(ObjectType::DEVICE, 20);
    for form in crate::server::event_recipient_route::broadcast_station_tests::FORMS {
        for confirmed in [false, true] {
            let mut reporter = reporter();
            reporter
                .set_issue_confirmed_notifications(confirmed)
                .unwrap();
            let mut fixture = try_server(
                reporter,
                &[10],
                Some(BACnetRecipient::Device(recipient)),
                vec![DeviceBinding::routed(recipient, 5, form, LOGGER).unwrap()],
            )
            .await
            .expect("a Device recipient starts");
            let case = format!("{form:02X?}, confirmed {confirmed}");
            let expected = if confirmed {
                Reliability::CONFIGURATION_ERROR
            } else {
                Reliability::NO_FAULT_DETECTED
            };
            assert_eq!(health(&fixture.server).await, expected, "{case}");
            assert!(matches!(
                write_value(&fixture.server, None).await,
                Apdu::SimpleAck(_)
            ));
            settle().await;
            if confirmed {
                assert!(fixture.transport.sent.lock().unwrap().is_empty(), "{case}");
            } else {
                assert_eq!(notifications(&fixture.transport.sent).len(), 1, "{case}");
                let wire = fixture.transport.sent.lock().unwrap()[0].clone();
                let to = decode_npdu(wire).unwrap().destination.unwrap();
                assert_eq!((to.network, to.mac_address.as_slice()), (5, form), "{case}");
            }
            assert_eq!(health(&fixture.server).await, expected, "{case}");
            fixture.server.stop().await.unwrap();
        }
    }
}

/// #2738: the same binding marked one station gives a confirmed reporter a
/// route at each form the mark lifts, and its record goes there as a
/// confirmed request through the router; at a form the mark doesn't lift it
/// reads a configuration error and sends nothing, as unmarked.
#[tokio::test(start_paused = true)]
async fn a_device_bound_marked_one_station_gets_confirmed_records_where_the_mark_lifts() {
    use crate::server::event_recipient_route::broadcast_station_tests::{
        MARKED_FORMS, MARKED_STATIONS,
    };
    let recipient = oid(ObjectType::DEVICE, 20);
    let lifted = MARKED_STATIONS.into_iter().map(|mac| (mac, true));
    for (mac, routed) in lifted.chain(MARKED_FORMS.into_iter().map(|mac| (mac, false))) {
        let mut reporter = reporter();
        reporter.set_issue_confirmed_notifications(true).unwrap();
        let binding = DeviceBinding::routed(recipient, 5, mac, LOGGER)
            .unwrap()
            .one_station();
        let mut fixture = try_server(
            reporter,
            &[10],
            Some(BACnetRecipient::Device(recipient)),
            vec![binding],
        )
        .await
        .expect("a Device recipient starts");
        let expected = if routed {
            Reliability::NO_FAULT_DETECTED
        } else {
            Reliability::CONFIGURATION_ERROR
        };
        assert_eq!(health(&fixture.server).await, expected, "{mac:02X?}");
        assert!(matches!(
            write_value(&fixture.server, None).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        let sent = fixture.transport.sent.lock().unwrap().clone();
        if routed {
            let [wire] = sent.try_into().expect("one record");
            let npdu = decode_npdu(wire).unwrap();
            let to = npdu.destination.unwrap();
            assert_eq!((to.network, to.mac_address.as_slice()), (5, mac));
            assert!(matches!(
                decode_apdu(npdu.payload).unwrap(),
                Apdu::ConfirmedRequest(request)
                    if request.service_choice == ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
            ));
        } else {
            assert!(sent.is_empty(), "{mac:02X?}");
        }
        fixture.server.stop().await.unwrap();
    }
}

/// The copies of a record changing the recipient: each unicast as (its link
/// MAC, confirmed), then each global broadcast as (`None`, confirmed).
fn change_copies(fixture: &Fixture) -> Vec<(Option<Vec<u8>>, bool)> {
    let read = |bytes: &Bytes| {
        let npdu = decode_npdu(bytes.clone()).unwrap();
        match decode_apdu(npdu.payload).unwrap() {
            Apdu::ConfirmedRequest(request) => {
                assert_eq!(
                    request.service_choice,
                    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
                );
                true
            }
            Apdu::UnconfirmedRequest(request) => {
                assert_eq!(
                    request.service_choice,
                    UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
                );
                assert_eq!(npdu.destination.map(|to| to.network), Some(0xFFFF));
                false
            }
            other => panic!("unexpected {other:?}"),
        }
    };
    let macs = std::mem::take(&mut *fixture.transport.destinations.lock().unwrap());
    let sent = std::mem::take(&mut *fixture.transport.sent.lock().unwrap());
    let broadcasts = std::mem::take(&mut *fixture.transport.broadcasts.lock().unwrap());
    macs.into_iter()
        .zip(sent)
        .map(|(mac, bytes)| (Some(mac), read(&bytes)))
        .chain(broadcasts.iter().map(|bytes| (None, read(bytes))))
        .collect()
}

/// #2732, Clause 12.11.66: a confirmed reporter's record of a recipient
/// change goes to both recipients or by global broadcast. A recipient at a
/// broadcast or group form on its network has no route for it, so, as for
/// an old recipient with no route, an unconfirmed global broadcast stands in
/// for its copy, whichever side of the change it is on.
#[tokio::test(start_paused = true)]
async fn a_recipient_change_broadcasts_for_a_side_at_a_remote_broadcast_form() {
    let bad = oid(ObjectType::DEVICE, 20);
    let good = oid(ObjectType::DEVICE, 21);
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let mut capture = AuditCapture::default();
    capture.record_broadcasts = true;
    let mut fixture = try_servers_config(
        vec![reporter],
        &[10],
        Some(BACnetRecipient::Device(bad)),
        vec![
            DeviceBinding::routed(bad, 5, [0xFF], LOGGER).unwrap(),
            DeviceBinding::local(good, NEW_LOGGER).unwrap(),
        ],
        true,
        1476,
        capture,
    )
    .await
    .expect("a Device recipient starts");
    // Away from the broadcast form, then back to it: each time the other
    // side gets its confirmed copy and the broadcast form's goes by global
    // broadcast.
    for to in [good, bad] {
        let mut bytes = BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(&mut bytes, &BACnetRecipient::Device(to))
            .unwrap();
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
            .expect("the change is taken");
        settle().await;
        assert_eq!(
            change_copies(&fixture),
            [(Some(NEW_LOGGER.to_vec()), true), (None, false)],
            "to {to}"
        );
    }
    fixture.server.stop().await.unwrap();
}
