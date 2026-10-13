//! Once an endpoint session knows the number of its own network (#1403),
//! traffic it starts for a station on that network goes as local traffic: to
//! the station's MAC with no DNET. A routed read naming the number goes to its
//! DADR, and the answer from there completes it, as does one relayed back
//! with this network as its SNET and the DADR as its SADR (#1465). Another
//! network keeps its DNET, every network does while the number is unknown,
//! and answers keep the route their request arrived by. The source Audit
//! cases are in `source`.
//!
//! The capture link carries B/IP-shaped MACs: the session is `10.0.0.1`, the
//! peer `10.0.0.3`, behind router `10.0.0.9` when routed. The number is
//! learned through the session's own Network-Number-Is intake.
use super::*;
use crate::roles::EndpointApduDestination;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, ComplexAck, ConfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::time::{timeout, Duration};

#[path = "local_network_source_tests.rs"]
mod source;

/// The number of the network the session is attached to.
const THIS_NETWORK: u16 = 77;
const REMOTE_NETWORK: u16 = 5;
const SELF: u8 = 1;
const PEER: u8 = 3;
const ROUTER: u8 = 9;
const PORT: u16 = 0xBAC0;

fn host(last: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, last), PORT)
}

fn mac(last: u8) -> MacAddr {
    MacAddr::from_slice(&encode_bip_mac([10, 0, 0, last], PORT))
}

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

fn analog() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    timeout(Duration::from_secs(5), future)
        .await
        .expect("local network fixture made no progress")
}

/// One frame the session handed the link: its link MAC (empty for a link
/// broadcast) and NPDU.
struct Sent {
    link: MacAddr,
    npdu: Bytes,
}

/// A B/IP-shaped link that records every send and takes injected input.
struct Capture {
    local: MacAddr,
    inbound: Option<mpsc::Receiver<ReceivedNpdu>>,
    outbound: mpsc::Sender<Sent>,
    lease: Option<Arc<()>>,
    fail_stop: bool,
}

impl Capture {
    fn record(&self, link: &[u8], npdu: &[u8]) -> Result<(), Error> {
        self.outbound
            .try_send(Sent {
                link: MacAddr::from_slice(link),
                npdu: Bytes::copy_from_slice(npdu),
            })
            .expect("bounded send observation");
        Ok(())
    }
}

impl TransportPort for Capture {
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        Some(host(255))
    }
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        true
    }
    fn bip_port(&self) -> Option<bacnet_transport::port::BipPort> {
        Some(bacnet_transport::port::BipPort {
            endpoint: host(SELF),
            mode: bacnet_types::bip_port::BipPortMode::Normal,
        })
    }
    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        self.lease = Some(lease);
        Ok(())
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.inbound.take().expect("started once"))
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.lease = None;
        if std::mem::take(&mut self.fail_stop) {
            Err(Error::Encoding("injected capture stop failure".into()))
        } else {
            Ok(())
        }
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.record(mac, npdu)
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.record(&[], npdu)
    }
    fn local_mac(&self) -> &[u8] {
        &self.local
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}

/// Where a frame went: its link MAC and the NPDU's DNET/DADR, if any.
#[derive(Debug, PartialEq, Eq)]
struct Route {
    link: MacAddr,
    destination: Option<NpduAddress>,
}

/// Straight to `station` on this link, with no DNET.
fn local(station: u8) -> Route {
    Route {
        link: mac(station),
        destination: None,
    }
}

/// Through link MAC `link` (empty for a link broadcast) to the peer on
/// `network`.
fn routed(link: MacAddr, network: u16) -> Route {
    Route {
        link,
        destination: Some(NpduAddress {
            network,
            mac_address: mac(PEER),
        }),
    }
}

/// The peer on `network`, as a routed SNET/SADR or DNET/DADR.
fn peer_on(network: u16) -> Option<NpduAddress> {
    Some(NpduAddress {
        network,
        mac_address: mac(PEER),
    })
}

struct Endpoint {
    session: EndpointSession<Capture>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    sent: mpsc::Receiver<Sent>,
}

impl Endpoint {
    /// A session of `role` on the capture link, not yet started.
    fn new(role: SessionRole) -> Self {
        Self::with_stop_failure(role, false)
    }

    fn with_stop_failure(role: SessionRole, fail_stop: bool) -> Self {
        let (inbound, input) = mpsc::channel(32);
        let (outbound, sent) = mpsc::channel(32);
        let transport = Capture {
            local: mac(SELF),
            inbound: Some(input),
            outbound,
            lease: None,
            fail_stop,
        };
        let config = SessionConfig {
            apdu_timeout_ms: 5_000,
            ..SessionConfig::default()
        };
        Self {
            session: EndpointSession::new(transport, role, config).unwrap(),
            inbound,
            sent,
        }
    }

    /// A started session of `role` that has learned `THIS_NETWORK`, or no
    /// number at all.
    async fn started(role: SessionRole, learned: bool) -> Self {
        let mut endpoint = Self::new(role);
        let db = crate::DeviceIdentity::new(123, 42)
            .unwrap()
            .build_database()
            .unwrap();
        endpoint.session = endpoint.session.with_database(db);
        endpoint.start(learned).await;
        endpoint
    }

    async fn start(&mut self, learned: bool) {
        bounded(self.session.start()).await.unwrap();
        if learned {
            self.learn(THIS_NETWORK).await;
        }
    }

    /// Announce `number` by local broadcast, then query it: the Number owner
    /// takes controls in order, so its answer holds the announced number and
    /// has published it.
    async fn learn(&mut self, number: u16) {
        let [high, low] = number.to_be_bytes();
        let announcement = [1, 0x80, 0x13, high, low, 0];
        self.deliver(mac(ROUTER), Bytes::copy_from_slice(&announcement), true)
            .await;
        self.deliver(mac(PEER), Bytes::from_static(&[1, 0x80, 0x12]), false)
            .await;
        let answer = self.next().await;
        assert!(answer.link.is_empty(), "Network-Number-Is is broadcast");
        assert_eq!(answer.npdu.as_ref(), announcement);
    }

    async fn deliver(&self, link: MacAddr, npdu: Bytes, group: bool) {
        self.inbound
            .send(ReceivedNpdu {
                npdu,
                source_mac: link,
                link_layer_group: group,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                direct_response: None,
                reply_tx: None,
            })
            .await
            .unwrap();
    }

    /// Hand the session `apdu` from link MAC `link`, with `source` as its
    /// SNET/SADR when relayed by a router.
    async fn deliver_apdu(&self, apdu: Apdu, link: MacAddr, source: Option<NpduAddress>) {
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &apdu).unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                source,
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.deliver(link, npdu.freeze(), false).await;
    }

    async fn next(&mut self) -> Sent {
        bounded(self.sent.recv()).await.expect("capture link open")
    }

    /// Start a ReadProperty of the peer's analog input at `destination`.
    fn read(
        &self,
        destination: EndpointApduDestination,
    ) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
        let client = self.session.cloned_client_handle().unwrap();
        tokio::spawn(async move {
            client
                .read_property_with_destination(
                    destination,
                    Vec::new(),
                    analog(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                )
                .await
        })
    }

    /// The confirmed request the session sends next, and where it went.
    async fn request(&mut self) -> (Route, ConfirmedRequest) {
        let sent = self.next().await;
        let npdu = decode_npdu(sent.npdu).unwrap();
        let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
            panic!("a confirmed request")
        };
        let route = Route {
            link: sent.link,
            destination: npdu.destination,
        };
        (route, request)
    }

    async fn stop(mut self) {
        bounded(self.session.stop()).await.unwrap();
    }
}

/// The ReadPropertyACK answering `request` with Unsigned `value`.
fn ack(request: &ConfirmedRequest, value: u8) -> Apdu {
    let mut service = BytesMut::new();
    ReadPropertyACK {
        object_identifier: analog(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: vec![0x21, value],
    }
    .encode(&mut service);
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: request.invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: service.freeze(),
    })
}

fn routed_read(network: u16) -> EndpointApduDestination {
    EndpointApduDestination::Routed {
        destination_network: network,
        destination_mac: mac(PEER),
        router_mac: mac(ROUTER),
    }
}

fn routed_by_broadcast(network: u16) -> EndpointApduDestination {
    EndpointApduDestination::RoutedViaLocalBroadcast {
        destination_network: network,
        destination_mac: mac(PEER),
    }
}

#[tokio::test]
async fn routed_reads_naming_this_network_go_local_and_a_direct_or_relayed_answer_completes_them() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let mut endpoint = Endpoint::started(role, true).await;
        for destination in [routed_read(THIS_NETWORK), routed_by_broadcast(THIS_NETWORK)] {
            // Straight from the peer's MAC, or relayed back by a router with
            // this network as its SNET and the peer's MAC as its SADR: both
            // name the station the request went to (#1465).
            for (link, source) in [(mac(PEER), None), (mac(ROUTER), peer_on(THIS_NETWORK))] {
                let relayed = source.is_some();
                let read = endpoint.read(destination.clone());
                let (route, request) = endpoint.request().await;
                assert_eq!(route, local(PEER), "{role:?}");
                endpoint.deliver_apdu(ack(&request, 2), link, source).await;
                let answer = bounded(read).await.unwrap().unwrap();
                assert_eq!(
                    answer.property_value,
                    [0x21, 2],
                    "{role:?}, relayed {relayed}"
                );
                assert_eq!(endpoint.session.active_leases(), 0);
            }
        }
        endpoint.stop().await;
    }
}

/// A relayed answer completes a local read only when it names this network,
/// the station the read went to and the read's invoke ID. Answers relayed
/// from another network, from another station here or for another invoke ID
/// complete nothing, so the station's own answer after them is the one the
/// read returns.
#[tokio::test]
async fn a_relayed_answer_naming_another_network_station_or_invoke_id_completes_nothing() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let mut endpoint = Endpoint::started(role, true).await;
        let read = endpoint.read(routed_read(THIS_NETWORK));
        let (route, request) = endpoint.request().await;
        assert_eq!(route, local(PEER), "{role:?}");
        let another_station = Some(NpduAddress {
            network: THIS_NETWORK,
            mac_address: mac(4),
        });
        for (value, source) in [(5, peer_on(REMOTE_NETWORK)), (6, another_station)] {
            endpoint
                .deliver_apdu(ack(&request, value), mac(ROUTER), source)
                .await;
        }
        let mut another_invoke_id = request.clone();
        another_invoke_id.invoke_id = request.invoke_id.wrapping_add(1);
        endpoint
            .deliver_apdu(
                ack(&another_invoke_id, 7),
                mac(ROUTER),
                peer_on(THIS_NETWORK),
            )
            .await;
        endpoint
            .deliver_apdu(ack(&request, 8), mac(ROUTER), peer_on(THIS_NETWORK))
            .await;
        let answer = bounded(read).await.unwrap().unwrap();
        assert_eq!(answer.property_value, [0x21, 8], "{role:?}");
        assert_eq!(endpoint.session.active_leases(), 0);
        endpoint.stop().await;
    }
}

/// While the number is unknown nothing changes: an answer relayed with SNET
/// 77 is not from the MAC a direct read went to, so only that MAC's own
/// answer completes it.
#[tokio::test]
async fn with_the_number_unknown_a_relayed_answer_leaves_a_direct_read_open() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let mut endpoint = Endpoint::started(role, false).await;
        let read = endpoint.read(EndpointApduDestination::Direct {
            destination_mac: mac(PEER),
        });
        let (route, request) = endpoint.request().await;
        assert_eq!(route, local(PEER), "{role:?}");
        endpoint
            .deliver_apdu(ack(&request, 1), mac(ROUTER), peer_on(THIS_NETWORK))
            .await;
        endpoint
            .deliver_apdu(ack(&request, 2), mac(PEER), None)
            .await;
        let answer = bounded(read).await.unwrap().unwrap();
        assert_eq!(answer.property_value, [0x21, 2], "{role:?}");
        assert_eq!(endpoint.session.active_leases(), 0);
        endpoint.stop().await;
    }
}

/// A read routed to this network while its number was unknown goes through
/// the router with the DNET and keeps that routed key. When the number is
/// learned before the answer comes back, the answer the router relays with
/// this network as its SNET still completes it, as it would have before
/// (#1465).
#[tokio::test]
async fn a_read_routed_before_the_number_was_learned_completes_on_its_relayed_answer() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let mut endpoint = Endpoint::started(role, false).await;
        let read = endpoint.read(routed_read(THIS_NETWORK));
        let (route, request) = endpoint.request().await;
        assert_eq!(route, routed(mac(ROUTER), THIS_NETWORK), "{role:?}");
        endpoint.learn(THIS_NETWORK).await;
        endpoint
            .deliver_apdu(ack(&request, 9), mac(ROUTER), peer_on(THIS_NETWORK))
            .await;
        let answer = bounded(read).await.unwrap().unwrap();
        assert_eq!(answer.property_value, [0x21, 9], "{role:?}");
        assert_eq!(endpoint.session.active_leases(), 0);
        endpoint.stop().await;
    }
}

#[tokio::test]
async fn routed_reads_keep_their_dnet_for_another_network_or_an_unknown_number() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for learned in [true, false] {
            let mut endpoint = Endpoint::started(role, learned).await;
            // With the number known only another network is routed; while it
            // is unknown, this network is routed by its number as well.
            let network = if learned {
                REMOTE_NETWORK
            } else {
                THIS_NETWORK
            };
            for (destination, link) in [
                (routed_read(network), mac(ROUTER)),
                (routed_by_broadcast(network), MacAddr::new()),
            ] {
                let read = endpoint.read(destination);
                let (route, request) = endpoint.request().await;
                assert_eq!(route, routed(link, network), "{role:?}, learned {learned}");
                endpoint
                    .deliver_apdu(ack(&request, 3), mac(ROUTER), peer_on(network))
                    .await;
                let answer = bounded(read).await.unwrap().unwrap();
                assert_eq!(answer.property_value, [0x21, 3]);
            }
            endpoint.stop().await;
        }
    }
}

#[tokio::test]
async fn a_registered_ports_configured_number_is_local_from_startup() {
    let port = oid(ObjectType::NETWORK_PORT, 2);
    let identity = crate::DeviceIdentity::new(123, 42)
        .unwrap()
        .with_bip_port(2, THIS_NETWORK.into(), *host(SELF).ip(), PORT)
        .unwrap();
    let db = identity.build_database().unwrap();
    let mut endpoint = Endpoint::new(SessionRole::Both);
    endpoint.session = endpoint
        .session
        .with_database(db)
        .with_identity(identity)
        .with_registered_network_port(port);
    // No Number control arrives: the port's number is published at startup.
    endpoint.start(false).await;
    let read = endpoint.read(routed_read(THIS_NETWORK));
    let (route, request) = endpoint.request().await;
    assert_eq!(route, local(PEER));
    endpoint
        .deliver_apdu(ack(&request, 4), mac(PEER), None)
        .await;
    assert_eq!(
        bounded(read).await.unwrap().unwrap().property_value,
        [0x21, 4]
    );
    endpoint.stop().await;
}

#[tokio::test]
async fn answers_keep_the_route_their_request_arrived_by() {
    for role in [SessionRole::ServerOnly, SessionRole::Both] {
        let mut endpoint = Endpoint::started(role, true).await;
        let egress = endpoint.session.egress.as_ref().unwrap();
        assert_eq!(egress.local_network_number().get(), Some(THIS_NETWORK));
        let mut service = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: oid(ObjectType::DEVICE, 123),
            property_identifier: PropertyIdentifier::OBJECT_IDENTIFIER,
            property_array_index: None,
        }
        .encode(&mut service);
        let request = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 42,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: service.freeze(),
        });
        // A request relayed from this network by its number is answered the
        // way it came, through the router that relayed it.
        endpoint
            .deliver_apdu(request, mac(ROUTER), peer_on(THIS_NETWORK))
            .await;
        let sent = endpoint.next().await;
        let npdu = decode_npdu(sent.npdu).unwrap();
        let route = Route {
            link: sent.link,
            destination: npdu.destination,
        };
        assert_eq!(route, routed(mac(ROUTER), THIS_NETWORK), "{role:?}");
        assert!(
            matches!(decode_apdu(npdu.payload).unwrap(), Apdu::ComplexAck(ack) if ack.invoke_id == 42)
        );
        endpoint.stop().await;
    }
}
