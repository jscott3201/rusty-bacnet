//! Raw BVLL/NPDU fixtures exercise the two actual NORMAL B/IP owners.
use super::*;
use bacnet_transport::{bip::BipTransport, port::ReceivedNpdu};
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    time::Duration,
};
use tokio::net::UdpSocket;

#[path = "sc_network_number_tests.rs"]
mod sc_network_number_tests;

fn selected() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 2).unwrap()
}
struct ObservedBip {
    bip: BipTransport,
    sent: mpsc::Sender<Vec<u8>>,
    gates: Option<Arc<Gates>>,
    input_override: Option<mpsc::Receiver<ReceivedNpdu>>,
}
impl TransportPort for ObservedBip {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        self.bip.supports_local_nonrouter_number_controls()
    }
    fn bip_port(&self) -> Option<bacnet_transport::port::BipPort> {
        self.bip.bip_port()
    }
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        self.bip.bip_broadcast_endpoint()
    }
    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        self.bip.retain_network_port_lease_internal(lease)
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let real = self.bip.start().await?;
        Ok(self.input_override.take().unwrap_or(real))
    }
    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(gates) = &self.gates {
            if gates.hold_stop.load(Ordering::SeqCst) {
                gates.stop_entered.notify_one();
                gates.stop_release.acquire().await.unwrap().forget();
            }
        }
        self.bip.stop().await?;
        if let Some(gates) = &self.gates {
            gates.stop_finished.notify_one();
        }
        Ok(())
    }
    fn abort(&mut self) {
        self.bip.abort();
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.bip.local_receive_apdu_capacity()
    }

    fn local_mac(&self) -> &[u8] {
        self.bip.local_mac()
    }
    fn egress_apdu_limit(&self) -> u16 {
        self.bip.egress_apdu_limit()
    }
    async fn send_unicast(&self, mac: &[u8], npdu: &[u8]) -> Result<(), Error> {
        self.bip.send_unicast(mac, npdu).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        if let Some(gates) = &self.gates {
            if gates.hold_send.load(Ordering::SeqCst) {
                gates.send_entered.notify_one();
                gates.send_release.acquire().await.unwrap().forget();
            }
        }
        self.bip.send_broadcast(npdu).await?;
        self.sent
            .try_send(npdu.to_vec())
            .expect("bounded test observation");
        Ok(())
    }
}
#[allow(
    clippy::large_enum_variant,
    reason = "one fixture per test; the two owners differ in size"
)]
enum Owner {
    Server(bacnet_server::server::BACnetServer<ObservedBip>),
    Endpoint(EndpointSession<ObservedBip>),
}
struct Fixture {
    owner: Owner,
    peer: UdpSocket,
    address: SocketAddrV4,
    sent: mpsc::Receiver<Vec<u8>>,
}
impl Fixture {
    async fn start(full: bool, registered: bool, number: u16) -> Self {
        Self::start_gated(full, registered, number, None).await
    }
    async fn start_gated(
        full: bool,
        registered: bool,
        number: u16,
        gates: Option<Arc<Gates>>,
    ) -> Self {
        Self::start_options(full, registered, number, gates, None).await
    }
    async fn start_options(
        full: bool,
        registered: bool,
        number: u16,
        gates: Option<Arc<Gates>>,
        input_override: Option<mpsc::Receiver<ReceivedNpdu>>,
    ) -> Self {
        // Observe outgoing NPDUs at the real transport call boundary: Darwin
        // cannot share the production wildcard socket for a loopback capture.
        let broadcast = Ipv4Addr::new(127, 255, 255, 255);
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        peer.set_broadcast(true).unwrap();
        let port = 0;
        let (sent_tx, sent) = mpsc::channel(32);
        let identity = crate::DeviceIdentity::new(875, 555)
            .unwrap()
            .with_bip_port(1, 91, Ipv4Addr::LOCALHOST, 19)
            .unwrap()
            .with_bip_port(2, number.into(), Ipv4Addr::LOCALHOST, port)
            .unwrap();
        let db = identity.build_database().unwrap();
        let transport = ObservedBip {
            bip: BipTransport::new(Ipv4Addr::LOCALHOST, port, broadcast),
            sent: sent_tx,
            gates,
            input_override,
        };
        let owner = if full {
            let mut config = identity.server_config();
            config.registered_network_port = registered.then_some(selected());
            Owner::Server(
                bacnet_server::server::BACnetServer::start(config, db, transport)
                    .await
                    .unwrap(),
            )
        } else {
            let mut session =
                EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())
                    .unwrap()
                    .with_database(db)
                    .with_identity(identity);
            if registered {
                session = session.with_registered_network_port(selected());
            }
            session.start().await.unwrap();
            Owner::Endpoint(session)
        };
        let mac = match &owner {
            Owner::Server(s) => s.local_mac(),
            Owner::Endpoint(s) => {
                let address = s.bip_local_address().unwrap();
                return Self {
                    owner,
                    peer,
                    address,
                    sent,
                };
            }
        };
        let (ip, port) = bacnet_transport::bvll::decode_bip_mac(mac).unwrap();
        Self {
            owner,
            peer,
            address: SocketAddrV4::new(ip.into(), port),
            sent,
        }
    }
    async fn send(&self, function: u8, npdu: &[u8]) {
        let length = 4 + npdu.len() + if function == 0x04 { 6 } else { 0 };
        let mut bytes = vec![0x81, function, (length >> 8) as u8, length as u8];
        if function == 0x04 {
            bytes.extend_from_slice(&[10, 1, 1, 8, 0xba, 0xc0]);
        }
        bytes.extend_from_slice(npdu);
        let target = if function == 0x0b {
            SocketAddrV4::new(Ipv4Addr::new(127, 255, 255, 255), self.address.port())
        } else {
            self.address
        };
        self.peer.send_to(&bytes, target).await.unwrap();
    }
    async fn response(&mut self, number: u16, flag: u8) {
        let bytes = tokio::time::timeout(Duration::from_secs(1), self.sent.recv())
            .await
            .expect("missing Network-Number-Is local broadcast")
            .unwrap();
        assert_eq!(
            bytes,
            [1, 0x80, 0x13, (number >> 8) as u8, number as u8, flag]
        );
    }
    async fn stop(&mut self) {
        match &mut self.owner {
            Owner::Server(server) => server.stop().await.unwrap(),
            Owner::Endpoint(endpoint) => {
                endpoint.stop().await.unwrap();
            }
        }
    }
}
#[tokio::test]
async fn network_number_full_server_answers_configured_local_what_is() {
    let mut fixture = Fixture::start(true, true, 17).await;
    fixture.send(0x0a, &[1, 0x80, 0x12]).await;
    fixture.response(17, 1).await;
    fixture.stop().await;
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        0
    );
    assert!(fixture
        .database()
        .write()
        .await
        .remove(&selected())
        .unwrap()
        .is_some());
    let _rebound = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, fixture.address.port()))
        .await
        .unwrap();
}
#[tokio::test]
async fn network_number_endpoint_answers_configured_local_what_is() {
    let mut fixture = Fixture::start(false, true, 17).await;
    fixture.send(0x04, &[1, 0x80, 0x12]).await;
    fixture.response(17, 1).await;
    fixture.stop().await;
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        0
    );
    assert!(fixture
        .database()
        .write()
        .await
        .remove(&selected())
        .unwrap()
        .is_some());
    let _rebound = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, fixture.address.port()))
        .await
        .unwrap();
}

impl Fixture {
    fn database(&self) -> Arc<RwLock<ObjectDatabase>> {
        match &self.owner {
            Owner::Server(s) => s.database().clone(),
            Owner::Endpoint(s) => s.database.as_ref().unwrap().clone(),
        }
    }
    async fn quiet(&mut self) {
        assert!(
            tokio::time::timeout(Duration::from_millis(40), self.sent.recv())
                .await
                .is_err()
        );
    }
    async fn query(&mut self, number: u16, flag: u8) {
        self.send(0x0a, &[1, 0x80, 0x12]).await;
        self.response(number, flag).await;
    }
    async fn announce(&self, number: u16, flag: u8) {
        // Annex J logical broadcast, with a unicast UDP hop from a BBMD.
        self.send(
            0x04,
            &[1, 0x80, 0x13, (number >> 8) as u8, number as u8, flag],
        )
        .await;
    }
    async fn read_pair(&self, number: u16, quality: u32) {
        use bacnet_encoding::{
            apdu::{decode_apdu, Apdu},
            primitives::decode_application_value,
        };
        use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
        use bacnet_types::enums::PropertyIdentifier as P;
        use bytes::{Bytes, BytesMut};
        static INVOKE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
        for (property, value) in [
            (P::NETWORK_NUMBER, PropertyValue::Unsigned(number.into())),
            (
                P::NETWORK_NUMBER_QUALITY,
                PropertyValue::Enumerated(quality),
            ),
        ] {
            let mut body = BytesMut::new();
            ReadPropertyRequest {
                object_identifier: selected(),
                property_identifier: property,
                property_array_index: None,
            }
            .encode(&mut body);
            let mut npdu = vec![1, 4, 0, 5, INVOKE.fetch_add(1, Ordering::Relaxed), 12];
            npdu.extend_from_slice(&body);
            self.send(0x0a, &npdu).await;
            let mut data = [0; 512];
            let (size, _) =
                tokio::time::timeout(Duration::from_secs(1), self.peer.recv_from(&mut data))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&data[..2], &[0x81, 0x0a]);
            let Apdu::ComplexAck(ack) =
                decode_apdu(Bytes::copy_from_slice(&data[6..size])).unwrap()
            else {
                panic!("RP must succeed")
            };
            let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
            assert_eq!(ack.object_identifier, selected());
            assert_eq!(
                decode_application_value(&ack.property_value, 0).unwrap().0,
                value
            );
        }
    }
}
#[tokio::test]
async fn network_number_real_owners_learn_precedence_and_refuse_invalid_controls() {
    for full in [true, false] {
        let mut f = Fixture::start(full, true, 0).await;
        f.send(0x0a, &[1, 0x80, 0x12]).await;
        f.quiet().await;
        f.read_pair(0, 0).await;
        for (number, flag, expected, quality) in [
            (10, 0, 10, 1),
            (11, 0, 11, 1),
            (11, 1, 11, 2),
            (11, 0, 11, 2),
            (12, 0, 11, 2),
            (12, 1, 12, 2),
            (65534, 1, 65534, 2),
            (1, 1, 1, 2),
        ] {
            f.announce(number, flag).await;
            f.query(expected, 0).await;
            f.read_pair(expected, quality).await;
        }
        for bad in [
            vec![1, 0x80, 0x13, 0, 0, 1],
            vec![1, 0x80, 0x13, 255, 255, 1],
            vec![1, 0x80, 0x13, 0, 99, 2],
            vec![1, 0x80, 0x13, 0, 99],
            vec![1, 0x80, 0x13, 0, 99, 1, 0],
            vec![1, 0x88, 0, 7, 1, 9, 0x13, 0, 99, 1],
            vec![1, 0xa0, 0, 7, 1, 9, 255, 0x13, 0, 99, 1],
        ] {
            f.send(0x04, &bad).await;
            f.query(1, 0).await;
            f.read_pair(1, 2).await;
        }
        // Original-Unicast NNI is not a logical local broadcast.
        f.send(0x0a, &[1, 0x80, 0x13, 0, 99, 1]).await;
        f.query(1, 0).await;
        f.read_pair(1, 2).await;
        for bad in [
            vec![1, 0x80, 0x12, 0],
            vec![1, 0x88, 0, 7, 1, 9, 0x12],
            vec![1, 0xa0, 0, 7, 1, 9, 255, 0x12],
            vec![1, 0x80, 0x01],
        ] {
            f.send(0x0a, &bad).await;
        }
        f.query(1, 0).await;
        f.quiet().await;
        f.stop().await;
        assert!(f
            .database()
            .write()
            .await
            .remove(&selected())
            .unwrap()
            .is_some());
        let mut restarted = Fixture::start(full, true, 0).await;
        restarted.read_pair(0, 0).await;
        restarted.send(0x0a, &[1, 0x80, 0x12]).await;
        restarted.quiet().await;
        restarted.stop().await;
    }
}
#[tokio::test]
async fn network_number_real_owners_configured_and_unregistered_are_isolated() {
    for full in [true, false] {
        let mut configured = Fixture::start(full, true, 17).await;
        configured.send(0x04, &[1, 0x80, 0x12]).await;
        configured.response(17, 1).await;
        for flag in [0, 1] {
            configured.announce(99, flag).await;
            configured.query(17, 1).await;
            configured.read_pair(17, 3).await;
        }
        configured.stop().await;
        let mut unregistered = Fixture::start(full, false, 17).await;
        unregistered.send(0x0a, &[1, 0x80, 0x12]).await;
        unregistered.quiet().await;
        unregistered.announce(19, 1).await;
        unregistered.query(19, 0).await;
        unregistered.announce(20, 0).await;
        unregistered.query(19, 0).await;
        // Declared objects remain configured snapshots, not this owner's state.
        unregistered.read_pair(17, 3).await;
        unregistered.stop().await;
    }
}

struct Gates {
    hold_send: std::sync::atomic::AtomicBool,
    hold_stop: std::sync::atomic::AtomicBool,
    send_entered: tokio::sync::Notify,
    send_release: tokio::sync::Semaphore,
    stop_entered: tokio::sync::Notify,
    stop_finished: tokio::sync::Notify,
    stop_release: tokio::sync::Semaphore,
}
impl Gates {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            hold_send: true.into(),
            hold_stop: true.into(),
            send_entered: tokio::sync::Notify::new(),
            send_release: tokio::sync::Semaphore::new(0),
            stop_entered: tokio::sync::Notify::new(),
            stop_finished: tokio::sync::Notify::new(),
            stop_release: tokio::sync::Semaphore::new(0),
        })
    }
}
#[tokio::test]
async fn network_number_real_owner_stop_cancel_joins_control_and_keeps_socket_lease() {
    for full in [true, false] {
        let gates = Gates::new();
        let mut f = Fixture::start_gated(full, true, 17, Some(gates.clone())).await;
        f.send(0x0a, &[1, 0x80, 0x12]).await;
        tokio::time::timeout(Duration::from_secs(1), gates.send_entered.notified())
            .await
            .unwrap();
        let db = f.database();
        assert!(db.write().await.remove(&selected()).is_err());
        {
            let stop = f.stop();
            tokio::pin!(stop);
            tokio::select! { biased; _ = &mut stop => panic!("held transport cleanup completed"), _ = gates.stop_entered.notified() => {} }
        }
        // Caller stop cancellation does not release a still-owned socket.
        assert!(db.write().await.remove(&selected()).is_err());
        gates.send_release.add_permits(1);
        gates.stop_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), f.stop())
            .await
            .unwrap();
        assert!(
            f.sent.try_recv().is_err(),
            "aborted control never reached inner send"
        );
        assert!(db.write().await.remove(&selected()).unwrap().is_some());
        let rebound = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, f.address.port()))
            .await
            .unwrap();
        drop(rebound);
    }
}
#[tokio::test]
async fn network_number_held_database_does_not_block_shutdown() {
    for full in [true, false] {
        let mut f = Fixture::start(full, true, 0).await;
        let db = f.database();
        let guard = db.read().await;
        f.announce(19, 1).await;
        // Tokio's write-preferring lock makes a queued control writer observable.
        tokio::time::timeout(Duration::from_secs(1), async {
            while db.try_read().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(1), f.stop())
            .await
            .unwrap();
        assert_eq!(
            guard
                .get(&selected())
                .unwrap()
                .read_property(
                    bacnet_types::enums::PropertyIdentifier::NETWORK_NUMBER,
                    None
                )
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        drop(guard);
        assert!(db.write().await.remove(&selected()).unwrap().is_some());
    }
}

#[tokio::test]
async fn network_number_endpoint_held_learning_preserves_audit_ack_progress() {
    use bacnet_endpoint_core::coordinator::CanonicalPeer;
    use bacnet_types::enums::ConfirmedServiceChoice as S;
    let mut f = Fixture::start(false, true, 0).await;
    let db = f.database();
    let guard = db.read().await;
    let Owner::Endpoint(endpoint) = &f.owner else {
        unreachable!()
    };
    let mac =
        bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], f.peer.local_addr().unwrap().port());
    let (operation, acknowledged) = endpoint
        .notifications
        .as_ref()
        .unwrap()
        .reserve(CanonicalPeer::direct(&mac), S::CONFIRMED_AUDIT_NOTIFICATION)
        .unwrap();
    f.announce(19, 1).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while db.try_read().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.send(
        0x0a,
        &[
            1,
            0,
            0x20,
            operation.invoke_id(),
            S::CONFIRMED_AUDIT_NOTIFICATION.to_raw(),
        ],
    )
    .await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), acknowledged)
            .await
            .unwrap()
            .unwrap(),
        bacnet_server::server::CovAckResult::Ack
    ));
    drop(operation);
    drop(guard);
    f.query(19, 0).await;
    f.stop().await;
}

#[tokio::test]
async fn network_number_bare_drop_cancels_held_control_without_leaking_lease() {
    for full in [true, false] {
        let gates = Gates::new();
        let f = Fixture::start_gated(full, true, 17, Some(gates.clone())).await;
        f.send(0x0a, &[1, 0x80, 0x12]).await;
        tokio::time::timeout(Duration::from_secs(1), gates.send_entered.notified())
            .await
            .unwrap();
        let db = f.database();
        let address = f.address;
        drop(f); // No send/stop gate release: task aborts must destroy these owners.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if db.write().await.remove(&selected()).is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let _rebound = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, address.port()))
            .await
            .unwrap();
    }
}
#[tokio::test]
async fn network_number_endpoint_input_closed_keeps_admitted_control_lease_until_completion() {
    let gates = Gates::new();
    gates.hold_send.store(false, Ordering::SeqCst);
    gates.hold_stop.store(false, Ordering::SeqCst);
    let (input, receiver) = mpsc::channel(4);
    let mut f = Fixture::start_options(false, true, 17, Some(gates.clone()), Some(receiver)).await;
    let db = f.database();
    let guard = db.read().await;
    input
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: bytes::Bytes::from_static(&[1, 0x80, 0x12]),
            source_mac: bacnet_types::MacAddr::from_slice(&[127, 0, 0, 1, 0xba, 0xc0]),
            link_layer_group: false,
            data_attributes: vec![],
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while db.try_read().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(input);
    tokio::time::timeout(Duration::from_secs(1), gates.stop_finished.notified())
        .await
        .unwrap();
    assert_eq!(
        guard.registered_bip_port_internal(),
        Some(selected()),
        "control still owns the lease after actual socket cleanup"
    );
    let _rebound = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, f.address.port()))
        .await
        .unwrap();
    drop(guard);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if db.write().await.remove(&selected()).is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        f.sent.try_recv().is_err(),
        "closed ingress refuses the control response"
    );
    f.stop().await;
}
