//! Devices are known by address (#2560): two addresses claiming one instance
//! are both listed and both reachable by address, a request by that instance
//! is refused with every candidate named, and a device that keeps answering
//! stays in the table without repeating its I-Am.
//!
//! The peer across the loopback pair is two stations: a device on the local
//! network at [`PEER`], and the router at [`PEER`] to a device at
//! [`STATION`] on [`NETWORK`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_encoding::apdu::{self, Apdu, ComplexAck, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation, UnconfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{Duration, timeout};

use super::{BACnetClient, DeviceEventKind};

const CLIENT: [u8; 1] = [0x01];
const PEER: [u8; 1] = [0x02];
const NETWORK: u16 = 5;
const STATION: [u8; 1] = [0x07];
const INSTANCE: u32 = 1001;

/// Where a request went: `None` for the local device, else the routed
/// station's network and MAC.
type Destination = Option<(u16, MacAddr)>;

struct Peer {
    transport: Arc<LoopbackTransport>,
    /// Every confirmed request's destination, in arrival order.
    requests: Arc<StdMutex<Vec<Destination>>>,
    /// Whether the routed station answers.
    routed_answers: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Peer {
    /// An I-Am for `instance` from the local device, or from the routed
    /// station when `routed`.
    async fn i_am(&self, routed: bool, instance: u32) {
        let mut service_request = BytesMut::new();
        IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            vendor_id: 260,
        }
        .encode(&mut service_request);
        let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service_request.freeze(),
        });
        let source = routed.then(|| (NETWORK, MacAddr::from_slice(&STATION)));
        send(&self.transport, source, &apdu).await;
    }

    fn requests(&self) -> Vec<Destination> {
        self.requests.lock().unwrap().clone()
    }
}

/// `apdu` to the client, from the routed station `source` names or else
/// from the local device.
async fn send(transport: &LoopbackTransport, source: Destination, apdu: &Apdu) {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let npdu = Npdu {
        source: source.map(|(network, mac_address)| NpduAddress {
            network,
            mac_address,
        }),
        payload: payload.freeze(),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    transport.send_unicast(&buf, &CLIENT).await.unwrap();
}

/// A started client and the peer, answering every ReadProperty with
/// ENUMERATED 0 for the object and property asked about.
async fn start() -> (BACnetClient<LoopbackTransport>, Peer) {
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(CLIENT.to_vec(), PEER.to_vec());
    let mut inbound = peer_transport.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let transport = Arc::new(peer_transport);
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let routed_answers = Arc::new(AtomicBool::new(true));
    let (replies, log, answers) = (
        Arc::clone(&transport),
        Arc::clone(&requests),
        Arc::clone(&routed_answers),
    );
    let task = tokio::spawn(async move {
        while let Some(received) = inbound.recv().await {
            let npdu = decode_npdu(received.npdu).unwrap();
            let Ok(Apdu::ConfirmedRequest(request)) = apdu::decode_apdu(npdu.payload) else {
                continue;
            };
            let to = npdu.destination.map(|d| (d.network, d.mac_address));
            log.lock().unwrap().push(to.clone());
            if to.is_some() && !answers.load(Ordering::SeqCst) {
                continue;
            }
            let asked = ReadPropertyRequest::decode(&request.service_request).unwrap();
            let mut service_ack = BytesMut::new();
            ReadPropertyACK {
                object_identifier: asked.object_identifier,
                property_identifier: asked.property_identifier,
                property_array_index: asked.property_array_index,
                property_value: vec![0x91, 0x00],
            }
            .encode(&mut service_ack)
            .unwrap();
            let ack = Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_ack: service_ack.freeze(),
            });
            send(&replies, to, &ack).await;
        }
    });
    let peer = Peer {
        transport,
        requests,
        routed_answers,
        task,
    };
    (client, peer)
}

fn local() -> BACnetAddress {
    BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&PEER),
    }
}

fn routed() -> BACnetAddress {
    BACnetAddress {
        network_number: NETWORK,
        mac_address: MacAddr::from_slice(&STATION),
    }
}

fn object() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

async fn recv<E: Clone>(rx: &mut broadcast::Receiver<E>) -> E {
    timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for an event")
        .expect("event channel closed")
}

async fn addresses(client: &BACnetClient<LoopbackTransport>) -> Vec<BACnetAddress> {
    let devices = client.discovered_devices().await;
    devices.iter().map(|d| d.address().unwrap()).collect()
}

#[tokio::test]
async fn two_claimants_are_listed_reachable_by_address_and_refused_by_instance() {
    let (mut client, peer) = start().await;
    let mut collisions = client.device_collision_events();
    let present = PropertyIdentifier::PRESENT_VALUE;

    peer.i_am(false, INSTANCE).await;
    peer.i_am(true, INSTANCE).await;
    let collision = recv(&mut collisions).await;
    assert_eq!(collision.instance, INSTANCE);
    assert!(!collision.is_resolved());
    assert_eq!(addresses(&client).await, [local(), routed()]);

    client
        .read_property(&PEER, object(), present, None)
        .await
        .unwrap();
    client
        .read_property_routed(&PEER, NETWORK, &STATION, object(), present, None)
        .await
        .unwrap();
    let refused = client
        .read_property_from_device(INSTANCE, object(), present, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, Error::AmbiguousDevice { instance: INSTANCE, candidates }
            if *candidates == [local(), routed()]),
        "{refused:?}"
    );
    let routed_to = Some((NETWORK, MacAddr::from_slice(&STATION)));
    assert_eq!(peer.requests(), [None, routed_to.clone()], "nothing sent");

    // The routed station announces another instance: the contest ends, and
    // each instance names one device again.
    peer.i_am(true, 2002).await;
    let collision = recv(&mut collisions).await;
    assert_eq!(collision.instance, INSTANCE);
    assert!(collision.is_resolved());
    assert_eq!(collision.claimants[0].address(), Some(local()));
    for instance in [INSTANCE, 2002] {
        client
            .read_property_from_device(instance, object(), present, None)
            .await
            .unwrap();
    }
    assert_eq!(peer.requests()[2..], [None, routed_to]);
    assert_eq!(addresses(&client).await, [local(), routed()]);

    client.stop().await.unwrap();
}

/// The local device answers a read every minute and never repeats its I-Am;
/// the routed claimant of the same instance falls silent. Twenty minutes
/// on, the silent one is gone, the contest with it, and the instance names
/// the device that kept answering.
#[tokio::test(start_paused = true)]
async fn a_device_that_keeps_answering_outlives_a_silent_one() {
    let (mut client, peer) = start().await;
    let mut events = client.device_events();
    let mut collisions = client.device_collision_events();
    let present = PropertyIdentifier::PRESENT_VALUE;

    peer.i_am(false, INSTANCE).await;
    peer.i_am(true, INSTANCE).await;
    for _ in 0..2 {
        assert_eq!(recv(&mut events).await.kind, DeviceEventKind::Discovered);
    }
    assert!(!recv(&mut collisions).await.is_resolved());
    peer.routed_answers.store(false, Ordering::SeqCst);

    let mut lost = Vec::new();
    let mut resolved_at = None;
    for minute in 1..=20 {
        tokio::time::sleep(Duration::from_secs(60)).await;
        client
            .read_property(&PEER, object(), present, None)
            .await
            .unwrap();
        while let Ok(event) = events.try_recv() {
            assert_eq!(event.kind, DeviceEventKind::Lost);
            lost.push(event.device.address().unwrap());
        }
        while let Ok(collision) = collisions.try_recv() {
            assert!(collision.is_resolved());
            resolved_at = Some(minute);
        }
    }

    assert_eq!(lost, [routed()]);
    // The purge runs every 5 minutes and takes rows over 10 minutes old.
    assert!(
        resolved_at.is_some_and(|minute| (15..=16).contains(&minute)),
        "{resolved_at:?}"
    );
    assert_eq!(addresses(&client).await, [local()]);
    client
        .read_property_from_device(INSTANCE, object(), present, None)
        .await
        .unwrap();

    client.stop().await.unwrap();
}
