use super::*;
use crate::decode::testing::*;
use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest};
use bacnet_transport::bip6::{Bip6Address, Bip6Vmac, Bvll6, encode_bvll6};
use bacnet_transport::bvll::{Bvll, encode_bvll};
use bacnet_transport::mstp::codec::FrameType;
use bacnet_transport::sc::ScActiveHub;
use bacnet_transport::tap::FrameTap;
use bacnet_types::enums::{ObjectType, Segmentation, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use std::net::SocketAddrV6;

const DEVICE: SocketAddr = SocketAddr::V4(std::net::SocketAddrV4::new(
    std::net::Ipv4Addr::new(192, 0, 2, 3),
    47808,
));
const BBMD: SocketAddr = SocketAddr::V4(std::net::SocketAddrV4::new(
    std::net::Ipv4Addr::new(192, 0, 2, 1),
    47808,
));
const ROUTER: SocketAddr = SocketAddr::V4(std::net::SocketAddrV4::new(
    std::net::Ipv4Addr::new(192, 0, 2, 9),
    47808,
));

/// What a transport hands its tap, read back as one frame.
fn tapped(direction: TapDirection, link: DataLink, peer: TapPeer, octets: &[u8]) -> TappedFrame {
    let tap = FrameTap::new();
    let mut receiver = tap.subscribe();
    tap.record_for_test(direction, link, peer, octets);
    receiver.try_recv().expect("the frame is queued")
}

/// The record an inbound frame makes, which must be one.
fn arrival(link: DataLink, peer: TapPeer, octets: &[u8]) -> IAmArrival {
    IAmArrival::from_frame(&tapped(TapDirection::Inbound, link, peer, octets))
        .expect("well formed")
        .expect("an I-Am")
}

fn mac_of(socket: SocketAddr) -> MacAddr {
    let SocketAddr::V4(socket) = socket else {
        panic!("an IPv4 socket")
    };
    MacAddr::from_slice(&encode_bip_mac(socket.ip().octets(), socket.port()))
}

/// A B/IP Forwarded-NPDU of `npdu` from `origin`.
fn forwarded(origin: SocketAddr, npdu: &[u8]) -> Vec<u8> {
    let SocketAddr::V4(origin) = origin else {
        panic!("an IPv4 origin")
    };
    let mut out = BytesMut::new();
    encode_bvll(&mut out, &Bvll::ForwardedNpdu { origin, npdu }).unwrap();
    out.to_vec()
}

#[test]
fn a_direct_bip_i_am_is_from_its_sender() {
    let record = arrival(
        DataLink::Bip,
        TapPeer::Udp(DEVICE),
        &bip(&npdu(&i_am(1234), false), true),
    );
    assert_eq!(record.i_am.object_identifier.instance_number(), 1234);
    assert_eq!(record.i_am.max_apdu_length, 1476);
    assert_eq!(record.i_am.vendor_id, 260);
    assert_eq!(record.link_sender, TapPeer::Udp(DEVICE));
    assert_eq!(record.link_source, mac_of(DEVICE));
    assert_eq!(
        (record.forwarded, record.original_source, record.broadcast),
        (false, None, true)
    );
    assert_eq!(record.source, None);
    assert_eq!(
        record.device_address(),
        Some(DeviceAddress::local(&mac_of(DEVICE)).unwrap())
    );
    assert_eq!(record.next_hop_router(), None);
    assert_eq!(record.forwarding_bbmd(), None);
}

#[test]
fn a_forwarded_bip_i_am_names_the_device_and_the_bbmd_apart() {
    let wire = forwarded(DEVICE, &npdu(&i_am(1234), false));
    let record = arrival(DataLink::Bip, TapPeer::Udp(BBMD), &wire);
    assert_eq!(record.link_sender, TapPeer::Udp(BBMD));
    assert_eq!(
        record.link_source,
        mac_of(DEVICE),
        "the origin, not the BBMD"
    );
    assert_eq!(record.original_source, Some(DEVICE));
    assert!(record.forwarded);
    assert!(record.broadcast);
    assert_eq!(record.forwarding_bbmd(), Some(TapPeer::Udp(BBMD)));
    assert_eq!(
        record.device_address(),
        Some(DeviceAddress::local(&mac_of(DEVICE)).unwrap())
    );
    // A BBMD forwarding its own device's broadcast is its origin: nothing
    // stood between them.
    let own_wire = forwarded(BBMD, &npdu(&i_am(1), false));
    let own = arrival(DataLink::Bip, TapPeer::Udp(BBMD), &own_wire);
    assert!(!own.forwarded);
    assert_eq!(own.forwarding_bbmd(), None);
}

#[test]
fn a_routed_i_am_names_the_device_by_network_and_the_router_as_next_hop() {
    let wire = bip(&routed_npdu(&i_am(77), false, Some((5, &[7]))), true);
    let record = arrival(DataLink::Bip, TapPeer::Udp(ROUTER), &wire);
    let routed = record.source.clone().expect("SNET and SADR");
    assert_eq!((routed.network, &routed.mac_address[..]), (5, &[7][..]));
    assert_eq!(record.link_source, mac_of(ROUTER));
    assert_eq!(record.next_hop_router(), Some(&mac_of(ROUTER)[..]));
    assert_eq!(
        record.device_address(),
        Some(DeviceAddress::new(5, &[7]).unwrap()),
        "the device, not its router"
    );
    assert!(!record.forwarded);
}

#[test]
fn a_routed_i_am_a_bbmd_forwarded_has_all_three() {
    let wire = forwarded(ROUTER, &routed_npdu(&i_am(77), false, Some((5, &[7]))));
    let record = arrival(DataLink::Bip, TapPeer::Udp(BBMD), &wire);
    assert!(record.forwarded);
    assert_eq!(record.forwarding_bbmd(), Some(TapPeer::Udp(BBMD)));
    assert_eq!(record.next_hop_router(), Some(&mac_of(ROUTER)[..]));
    assert_eq!(
        record.device_address(),
        Some(DeviceAddress::new(5, &[7]).unwrap())
    );
}

fn v6(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

#[test]
fn a_bip6_i_am_is_from_its_vmac() {
    let sender = v6("[2001:db8::2]:47808");
    let record = arrival(
        DataLink::Bip6,
        TapPeer::Udp(sender),
        &bip6_broadcast(&npdu(&i_am(1234), false)),
    );
    assert_eq!(&record.link_source[..], &[0, 1, 2]);
    assert_eq!(record.link_sender, TapPeer::Udp(sender));
    assert_eq!((record.forwarded, record.original_source), (false, None));
    assert_eq!(
        record.device_address().unwrap().to_string(),
        "bip6:0x000102"
    );
}

#[test]
fn a_forwarded_bip6_i_am_keeps_the_original_vmac_and_address() {
    let payload = npdu(&i_am(1234), false);
    let mut wire = BytesMut::new();
    encode_bvll6(
        &mut wire,
        &Bvll6::ForwardedNpdu {
            original_source: Bip6Vmac::new([0, 4, 0xD2]),
            original_address: Bip6Address::new("2001:db8::3".parse().unwrap(), 47808),
            npdu: &payload,
        },
    )
    .unwrap();
    let bbmd = v6("[2001:db8::1]:47808");
    let record = arrival(DataLink::Bip6, TapPeer::Udp(bbmd), &wire);
    assert_eq!(&record.link_source[..], &[0, 4, 0xD2]);
    assert_eq!(record.original_source, Some(v6("[2001:db8::3]:47808")));
    assert!(record.forwarded);
    assert_eq!(record.forwarding_bbmd(), Some(TapPeer::Udp(bbmd)));
    // The same address with a scope or flow label is no other node.
    let scoped = SocketAddr::V6(SocketAddrV6::new(
        "2001:db8::3".parse().unwrap(),
        47808,
        9,
        2,
    ));
    let from_origin = arrival(DataLink::Bip6, TapPeer::Udp(scoped), &wire);
    assert!(!from_origin.forwarded);
}

#[test]
fn an_sc_i_am_is_from_its_originating_vmac() {
    let wire = sc_npdu(&npdu(&i_am(1234), false), Some([0xFF; 6]));
    let record = arrival(DataLink::Sc, TapPeer::ScHub(ScActiveHub::Primary), &wire);
    assert_eq!(&record.link_source[..], &[2, 0, 0, 0, 0, 1]);
    assert_eq!(record.link_sender, TapPeer::ScHub(ScActiveHub::Primary));
    assert!(record.broadcast);
    assert!(!record.forwarded);
    assert_eq!(
        record.device_address(),
        Some(DeviceAddress::local(&[2, 0, 0, 0, 0, 1]).unwrap())
    );
}

#[test]
fn an_mstp_i_am_is_from_its_station_to_anyone_on_the_bus() {
    let data = npdu(&i_am(1234), false);
    let broadcast = mstp(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 255, 9, &data);
    let record = arrival(DataLink::Mstp, TapPeer::Mstp(9), &broadcast);
    assert_eq!(&record.link_source[..], &[9]);
    assert_eq!(record.link_sender, TapPeer::Mstp(9));
    assert!(record.broadcast);
    assert_eq!(record.device_address().unwrap().to_string(), "mstp:9");
    // An answer one station unicasts to another is heard on the bus too.
    let unicast = mstp(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 4, 9, &data);
    let record = arrival(DataLink::Mstp, TapPeer::Mstp(9), &unicast);
    assert!(!record.broadcast);
}

/// An I-Am of `identifier` with the parameters given, as an older device
/// may send them.
fn i_am_with(
    identifier: ObjectIdentifier,
    max_apdu_length: u32,
    segmentation: Segmentation,
    vendor_id: u16,
) -> Apdu {
    let mut request = BytesMut::new();
    IAmRequest {
        object_identifier: identifier,
        max_apdu_length,
        segmentation_supported: segmentation,
        vendor_id,
    }
    .encode(&mut request);
    Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: request.freeze(),
    })
}

#[test]
fn odd_but_decodable_parameters_are_kept_as_sent() {
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 4_194_303).unwrap();
    for (identifier, max_apdu, segmentation, vendor) in [
        (device, 50, Segmentation::NO_SEGMENTATION, 0),
        (device, 49, Segmentation::from_raw(9), u16::MAX),
        (analog, 1_000_000, Segmentation::SEGMENTED_BOTH, 5),
    ] {
        let wire = bip(
            &npdu(
                &i_am_with(identifier, max_apdu, segmentation, vendor),
                false,
            ),
            true,
        );
        let record = arrival(DataLink::Bip, TapPeer::Udp(DEVICE), &wire);
        assert_eq!(record.i_am.object_identifier, identifier);
        assert_eq!(record.i_am.max_apdu_length, max_apdu);
        assert_eq!(record.i_am.segmentation_supported, segmentation);
        assert_eq!(record.i_am.vendor_id, vendor);
    }
}

#[test]
fn only_inbound_i_ams_make_records() {
    let wire = bip(&npdu(&i_am(1), false), true);
    let outbound = tapped(
        TapDirection::Outbound,
        DataLink::Bip,
        TapPeer::Udp(DEVICE),
        &wire,
    );
    assert_eq!(IAmArrival::from_frame(&outbound), Ok(None));
    let request = bip(&npdu(&rp_request(1), true), false);
    let inbound = tapped(
        TapDirection::Inbound,
        DataLink::Bip,
        TapPeer::Udp(DEVICE),
        &request,
    );
    assert_eq!(IAmArrival::from_frame(&inbound), Ok(None));
}

/// A tap with a subscription of `capacity` frames read as arrivals.
fn stream(capacity: usize) -> (FrameTap, IAmArrivals) {
    let tap = FrameTap::new();
    let arrivals = IAmArrivals::new(tap.subscribe_with_capacity(capacity));
    (tap, arrivals)
}

fn inbound(tap: &FrameTap, from: SocketAddr, octets: &[u8]) {
    tap.record_for_test(
        TapDirection::Inbound,
        DataLink::Bip,
        TapPeer::Udp(from),
        octets,
    );
}

#[test]
fn repeats_and_duplicate_instances_are_kept_in_order() {
    let (tap, mut arrivals) = stream(16);
    let same = bip(&npdu(&i_am(1234), false), true);
    for from in [DEVICE, ROUTER, DEVICE, DEVICE] {
        inbound(&tap, from, &same);
    }
    let senders: Vec<_> = std::iter::from_fn(|| arrivals.try_recv())
        .map(|record| {
            assert_eq!(record.i_am.object_identifier.instance_number(), 1234);
            record.link_sender
        })
        .collect();
    let expected = [DEVICE, ROUTER, DEVICE, DEVICE].map(TapPeer::Udp);
    assert_eq!(senders, expected);
}

#[test]
fn malformed_i_ams_are_counted_and_header_failures_are_not() {
    let (tap, mut arrivals) = stream(16);
    let truncated = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: bytes::Bytes::from_static(&[0xC4, 0x02, 0x00]),
    });
    inbound(&tap, DEVICE, &bip(&npdu(&truncated, false), true));
    // A trailing octet after the vendor identifier is no I-Am either.
    let mut long = npdu(&i_am(5), false);
    long.push(0x00);
    inbound(&tap, DEVICE, &bip(&long, true));
    // An SC I-Am that names no originating VMAC.
    let mut sc = crate::decode::testing::sc_npdu(&npdu(&i_am(6), false), Some([0xFF; 6]));
    sc = strip_originating_vmac(&sc);
    tap.record_for_test(
        TapDirection::Inbound,
        DataLink::Sc,
        TapPeer::ScHub(ScActiveHub::Primary),
        &sc,
    );
    // Headers that fail: not known to be an I-Am.
    inbound(&tap, DEVICE, &[0x81, 0x0B, 0x00]);
    inbound(&tap, DEVICE, &bip(&[0x01, 0x00, 0xFF], true));
    inbound(&tap, DEVICE, &bip(&npdu(&i_am(7), false), true));
    let record = arrivals.try_recv().expect("the good one");
    assert_eq!(record.i_am.object_identifier.instance_number(), 7);
    assert!(arrivals.try_recv().is_none());
    assert_eq!(arrivals.malformed(), 3);
}

/// `wire`, a BVLC-SC Encapsulated-NPDU with an originating VMAC, without it.
fn strip_originating_vmac(wire: &[u8]) -> Vec<u8> {
    use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message};
    let mut message = decode_sc_message(wire).unwrap();
    message.originating_vmac = None;
    let mut out = BytesMut::new();
    encode_sc_message(&mut out, &message);
    out.to_vec()
}

#[test]
fn a_full_queue_counts_the_frames_lost_before_the_next_record() {
    let (tap, mut arrivals) = stream(1);
    let i_am_frame = bip(&npdu(&i_am(1), false), true);
    let other = bip(&npdu(&rp_request(1), true), false);
    inbound(&tap, DEVICE, &i_am_frame);
    // Lost behind the first: an I-Am and a request, of any kind.
    inbound(&tap, DEVICE, &i_am_frame);
    inbound(&tap, DEVICE, &other);
    assert_eq!(arrivals.missed(), 2);
    assert_eq!(arrivals.try_recv().unwrap().missed_before, 0);
    // The next frame queued, no I-Am, carries the gap; the record after it
    // reports it.
    inbound(&tap, DEVICE, &other);
    assert!(arrivals.try_recv().is_none());
    inbound(&tap, DEVICE, &i_am_frame);
    assert_eq!(arrivals.try_recv().unwrap().missed_before, 2);
    inbound(&tap, DEVICE, &i_am_frame);
    assert_eq!(arrivals.try_recv().unwrap().missed_before, 0);
    assert_eq!(arrivals.missed(), 2);
}

/// A client names no address of its own: an I-Am from its address, of any
/// instance, direct or returned by a BBMD, is a record.
#[test]
fn a_client_leaves_nothing_out() {
    let (tap, mut arrivals) = stream(16);
    inbound(&tap, DEVICE, &bip(&npdu(&i_am(1), false), true));
    inbound(&tap, BBMD, &forwarded(DEVICE, &npdu(&i_am(1), false)));
    assert_eq!(std::iter::from_fn(|| arrivals.try_recv()).count(), 2);
    assert_eq!(arrivals.own_echoes(), 0);
}

/// An endpoint's own I-Am come back, direct or returned by a BBMD, is left
/// out and counted, and is no loss: the frame lost behind the echoes is
/// the next record's whole gap.
#[test]
fn an_endpoints_own_i_am_come_back_is_left_out_and_counted() {
    let (tap, arrivals) = stream(2);
    let mut arrivals = arrivals.with_own(DEVICE, 1);
    let own = bip(&npdu(&i_am(1), false), true);
    inbound(&tap, DEVICE, &own);
    inbound(&tap, BBMD, &forwarded(DEVICE, &npdu(&i_am(1), false)));
    inbound(&tap, ROUTER, &own); // Lost: the queue holds two.
    assert!(arrivals.try_recv().is_none());
    assert_eq!(arrivals.own_echoes(), 2);
    inbound(&tap, ROUTER, &own);
    let record = arrivals.try_recv().expect("another node's");
    assert_eq!(
        (record.link_sender, record.missed_before),
        (TapPeer::Udp(ROUTER), 1)
    );
    assert_eq!((arrivals.own_echoes(), arrivals.missed()), (2, 1));
}

/// Another device at this node's own address is a conflict to see, not an
/// echo: another instance, or this instance in another object type.
#[test]
fn another_device_at_this_nodes_own_address_is_a_record() {
    let (tap, arrivals) = stream(16);
    let mut arrivals = arrivals.with_own(DEVICE, 1);
    inbound(&tap, DEVICE, &bip(&npdu(&i_am(2), false), true));
    let record = arrivals.try_recv().expect("a conflict");
    assert_eq!(record.i_am.object_identifier.instance_number(), 2);
    assert_eq!(record.link_source, mac_of(DEVICE));
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let not_a_device = i_am_with(analog, 1476, Segmentation::NO_SEGMENTATION, 5);
    inbound(&tap, DEVICE, &bip(&npdu(&not_a_device, false), true));
    assert!(arrivals.try_recv().is_some());
    assert_eq!(arrivals.own_echoes(), 0);
}

/// Behind NAT another site may reuse this node's private address (Annex
/// J.7.5): a forwarded origin equal to it, of another instance, is that
/// site's device.
#[test]
fn a_nat_site_reusing_this_nodes_address_is_a_record() {
    let (tap, arrivals) = stream(16);
    let mut arrivals = arrivals.with_own(DEVICE, 1);
    inbound(&tap, BBMD, &forwarded(DEVICE, &npdu(&i_am(5005), false)));
    let record = arrivals.try_recv().expect("the other site's device");
    assert!(record.forwarded);
    assert_eq!(record.original_source, Some(DEVICE));
    assert_eq!(arrivals.own_echoes(), 0);
}

/// A routed I-Am sent to every network (DNET 65535) by unicast is a
/// broadcast by its network header alone.
#[test]
fn a_global_broadcast_sent_by_unicast_is_a_broadcast() {
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu};
    let routed = routed_npdu(&i_am(77), false, Some((5, &[7])));
    let mut global = decode_npdu(bytes::Bytes::from(routed)).unwrap();
    global.destination = Some(NpduAddress {
        network: u16::MAX,
        mac_address: MacAddr::new(),
    });
    let mut data = BytesMut::new();
    encode_npdu(&mut data, &global).unwrap();
    let record = arrival(DataLink::Bip, TapPeer::Udp(ROUTER), &bip(&data, false));
    assert!(record.broadcast);
    assert_eq!(record.destination, global.destination);
    let local = arrival(
        DataLink::Bip,
        TapPeer::Udp(ROUTER),
        &bip(&npdu(&i_am(77), false), false),
    );
    assert!(!local.broadcast);
    assert!(local.destination.is_none());
}

/// A BBMD-forwarded I-Am keeps the destination its NPDU carries: the
/// network and the station octets, beside the source network.
#[test]
fn a_forwarded_i_am_keeps_the_destination_it_carries() {
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu};
    let routed = routed_npdu(&i_am(77), false, Some((5, &[7])));
    let mut addressed = decode_npdu(bytes::Bytes::from(routed)).unwrap();
    addressed.destination = Some(NpduAddress {
        network: 9,
        mac_address: MacAddr::from_slice(&[19, 20]),
    });
    let mut data = BytesMut::new();
    encode_npdu(&mut data, &addressed).unwrap();
    let record = arrival(DataLink::Bip, TapPeer::Udp(BBMD), &forwarded(ROUTER, &data));
    assert!(record.forwarded);
    assert_eq!(record.destination, addressed.destination);
    assert_eq!(record.source, addressed.source);
}

struct Woken(std::sync::atomic::AtomicBool);

impl std::task::Wake for Woken {
    fn wake(self: std::sync::Arc<Self>) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A read passes over at most a batch of frames that are no record, then
/// wakes itself and yields; the next poll goes on.
#[test]
fn a_read_yields_after_a_batch_of_other_frames() {
    let (tap, mut arrivals) = stream(SKIP_BATCH * 2);
    let other = bip(&npdu(&rp_request(1), true), false);
    for _ in 0..SKIP_BATCH + 1 {
        inbound(&tap, DEVICE, &other);
    }
    inbound(&tap, ROUTER, &bip(&npdu(&i_am(9), false), true));
    let woken = std::sync::Arc::new(Woken(Default::default()));
    let waker = std::task::Waker::from(woken.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(arrivals.poll_recv(&mut cx).is_pending());
    assert!(woken.0.load(std::sync::atomic::Ordering::SeqCst));
    let Poll::Ready(Some(record)) = arrivals.poll_recv(&mut cx) else {
        panic!("the record after the batch");
    };
    assert_eq!(record.link_sender, TapPeer::Udp(ROUTER));
}

#[tokio::test]
async fn a_read_waits_for_a_record_past_other_frames_and_ends_with_the_tap() {
    let (tap, mut arrivals) = stream(16);
    inbound(&tap, DEVICE, &bip(&npdu(&rp_request(1), true), false));
    let reader = tokio::spawn(async move {
        let first = arrivals.recv().await;
        (first, arrivals.recv().await)
    });
    tokio::task::yield_now().await;
    inbound(&tap, ROUTER, &bip(&npdu(&i_am(9), false), true));
    drop(tap);
    let (first, end) = reader.await.unwrap();
    assert_eq!(first.unwrap().link_sender, TapPeer::Udp(ROUTER));
    assert!(end.is_none());
}
