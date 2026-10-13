//! Network Port host and broadcast-table values (#939): golden vectors built
//! by hand from the Clause 21 productions, and refusal of malformed values.

use super::*;
use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// `fe80::1`, the IPv6 address the vectors use.
const V6: [u8; 16] = [0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

fn v6() -> Ipv6Addr {
    Ipv6Addr::from(V6)
}

fn host(ip: [u8; 4], port: u16) -> BACnetHostNPort {
    BACnetHostNPort {
        host: BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::from(ip))),
        port,
    }
}

fn with(prefix: &[u8], body: &[u8], suffix: &[u8]) -> Vec<u8> {
    [prefix, body, suffix].concat()
}

#[test]
fn host_address_choice_arms_have_their_own_tags() {
    // "bbmd.example": charset octet 0 (UTF-8) plus 12 characters.
    let name = b"\x00bbmd.example";
    let cases: Vec<(Vec<u8>, BACnetHostAddress)> = vec![
        // none [0] NULL: a primitive context tag 0 with no contents.
        (vec![0x08], BACnetHostAddress::None),
        // ip-address [1] OCTET STRING of 4 octets.
        (
            vec![0x1C, 192, 168, 1, 10],
            BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))),
        ),
        // ip-address [1] of 16 octets: extended length 0x10.
        (
            with(&[0x1D, 0x10], &V6, &[]),
            BACnetHostAddress::Ip(IpAddr::V6(v6())),
        ),
        // name [2] CharacterString: extended length 13.
        (
            with(&[0x2D, 0x0D], name, &[]),
            BACnetHostAddress::Name("bbmd.example".into()),
        ),
    ];
    for (bytes, value) in cases {
        let mut buf = BytesMut::new();
        encode_host_address(&mut buf, &value).unwrap();
        assert_eq!(&buf[..], &bytes[..], "{value:?}");
        assert_eq!(
            decode_host_address(&bytes, 0).unwrap(),
            (value, bytes.len())
        );
    }
}

#[test]
fn host_n_port_frames_the_host_choice_in_tag_zero() {
    let name = b"\x00bbmd.example";
    let cases: Vec<(Vec<u8>, BACnetHostNPort)> = vec![
        (
            vec![0x0E, 0x1C, 10, 0, 0, 1, 0x0F, 0x1A, 0xBA, 0xC0],
            host([10, 0, 0, 1], 0xBAC0),
        ),
        // The unset global address of Clause 12.56.33: no host, port 0.
        (
            vec![0x0E, 0x08, 0x0F, 0x19, 0x00],
            BACnetHostNPort {
                host: BACnetHostAddress::None,
                port: 0,
            },
        ),
        (
            with(&[0x0E, 0x2D, 0x0D], name, &[0x0F, 0x1A, 0xBA, 0xC1]),
            BACnetHostNPort {
                host: BACnetHostAddress::Name("bbmd.example".into()),
                port: 0xBAC1,
            },
        ),
        (
            with(&[0x0E, 0x1D, 0x10], &V6, &[0x0F, 0x19, 80]),
            BACnetHostNPort {
                host: BACnetHostAddress::Ip(IpAddr::V6(v6())),
                port: 80,
            },
        ),
    ];
    for (bytes, value) in cases {
        let mut buf = BytesMut::new();
        encode_host_n_port(&mut buf, &value).unwrap();
        assert_eq!(&buf[..], &bytes[..], "{value:?}");
        assert_eq!(decode_host_n_port(&bytes, 0).unwrap(), (value, bytes.len()));
    }
    assert_eq!(
        BACnetHostNPort::from_socket_addr(SocketAddr::from(([10, 0, 0, 1], 0xBAC0))),
        host([10, 0, 0, 1], 0xBAC0)
    );
}

#[test]
fn bdt_entries_frame_the_bbmd_address_and_carry_an_optional_mask() {
    let ipv4 = BACnetBDTEntry {
        bbmd_address: host([10, 0, 0, 1], 0xBAC0),
        broadcast_mask: Some([0xFF, 0xFF, 0xFF, 0xFF]),
    };
    let ipv4_bytes = [
        0x0E, // bbmd-address [0] opens
        0x0E, 0x1C, 10, 0, 0, 1, 0x0F, // host [0] holding ip-address [1]
        0x1A, 0xBA, 0xC0, // port [1]
        0x0F, // bbmd-address [0] closes
        0x1C, 0xFF, 0xFF, 0xFF, 0xFF, // broadcast-mask [1]
    ];
    // B/IPv6 leaves the mask out.
    let ipv6 = BACnetBDTEntry {
        bbmd_address: BACnetHostNPort {
            host: BACnetHostAddress::Ip(IpAddr::V6(v6())),
            port: 0xBAC0,
        },
        broadcast_mask: None,
    };
    let ipv6_bytes = with(
        &[0x0E, 0x0E, 0x1D, 0x10],
        &V6,
        &[0x0F, 0x1A, 0xBA, 0xC0, 0x0F],
    );
    for (bytes, entry) in [(&ipv4_bytes[..], &ipv4), (&ipv6_bytes[..], &ipv6)] {
        let mut buf = BytesMut::new();
        encode_bdt_entry(&mut buf, entry).unwrap();
        assert_eq!(&buf[..], bytes);
        assert_eq!(
            decode_bdt_entry(bytes, 0).unwrap(),
            (entry.clone(), bytes.len())
        );
    }
    // In a list, an entry without a mask is followed directly by the next
    // entry's opening tag, which is not mistaken for a mask.
    let list = [ipv6.clone(), ipv4.clone()];
    let mut buf = BytesMut::new();
    encode_bdt_entry_list(&mut buf, &list).unwrap();
    assert_eq!(&buf[..], [&ipv6_bytes[..], &ipv4_bytes[..]].concat());
    assert_eq!(decode_bdt_entry_list(&buf).unwrap(), list);
    assert_eq!(decode_bdt_entry_list(&[]).unwrap(), Vec::new());
}

#[test]
fn fdt_entries_are_three_primitives() {
    let ipv4 = BACnetFDTEntry {
        address: SocketAddr::from(([10, 0, 0, 5], 0xBAC0)),
        time_to_live: 60,
        remaining_time_to_live: 85,
    };
    let ipv4_bytes = [
        0x0D, 0x06, 10, 0, 0, 5, 0xBA, 0xC0, // bacnetip-address [0], 6 octets
        0x19, 60, // time-to-live [1]
        0x29, 85, // remaining-time-to-live [2]
    ];
    let ipv6 = BACnetFDTEntry {
        address: SocketAddr::from((v6(), 0xBAC0)),
        time_to_live: 300,
        remaining_time_to_live: 330,
    };
    let ipv6_bytes = with(
        &[0x0D, 0x12],
        &V6,
        &[0xBA, 0xC0, 0x1A, 0x01, 0x2C, 0x2A, 0x01, 0x4A],
    );
    for (bytes, entry) in [(&ipv4_bytes[..], &ipv4), (&ipv6_bytes[..], &ipv6)] {
        let mut buf = BytesMut::new();
        encode_fdt_entry(&mut buf, entry);
        assert_eq!(&buf[..], bytes);
        assert_eq!(
            decode_fdt_entry(bytes, 0).unwrap(),
            (entry.clone(), bytes.len())
        );
    }
    let list = [ipv4, ipv6];
    let mut buf = BytesMut::new();
    encode_fdt_entry_list(&mut buf, &list);
    assert_eq!(&buf[..], [&ipv4_bytes[..], &ipv6_bytes[..]].concat());
    assert_eq!(decode_fdt_entry_list(&buf).unwrap(), list);
    assert_eq!(decode_fdt_entry_list(&[]).unwrap(), Vec::new());
}

#[test]
fn host_address_refuses_malformed_alternatives() {
    for bytes in [
        // NULL with a contents octet.
        &[0x09, 0x00][..],
        // IP addresses of 0, 5 and 6 octets.
        &[0x18][..],
        &[0x1D, 0x05, 1, 2, 3, 4, 5][..],
        &[0x1D, 0x06, 1, 2, 3, 4, 5, 6][..],
        // Truncated contents.
        &[0x1C, 10, 0][..],
        // A name with no charset octet, or invalid UTF-8.
        &[0x28][..],
        &[0x2A, 0x00, 0xFF][..],
        // A tag the CHOICE doesn't have, an application tag, a constructed
        // alternative and an empty input.
        &[0x39, 0x00][..],
        &[0x21, 0x00][..],
        &[0x1E, 0x1F][..],
        &[][..],
    ] {
        assert!(
            decode_host_address(bytes, 0).is_err(),
            "{bytes:02X?} must not decode"
        );
    }
}

#[test]
fn host_n_port_refuses_malformed_sequences() {
    for bytes in [
        // The host is not framed in [0].
        &[0x1C, 10, 0, 0, 1, 0x1A, 0xBA, 0xC0][..],
        // The frame never closes, or holds two alternatives.
        &[0x0E, 0x08, 0x19, 0x00][..],
        &[0x0E, 0x08, 0x08, 0x0F, 0x19, 0x00][..],
        // An empty frame.
        &[0x0E, 0x0F, 0x19, 0x00][..],
        // No port, a port past Unsigned16, an application-tagged port and an
        // empty port.
        &[0x0E, 0x08, 0x0F][..],
        &[0x0E, 0x08, 0x0F, 0x1B, 0x01, 0x00, 0x00][..],
        &[0x0E, 0x08, 0x0F, 0x21, 0x00][..],
        &[0x0E, 0x08, 0x0F, 0x18][..],
    ] {
        assert!(
            decode_host_n_port(bytes, 0).is_err(),
            "{bytes:02X?} must not decode"
        );
    }
}

#[test]
fn bdt_entry_refuses_malformed_entries() {
    let inner = [0x0E, 0x1C, 10, 0, 0, 1, 0x0F, 0x1A, 0xBA, 0xC0];
    for bytes in [
        // The BBMD address is not framed in [0], or its frame never closes.
        inner.to_vec(),
        with(&[0x0E], &inner, &[]),
        with(&[0x0E], &inner, &[0x1C, 0xFF, 0xFF, 0xFF, 0xFF]),
        // Masks of 3 and 16 octets.
        with(&[0x0E], &inner, &[0x0F, 0x1B, 0xFF, 0xFF, 0xFF]),
        with(
            &[0x0E],
            &inner,
            &with(&[0x0F, 0x1D, 0x10], &[0xFF; 16], &[]),
        ),
        // A truncated mask.
        with(&[0x0E], &inner, &[0x0F, 0x1C, 0xFF]),
    ] {
        assert!(
            decode_bdt_entry(&bytes, 0).is_err(),
            "{bytes:02X?} must not decode"
        );
    }
    // A list's trailing octets that start no entry.
    let mut list = with(&[0x0E], &inner, &[0x0F]);
    list.push(0x1C);
    assert!(decode_bdt_entry_list(&list).is_err());
}

#[test]
fn fdt_entry_refuses_malformed_entries() {
    for bytes in [
        // Addresses of 4, 0 and 7 octets.
        &[0x0C, 10, 0, 0, 5, 0x19, 60, 0x29, 85][..],
        &[0x08, 0x19, 60, 0x29, 85][..],
        &[0x0D, 0x07, 10, 0, 0, 5, 0xBA, 0xC0, 0, 0x19, 60, 0x29, 85][..],
        // A time to live past Unsigned16, and an empty one.
        &[0x0D, 0x06, 10, 0, 0, 5, 0xBA, 0xC0, 0x1B, 1, 0, 0, 0x29, 85][..],
        &[0x0D, 0x06, 10, 0, 0, 5, 0xBA, 0xC0, 0x18, 0x29, 85][..],
        // No time remaining, and the times swapped.
        &[0x0D, 0x06, 10, 0, 0, 5, 0xBA, 0xC0, 0x19, 60][..],
        &[0x0D, 0x06, 10, 0, 0, 5, 0xBA, 0xC0, 0x29, 85, 0x19, 60][..],
        // A constructed address, and a truncated one.
        &[0x0E, 0x0F, 0x19, 60, 0x29, 85][..],
        &[0x0D, 0x06, 10, 0, 0][..],
    ] {
        assert!(
            decode_fdt_entry(bytes, 0).is_err(),
            "{bytes:02X?} must not decode"
        );
    }
    let mut list = vec![0x0D, 0x06, 10, 0, 0, 5, 0xBA, 0xC0, 0x19, 60, 0x29, 85];
    list.push(0x0D);
    assert!(decode_fdt_entry_list(&list).is_err());
}

#[test]
fn every_truncation_and_octet_flip_decodes_or_errors_without_panicking() {
    let mut bdt = BytesMut::new();
    encode_bdt_entry_list(
        &mut bdt,
        &[
            BACnetBDTEntry {
                bbmd_address: host([10, 0, 0, 1], 0xBAC0),
                broadcast_mask: Some([0xFF; 4]),
            },
            BACnetBDTEntry {
                bbmd_address: BACnetHostNPort {
                    host: BACnetHostAddress::Name("bbmd.example".into()),
                    port: 0xBAC0,
                },
                broadcast_mask: None,
            },
        ],
    )
    .unwrap();
    let mut fdt = BytesMut::new();
    encode_fdt_entry_list(
        &mut fdt,
        &[BACnetFDTEntry {
            address: SocketAddr::from((v6(), 0xBAC0)),
            time_to_live: 300,
            remaining_time_to_live: 330,
        }],
    );
    type Decodes = fn(&[u8]) -> bool;
    let decoders: [(&[u8], Decodes); 2] = [
        (&bdt, |b| decode_bdt_entry_list(b).is_ok()),
        (&fdt, |b| decode_fdt_entry_list(b).is_ok()),
    ];
    for (bytes, decodes) in decoders {
        assert!(decodes(bytes));
        for len in 0..bytes.len() {
            decodes(&bytes[..len]);
        }
        for at in 0..bytes.len() {
            for flip in [0x01, 0x08, 0x80, 0xFF] {
                let mut mutated = bytes.to_vec();
                mutated[at] ^= flip;
                decodes(&mutated);
            }
        }
    }
}
