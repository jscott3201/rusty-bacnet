//! Once the client has learned its network's number, a routed request or a
//! network broadcast naming that number goes out as local traffic (#1358):
//! a unicast to the destination MAC, or a local broadcast, with no DNET. A
//! routed confirmed request is then answered from that MAC with no SNET, or
//! relayed back by a router with this network as its SNET and that MAC as
//! its SADR, and either answer completes it (#1465). The peer's limits still
//! come from its routed device-table row, and a request past them is refused
//! or segmented locally. Another network keeps its DNET, and so does every
//! network while the number is unknown.
//!
//! The peer is `[3]`, behind router `[9]` when routed. The number is learned
//! through the client's own Network-Number-Is intake.
use super::*;
use crate::client::BACnetClient;
use crate::discovery::RoutedDeviceConfig;
use bacnet_encoding::{
    apdu::{Apdu, ConfirmedRequest, SegmentAck, SimpleAck, decode_apdu, encode_apdu},
    npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu},
};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::time::Duration;

/// The number of the network this client is attached to.
const THIS_NETWORK: u16 = 77;
const REMOTE_NETWORK: u16 = 5;
const PEER: [u8; 1] = [3];
const ROUTER: [u8; 1] = [9];

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
    BACnetClient<Capture>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::Receiver<Sent>,
) {
    let (client, inbound, mut outbound, _) = harness(false).await;
    if learned {
        inject(&inbound, &number(THIS_NETWORK, 0), true).await;
        // The worker takes controls in order: once it answers, it holds the
        // number the announcement gave.
        inject(&inbound, QUERY, false).await;
        reply(&mut outbound, THIS_NETWORK).await;
    }
    (client, inbound, outbound)
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
async fn send_to_client(
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
        tokio::join!(
            client.confirmed_request_routed(
                &ROUTER,
                network,
                &PEER,
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
                segmentation_supported: Segmentation::NONE,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
        let (result, route) = bounded(async {
            tokio::join!(
                client.write_property_to_device(
                    9,
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
        client.who_is_network(network, None).await.unwrap();
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

/// A client that has learned `THIS_NETWORK` and holds Device 9 as routed
/// through `ROUTER` on it, accepting APDUs of at most 128 octets and
/// `segmentation`.
async fn client_with_routed_row(
    segmentation: Segmentation,
) -> (
    BACnetClient<Capture>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::Receiver<Sent>,
) {
    let (client, inbound, outbound) = client(true).await;
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
    (client, inbound, outbound)
}

/// The routed row the device was added with still sets the peer's limits
/// when the request goes locally: a peer that takes no segmented request
/// gets nothing too long for it.
#[tokio::test]
async fn a_routed_row_on_this_network_still_bounds_a_local_request() {
    let (mut client, _inbound, mut outbound) = client_with_routed_row(Segmentation::NONE).await;
    let result = bounded(client.confirmed_request_routed(
        &ROUTER,
        THIS_NETWORK,
        &PEER,
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
    let (mut client, inbound, mut outbound) = client_with_routed_row(Segmentation::BOTH).await;
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
        tokio::join!(
            client.confirmed_request_routed(
                &ROUTER,
                THIS_NETWORK,
                &PEER,
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
fn relayed_from(network: u16, station: &[u8]) -> Option<NpduAddress> {
    Some(NpduAddress {
        network,
        mac_address: MacAddr::from_slice(station),
    })
}

/// Send a WriteProperty to the peer, routed to this network or straight to
/// its MAC, while `peer` answers it, and return how long it took to complete.
async fn write_answered_by(
    client: &BACnetClient<Capture>,
    routed: bool,
    peer: impl std::future::Future<Output = ()>,
) -> Duration {
    const WRITE: [u8; 5] = [0x0C, 0x00, 0x80, 0x00, 0x01];
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let started = tokio::time::Instant::now();
    let request = async {
        if routed {
            client
                .confirmed_request_routed(&ROUTER, THIS_NETWORK, &PEER, service, &WRITE)
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
/// network as its SNET and the peer's MAC as its SADR completes a request
/// sent to that MAC as the direct answer would (#1465): network numbers are
/// unique, so both name the same station. That holds for a request routed to
/// this network and for one sent straight to the MAC, and the answer
/// completes it at once, with nothing sent again.
#[tokio::test(start_paused = true)]
async fn an_answer_relayed_with_this_networks_snet_completes_a_local_request() {
    let (mut client, inbound, mut outbound) = client(true).await;
    for routed in [true, false] {
        let peer = async {
            let request = local_request(outbound.recv().await.unwrap());
            let relayed = relayed_from(THIS_NETWORK, &PEER);
            send_to_client(&inbound, &simple_ack(&request), &ROUTER, relayed).await;
        };
        let took = write_answered_by(&client, routed, peer).await;
        // The harness's APDU timeout is 5 s.
        assert!(took < Duration::from_secs(5), "routed {routed}: {took:?}");
        assert!(outbound.try_recv().is_err(), "routed {routed}: sent once");
    }
    client.stop().await.unwrap();
}

/// A relayed answer completes a local request only when it names this
/// network, the station the request went to and the request's invoke ID.
/// Another SNET, another SADR or another invoke ID completes nothing, so the
/// request goes again after the APDU timeout, and the station's relayed
/// answer to that completes it.
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
        send_to_client(&inbound, &another_invoke_id, &ROUTER, relayed.clone()).await;
        let retry = retry_of(&mut outbound, &request).await;
        send_to_client(&inbound, &simple_ack(&retry), &ROUTER, relayed).await;
    };
    let took = write_answered_by(&client, true, peer).await;
    assert!(took >= Duration::from_secs(5), "{took:?}");
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
/// would have before the number was known (#1465).
#[tokio::test(start_paused = true)]
async fn a_request_routed_before_the_number_was_learned_completes_on_its_relayed_answer() {
    let (mut client, inbound, mut outbound) = client(false).await;
    let peer = async {
        let sent = outbound.recv().await.unwrap();
        assert_eq!(sent.destination.as_slice(), ROUTER, "through the router");
        let npdu = decode_npdu(sent.npdu).unwrap();
        assert_eq!(npdu.destination.map(|to| to.network), Some(THIS_NETWORK));
        let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
            panic!("a confirmed request")
        };
        inject(&inbound, &number(THIS_NETWORK, 0), true).await;
        inject(&inbound, QUERY, false).await;
        reply(&mut outbound, THIS_NETWORK).await;
        let relayed = relayed_from(THIS_NETWORK, &PEER);
        send_to_client(&inbound, &simple_ack(&request), &ROUTER, relayed).await;
    };
    let took = write_answered_by(&client, true, peer).await;
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(outbound.try_recv().is_err(), "sent once");
    client.stop().await.unwrap();
}
