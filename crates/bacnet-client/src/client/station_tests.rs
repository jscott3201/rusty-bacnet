//! Requests by address (#2560): a local station at its MAC, a remote one
//! through the router on its row, sized by the row at the address and
//! keeping it alive, refused before sending with no route; and a station
//! whose instance two addresses claim, which the by-instance call refuses.

use std::time::Instant;

use bacnet_encoding::apdu::{Apdu, ErrorPdu, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, Segmentation};
use bacnet_types::error::SegmentationError;
use bacnet_types::primitives::ObjectIdentifier;

use super::*;
use crate::discovery::DiscoveredDevice;

const CLIENT: &[u8] = &[0x01];
const STATION: &[u8] = &[0x02];
const OTHER: &[u8] = &[0x03];
const ROUTER: &[u8] = &[0x09];
const DNET: u16 = 2;
const DADR: &[u8] = &[0x05];
const BROADCAST: &[u8] = &[0xFF];
const INSTANCE: u32 = 77;

struct Site {
    client: BACnetClient<TestTransport>,
    sent: SendLog,
    inbox: mpsc::Sender<ReceivedNpdu>,
}

async fn site() -> Site {
    let (inbox, inbound) = mpsc::channel(16);
    let transport = TestTransport::builder()
        .local_mac(CLIENT)
        .broadcast_mac(BROADCAST)
        .inbound(inbound)
        .number_controls()
        .build();
    let sent = transport.sent();
    let config = ClientConfig {
        apdu_timeout_ms: 500,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    Site {
        client,
        sent,
        inbox,
    }
}

fn address(network: u16, mac: &[u8]) -> BACnetAddress {
    BACnetAddress {
        network_number: network,
        mac_address: MacAddr::from_slice(mac),
    }
}

/// A row for `INSTANCE` at `mac` on `network` (0 for local) that accepts
/// `max_apdu` octets and no segments, as an I-Am says.
fn row(network: u16, mac: &[u8], max_apdu: u32) -> DiscoveredDevice {
    let routed = network != 0;
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE).unwrap(),
        mac_address: MacAddr::from_slice(if routed { ROUTER } else { mac }),
        max_apdu_length: max_apdu,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
        vendor_id: 1,
        last_seen: crate::discovery::now(),
        last_announced: Some(crate::discovery::now()),
        source_network: routed.then_some(network),
        source_address: routed.then(|| MacAddr::from_slice(mac)),
    }
}

impl Site {
    /// A WriteProperty of `octets` octets of service data to `to`.
    async fn write(&self, to: impl ToStation, octets: usize) -> Result<Bytes, Error> {
        let data = vec![0u8; octets];
        self.client
            .confirmed_request(&to, ConfirmedServiceChoice::WRITE_PROPERTY, &data)
            .await
    }

    fn add(&self, device: DiscoveredDevice) {
        self.client.device_table.lock().upsert(device);
    }

    fn last_seen(&self, network: u16, mac: &[u8]) -> Instant {
        let table = self.client.device_table.lock();
        table.get(network, mac).unwrap().last_seen
    }

    /// Wait for send `index`, and answer it as the device does: from the
    /// frame's MAC, through the router with the station as SNET/SADR when
    /// the request was routed. `error` answers with an Error PDU.
    async fn answer(&self, index: usize, error: bool) -> SentFrame {
        // A request that never goes out fails here, not by hanging.
        tokio::time::timeout(Duration::from_secs(5), self.sent.wait_for_len(index + 1))
            .await
            .expect("the request was sent");
        let frame = self.sent.frame(index);
        let npdu = frame.decode_npdu();
        let Apdu::ConfirmedRequest(request) = frame.apdu() else {
            panic!("expected a confirmed request");
        };
        let reply = if error {
            Apdu::Error(ErrorPdu {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
                error_class: ErrorClass::OBJECT,
                error_code: ErrorCode::UNKNOWN_OBJECT,
                error_data: Bytes::new(),
            })
        } else {
            Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            })
        };
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &reply).unwrap();
        let source = npdu.destination.map(|to| NpduAddress {
            network: to.network,
            mac_address: to.mac_address,
        });
        let mut buf = BytesMut::new();
        let back = Npdu {
            source,
            payload: payload.freeze(),
            ..Npdu::default()
        };
        encode_npdu(&mut buf, &back).unwrap();
        let received =
            ReceivedNpdu::unverified(buf.freeze(), frame.mac.clone(), false, Vec::new(), None);
        self.inbox.send(received).await.unwrap();
        frame
    }

    /// Teach the client that its own network is `network`, by the
    /// Network-Number-Is a router broadcasts.
    async fn learn(&self, network: u16) {
        let [hi, lo] = network.to_be_bytes();
        let npdu = Bytes::from(vec![0x01, 0x80, 0x13, hi, lo, 1]);
        let received =
            ReceivedNpdu::unverified(npdu, MacAddr::from_slice(ROUTER), true, Vec::new(), None);
        self.inbox.send(received).await.unwrap();
        for _ in 0..1_000 {
            if self.client.network_number() == Some(network) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("the client never learned network {network}");
    }
}

/// Where `frame` went: its link MAC, and its DNET/DADR when routed.
fn sent_to(frame: &SentFrame) -> (Vec<u8>, Option<(u16, Vec<u8>)>) {
    let to = frame
        .decode_npdu()
        .destination
        .map(|to| (to.network, to.mac_address.to_vec()));
    (frame.mac.to_vec(), to)
}

/// A local station goes by its MAC. Its row's Max APDU and segmentation
/// bound the request, as a request by instance is bounded; with no row the
/// configured limits apply; an answer keeps the row alive.
#[tokio::test(start_paused = true)]
async fn a_local_station_goes_by_its_mac_sized_by_its_row() {
    let site = site().await;
    site.add(row(0, STATION, 128));
    let refused = site.write(address(0, STATION), 200).await;
    assert!(
        matches!(
            refused,
            Err(Error::Segmentation(
                SegmentationError::PeerCannotReceive { .. }
            ))
        ),
        "{refused:?}"
    );
    assert!(site.sent.is_empty(), "the row refused it before sending");

    // No row at the other station: the configured 1476 octets apply.
    let (result, frame) = tokio::join!(site.write(address(0, OTHER), 200), site.answer(0, false));
    assert_eq!(result.unwrap(), Bytes::new());
    assert_eq!(sent_to(&frame), (OTHER.to_vec(), None));

    let before = site.last_seen(0, STATION);
    tokio::time::advance(Duration::from_secs(5)).await;
    let (result, frame) = tokio::join!(site.write(address(0, STATION), 8), site.answer(1, false));
    result.unwrap();
    assert_eq!(sent_to(&frame), (STATION.to_vec(), None));
    assert!(
        site.last_seen(0, STATION) > before,
        "the answer kept the row alive"
    );
}

/// A remote station goes through the router its row names, with its
/// network and MAC as DNET and DADR, sized by that row; an answer from it,
/// an Error too, keeps the row alive.
#[tokio::test(start_paused = true)]
async fn a_routed_station_goes_through_its_rows_router() {
    let site = site().await;
    site.add(row(DNET, DADR, 128));
    let to = address(DNET, DADR);
    let refused = site.write(to.clone(), 200).await;
    assert!(
        matches!(refused, Err(Error::Segmentation(_))),
        "{refused:?}"
    );
    assert!(site.sent.is_empty());

    let before = site.last_seen(DNET, DADR);
    tokio::time::advance(Duration::from_secs(5)).await;
    let (result, frame) = tokio::join!(site.write(to.clone(), 8), site.answer(0, true));
    assert!(matches!(result, Err(Error::Remote(_))), "{result:?}");
    assert_eq!(
        sent_to(&frame),
        (ROUTER.to_vec(), Some((DNET, DADR.to_vec())))
    );
    assert!(
        site.last_seen(DNET, DADR) > before,
        "the device's Error is an answer"
    );
}

/// A remote address the table has no row for has no route: it fails as an
/// unknown device, confirmed or not, and nothing is sent. So do addresses
/// that name no one station.
#[tokio::test(start_paused = true)]
async fn no_route_and_no_station_fail_before_sending() {
    let site = site().await;
    let unknown = site
        .client
        .confirmed_request(
            &address(3, &[7]),
            ConfirmedServiceChoice::READ_PROPERTY,
            &[],
        )
        .await;
    assert!(
        matches!(&unknown, Err(Error::UnknownDevice { device }) if device == "3:0x07"),
        "{unknown:?}"
    );
    let unknown = site
        .client
        .unconfirmed_request(
            &address(3, &[7]),
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            &[],
        )
        .await;
    assert!(
        matches!(unknown, Err(Error::UnknownDevice { .. })),
        "{unknown:?}"
    );
    for nobody in [address(0, &[]), address(DNET, &[]), address(u16::MAX, DADR)] {
        let refused = site
            .client
            .confirmed_request(&nobody, ConfirmedServiceChoice::READ_PROPERTY, &[])
            .await;
        assert!(
            matches!(refused, Err(Error::InvalidArgument(_))),
            "{nobody:?}: {refused:?}"
        );
    }
    assert!(site.sent.is_empty());
}

/// An empty MAC on this link names no one station, whichever form names it
/// (an address on network 0, a local `Station`, a bare MAC): confirmed or
/// not, each is refused with the same error, and nothing is sent.
#[tokio::test(start_paused = true)]
async fn an_empty_local_mac_is_refused_alike_in_every_form() {
    let site = site().await;
    let empty: &[u8] = &[];
    let (read, notice) = (
        ConfirmedServiceChoice::READ_PROPERTY,
        UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
    );
    let text = |error: Error| match error {
        Error::InvalidArgument(text) => text.to_string(),
        other => panic!("not an encoding error: {other:?}"),
    };
    let (by_address, by_station) = (address(0, empty), Station::local(empty));
    let refusals = [
        text(
            site.client
                .confirmed_request(&by_address, read, &[])
                .await
                .unwrap_err(),
        ),
        text(
            site.client
                .confirmed_request(&by_station, read, &[])
                .await
                .unwrap_err(),
        ),
        text(
            site.client
                .confirmed_request(empty, read, &[])
                .await
                .unwrap_err(),
        ),
        text(
            site.client
                .unconfirmed_request(&by_address, notice, &[])
                .await
                .unwrap_err(),
        ),
        text(
            site.client
                .unconfirmed_request(&by_station, notice, &[])
                .await
                .unwrap_err(),
        ),
        text(
            site.client
                .unconfirmed_request(empty, notice, &[])
                .await
                .unwrap_err(),
        ),
    ];
    assert!(
        refusals.iter().all(|refusal| *refusal == refusals[0]),
        "{refusals:?}"
    );
    assert_eq!(
        refusals[0],
        "network 0 with an empty MAC is a broadcast, not one station"
    );
    assert!(site.sent.is_empty());
}

/// An instance claimed at two addresses: the lookup by instance is refused
/// as ambiguous, and each address reaches its own station.
#[tokio::test(start_paused = true)]
async fn an_address_reaches_a_station_whose_instance_is_contested() {
    let site = site().await;
    site.add(row(0, STATION, 1476));
    site.add(row(DNET, DADR, 1476));
    let by_instance = site.client.resolve_device(INSTANCE).await;
    assert!(
        matches!(by_instance, Err(Error::AmbiguousDevice { .. })),
        "{by_instance:?}"
    );
    assert!(site.sent.is_empty());
    let (result, frame) = tokio::join!(site.write(address(DNET, DADR), 8), site.answer(0, false));
    result.unwrap();
    assert_eq!(
        sent_to(&frame),
        (ROUTER.to_vec(), Some((DNET, DADR.to_vec())))
    );
    let (result, frame) = tokio::join!(site.write(address(0, STATION), 8), site.answer(1, false));
    result.unwrap();
    assert_eq!(sent_to(&frame), (STATION.to_vec(), None));
}

/// An unconfirmed request goes to a local station at its MAC, and to a
/// remote one through its row's router with the station as DNET/DADR.
#[tokio::test(start_paused = true)]
async fn an_unconfirmed_request_goes_to_the_station() {
    let site = site().await;
    site.add(row(DNET, DADR, 1476));
    let service = UnconfirmedServiceChoice::TIME_SYNCHRONIZATION;
    site.client
        .unconfirmed_request(&address(0, STATION), service, &[1, 2])
        .await
        .unwrap();
    site.client
        .unconfirmed_request(&address(DNET, DADR), service, &[1, 2])
        .await
        .unwrap();
    let frames = site.sent.frames();
    assert_eq!(sent_to(&frames[0]), (STATION.to_vec(), None));
    assert_eq!(
        sent_to(&frames[1]),
        (ROUTER.to_vec(), Some((DNET, DADR.to_vec())))
    );
    for frame in &frames {
        let Apdu::UnconfirmedRequest(request) = frame.apdu() else {
            panic!("expected an unconfirmed request");
        };
        assert_eq!(request.service_choice, service);
        assert_eq!(request.service_request.as_ref(), &[1, 2]);
    }
}

/// An address on the client's own network, once it knows the number, is
/// the local station: no row or router is needed, and no DNET goes out. A
/// broadcast there is no station for an unconfirmed request.
#[tokio::test(start_paused = true)]
async fn an_address_on_the_own_network_is_local() {
    let site = site().await;
    site.learn(77).await;
    let before = site.sent.len();
    let (result, frame) = tokio::join!(
        site.write(address(77, STATION), 8),
        site.answer(before, false)
    );
    result.unwrap();
    assert_eq!(sent_to(&frame), (STATION.to_vec(), None));
    let refused = site
        .client
        .unconfirmed_request(
            &address(77, BROADCAST),
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            &[],
        )
        .await;
    assert!(
        matches!(refused, Err(Error::InvalidArgument(_))),
        "{refused:?}"
    );
    assert_eq!(site.sent.len(), before + 1);
}

/// A named router needs no row: the request goes through it, even when a
/// row names another router for the station, while the same station with
/// no router named and no row fails before sending.
#[tokio::test(start_paused = true)]
async fn a_named_router_needs_no_row_and_wins_over_the_rows() {
    let site = site().await;
    let named = Station::routed(OTHER, 3, &[7]);
    let (result, frame) = tokio::join!(site.write(&named, 8), site.answer(0, false));
    result.unwrap();
    assert_eq!(sent_to(&frame), (OTHER.to_vec(), Some((3, vec![7]))));
    let unknown = site.write(Station::remote(3, &[7]), 8).await;
    assert!(
        matches!(unknown, Err(Error::UnknownDevice { .. })),
        "{unknown:?}"
    );
    site.add(row(DNET, DADR, 1476));
    let (result, frame) = tokio::join!(
        site.write(Station::routed(OTHER, DNET, DADR), 8),
        site.answer(1, false)
    );
    result.unwrap();
    assert_eq!(
        sent_to(&frame),
        (OTHER.to_vec(), Some((DNET, DADR.to_vec())))
    );
    site.client
        .unconfirmed_request(
            &Station::routed(OTHER, DNET, DADR),
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            &[1, 2],
        )
        .await
        .unwrap();
    assert_eq!(
        sent_to(&site.sent.frame(2)),
        (OTHER.to_vec(), Some((DNET, DADR.to_vec())))
    );
    assert_eq!(site.sent.len(), 3);
}

/// A named router to this client's own network, once it knows the number,
/// is the station on this link: no router, no DNET. An unconfirmed request
/// there must name one station, and a routed station must be one a router
/// can carry, before anything is sent.
#[tokio::test(start_paused = true)]
async fn a_named_router_to_the_own_network_goes_local() {
    let site = site().await;
    site.learn(77).await;
    let before = site.sent.len();
    let (result, frame) = tokio::join!(
        site.write(Station::routed(ROUTER, 77, STATION), 8),
        site.answer(before, false)
    );
    result.unwrap();
    assert_eq!(sent_to(&frame), (STATION.to_vec(), None));
    let service = UnconfirmedServiceChoice::TIME_SYNCHRONIZATION;
    for nobody in [
        Station::routed(ROUTER, 77, BROADCAST),
        Station::routed(ROUTER, DNET, &[]),
        Station::routed(ROUTER, u16::MAX, DADR),
    ] {
        let refused = site.client.unconfirmed_request(&nobody, service, &[]).await;
        assert!(
            matches!(refused, Err(Error::InvalidArgument(_))),
            "{nobody:?}: {refused:?}"
        );
    }
    assert_eq!(site.sent.len(), before + 1);
}
