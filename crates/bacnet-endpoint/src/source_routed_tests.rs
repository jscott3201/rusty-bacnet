//! Routed source Audit bindings (#2371): the routed rule and the builders'
//! refusals, and a session whose recipient Device sits behind a
//! `BACnetRouter`, every link an in-memory loopback on paused time.
use super::*;
use crate::source_audit::routers::SourceRoute;
use bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination;
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::network_number::NetworkNumber;

/// The recipient Device's network, behind the router.
const REMOTE: u16 = 2;
/// The session's MAC on network 1.
const SESSION: [u8; 1] = [1];
/// The router's MAC on network 1.
const ROUTER_NEAR: [u8; 1] = [0xFE];
/// The router's MAC on network 2.
const ROUTER_FAR: [u8; 1] = [0xFD];
/// The recipient Device 999's MAC on network 2.
const STATION: [u8; 1] = [5];

fn denied<T>(result: Result<T, Error>) -> bool {
    matches!(
        result,
        Err(Error::Remote(bacnet_types::error::RemoteError::Error {
            class: ErrorClass::SERVICES,
            code: ErrorCode::SERVICE_REQUEST_DENIED,
        }))
    )
}

fn routed(network: u16, mac: &[u8]) -> Result<SourceRoutes, Error> {
    SourceRoutes::for_link(&[], other(&SESSION)).and_then(|routes| {
        routes.bind_routed(&[(
            oid(ObjectType::DEVICE, 999),
            network,
            MacAddr::from_slice(mac),
        )])
    })
}

/// Network 0 (an address's name for this network), the global network and
/// an empty or over-long DADR are refused; any other network and a DADR of
/// any length up to 18 octets, of whatever link, are taken.
#[test]
fn a_routed_binding_names_one_station_on_another_network() {
    for (network, mac) in [
        (0, &[5][..]),
        (0xFFFF, &[5]),
        (REMOTE, &[]),
        (REMOTE, &[5; 19]),
    ] {
        assert!(denied(routed(network, mac)), "{network} {mac:?}");
    }
    for (network, mac) in [(1, &[5][..]), (REMOTE, &[5; 18]), (0xFFFE, &[0xFF; 6])] {
        assert!(routed(network, mac).is_ok(), "{network} {mac:?}");
    }
    // One Device, one route: a routed binding can't repeat a direct one.
    let device = oid(ObjectType::DEVICE, 999);
    assert!(
        SourceRoutes::for_link(&[(device, MacAddr::from_slice(&[4]))], other(&SESSION))
            .unwrap()
            .bind_routed(&[(device, REMOTE, MacAddr::from_slice(&STATION))])
            .is_err()
    );
}

/// A routed binding resolves to its network and MAC. Once the session's own
/// number is that network, it is a station on this link, by the link's
/// unicast rule: a MAC of another length there has no route.
#[test]
fn a_routed_binding_on_this_network_goes_direct() {
    let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
    for (mac, local) in [(&STATION[..], Some(&STATION[..])), (&[5, 5][..], None)] {
        let mut routes = routed(REMOTE, mac).unwrap();
        assert_eq!(
            routes.resolve(&recipient),
            Some(SourceRoute::Routed {
                network: REMOTE,
                mac: MacAddr::from_slice(mac),
            })
        );
        let number = bacnet_network::network_number::LocalNetworkNumber::default();
        number.publish(NetworkNumber::configured(REMOTE).unwrap());
        routes.finalize(other(&SESSION), number);
        assert_eq!(
            routes.resolve(&recipient),
            local.map(|mac| SourceRoute::Direct(MacAddr::from_slice(mac)))
        );
    }
}

/// Every builder refuses a routed binding to the global network, and those
/// with a bare `build_transport` refuse any routed binding.
#[test]
fn the_builders_check_routed_bindings() {
    let device = oid(ObjectType::DEVICE, 999);
    for network in [REMOTE, 0xFFFF] {
        let mac = MacAddr::from_slice(&STATION);
        let sc = crate::sc::ScEndpointBuilder::new([2; 6], [0x11; 16])
            .source_audit_routed_device_binding(device, network, mac.clone())
            .validate_only();
        let (line, _peer) = LoopbackSerial::pair();
        let mstp = crate::mstp::MstpEndpointBuilder::new(line, 3)
            .source_audit_routed_device_binding(device, network, mac.clone())
            .build_session()
            .map(drop);
        let bip = crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .source_audit_routed_device_binding(device, network, mac)
            .build_session_over(BusPort::new())
            .map(drop);
        let expected = network == REMOTE;
        for (link, result) in [("SC", sc), ("MS/TP", mstp), ("B/IP", bip)] {
            assert_eq!(result.is_ok(), expected, "{link} {network}");
        }
    }
    let mac = MacAddr::from_slice(&STATION);
    let (line, _peer) = LoopbackSerial::pair();
    assert!(
        crate::mstp::MstpEndpointBuilder::new(line, 3)
            .source_audit_routed_device_binding(device, REMOTE, mac.clone())
            .build_transport()
            .is_err()
    );
    assert!(
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .source_audit_routed_device_binding(device, REMOTE, mac)
            .build_transport()
            .is_err()
    );
}

/// A session's start refuses a routed binding to the global network.
#[tokio::test(start_paused = true)]
async fn a_session_refuses_a_routed_binding_to_the_global_network() {
    let (transport, _peer) = LoopbackTransport::pair(SESSION.to_vec(), ROUTER_NEAR.to_vec());
    let mut session =
        EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
            .unwrap()
            .with_database(provisioned())
            .with_source_audit_routed_device_binding(
                oid(ObjectType::DEVICE, 999),
                0xFFFF,
                MacAddr::from_slice(&STATION),
            )
            .with_source_audit_reporter(selected());
    assert!(denied(session.start().await));
}

/// The session on network 1, a `BACnetRouter` between networks 1 and 2,
/// and Device 999 as station 5 on network 2.
struct Site {
    session: EndpointSession<LoopbackTransport>,
    router: BACnetRouter,
    peer: NetworkLayer<LoopbackTransport>,
    rx: bacnet_network::layer::ApduReceiver,
    /// The MAC of each unicast the session sends; a broadcast adds none.
    unicasts: tokio::sync::mpsc::UnboundedReceiver<MacAddr>,
}

impl Site {
    /// The session's Reporter records `operations`, confirmed or not, to
    /// Device 999, bound behind the router.
    async fn start(confirmed: bool, operations: &[AuditOperation]) -> Self {
        let (mut near, router_near) =
            LoopbackTransport::pair(SESSION.to_vec(), ROUTER_NEAR.to_vec());
        let unicasts = near.record_unicast_destinations();
        let (router_far, station) = LoopbackTransport::pair(ROUTER_FAR.to_vec(), STATION.to_vec());
        let ports = vec![
            RouterPort {
                transport: router_near,
                network_number: 1,
            },
            RouterPort {
                transport: router_far,
                network_number: REMOTE,
            },
        ];
        let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
            .await
            .unwrap();
        let mut flags = AuditOperationFlags::empty();
        for operation in operations {
            flags.insert(*operation);
        }
        let mut db = provisioned();
        db.get_mut(&selected())
            .unwrap()
            .audit_reporter_mut()
            .unwrap()
            .configure(
                AuditLevel::AUDIT_ALL,
                flags,
                confirmed,
                None,
                BACnetPriorityFilter::empty(),
                None,
            )
            .unwrap();
        let mut session = EndpointSession::new(
            near,
            SessionRole::Both,
            SessionConfig {
                apdu_timeout_ms: 1_000,
                apdu_retries: 0,
                ..SessionConfig::default()
            },
        )
        .unwrap()
        .with_database(db)
        .with_source_audit_routed_device_binding(
            oid(ObjectType::DEVICE, 999),
            REMOTE,
            MacAddr::from_slice(&STATION),
        )
        .with_source_audit_reporter(selected())
        .with_device_writes(Arc::new(|_| true))
        .with_reinitialize(|_, _| Ok(()));
        session.start().await.unwrap();
        let mut peer = NetworkLayer::new(station);
        let rx = peer.start().await.unwrap();
        Self {
            session,
            router,
            peer,
            rx,
            unicasts,
        }
    }

    /// The router the session has learned for network 2.
    fn learned(&self) -> Option<MacAddr> {
        self.session
            .source_recipient
            .as_ref()
            .unwrap()
            .routes
            .routers()
            .router(REMOTE)
    }

    /// The next APDU the peer receives: through the router, from the
    /// session.
    async fn receive(&mut self) -> ReceivedApdu {
        let received = timeout(WAIT, self.rx.recv()).await.unwrap().unwrap();
        let source = received.source_network.clone().unwrap();
        assert_eq!(source.mac_address.as_slice(), SESSION);
        assert_eq!(received.source_mac.as_slice(), ROUTER_FAR);
        received
    }

    /// Send `pdu` from the peer to the session, through the router.
    async fn send(&self, pdu: Apdu, expecting_reply: bool) {
        let mut bytes = BytesMut::new();
        encode_apdu(&mut bytes, &pdu).unwrap();
        self.peer
            .send_apdu_routed(
                &bytes,
                1,
                &SESSION,
                &ROUTER_FAR,
                expecting_reply,
                NetworkPriority::Normal,
            )
            .await
            .unwrap();
    }

    async fn acknowledge(&self, invoke_id: u8) {
        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        });
        self.send(ack, false).await;
    }

    /// The unicasts the session has sent so far.
    fn sent_unicasts(&mut self) -> Vec<MacAddr> {
        std::iter::from_fn(|| self.unicasts.try_recv().ok()).collect()
    }

    async fn stop(mut self) {
        self.session.stop().await.unwrap();
        self.peer.stop().await.unwrap();
        self.router.stop().await.unwrap();
    }
}

/// The Reporter's records reach Device 999 behind the router: the first
/// with the link's broadcast MAC, since no router is known yet. The peer's
/// routed ReinitializeDevice teaches the session its network's router, so
/// the record of that reset, the SimpleAck and the next local report all go
/// to the router as unicasts. Each record names its requester, or this
/// device for a local reset.
#[tokio::test(start_paused = true)]
async fn a_reporter_records_resets_to_a_device_behind_a_router() {
    let mut site = Site::start(false, &[AuditOperation::DEVICE_RESET]).await;
    let this_device = BACnetRecipient::Device(oid(ObjectType::DEVICE, 123));
    site.session.report_device_reset_local().await.unwrap();
    let (first, _) = notification(&site.receive().await, false);
    assert_eq!(first.operation, AuditOperation::DEVICE_RESET);
    assert_eq!(first.source_device, this_device);
    assert!(site.sent_unicasts().is_empty(), "no router was known yet");
    assert_eq!(site.learned(), None);

    let mut service = BytesMut::new();
    bacnet_services::device_mgmt::ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::WARMSTART,
        password: None,
    }
    .encode(&mut service)
    .unwrap();
    let request = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 3,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        service_request: service.freeze(),
    });
    site.send(request, true).await;
    let (mut record, mut acked) = (None, false);
    while record.is_none() || !acked {
        let received = site.receive().await;
        match decode_apdu(received.apdu.clone()).unwrap() {
            Apdu::SimpleAck(ack) if ack.invoke_id == 3 => acked = true,
            Apdu::UnconfirmedRequest(_) => record = Some(notification(&received, false).0),
            other => panic!("unexpected {other:?}"),
        }
    }
    let peers = record.unwrap();
    assert_eq!(peers.operation, AuditOperation::DEVICE_RESET);
    assert_eq!(
        peers.source_device,
        BACnetRecipient::Address(BACnetAddress {
            network_number: REMOTE,
            mac_address: MacAddr::from_slice(&STATION),
        })
    );
    assert_eq!(peers.invoke_id, Some(3));
    assert_eq!(site.learned().as_deref(), Some(&ROUTER_NEAR[..]));

    site.session.report_device_reset_local().await.unwrap();
    let (local, _) = notification(&site.receive().await, false);
    assert_eq!(local.source_device, this_device);
    assert_eq!(local.invoke_id, None);
    assert_eq!(local.target_device, this_device);
    let sent = site.sent_unicasts();
    assert_eq!(sent.len(), 3, "the record, the SimpleAck, the local record");
    assert!(sent.iter().all(|mac| mac.as_slice() == ROUTER_NEAR));
    site.stop().await;
}

/// A confirmed record's answer comes back through the router naming Device
/// 999's network and MAC, completes the record and teaches the router. A
/// record through that router that gets no answer fails as a direct one
/// does, setting COMMUNICATION_FAILURE, and the router is forgotten: the
/// next record goes with the broadcast MAC again, and its answer clears
/// the failure and teaches the router anew.
#[tokio::test(start_paused = true)]
async fn a_confirmed_routed_record_learns_its_router_and_fails_like_a_direct_one() {
    let mut site = Site::start(true, &[AuditOperation::DEVICE_RESET]).await;
    for (round, answer) in [(0, true), (1, false), (2, true)] {
        site.session.report_device_reset_local().await.unwrap();
        let (record, invoke) = notification(&site.receive().await, true);
        assert_eq!(record.operation, AuditOperation::DEVICE_RESET);
        let unicasts = site.sent_unicasts();
        if round == 1 {
            assert_eq!(unicasts, [MacAddr::from_slice(&ROUTER_NEAR)]);
        } else {
            assert!(unicasts.is_empty(), "round {round} knew a router");
        }
        let site = &site;
        if answer {
            site.acknowledge(invoke.unwrap()).await;
            eventually(WAIT, "the answer completes the record", async || {
                reliability(&site.session).await == Reliability::NO_FAULT_DETECTED.to_raw()
                    && site.learned().is_some()
            })
            .await;
            assert_eq!(site.learned().as_deref(), Some(&ROUTER_NEAR[..]));
        } else {
            let failed = Reliability::COMMUNICATION_FAILURE.to_raw();
            eventually(Duration::from_secs(10), "no answer fails it", async || {
                reliability(&site.session).await == failed
            })
            .await;
            assert_eq!(site.learned(), None, "the failed router is forgotten");
        }
    }
    site.stop().await;
}

/// Records lost for want of resources count toward one AUDITING_FAILURE
/// summary, as for a direct recipient, and set COMMUNICATION_FAILURE. The
/// summary goes to Device 999 behind the router.
#[tokio::test(start_paused = true)]
async fn lost_routed_records_are_counted_and_summarized_there() {
    let operations = [
        AuditOperation::DEVICE_RESET,
        AuditOperation::AUDITING_FAILURE,
    ];
    let mut site = Site::start(false, &operations).await;
    let owner = Arc::clone(site.session.notifications.as_ref().unwrap());
    let permits: Vec<_> = (0..64).map(|_| owner.try_admit_audit().unwrap()).collect();
    for _ in 0..2 {
        site.session.report_device_reset_local().await.unwrap();
    }
    assert_eq!(
        reliability(&site.session).await,
        Reliability::COMMUNICATION_FAILURE.to_raw()
    );
    assert!(site.rx.try_recv().is_err());
    drop(permits);
    let (summary, _) = notification(&site.receive().await, false);
    assert_eq!(summary.operation, AuditOperation::AUDITING_FAILURE);
    let mut count = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut count, 2);
    assert_eq!(summary.current_value, Some(count.to_vec()));
    let session = &site.session;
    eventually(WAIT, "the summary clears the failure", async || {
        reliability(session).await == Reliability::NO_FAULT_DETECTED.to_raw()
    })
    .await;
    site.stop().await;
}

/// An audited read of a station behind a router is recorded: the request
/// goes routed, as every routed confirmed request must name one station
/// (#1278), and its record names the station by its network and MAC.
#[tokio::test(start_paused = true)]
async fn an_audited_read_of_a_routed_station_names_its_network() {
    let mut site = Site::start(false, &[AuditOperation::READ]).await;
    let client = site.session.cloned_client_handle().unwrap();
    let read = tokio::spawn(async move {
        client
            .read_property_with_destination(
                EndpointApduDestination::Routed {
                    destination_network: REMOTE,
                    destination_mac: MacAddr::from_slice(&STATION),
                    router_mac: MacAddr::from_slice(&ROUTER_NEAR),
                },
                Vec::new(),
                target(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let request = site.receive().await;
    let (invoke, rp) = read_request(&request);
    site.send(ack(invoke, &rp), false).await;
    assert!(read.await.unwrap().is_ok());
    let (record, _) = notification(&site.receive().await, false);
    assert_eq!(record.operation, AuditOperation::READ);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(
        record.target_device,
        BACnetRecipient::Address(BACnetAddress {
            network_number: REMOTE,
            mac_address: MacAddr::from_slice(&STATION),
        })
    );
    site.stop().await;
}
