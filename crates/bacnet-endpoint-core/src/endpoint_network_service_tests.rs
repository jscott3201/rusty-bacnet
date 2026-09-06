use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest, UnconfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, NpduAddress};
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort};
use bacnet_types::enums::{ConfirmedServiceChoice, NetworkPriority, UnconfirmedServiceChoice};
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

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

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

#[allow(clippy::too_many_arguments)]
async fn assert_command(
    egress: &EndpointEgress,
    handle: &mut CaptureHandle,
    destination: EndpointApduDestination,
    expected_link_destination: LinkDestination,
    expected_npdu_destination: Option<NpduAddress>,
    attribute_type: u8,
    expecting_reply: bool,
    priority: NetworkPriority,
) {
    let apdu = encoded_unconfirmed_request();
    let data_attributes = vec![DataAttribute {
        option_type: attribute_type,
        must_understand: attribute_type % 2 == 0,
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
        EndpointApduDestination::Direct {
            destination_mac: MacAddr::from_slice(&[0x10]),
        },
        LinkDestination::Unicast(MacAddr::from_slice(&[0x10])),
        None,
        1,
        true,
        NetworkPriority::URGENT,
    )
    .await;
    let routed_destination = NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0x20]),
    };
    assert_command(
        &egress,
        &mut handle,
        EndpointApduDestination::Routed {
            destination_network: routed_destination.network,
            destination_mac: routed_destination.mac_address.clone(),
            router_mac: MacAddr::from_slice(&[0x21]),
        },
        LinkDestination::Unicast(MacAddr::from_slice(&[0x21])),
        Some(routed_destination),
        2,
        false,
        NetworkPriority::CRITICAL_EQUIPMENT,
    )
    .await;
    let unknown_router_destination = NpduAddress {
        network: 300,
        mac_address: MacAddr::from_slice(&[0x30]),
    };
    assert_command(
        &egress,
        &mut handle,
        EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network: unknown_router_destination.network,
            destination_mac: unknown_router_destination.mac_address.clone(),
        },
        LinkDestination::Broadcast,
        Some(unknown_router_destination),
        3,
        true,
        NetworkPriority::LIFE_SAFETY,
    )
    .await;
    assert_command(
        &egress,
        &mut handle,
        EndpointApduDestination::LocalBroadcast,
        LinkDestination::Broadcast,
        None,
        4,
        false,
        NetworkPriority::NORMAL,
    )
    .await;
    assert_command(
        &egress,
        &mut handle,
        EndpointApduDestination::RemoteBroadcast {
            destination_network: 400,
        },
        LinkDestination::Broadcast,
        Some(NpduAddress {
            network: 400,
            mac_address: MacAddr::new(),
        }),
        5,
        true,
        NetworkPriority::URGENT,
    )
    .await;
    assert_command(
        &egress,
        &mut handle,
        EndpointApduDestination::GlobalBroadcast,
        LinkDestination::Broadcast,
        Some(NpduAddress {
            network: 0xffff,
            mac_address: MacAddr::new(),
        }),
        6,
        false,
        NetworkPriority::CRITICAL_EQUIPMENT,
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
    ] {
        for apdu in [encoded_confirmed_request(), vec![0xff]] {
            assert!(matches!(
                egress
                    .send_apdu(
                        apdu,
                        destination.clone(),
                        false,
                        NetworkPriority::NORMAL,
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

    endpoint.stop().await.unwrap();
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
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
            NetworkPriority::LIFE_SAFETY,
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
    assert_eq!(npdu.priority, NetworkPriority::LIFE_SAFETY);
    assert_eq!(npdu.payload.as_ref(), apdu);
    assert!(matches!(
        decode_apdu(npdu.payload),
        Ok(Apdu::ConfirmedRequest(_))
    ));

    endpoint.stop().await.unwrap();
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
}
