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
/// form on its network keeps its route, which serves both kinds: an
/// unconfirmed reporter's record still goes there through the router, and a
/// confirmed reporter's is never sent, which the reporter reads as a
/// communication failure.
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
            assert!(matches!(
                write_value(&fixture.server, None).await,
                Apdu::SimpleAck(_)
            ));
            settle().await;
            let case = format!("{form:02X?}, confirmed {confirmed}");
            if confirmed {
                assert!(fixture.transport.sent.lock().unwrap().is_empty(), "{case}");
                assert_eq!(
                    health(&fixture.server).await,
                    Reliability::COMMUNICATION_FAILURE,
                    "{case}"
                );
            } else {
                assert_eq!(notifications(&fixture.transport.sent).len(), 1, "{case}");
                let wire = fixture.transport.sent.lock().unwrap()[0].clone();
                let to = decode_npdu(wire).unwrap().destination.unwrap();
                assert_eq!((to.network, to.mac_address.as_slice()), (5, form), "{case}");
                assert_eq!(
                    health(&fixture.server).await,
                    Reliability::NO_FAULT_DETECTED,
                    "{case}"
                );
            }
            fixture.server.stop().await.unwrap();
        }
    }
}
