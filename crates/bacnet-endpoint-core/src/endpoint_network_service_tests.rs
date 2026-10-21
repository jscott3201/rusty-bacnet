use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest, UnconfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, NpduAddress};
use bacnet_npdu::NetworkPriority;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort};
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use super::*;

const WAIT: Duration = Duration::from_secs(1);
const EFFECTIVE_GROUP_ERROR: &str =
    "effective group destination requires a valid unconfirmed request APDU";

#[derive(Debug, PartialEq, Eq)]
enum LinkDestination {
    Unicast(MacAddr),
    Broadcast,
}

struct CapturedSend {
    npdu: Bytes,
    destination: LinkDestination,
    data_attributes: Vec<DataAttribute>,
}

struct CaptureTransport {
    receiver: Option<mpsc::Receiver<ReceivedNpdu>>,
    sent: mpsc::Sender<CapturedSend>,
    stops: Arc<AtomicUsize>,
    local_mac: MacAddr,
}

struct CaptureHandle {
    _sender: mpsc::Sender<ReceivedNpdu>,
    sent: mpsc::Receiver<CapturedSend>,
    stops: Arc<AtomicUsize>,
}

fn capture_transport() -> (CaptureTransport, CaptureHandle) {
    let (sender, receiver) = mpsc::channel(1);
    let (sent_tx, sent_rx) = mpsc::channel(8);
    let stops = Arc::new(AtomicUsize::new(0));
    (
        CaptureTransport {
            receiver: Some(receiver),
            sent: sent_tx,
            stops: Arc::clone(&stops),
            local_mac: MacAddr::from_slice(&[0xaa]),
        },
        CaptureHandle {
            _sender: sender,
            sent: sent_rx,
            stops,
        },
    )
}

impl CaptureTransport {
    async fn capture(
        &self,
        npdu: &[u8],
        destination: LinkDestination,
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.sent
            .send(CapturedSend {
                npdu: Bytes::copy_from_slice(npdu),
                destination,
                data_attributes: data_attributes.to_vec(),
            })
            .await
            .map_err(|_| Error::Encoding("capture receiver closed".into()))
    }
}

impl TransportPort for CaptureTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.receiver
            .take()
            .ok_or_else(|| Error::Encoding("capture transport already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.capture(
            npdu,
            LinkDestination::Unicast(MacAddr::from_slice(mac)),
            &[],
        )
        .await
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.capture(
            npdu,
            LinkDestination::Unicast(MacAddr::from_slice(mac)),
            data_attributes,
        )
        .await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.capture(npdu, LinkDestination::Broadcast, &[]).await
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.capture(npdu, LinkDestination::Broadcast, data_attributes)
            .await
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        mac == BROADCAST_MAC
    }

    /// Its broadcast MAC, and every group form a B/IP link with broadcast
    /// address 192.168.1.255 recognises (#1479).
    fn is_group_destination(&self, mac: &[u8]) -> bool {
        mac == BROADCAST_MAC
            || bacnet_transport::bip::BipTransport::new(
                std::net::Ipv4Addr::LOCALHOST,
                0xBAC0,
                std::net::Ipv4Addr::new(192, 168, 1, 255),
            )
            .is_group_destination(mac)
    }
}

/// The capture link's broadcast MAC, as on MS/TP.
const BROADCAST_MAC: [u8; 1] = [0xff];

/// The B/IP group forms the capture link recognises: the limited broadcast
/// and the configured broadcast IP at this port and another, and IPv4
/// multicast.
const BIP_GROUPS: [[u8; 6]; 5] = [
    [255, 255, 255, 255, 0xBA, 0xC0],
    [255, 255, 255, 255, 0xBA, 0xC1],
    [192, 168, 1, 255, 0xBA, 0xC1],
    [224, 0, 0, 1, 0xBA, 0xC0],
    [239, 255, 255, 250, 0x07, 0x6C],
];

fn encoded_unconfirmed_request() -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_apdu(
        &mut encoded,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        }),
    )
    .unwrap();
    encoded.to_vec()
}

fn encoded_confirmed_request() -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_apdu(
        &mut encoded,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id: 0x41,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::new(),
        }),
    )
    .unwrap();
    encoded.to_vec()
}

/// One send through the endpoint egress and the wire it must produce.
struct CommandCase {
    destination: EndpointApduDestination,
    expected_link_destination: LinkDestination,
    expected_npdu_destination: Option<NpduAddress>,
    attribute_type: u8,
    expecting_reply: bool,
    priority: NetworkPriority,
}

async fn assert_command(egress: &EndpointEgress, handle: &mut CaptureHandle, case: CommandCase) {
    let CommandCase {
        destination,
        expected_link_destination,
        expected_npdu_destination,
        attribute_type,
        expecting_reply,
        priority,
    } = case;
    let apdu = encoded_unconfirmed_request();
    let data_attributes = vec![DataAttribute {
        option_type: attribute_type,
        must_understand: attribute_type.is_multiple_of(2),
        data: vec![attribute_type, attribute_type.wrapping_add(1)],
    }];

    egress
        .send_apdu(
            apdu.clone(),
            destination,
            expecting_reply,
            priority,
            data_attributes.clone(),
        )
        .await
        .unwrap();

    let captured = timeout(WAIT, handle.sent.recv())
        .await
        .expect("network-service send timed out")
        .expect("capture channel closed");
    assert_eq!(captured.destination, expected_link_destination);
    assert_eq!(captured.data_attributes, data_attributes);
    let decoded = decode_npdu(captured.npdu).unwrap();
    assert_eq!(decoded.destination, expected_npdu_destination);
    assert_eq!(decoded.expecting_reply, expecting_reply);
    assert_eq!(decoded.priority, priority);
    assert_eq!(decoded.payload.as_ref(), apdu);
    assert!(matches!(
        decode_apdu(decoded.payload),
        Ok(Apdu::UnconfirmedRequest(_))
    ));
}

#[tokio::test]
async fn network_service_delegates_every_apdu_destination_with_attributes() {
    let (transport, mut handle) = capture_transport();
    let mut endpoint = EndpointIngress::new(transport, 8);
    let ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress;

    assert_command(
        &egress,
        &mut handle,
        CommandCase {
            destination: EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(&[0x10]),
            },
            expected_link_destination: LinkDestination::Unicast(MacAddr::from_slice(&[0x10])),
            expected_npdu_destination: None,
            attribute_type: 1,
            expecting_reply: true,
            priority: NetworkPriority::Urgent,
        },
    )
    .await;
    let routed_destination = NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0x20]),
    };
    assert_command(
        &egress,
        &mut handle,
        CommandCase {
            destination: EndpointApduDestination::Routed {
                destination_network: routed_destination.network,
                destination_mac: routed_destination.mac_address.clone(),
                router_mac: MacAddr::from_slice(&[0x21]),
            },
            expected_link_destination: LinkDestination::Unicast(MacAddr::from_slice(&[0x21])),
            expected_npdu_destination: Some(routed_destination),
            attribute_type: 2,
            expecting_reply: false,
            priority: NetworkPriority::CriticalEquipment,
        },
    )
    .await;
    let unknown_router_destination = NpduAddress {
        network: 300,
        mac_address: MacAddr::from_slice(&[0x30]),
    };
    assert_command(
        &egress,
        &mut handle,
        CommandCase {
            destination: EndpointApduDestination::RoutedViaLocalBroadcast {
                destination_network: unknown_router_destination.network,
                destination_mac: unknown_router_destination.mac_address.clone(),
            },
            expected_link_destination: LinkDestination::Broadcast,
            expected_npdu_destination: Some(unknown_router_destination),
            attribute_type: 3,
            expecting_reply: true,
            priority: NetworkPriority::LifeSafety,
        },
    )
    .await;
    assert_command(
        &egress,
        &mut handle,
        CommandCase {
            destination: EndpointApduDestination::LocalBroadcast,
            expected_link_destination: LinkDestination::Broadcast,
            expected_npdu_destination: None,
            attribute_type: 4,
            expecting_reply: false,
            priority: NetworkPriority::Normal,
        },
    )
    .await;
    assert_command(
        &egress,
        &mut handle,
        CommandCase {
            destination: EndpointApduDestination::RemoteBroadcast {
                destination_network: 400,
            },
            expected_link_destination: LinkDestination::Broadcast,
            expected_npdu_destination: Some(NpduAddress {
                network: 400,
                mac_address: MacAddr::new(),
            }),
            attribute_type: 5,
            expecting_reply: true,
            priority: NetworkPriority::Urgent,
        },
    )
    .await;
    assert_command(
        &egress,
        &mut handle,
        CommandCase {
            destination: EndpointApduDestination::GlobalBroadcast,
            expected_link_destination: LinkDestination::Broadcast,
            expected_npdu_destination: Some(NpduAddress {
                network: 0xffff,
                mac_address: MacAddr::new(),
            }),
            attribute_type: 6,
            expecting_reply: false,
            priority: NetworkPriority::CriticalEquipment,
        },
    )
    .await;

    endpoint.stop().await.unwrap();
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn effective_group_destinations_reject_confirmed_and_malformed_apdus_without_emission() {
    let (transport, mut handle) = capture_transport();
    let mut endpoint = EndpointIngress::new(transport, 8);
    let ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress;

    for destination in [
        EndpointApduDestination::LocalBroadcast,
        EndpointApduDestination::RemoteBroadcast {
            destination_network: 400,
        },
        EndpointApduDestination::GlobalBroadcast,
    ]
    .into_iter()
    // With no DNET, the link's broadcast MAC, or any other group address, is
    // a local broadcast (#1479).
    .chain(
        std::iter::once(&BROADCAST_MAC[..])
            .chain(BIP_GROUPS.iter().map(|mac| &mac[..]))
            .map(|mac| EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(mac),
            }),
    ) {
        for apdu in [encoded_confirmed_request(), vec![0xff]] {
            assert!(matches!(
                egress
                    .send_apdu(
                        apdu,
                        destination.clone(),
                        false,
                        NetworkPriority::Normal,
                        Vec::new(),
                    )
                    .await,
                Err(Error::Encoding(message)) if message == EFFECTIVE_GROUP_ERROR
            ));
        }
    }
    assert!(matches!(
        handle.sent.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    // An Unconfirmed-Request still goes to the broadcast MAC: this link
    // gives no group rule for the network layer to judge by (#1677).
    egress
        .send_apdu(
            encoded_unconfirmed_request(),
            EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(&BROADCAST_MAC),
            },
            false,
            NetworkPriority::Normal,
            Vec::new(),
        )
        .await
        .unwrap();
    let captured = timeout(WAIT, handle.sent.recv()).await.unwrap().unwrap();
    assert_eq!(
        captured.destination,
        LinkDestination::Unicast(MacAddr::from_slice(&BROADCAST_MAC))
    );

    endpoint.stop().await.unwrap();
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
}

/// A routed destination with no DADR is a remote broadcast, so a confirmed
/// request to it is refused by the network layer, naming its PDU type, and
/// nothing goes out (#1479). The routed form through a known router still
/// sends an Unconfirmed-Request there; the local-broadcast form refuses an
/// empty DADR outright and points to the remote-broadcast send.
#[tokio::test]
async fn routed_destinations_without_a_dadr_refuse_all_but_an_unconfirmed_request() {
    let (transport, mut handle) = capture_transport();
    let mut endpoint = EndpointIngress::new(transport, 8);
    let ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress;
    let routed = EndpointApduDestination::Routed {
        destination_network: 400,
        destination_mac: MacAddr::new(),
        router_mac: MacAddr::from_slice(&[0x40]),
    };
    let via_broadcast = EndpointApduDestination::RoutedViaLocalBroadcast {
        destination_network: 400,
        destination_mac: MacAddr::new(),
    };
    let send = |apdu, destination| {
        egress.send_apdu(
            apdu,
            destination,
            false,
            NetworkPriority::Normal,
            Vec::new(),
        )
    };
    for (apdu, destination, refusal) in [
        (
            encoded_confirmed_request(),
            routed.clone(),
            "not PDU type CONFIRMED_REQUEST",
        ),
        (
            encoded_confirmed_request(),
            via_broadcast.clone(),
            "use broadcast_to_network",
        ),
        (
            encoded_unconfirmed_request(),
            via_broadcast,
            "use broadcast_to_network",
        ),
    ] {
        let message = send(apdu, destination).await.unwrap_err().to_string();
        assert!(message.contains(refusal), "{message}");
    }
    assert!(matches!(
        handle.sent.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    send(encoded_unconfirmed_request(), routed).await.unwrap();
    let captured = timeout(WAIT, handle.sent.recv()).await.unwrap().unwrap();
    assert_eq!(
        captured.destination,
        LinkDestination::Unicast(MacAddr::from_slice(&[0x40]))
    );
    let npdu = decode_npdu(captured.npdu).unwrap();
    assert_eq!(
        npdu.destination,
        Some(NpduAddress {
            network: 400,
            mac_address: MacAddr::new(),
        })
    );

    endpoint.stop().await.unwrap();
}

#[tokio::test]
async fn routed_via_local_broadcast_accepts_confirmed_request_for_ultimate_unicast() {
    let (transport, mut handle) = capture_transport();
    let mut endpoint = EndpointIngress::new(transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress;
    let apdu = encoded_confirmed_request();
    let destination = NpduAddress {
        network: 300,
        mac_address: MacAddr::from_slice(&[0x30]),
    };

    egress
        .send_apdu(
            apdu.clone(),
            EndpointApduDestination::RoutedViaLocalBroadcast {
                destination_network: destination.network,
                destination_mac: destination.mac_address.clone(),
            },
            true,
            NetworkPriority::LifeSafety,
            Vec::new(),
        )
        .await
        .unwrap();

    let captured = timeout(WAIT, handle.sent.recv())
        .await
        .expect("routed local-broadcast send timed out")
        .expect("capture channel closed");
    assert_eq!(captured.destination, LinkDestination::Broadcast);
    let npdu = decode_npdu(captured.npdu).unwrap();
    assert_eq!(npdu.destination, Some(destination));
    assert!(npdu.expecting_reply);
    assert_eq!(npdu.priority, NetworkPriority::LifeSafety);
    assert_eq!(npdu.payload.as_ref(), apdu);
    assert!(matches!(
        decode_apdu(npdu.payload),
        Ok(Apdu::ConfirmedRequest(_))
    ));

    endpoint.stop().await.unwrap();
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn only_destinations_on_the_known_local_network_are_localized() {
    let station = || MacAddr::from_slice(&[0x30]);
    let named = |network| {
        [
            EndpointApduDestination::Routed {
                destination_network: network,
                destination_mac: station(),
                router_mac: MacAddr::from_slice(&[0x09]),
            },
            EndpointApduDestination::RoutedViaLocalBroadcast {
                destination_network: network,
                destination_mac: station(),
            },
            EndpointApduDestination::RemoteBroadcast {
                destination_network: network,
            },
        ]
    };
    let direct = EndpointApduDestination::Direct {
        destination_mac: station(),
    };
    let [unicast, via_broadcast, broadcast] = named(300);
    assert_eq!(unicast.localized(Some(300)), direct);
    assert_eq!(via_broadcast.localized(Some(300)), direct);
    assert_eq!(
        broadcast.localized(Some(300)),
        EndpointApduDestination::LocalBroadcast
    );
    // Another network, and every network while the number is unknown, keeps
    // its DNET; destinations naming no network never change.
    for local_network in [Some(301), None] {
        for destination in named(300) {
            assert_eq!(destination.clone().localized(local_network), destination);
        }
    }
    for destination in [
        direct.clone(),
        EndpointApduDestination::LocalBroadcast,
        EndpointApduDestination::GlobalBroadcast,
    ] {
        assert_eq!(destination.clone().localized(Some(300)), destination);
    }
}

/// The egress frames each destination as its caller names it, even one
/// naming the published local number: answers keep the route their request
/// arrived by, and only the senders that start traffic localize it (#1403).
#[tokio::test]
async fn the_egress_sends_a_destination_as_named_once_a_number_is_published() {
    let (transport, mut handle) = capture_transport();
    let mut endpoint = EndpointIngress::new(transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress;
    let slot = egress.local_network_number().clone();
    slot.publish(bacnet_types::network_number::NetworkNumber::configured(300).unwrap());
    assert_eq!(egress.local_network_number().get(), Some(300));
    let destination = NpduAddress {
        network: 300,
        mac_address: MacAddr::from_slice(&[0x30]),
    };
    egress
        .send_apdu(
            encoded_confirmed_request(),
            EndpointApduDestination::Routed {
                destination_network: destination.network,
                destination_mac: destination.mac_address.clone(),
                router_mac: MacAddr::from_slice(&[0x09]),
            },
            false,
            NetworkPriority::Normal,
            Vec::new(),
        )
        .await
        .unwrap();
    let captured = timeout(WAIT, handle.sent.recv())
        .await
        .expect("routed send timed out")
        .expect("capture channel closed");
    assert_eq!(
        captured.destination,
        LinkDestination::Unicast(MacAddr::from_slice(&[0x09]))
    );
    assert_eq!(
        decode_npdu(captured.npdu).unwrap().destination,
        Some(destination)
    );

    endpoint.stop().await.unwrap();
}
