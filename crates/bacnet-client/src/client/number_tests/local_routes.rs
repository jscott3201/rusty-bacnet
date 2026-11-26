//! Once the client has learned its network's number, a routed request or a
//! network broadcast naming that number goes out as local traffic (#1358):
//! a unicast to the destination MAC, or a local broadcast, with no DNET. A
//! routed confirmed request is then completed only by its answer from that
//! MAC with no SNET: one relayed back with this network as its SNET and that
//! MAC as its SADR completes nothing and is counted (#2831). The peer's
//! limits come
//! from its local device-table row, which a routed row on this network
//! becomes when the number is learned (#2572), and a request past them is
//! refused or segmented locally. Another network keeps its DNET, and so does
//! every network while the number is unknown.
//!
//! The peer is `[3]`, behind router `[9]` when routed. The number is learned
//! through the client's own Network-Number-Is intake. The client asks for it
//! when it first stores a routed row or sends a Who-Is while it is unknown
//! (#2572), and each test takes that question where it goes.
use super::*;
use crate::client::Station;
use crate::client::{BACnetClient, Broadcast};
use crate::discovery::RoutedDeviceConfig;
use bacnet_encoding::{
    apdu::{Apdu, ConfirmedRequest, SegmentAck, SimpleAck, decode_apdu, encode_apdu},
    npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu},
};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::time::Duration;

/// The number of the network this client is attached to.
pub(super) const THIS_NETWORK: u16 = 77;
pub(super) const REMOTE_NETWORK: u16 = 5;
pub(super) const PEER: [u8; 1] = [3];
pub(super) const ROUTER: [u8; 1] = [9];

/// Where a request went: the link MAC and the NPDU's DNET, if any.
#[derive(Debug, PartialEq, Eq)]
struct Route {
    link: MacAddr,
    dnet: Option<u16>,
}

fn local() -> Route {
    Route {
        link: MacAddr::from_slice(&PEER),
        dnet: None,
    }
}

fn routed(network: u16) -> Route {
    Route {
        link: MacAddr::from_slice(&ROUTER),
        dnet: Some(network),
    }
}

/// A started client that has learned `THIS_NETWORK`, or no number at all.
async fn client(
    learned: bool,
) -> (
    BACnetClient<TestTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::Receiver<Sent>,
) {
    let (client, inbound, mut outbound, _) = harness(false).await;
    if learned {
        learn(&inbound, &mut outbound).await;
    }
    (client, inbound, outbound)
}

/// Have the client learn `THIS_NETWORK` through its Network-Number-Is intake.
async fn learn(inbound: &mpsc::Sender<ReceivedNpdu>, outbound: &mut mpsc::Receiver<Sent>) {
    inject(inbound, &number(THIS_NETWORK, 0), true).await;
    // The worker takes controls in order: once it answers, it holds the
    // number the announcement gave.
    inject(inbound, QUERY, false).await;
    reply(outbound, THIS_NETWORK).await;
}

/// An I-Am from device `instance`.
pub(super) fn i_am(instance: u32) -> Apdu {
    use bacnet_encoding::apdu::UnconfirmedRequest;
    use bacnet_services::who_is::IAmRequest;
    use bacnet_types::enums::UnconfirmedServiceChoice;
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 260,
    }
    .encode(&mut service_request);
    Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    })
}

/// Take the confirmed request sent next and answer it with a SimpleACK the
/// way it went: from the peer's MAC when sent locally, from the router with
/// the peer's SNET when routed. Where it went.
async fn answer(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    outbound: &mut mpsc::Receiver<Sent>,
) -> Route {
    let sent = bounded(outbound.recv()).await.unwrap();
    let npdu = decode_npdu(sent.npdu).unwrap();
    let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("a confirmed request")
    };
    let source = npdu.destination.map(|to| {
        assert_eq!(to.mac_address.as_slice(), PEER, "the DADR");
        NpduAddress {
            network: to.network,
            mac_address: MacAddr::from_slice(&PEER),
        }
    });
    let route = Route {
        link: sent.destination.clone(),
        dnet: source.as_ref().map(|from| from.network),
    };
    send_to_client(inbound, &simple_ack(&request), &sent.destination, source).await;
    route
}

/// The SimpleACK that completes `request`.
fn simple_ack(request: &ConfirmedRequest) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id: request.invoke_id,
        service_choice: request.service_choice,
    })
}

/// Hand the client `apdu` from link MAC `link`, with `source` as its SNET/SADR
/// when routed.
pub(super) async fn send_to_client(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    apdu: &Apdu,
    link: &[u8],
    source: Option<NpduAddress>,
) {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
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
    inject_from(inbound, &npdu, link).await;
}

/// The confirmed request in a frame the client sent, checked to have gone to
/// the peer's MAC with no DNET.
fn local_request(sent: Sent) -> ConfirmedRequest {
    assert_eq!(sent.destination.as_slice(), PEER, "sent to the peer itself");
    let npdu = decode_npdu(sent.npdu).unwrap();
    assert_eq!(npdu.destination, None, "sent with no DNET");
    let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("a confirmed request")
    };
    request
}

/// Send a routed WriteProperty to the peer on `network` and return where it
/// went, once its answer has completed it.
async fn request(learned: bool, network: u16) -> Route {
    let (mut client, inbound, mut outbound) = client(learned).await;
    let (result, route) = bounded(async {
        let station = Station::routed(&ROUTER, network, &PEER);
        tokio::join!(
            client.confirmed_request(
                &station,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C, 0x00, 0x80, 0x00, 0x01],
            ),
            answer(&inbound, &mut outbound),
        )
    })
    .await;
    assert!(result.unwrap().is_empty());
    client.stop().await.unwrap();
    route
}

#[tokio::test]
async fn a_routed_request_naming_this_network_goes_local_and_its_direct_answer_completes_it() {
    assert_eq!(request(true, THIS_NETWORK).await, local());
    assert_eq!(request(true, REMOTE_NETWORK).await, routed(REMOTE_NETWORK));
    assert_eq!(request(false, THIS_NETWORK).await, routed(THIS_NETWORK));
}

#[tokio::test]
async fn a_device_added_as_routed_on_this_network_is_written_locally() {
    for (learned, network, expected) in [
        (true, THIS_NETWORK, local()),
        (true, REMOTE_NETWORK, routed(REMOTE_NETWORK)),
        (false, THIS_NETWORK, routed(THIS_NETWORK)),
    ] {
        let (mut client, inbound, mut outbound) = client(learned).await;
        client
            .add_routed_device(RoutedDeviceConfig {
                instance: 9,
                router_mac: ROUTER.to_vec(),
                remote_network: network,
                remote_mac: PEER.to_vec(),
                max_apdu_length: 480,
                segmentation_supported: Segmentation::NO_SEGMENTATION,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
        // A routed row stored while the number is unknown asks for it.
        if !learned {
            asked(&mut outbound).await;
        }
        let (result, route) = bounded(async {
            let to = client.resolve_device(9).await.unwrap();
            tokio::join!(
                client.write_property(
                    &to,
                    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    vec![0x44, 0x42, 0xA0, 0x00, 0x00],
                    None,
                ),
                answer(&inbound, &mut outbound),
            )
        })
        .await;
        result.unwrap();
        assert_eq!(route, expected, "learned {learned}, network {network}");
        client.stop().await.unwrap();
    }
}

#[tokio::test]
async fn a_who_is_on_this_network_by_number_goes_as_a_local_broadcast() {
    for (learned, network, dnet) in [
        (true, THIS_NETWORK, None),
        (true, REMOTE_NETWORK, Some(REMOTE_NETWORK)),
        (false, THIS_NETWORK, Some(THIS_NETWORK)),
    ] {
        let (mut client, _inbound, mut outbound) = client(learned).await;
        client
            .who_is(Broadcast::Network(network), None)
            .await
            .unwrap();
        if !learned {
            asked(&mut outbound).await;
        }
        let sent = bounded(outbound.recv()).await.unwrap();
        assert!(sent.destination.is_empty(), "a broadcast");
        let destination = decode_npdu(sent.npdu).unwrap().destination;
        assert_eq!(
            destination.map(|to| (to.network, to.mac_address.is_empty())),
            dnet.map(|network| (network, true)),
            "learned {learned}, network {network}"
        );
        client.stop().await.unwrap();
    }
}

/// A WriteProperty longer than one 128-octet APDU.
const LONG_WRITE: [u8; 300] = [0x44; 300];

/// A client that has learned `THIS_NETWORK` and holds Device 9, accepting
/// APDUs of at most 128 octets and `segmentation`: registered as routed
/// through `ROUTER` on that network before the number was learned, so the
/// routed row became its local row when it was (#2572).
async fn client_with_routed_row(
    segmentation: Segmentation,
) -> (
    BACnetClient<TestTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::Receiver<Sent>,
) {
    let (client, inbound, mut outbound) = client(false).await;
    client
        .add_routed_device(RoutedDeviceConfig {
            instance: 9,
            router_mac: ROUTER.to_vec(),
            remote_network: THIS_NETWORK,
            remote_mac: PEER.to_vec(),
            max_apdu_length: 128,
            segmentation_supported: segmentation,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    asked(&mut outbound).await;
    learn(&inbound, &mut outbound).await;
    (client, inbound, outbound)
}

/// The limits the device was added with as routed still bound a request
/// that goes locally: a peer that takes no segmented request gets nothing
/// too long for it.
#[tokio::test]
async fn a_routed_row_on_this_network_still_bounds_a_local_request() {
    let (mut client, _inbound, mut outbound) =
        client_with_routed_row(Segmentation::NO_SEGMENTATION).await;
    let result = bounded(client.confirmed_request(
        &Station::routed(&ROUTER, THIS_NETWORK, &PEER),
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &LONG_WRITE,
    ))
    .await;
    assert!(matches!(result, Err(Error::Segmentation(_))), "{result:?}");
    assert!(outbound.try_recv().is_err(), "nothing was sent");
    client.stop().await.unwrap();
}

/// A local request longer than the routed row allows goes to the peer in
/// segments, each with no DNET, and the peer's own SegmentACKs and SimpleACK,
/// with no SNET, carry it through.
#[tokio::test]
async fn a_long_request_to_this_network_is_segmented_to_the_peer() {
    let (mut client, inbound, mut outbound) =
        client_with_routed_row(Segmentation::SEGMENTED_BOTH).await;
    let peer = async {
        let mut segments = 0;
        loop {
            let request = local_request(bounded(outbound.recv()).await.unwrap());
            assert!(request.segmented, "frame {segments} is a segment");
            segments += 1;
            let ack = Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id: request.invoke_id,
                sequence_number: request.sequence_number.unwrap(),
                actual_window_size: 1,
            });
            send_to_client(&inbound, &ack, &PEER, None).await;
            if !request.more_follows {
                send_to_client(&inbound, &simple_ack(&request), &PEER, None).await;
                return segments;
            }
        }
    };
    let (result, segments) = bounded(async {
        let station = Station::routed(&ROUTER, THIS_NETWORK, &PEER);
        tokio::join!(
            client.confirmed_request(
                &station,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &LONG_WRITE,
            ),
            peer,
        )
    })
    .await;
    assert!(result.unwrap().is_empty());
    assert!(segments > 1, "{segments} segments");
    client.stop().await.unwrap();
}

/// `station` on `network`, as the SNET/SADR a router adds when it relays.
pub(super) fn relayed_from(network: u16, station: &[u8]) -> Option<NpduAddress> {
    Some(NpduAddress {
        network,
        mac_address: MacAddr::from_slice(station),
    })
}

/// Send a WriteProperty to the peer, routed to this network or straight to
/// its MAC, while `peer` answers it, and return how long it took to complete.
async fn write_answered_by(
    client: &BACnetClient<TestTransport>,
    routed: bool,
    peer: impl std::future::Future<Output = ()>,
) -> Duration {
    const WRITE: [u8; 5] = [0x0C, 0x00, 0x80, 0x00, 0x01];
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let started = tokio::time::Instant::now();
    let request = async {
        if routed {
            client
                .confirmed_request(
                    &Station::routed(&ROUTER, THIS_NETWORK, &PEER),
                    service,
                    &WRITE,
                )
                .await
        } else {
            client.confirmed_request(&PEER, service, &WRITE).await
        }
    };
    let (result, ()) = tokio::join!(request, peer);
    assert!(result.unwrap().is_empty());
    started.elapsed()
}

/// The request sent again after the APDU timeout, checked to have gone to
/// the peer's MAC with the first one's invoke ID.
async fn retry_of(
    outbound: &mut mpsc::Receiver<Sent>,
    first: &ConfirmedRequest,
) -> ConfirmedRequest {
    let retry = tokio::time::timeout(Duration::from_secs(10), outbound.recv())
        .await
        .expect("the request goes again")
        .unwrap();
    let retry = local_request(retry);
    assert_eq!(retry.invoke_id, first.invoke_id);
    retry
}

/// Once the number is known, an answer a router relays back with this
/// network as its SNET and the peer's MAC as its SADR does not complete a
/// request sent to that MAC (#2831): any station on this link could write
/// that SNET and SADR, and only the peer can send from its own MAC. That
/// holds for a request routed to this network and for one sent straight to
/// the MAC. The relayed answer is dropped and counted, the request goes
/// again after the APDU timeout, and the peer's own answer completes it.
#[tokio::test(start_paused = true)]
async fn an_answer_relayed_with_this_networks_snet_leaves_a_local_request_open() {
    let (mut client, inbound, mut outbound) = client(true).await;
    for (routed, drops) in [(true, 1), (false, 2)] {
        let peer = async {
            let request = local_request(outbound.recv().await.unwrap());
            let relayed = relayed_from(THIS_NETWORK, &PEER);
            send_to_client(&inbound, &simple_ack(&request), &ROUTER, relayed).await;
            let retry = retry_of(&mut outbound, &request).await;
            send_to_client(&inbound, &simple_ack(&retry), &PEER, None).await;
        };
        let took = write_answered_by(&client, routed, peer).await;
        // The harness's APDU timeout is 5 s.
        assert!(took >= Duration::from_secs(5), "routed {routed}: {took:?}");
        assert_eq!(client.off_path_answer_drops(), drops, "routed {routed}");
    }
    client.stop().await.unwrap();
}

/// A relayed answer completes no local request, whatever it names: another
/// SNET, another SADR or another invoke ID completes nothing and names no
/// open request, so none is counted. The request goes again after the APDU
/// timeout, and the station's own answer to that completes it.
#[tokio::test(start_paused = true)]
async fn a_relayed_answer_naming_another_network_station_or_invoke_id_leaves_the_request_open() {
    let (mut client, inbound, mut outbound) = client(true).await;
    let peer = async {
        let request = local_request(outbound.recv().await.unwrap());
        let ack = simple_ack(&request);
        for source in [
            relayed_from(REMOTE_NETWORK, &PEER),
            relayed_from(THIS_NETWORK, &[4]),
        ] {
            send_to_client(&inbound, &ack, &ROUTER, source).await;
        }
        let another_invoke_id = Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id.wrapping_add(1),
            service_choice: request.service_choice,
        });
        let relayed = relayed_from(THIS_NETWORK, &PEER);
        send_to_client(&inbound, &another_invoke_id, &ROUTER, relayed).await;
        let retry = retry_of(&mut outbound, &request).await;
        send_to_client(&inbound, &simple_ack(&retry), &PEER, None).await;
    };
    let took = write_answered_by(&client, true, peer).await;
    assert!(took >= Duration::from_secs(5), "{took:?}");
    assert_eq!(client.off_path_answer_drops(), 0);
    client.stop().await.unwrap();
}

/// While the number is unknown nothing changes: an answer relayed with SNET
/// 77 is not from the MAC a request went to, so only that MAC's own answer
/// to the retry completes it.
#[tokio::test(start_paused = true)]
async fn with_the_number_unknown_a_relayed_answer_leaves_a_local_request_open() {
    let (mut client, inbound, mut outbound) = client(false).await;
    let peer = async {
        let request = local_request(outbound.recv().await.unwrap());
        let relayed = relayed_from(THIS_NETWORK, &PEER);
        send_to_client(&inbound, &simple_ack(&request), &ROUTER, relayed).await;
        let retry = retry_of(&mut outbound, &request).await;
        send_to_client(&inbound, &simple_ack(&retry), &PEER, None).await;
    };
    let took = write_answered_by(&client, false, peer).await;
    assert!(took >= Duration::from_secs(5), "{took:?}");
    client.stop().await.unwrap();
}

/// A request routed to this network while its number was unknown goes
/// through the router with the DNET and keeps that routed key. When the
/// number is learned before its answer comes back, the answer the router
/// relays with this network as its SNET still completes it at once, as it
/// would have before the number was known. That answer is checked as any
/// routed answer is (#2831): the same answer from another station on this
/// link is dropped and counted first.
#[tokio::test(start_paused = true)]
async fn a_request_routed_before_the_number_was_learned_completes_on_its_relayed_answer() {
    const STRANGER: [u8; 1] = [4];
    let (mut client, inbound, mut outbound) = client(false).await;
    let peer = async {
        let sent = outbound.recv().await.unwrap();
        assert_eq!(sent.destination.as_slice(), ROUTER, "through the router");
        let npdu = decode_npdu(sent.npdu).unwrap();
        assert_eq!(npdu.destination.map(|to| to.network), Some(THIS_NETWORK));
        let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
            panic!("a confirmed request")
        };
        learn(&inbound, &mut outbound).await;
        let relayed = relayed_from(THIS_NETWORK, &PEER);
        let ack = simple_ack(&request);
        send_to_client(&inbound, &ack, &STRANGER, relayed.clone()).await;
        send_to_client(&inbound, &ack, &ROUTER, relayed).await;
    };
    let took = write_answered_by(&client, true, peer).await;
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(outbound.try_recv().is_err(), "sent once");
    assert_eq!(client.off_path_answer_drops(), 1);
    client.stop().await.unwrap();
}

/// An I-Am a router relays back from this network, with its number as
/// SNET, names a local device, as a request relayed that way does (#1404):
/// it lands on the
/// device's local row, so the device and its relayed I-Am are one device,
/// not an ambiguous instance (#2560). While the number is unknown the relay
/// is a routed station of its own, and the instance is ambiguous: learning
/// the number folds that row into the local one (below, #2572); a number
/// never learned leaves it until the purge takes it.
#[tokio::test]
async fn an_i_am_relayed_from_this_network_is_the_local_device() {
    use crate::client::DeviceEventKind;

    let i_am = i_am(9);
    for (learned, rows) in [(true, 1), (false, 2)] {
        let (mut client, inbound, _outbound) = client(learned).await;
        let mut events = client.device_events();
        send_to_client(&inbound, &i_am, &PEER, None).await;
        let relayed = relayed_from(THIS_NETWORK, &PEER);
        send_to_client(&inbound, &i_am, &ROUTER, relayed).await;
        let kinds = [
            bounded(events.recv()).await.unwrap().kind,
            bounded(events.recv()).await.unwrap().kind,
        ];
        let devices = client.discovered_devices().await;
        assert_eq!(devices.len(), rows, "learned {learned}");
        if learned {
            assert_eq!(
                kinds,
                [DeviceEventKind::Discovered, DeviceEventKind::Updated]
            );
            let device = client.get_device(9).await.unwrap().expect("one device");
            assert!(device.is_local());
            assert_eq!(device.mac_address.as_slice(), PEER);
        } else {
            assert!(matches!(
                client.get_device(9).await,
                Err(Error::AmbiguousDevice { instance: 9, .. })
            ));
        }
        client.stop().await.unwrap();
    }
}

/// The number learned a minute after start-up, with a router echoing the
/// device's I-Am back: learning it folds the routed row the first echo made
/// into the local row at once, reported lost, and the instance names one
/// device again then, not only at the next echo or once the purge takes the
/// routed row (#2572). The next echo refreshes the local row.
#[tokio::test(start_paused = true)]
async fn learning_the_number_folds_the_routed_row_an_echo_made() {
    use crate::client::DeviceEventKind;

    let (mut client, inbound, mut outbound) = client(false).await;
    let echo = || relayed_from(THIS_NETWORK, &PEER);
    send_to_client(&inbound, &i_am(9), &PEER, None).await;
    send_to_client(&inbound, &i_am(9), &ROUTER, echo()).await;
    // The routed row the echo made asks for the number.
    asked(&mut outbound).await;
    assert!(matches!(
        client.get_device(9).await,
        Err(Error::AmbiguousDevice { instance: 9, .. })
    ));

    tokio::time::sleep(Duration::from_secs(60)).await;
    let mut events = client.device_events();
    let mut collisions = client.device_collision_events();
    learn(&inbound, &mut outbound).await;
    let lost = bounded(events.recv()).await.unwrap();
    assert_eq!(lost.kind, DeviceEventKind::Lost);
    assert_eq!(lost.device.source_network, Some(THIS_NETWORK));
    let contest = bounded(collisions.recv()).await.unwrap();
    assert!(contest.is_resolved(), "{contest:?}");
    let device = client.get_device(9).await.unwrap().expect("one device");
    assert!(device.is_local());
    assert_eq!(device.mac_address.as_slice(), PEER);
    assert_eq!(client.discovered_devices().await.len(), 1);

    send_to_client(&inbound, &i_am(9), &ROUTER, echo()).await;
    let updated = bounded(events.recv()).await.unwrap();
    assert_eq!(updated.kind, DeviceEventKind::Updated);
    assert!(updated.device.is_local());
    assert_eq!(client.discovered_devices().await.len(), 1);
    assert!(collisions.try_recv().is_err(), "no contest left to report");
    client.stop().await.unwrap();
}

/// A device registered as routed on this network, once its number is
/// learned, is the local device at its MAC (#2560): the routed row a relay
/// heard before the number made became that local row when the number was
/// learned (#2572), and the registration refreshes it.
#[tokio::test(start_paused = true)]
async fn a_routed_registration_on_this_network_folds_to_the_local_row() {
    let (mut client, inbound, mut outbound) = client(false).await;
    let relayed = relayed_from(THIS_NETWORK, &PEER);
    send_to_client(&inbound, &i_am(9), &ROUTER, relayed).await;
    asked(&mut outbound).await;
    learn(&inbound, &mut outbound).await;
    client
        .add_routed_device(RoutedDeviceConfig {
            instance: 9,
            router_mac: ROUTER.to_vec(),
            remote_network: THIS_NETWORK,
            remote_mac: PEER.to_vec(),
            max_apdu_length: 480,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    let devices = client.discovered_devices().await;
    assert_eq!(devices.len(), 1, "{devices:?}");
    assert!(devices[0].is_local());
    assert_eq!(devices[0].mac_address.as_slice(), PEER);
    client.stop().await.unwrap();
}

/// A request named by the routed station on this network, when the device
/// has only its local row (as its relayed I-Am makes it), is sized by that
/// row: one too long for it is refused before anything is sent. A short
/// one's answer restarts that row's age (#2560).
#[tokio::test(start_paused = true)]
async fn the_local_row_sizes_and_is_refreshed_by_a_routed_named_request() {
    let (mut client, inbound, mut outbound) = client(true).await;
    let seen = crate::discovery::now();
    client
        .device_table
        .lock()
        .upsert(crate::discovery::DiscoveredDevice {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap(),
            mac_address: MacAddr::from_slice(&PEER),
            max_apdu_length: 128,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
            vendor_id: 1,
            last_seen: seen,
            last_announced: None,
            source_network: None,
            source_address: None,
        });
    let result = bounded(client.confirmed_request(
        &Station::routed(&ROUTER, THIS_NETWORK, &PEER),
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &LONG_WRITE,
    ))
    .await;
    assert!(
        matches!(result, Err(Error::Segmentation(_))),
        "sized by the local row: {result:?}"
    );
    assert!(outbound.try_recv().is_err(), "nothing was sent");

    tokio::time::sleep(Duration::from_secs(1)).await;
    let (result, route) = bounded(async {
        let station = Station::routed(&ROUTER, THIS_NETWORK, &PEER);
        tokio::join!(
            client.confirmed_request(
                &station,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0C, 0x00, 0x80, 0x00, 0x01],
            ),
            answer(&inbound, &mut outbound),
        )
    })
    .await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(route, local());
    let last_seen = client.device_table.lock().get(0, &PEER).unwrap().last_seen;
    assert!(last_seen > seen, "the local row's age did not restart");
    client.stop().await.unwrap();
}
