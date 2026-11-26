//! What a B/IPv6 node takes from inside a Forwarded-NPDU (#2829), and what
//! a B/IPv6 source VMAC is worth.
//!
//! A Forwarded-NPDU carries a broadcast a BBMD distributes (Annex U.2.9,
//! U.4.4), and its NPDU is taken as from the original source it names. A
//! broadcast carries only an Unconfirmed-Request or a network message
//! (Clause 6.3), so an answer or a confirmed request inside one is dropped,
//! as on B/IP. On B/IPv6 that rule is consistency rather than protection:
//! the source VMAC of every message is a field its sender writes, and no
//! rule binds it to the address it came from (U.5), so a station can name
//! another node's VMAC in a directed frame too.

use std::net::{Ipv6Addr, SocketAddrV6};

use bytes::BytesMut;

use super::frame::encoded;
use super::group_delivery_tests::{LOCAL_VMAC, datagram, local_ip, receiver};
use super::*;

/// The device a stranger names: its VMAC, and the address it sends from.
const DEVICE_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x04, 0xD2]);

fn device() -> SocketAddrV6 {
    SocketAddrV6::new("2001:db8::20".parse().unwrap(), 0xBAC0, 0, 0)
}

/// A local NPDU carrying a ComplexACK for ReadProperty.
const COMPLEX_ACK: &[u8] = &[
    0x01, 0x00, 0x30, 0x05, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x44, 0x26,
    0x80, 0x00, 0x3F,
];
/// A local NPDU carrying a Who-Is.
const WHO_IS: &[u8] = &[0x01, 0x00, 0x10, 0x08];

fn forwarded(npdu: &[u8]) -> BytesMut {
    encoded(&Bvll6::ForwardedNpdu {
        original_source: DEVICE_VMAC,
        original_address: device().into(),
        npdu,
    })
    .unwrap()
}

/// A forward a stranger multicasts, naming the device as its origin: its
/// ComplexACK is dropped and counted, its Who-Is handed up as the device's.
#[tokio::test]
async fn an_answer_inside_a_forward_is_never_shown_as_its_origins() {
    let (receiver, mut rx) = receiver().await;
    let to_group = |frame: &BytesMut| datagram(frame, BACNET_IPV6_MULTICAST_LINK_LOCAL, None);

    let ack = forwarded(COMPLEX_ACK);
    receiver.handle_datagram(&ack, &to_group(&ack)).await;
    assert!(rx.try_recv().is_err(), "the ComplexACK was handed up");
    assert_eq!(receiver.drops.snapshot().forwarded_not_broadcast, 1);

    let who_is = forwarded(WHO_IS);
    receiver.handle_datagram(&who_is, &to_group(&who_is)).await;
    let npdu = rx.try_recv().expect("the Who-Is is handed up");
    assert_eq!(npdu.source_mac.as_slice(), DEVICE_VMAC.octets());
    assert!(npdu.link_layer_group);
}

/// A directed frame is handed up as from the VMAC it names, whatever
/// address it came from: here a stranger's, after the device was heard at
/// its own. This is Annex U's design, not a fault of the forward rule, and
/// it bounds what any check on a B/IPv6 link source can rely on.
#[tokio::test]
async fn a_directed_frame_is_handed_up_as_the_vmac_it_names_from_any_address() {
    let (receiver, mut rx) = receiver().await;
    let from_device = encoded(&Bvll6::OriginalUnicastNpdu {
        source: DEVICE_VMAC,
        destination: LOCAL_VMAC,
        npdu: WHO_IS,
    })
    .unwrap();
    let stranger = SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x99),
        0xBAC0,
        0,
        0,
    );
    for peer in [device(), stranger] {
        let frame = decode_bvll6(&from_device).unwrap();
        receiver
            .dispatch(frame, peer, &datagram(&from_device, local_ip(), None))
            .await;
        let npdu = rx.try_recv().expect("handed up");
        assert_eq!(npdu.source_mac.as_slice(), DEVICE_VMAC.octets(), "{peer}");
    }
}
