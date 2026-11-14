//! Source Audit on links other than IPv4 B/IP (#2357): the per-link unicast
//! rule, the SC and MS/TP builders' bindings, and an MS/TP and an SC session
//! that each record a peer's ReinitializeDevice and a local reset report
//! through the Reporter's filters.
use super::*;
use crate::source_audit::recipient::{SourceLink, SourceRoutes};
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use bacnet_transport::port::GroupDestinations;
use bacnet_transport::test_transport::{SendMode, TestTransport};
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::ReinitializedState;

fn other(local: &[u8]) -> SourceLink {
    SourceLink::Other {
        local: MacAddr::from_slice(local),
        groups: GroupDestinations::unknown(),
    }
}

#[test]
fn each_link_takes_only_a_unicast_mac_of_its_own_format() {
    // MS/TP, station 3.
    let mstp = other(&[3]);
    assert!(mstp.unicast(&[4]) && mstp.unicast(&[200]));
    for refused in [&[0xFF][..], &[3], &[], &[4, 5]] {
        assert!(!mstp.unicast(refused), "{refused:?}");
    }
    // BACnet/SC, VMAC 02:..:02.
    let sc = other(&[2; 6]);
    assert!(sc.unicast(&[3; 6]));
    for refused in [&[0xFF; 6][..], &[0; 6], &[2; 6], &[3; 3], &[3; 7]] {
        assert!(!sc.unicast(refused), "{refused:?}");
    }
    // Ethernet: the all-zero IEEE MAC is no station either.
    let ethernet = other(&[0x02, 0, 0, 0, 0, 1]);
    assert!(ethernet.unicast(&[0x02, 0, 0, 0, 0, 2]));
    assert!(!ethernet.unicast(&[0; 6]));
    // B/IPv6, VMAC 0x000102; its group rule is the transport's.
    let bip6 = SourceLink::Other {
        local: MacAddr::from_slice(&[0, 1, 2]),
        groups: GroupDestinations::new(|mac| mac == [9, 9, 9]),
    };
    assert!(bip6.unicast(&[0, 1, 3]));
    for refused in [&[0, 1, 2][..], &[9, 9, 9], &[0, 1, 3, 4]] {
        assert!(!bip6.unicast(refused), "{refused:?}");
    }
    // A MAC learned only at start: length and self wait for it.
    assert!(other(&[]).unicast(&[0, 1, 3]));
    // A binding the link refuses is SERVICES / SERVICE_REQUEST_DENIED; a
    // repeated Device names itself.
    let denied = |error: Error| {
        matches!(error, Error::Remote(bacnet_types::error::RemoteError::Error { code, .. })
            if code == ErrorCode::SERVICE_REQUEST_DENIED)
    };
    let device = oid(ObjectType::DEVICE, 999);
    let bind =
        |mac: &[u8], link| SourceRoutes::for_link(&[(device, MacAddr::from_slice(mac))], link);
    assert!(denied(bind(&[0xFF], other(&[3])).err().unwrap()));
    assert!(denied(bind(&[0xFF; 6], other(&[2; 6])).err().unwrap()));
    assert!(bind(&[4], other(&[3])).is_ok());
    let twice = [
        (device, MacAddr::from_slice(&[4])),
        (device, MacAddr::from_slice(&[5])),
    ];
    let error = SourceRoutes::for_link(&twice, other(&[3])).err().unwrap();
    assert!(error.to_string().contains("duplicate"), "{error}");
}

#[test]
fn the_sc_and_mstp_builders_check_their_bindings() {
    let device = oid(ObjectType::DEVICE, 999);
    let sc = |vmac| {
        crate::sc::ScEndpointBuilder::new([2; 6], [0x11; 16])
            .source_audit_device_binding(device, vmac)
            .validate_only()
    };
    assert!(sc([3; 6]).is_ok());
    assert!(sc([0xFF; 6]).is_err());
    assert!(sc([0; 6]).is_err());
    assert!(sc([2; 6]).is_err());
    let mstp = |station| {
        let (line, _peer) = LoopbackSerial::pair();
        crate::mstp::MstpEndpointBuilder::new(line, 3)
            .source_audit_device_binding(device, station)
            .build_session()
            .map(drop)
    };
    assert!(mstp(4).is_ok());
    assert!(mstp(0xFF).is_err());
    assert!(mstp(3).is_err());
    // A bare transport can't keep route data.
    let (line, _peer) = LoopbackSerial::pair();
    assert!(
        crate::mstp::MstpEndpointBuilder::new(line, 3)
            .source_audit_device_binding(device, 4)
            .build_transport()
            .is_err()
    );
}

/// A B/IP session refuses link bindings, and a non-B/IP one B/IP bindings,
/// at start.
#[tokio::test(start_paused = true)]
async fn bindings_of_the_wrong_link_fail_start() {
    let (sink, _records) = network().await;
    let mut bip = session(database(false), SessionRole::ClientOnly, &sink)
        .with_source_audit_device_binding(oid(ObjectType::DEVICE, 998), MacAddr::from_slice(&[4]));
    assert!(bip.start().await.is_err());
    let (line, _peer) = LoopbackSerial::pair();
    let mut mstp = crate::mstp::MstpEndpointBuilder::new(line, 3)
        .role(SessionRole::ClientOnly)
        .database(provisioned())
        .build_session()
        .unwrap()
        .with_source_audit_reporter(selected());
    mstp.source_audit_bindings
        .push((oid(ObjectType::DEVICE, 999), address(&sink)));
    assert!(mstp.start().await.is_err());
}

/// A link that learns its MAC at start, as B/IPv6 does: it reports station
/// 7 as a placeholder before start, and the started node is station 1.
fn late_mac(inner: bacnet_transport::loopback::LoopbackTransport) -> TestTransport {
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .local_mac_until_started(&[7])
        .build()
}

/// The selected Reporter's Reliability.
async fn reporter_reliability<T: TransportPort + 'static>(
    session: &EndpointSession<T>,
) -> PropertyValue {
    let db = session.database.as_ref().unwrap().read().await;
    db.get(&selected())
        .unwrap()
        .audit_reporter()
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
}

/// The node's own MAC is checked again once the link has started (#2357):
/// a binding to the MAC the link learns at start passes the pre-start
/// check, then resolves to nothing, so the Reporter reads
/// CONFIGURATION_ERROR. A binding to the peer resolves.
#[tokio::test(start_paused = true)]
async fn a_binding_to_the_mac_learned_at_start_reads_configuration_error() {
    for (station, expected) in [
        (2, Reliability::NO_FAULT_DETECTED),
        (1, Reliability::CONFIGURATION_ERROR),
    ] {
        let (inner, _peer) = bacnet_transport::loopback::LoopbackTransport::pair(vec![1], vec![2]);
        let transport = late_mac(inner);
        let mut session =
            EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
                .unwrap()
                .with_database(provisioned())
                .with_source_audit_device_binding(
                    oid(ObjectType::DEVICE, 999),
                    MacAddr::from_slice(&[station]),
                )
                .with_source_audit_reporter(selected());
        session.start().await.unwrap();
        assert_eq!(
            reporter_reliability(&session).await,
            PropertyValue::Enumerated(expected.to_raw()),
            "station {station}"
        );
        session.stop().await.unwrap();
    }
}

/// B/IPv6 reports a zero VMAC until it starts, and VMAC 000000 is Device
/// 0's (Clause H.7.2), so the pre-start check leaves the self check to the
/// started link: a binding to 000000 resolves, and one to this node's own
/// VMAC, Device 123's, reads CONFIGURATION_ERROR.
#[tokio::test]
async fn a_bip6_session_checks_its_own_vmac_once_started() {
    use bacnet_transport::bip6::Bip6Transport;
    for (vmac, expected) in [
        ([0, 0, 0], Reliability::NO_FAULT_DETECTED),
        ([0, 0, 123], Reliability::CONFIGURATION_ERROR),
    ] {
        let transport = Bip6Transport::new(std::net::Ipv6Addr::LOCALHOST, 0, Some(123));
        let mut session =
            EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
                .unwrap()
                .with_database(provisioned())
                .with_source_audit_device_binding(
                    oid(ObjectType::DEVICE, 999),
                    MacAddr::from_slice(&vmac),
                )
                .with_source_audit_reporter(selected());
        session.start().await.unwrap();
        assert_eq!(
            reporter_reliability(&session).await,
            PropertyValue::Enumerated(expected.to_raw()),
            "VMAC {vmac:?}"
        );
        session.stop().await.unwrap();
    }
}

/// The source database with its Device's Audit recipient, Device 999.
fn provisioned() -> ObjectDatabase {
    let mut db = database(false);
    db.set_clock_reader(None);
    db.get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)))
        .unwrap();
    db
}

/// The selected Reporter now records only `operations`, at every level.
async fn record_only<T: TransportPort + 'static>(
    session: &EndpointSession<T>,
    operations: &[AuditOperation],
) {
    let mut flags = AuditOperationFlags::empty();
    for operation in operations {
        flags.insert(*operation);
    }
    session
        .database
        .as_ref()
        .unwrap()
        .write()
        .await
        .get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .unwrap()
        .configure(
            AuditLevel::AUDIT_ALL,
            flags,
            false,
            None,
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
}

/// The peer's side: it asks the session to reinitialize and collects the
/// Audit records the session sends it.
struct Peer<P: TransportPort + 'static> {
    network: NetworkLayer<P>,
    rx: bacnet_network::layer::ApduReceiver,
    wait: Duration,
}

impl<P: TransportPort + 'static> Peer<P> {
    /// Send `state` to `session` and return once its SimpleAck arrives,
    /// with any Audit record that came first.
    async fn reinitialize(
        &mut self,
        session: &[u8],
        state: ReinitializedState,
        invoke_id: u8,
    ) -> Vec<BACnetAuditNotification> {
        let mut service = BytesMut::new();
        ReinitializeDeviceRequest {
            reinitialized_state: state,
            password: None,
        }
        .encode(&mut service)
        .unwrap();
        let mut bytes = BytesMut::new();
        encode_apdu(
            &mut bytes,
            &Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 480,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::REINITIALIZE_DEVICE,
                service_request: service.freeze(),
            }),
        )
        .unwrap();
        self.network
            .send_apdu(&bytes, session, true, NetworkPriority::Normal)
            .await
            .unwrap();
        let mut early = Vec::new();
        loop {
            let envelope = timeout(self.wait, self.rx.recv()).await.unwrap().unwrap();
            match decode_apdu(envelope.apdu.clone()).unwrap() {
                Apdu::SimpleAck(ack) if ack.invoke_id == invoke_id => return early,
                Apdu::UnconfirmedRequest(_) => early.push(notification(&envelope, false).0),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    async fn record(&mut self) -> BACnetAuditNotification {
        let envelope = timeout(self.wait, self.rx.recv()).await.unwrap().unwrap();
        notification(&envelope, false).0
    }
}

/// What both links must show: filtered operations build no record, a
/// peer's reset names the peer, and a local one this device with no
/// invoke ID.
async fn records_resets<T: TransportPort + 'static, P: TransportPort + 'static>(
    session: &EndpointSession<T>,
    session_mac: &[u8],
    peer: &mut Peer<P>,
) {
    let requester = BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(peer.network.local_mac()),
    });
    let this_device = BACnetRecipient::Device(oid(ObjectType::DEVICE, 123));
    // Resets filtered out: the backup step after them is the first record.
    record_only(session, &[AuditOperation::DEVICE_BACKUP]).await;
    let mut seen = peer
        .reinitialize(session_mac, ReinitializedState::WARMSTART, 1)
        .await;
    session.report_device_reset_local().await.unwrap();
    seen.extend(
        peer.reinitialize(session_mac, ReinitializedState::START_BACKUP, 2)
            .await,
    );
    let backup = match seen.pop() {
        Some(record) => record,
        None => peer.record().await,
    };
    assert!(seen.is_empty(), "filtered resets built records: {seen:?}");
    assert_eq!(backup.operation, AuditOperation::DEVICE_BACKUP);
    assert_eq!(
        backup.target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    // Resets recorded: the peer's, then the local one.
    record_only(session, &[AuditOperation::DEVICE_RESET]).await;
    let mut seen = peer
        .reinitialize(session_mac, ReinitializedState::WARMSTART, 3)
        .await;
    let peers = match seen.pop() {
        Some(record) => record,
        None => peer.record().await,
    };
    assert_eq!(peers.operation, AuditOperation::DEVICE_RESET);
    assert_eq!(peers.source_device, requester);
    assert_eq!(peers.invoke_id, Some(3));
    session.report_device_reset_local().await.unwrap();
    let local = peer.record().await;
    assert_eq!(local.operation, AuditOperation::DEVICE_RESET);
    assert_eq!(local.source_device, this_device);
    assert_eq!(local.invoke_id, None);
    assert_eq!(local.target_device, this_device);
}

/// Two MS/TP masters on a loopback line: the session at station 1, the
/// peer, Device 999, at station 2.
#[tokio::test(start_paused = true)]
async fn an_mstp_session_records_resets_through_its_reporter() {
    let (session_line, peer_line) = LoopbackSerial::pair();
    let config = |this_station| MstpConfig {
        this_station,
        max_master: 3,
        ..MstpConfig::default()
    };
    let mut session = crate::mstp::MstpEndpointBuilder::new(session_line, 1)
        .max_master(3)
        .database(provisioned())
        .source_audit_device_binding(oid(ObjectType::DEVICE, 999), 2)
        .build_session()
        .unwrap()
        .with_source_audit_reporter(selected())
        .with_device_writes(Arc::new(|_| true))
        .with_reinitialize(|_, _| Ok(()));
    session.start().await.unwrap();
    let mut network = NetworkLayer::new(MstpTransport::new(peer_line, config(2)));
    let rx = network.start().await.unwrap();
    let mut peer = Peer {
        network,
        rx,
        wait: Duration::from_secs(10),
    };
    records_resets(&session, &[1], &mut peer).await;
    session.stop().await.unwrap();
    peer.network.stop().await.unwrap();
}

/// A BACnet/SC session and its peer, Device 999, on one hub.
#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn an_sc_session_records_resets_through_its_reporter() {
    use bacnet_transport::sc::ScTransport;
    use bacnet_transport::sc_hub::ScHub;
    use bacnet_transport::sc_tls::TlsWebSocket;
    let (hub_tls, node_tls) = sc_tls();
    let mut hub = ScHub::start_with_uuid("127.0.0.1:0", hub_tls, [0xAA; 6], [0xBB; 16])
        .await
        .unwrap();
    let url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());
    let (session_vmac, peer_vmac) = ([2; 6], [3; 6]);
    let socket = TlsWebSocket::connect(&url, node_tls.clone()).await.unwrap();
    let mut session = crate::sc::ScEndpointBuilder::new(session_vmac, [0x11; 16])
        .database(provisioned())
        .source_audit_device_binding(oid(ObjectType::DEVICE, 999), peer_vmac)
        .build_hub_session(socket)
        .unwrap()
        .with_source_audit_reporter(selected())
        .with_device_writes(Arc::new(|_| true))
        .with_reinitialize(|_, _| Ok(()));
    session.start().await.unwrap();
    let socket = TlsWebSocket::connect(&url, node_tls).await.unwrap();
    let mut network =
        NetworkLayer::new(ScTransport::new(socket, peer_vmac).with_device_uuid([0x22; 16]));
    let rx = network.start().await.unwrap();
    let mut peer = Peer {
        network,
        rx,
        wait: Duration::from_secs(10),
    };
    records_resets(&session, &session_vmac, &mut peer).await;
    session.stop().await.unwrap();
    peer.network.stop().await.unwrap();
    hub.stop().await;
}

/// A throwaway CA, a hub certificate and a node certificate for the SC test.
#[cfg(feature = "sc-tls")]
fn sc_tls() -> (
    bacnet_transport::sc_hub::ScHubTlsConfig,
    bacnet_transport::sc_tls::ScNodeTlsConfig,
) {
    use rcgen::{CertificateParams, Issuer, KeyPair};
    use rustls::pki_types::PrivatePkcs8KeyDer;
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let leaf = |names: Vec<String>| {
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(names)
            .unwrap()
            .signed_by(&key, &issuer)
            .unwrap();
        (cert, PrivatePkcs8KeyDer::from(key.serialize_der()))
    };
    let (hub_cert, hub_key) = leaf(vec!["localhost".into(), "127.0.0.1".into()]);
    let (node_cert, node_key) = leaf(vec!["node".into()]);
    (
        bacnet_transport::sc_hub::ScHubTlsConfig::from_der(
            vec![ca_cert.der().clone()],
            vec![hub_cert.der().clone()],
            hub_key.into(),
        )
        .unwrap(),
        bacnet_transport::sc_tls::ScNodeTlsConfig::from_der(
            vec![ca_cert.der().clone()],
            vec![node_cert.der().clone()],
            node_key.into(),
        )
        .unwrap(),
    )
}

#[path = "source_routed_tests.rs"]
mod routed;
