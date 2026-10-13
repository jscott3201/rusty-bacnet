//! Golden vectors for every B/IPv6 BVLL message Annex U.2 defines, written
//! out by hand from the format clauses (U.2.1.1-U.2.13.1), not produced by
//! the encoder. Each one must decode to the expected message and encode back
//! to the same octets.
//!
//! Annex U gives no example datagrams. The addresses are its own examples:
//! `[2001:DB8:1111:2222::CB0]:47808` from the notation clause (U.1.1.1.1)
//! and device A's global address in Figure U-1. The VMACs are device
//! instances 1234 (00 04 D2) and 5678 (00 16 2E) and a random-instance VMAC
//! (H.7.2), whose top bits are 01.

use core::net::Ipv6Addr;

use bacnet_bip_codec::ipv6::{
    decode, Bip6Address, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6, Bvll6Frame,
    BIP6_ADDRESS_LENGTH, VMAC_LENGTH,
};

/// Device instance 1234.
const DEVICE_1234: Bip6Vmac = Bip6Vmac::new([0x00, 0x04, 0xD2]);
/// Device instance 5678.
const DEVICE_5678: Bip6Vmac = Bip6Vmac::new([0x00, 0x16, 0x2E]);
/// A random-instance VMAC: 0x4A5B6C, in the 0x400000-0x7FFFFF range.
const RANDOM: Bip6Vmac = Bip6Vmac::new([0x4A, 0x5B, 0x6C]);

/// `[2001:DB8:1111:2222::CB0]:47808`.
const NOTATION_EXAMPLE: Bip6Address = Bip6Address::new(
    Ipv6Addr::new(0x2001, 0xDB8, 0x1111, 0x2222, 0, 0, 0, 0xCB0),
    47808,
);
/// The same address as wire octets: the IPv6 address, then the port 0xBAC0.
const NOTATION_EXAMPLE_OCTETS: [u8; 18] = [
    0x20, 0x01, 0x0D, 0xB8, 0x11, 0x11, 0x22, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0xB0,
    0xBA, 0xC0,
];

/// Device A of Figure U-1 at its global address, on port 47809 (0xBAC1), so
/// that the two port octets differ from the default's.
const DEVICE_A: Bip6Address = Bip6Address::new(
    Ipv6Addr::new(
        0x2001, 0xDB8, 0x1111, 0x2222, 0xD87C, 0x8A6C, 0xBC5A, 0xCA86,
    ),
    47809,
);
const DEVICE_A_OCTETS: [u8; 18] = [
    0x20, 0x01, 0x0D, 0xB8, 0x11, 0x11, 0x22, 0x22, 0xD8, 0x7C, 0x8A, 0x6C, 0xBC, 0x5A, 0xCA, 0x86,
    0xBA, 0xC1,
];

/// A global Who-Is NPDU: version 1, a destination (DNET 0xFFFF, no DADR),
/// hop count 255, then the unconfirmed Who-Is APDU.
const WHO_IS_NPDU: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// The header, then `fields` in order.
fn wire(header: [u8; 4], fields: &[&[u8]]) -> Vec<u8> {
    let mut wire = header.to_vec();
    for field in fields {
        wire.extend_from_slice(field);
    }
    wire
}

/// Decode `wire`, compare with `expected`, and encode `expected` back.
fn check(wire: &[u8], expected: Bvll6<'_>) {
    let decoded = decode(wire).unwrap_or_else(|error| panic!("{expected:?}: {error}"));
    assert_eq!(decoded, expected);
    let frame = Bvll6Frame::decode(wire).unwrap();
    assert_eq!(frame.function, expected.function());
    assert_eq!(frame.payload, &wire[4..]);
    // Every message's payload starts with a VMAC.
    assert_eq!(frame.payload[..3], expected.source_vmac().octets());

    let mut out = vec![0u8; wire.len()];
    assert_eq!(expected.encode_into(&mut out), Ok(wire.len()));
    assert_eq!(out, wire, "{expected:?}");
    assert_eq!(expected.encoded_len(), Ok(wire.len()));
}

#[test]
fn bvlc_result_codes() {
    // U.2.1.1: function 0x00, length 9, the source VMAC, the result code.
    let codes = [
        (Bvlc6ResultCode::SUCCESSFUL_COMPLETION, [0x00, 0x00]),
        (Bvlc6ResultCode::ADDRESS_RESOLUTION_NAK, [0x00, 0x30]),
        (
            Bvlc6ResultCode::VIRTUAL_ADDRESS_RESOLUTION_NAK,
            [0x00, 0x60],
        ),
        (Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK, [0x00, 0x90]),
        (
            Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
            [0x00, 0xA0],
        ),
        (
            Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
            [0x00, 0xC0],
        ),
    ];
    for (code, octets) in codes {
        check(
            &wire([0x82, 0x00, 0x00, 0x09], &[&[0x00, 0x04, 0xD2], &octets]),
            Bvll6::BvlcResult {
                source: DEVICE_1234,
                code,
            },
        );
    }
}

#[test]
fn original_unicast_npdu() {
    // U.2.2.1: function 0x01, length 4 + 3 + 3 + 8 = 18, source and
    // destination VMACs, then the NPDU.
    let expected = Bvll6::OriginalUnicastNpdu {
        source: DEVICE_1234,
        destination: DEVICE_5678,
        npdu: &WHO_IS_NPDU,
    };
    check(
        &wire(
            [0x82, 0x01, 0x00, 0x12],
            &[&[0x00, 0x04, 0xD2], &[0x00, 0x16, 0x2E], &WHO_IS_NPDU],
        ),
        expected,
    );
    assert_eq!(expected.destination_vmac(), Some(DEVICE_5678));
    assert_eq!(expected.npdu(), Some(&WHO_IS_NPDU[..]));
    assert_eq!(expected.origin(), None);

    // The NPDU may be empty: the codec leaves its content to the network
    // layer.
    check(
        &[0x82, 0x01, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x00, 0x16, 0x2E],
        Bvll6::OriginalUnicastNpdu {
            source: DEVICE_1234,
            destination: DEVICE_5678,
            npdu: &[],
        },
    );
}

#[test]
fn original_broadcast_npdu() {
    // U.2.3.1: function 0x02, length 4 + 3 + 8 = 15, the source VMAC, then
    // the NPDU.
    let expected = Bvll6::OriginalBroadcastNpdu {
        source: RANDOM,
        npdu: &WHO_IS_NPDU,
    };
    check(
        &wire(
            [0x82, 0x02, 0x00, 0x0F],
            &[&[0x4A, 0x5B, 0x6C], &WHO_IS_NPDU],
        ),
        expected,
    );
    assert_eq!(expected.destination_vmac(), None);
    check(
        &[0x82, 0x02, 0x00, 0x07, 0x4A, 0x5B, 0x6C],
        Bvll6::OriginalBroadcastNpdu {
            source: RANDOM,
            npdu: &[],
        },
    );
}

#[test]
fn address_resolution() {
    // U.2.4.1: function 0x03, length 10, the source VMAC, then the target.
    let expected = Bvll6::AddressResolution {
        source: DEVICE_1234,
        target: DEVICE_5678,
    };
    check(
        &[0x82, 0x03, 0x00, 0x0A, 0x00, 0x04, 0xD2, 0x00, 0x16, 0x2E],
        expected,
    );
    // The target is not a destination: the request goes to a group.
    assert_eq!(expected.destination_vmac(), None);
}

#[test]
fn forwarded_address_resolution() {
    // U.2.5.1: function 0x04, length 28, the original source VMAC, the
    // target VMAC, then the original source's 18-octet B/IPv6 address.
    let expected = Bvll6::ForwardedAddressResolution {
        original_source: DEVICE_1234,
        target: DEVICE_5678,
        original_address: NOTATION_EXAMPLE,
    };
    check(
        &wire(
            [0x82, 0x04, 0x00, 0x1C],
            &[
                &[0x00, 0x04, 0xD2],
                &[0x00, 0x16, 0x2E],
                &NOTATION_EXAMPLE_OCTETS,
            ],
        ),
        expected,
    );
    assert_eq!(expected.source_vmac(), DEVICE_1234);
    assert_eq!(expected.origin(), Some(NOTATION_EXAMPLE));
}

#[test]
fn address_resolution_ack() {
    // U.2.6.1: function 0x05, length 10, the source VMAC (the one that was
    // resolved), then the asker's VMAC.
    let expected = Bvll6::AddressResolutionAck {
        source: DEVICE_5678,
        destination: DEVICE_1234,
    };
    check(
        &[0x82, 0x05, 0x00, 0x0A, 0x00, 0x16, 0x2E, 0x00, 0x04, 0xD2],
        expected,
    );
    assert_eq!(expected.destination_vmac(), Some(DEVICE_1234));
}

#[test]
fn virtual_address_resolution() {
    // U.2.7.1: function 0x06, length 7, the source VMAC alone.
    check(
        &[0x82, 0x06, 0x00, 0x07, 0x00, 0x04, 0xD2],
        Bvll6::VirtualAddressResolution {
            source: DEVICE_1234,
        },
    );
}

#[test]
fn virtual_address_resolution_ack() {
    // U.2.8.1: function 0x07, length 10, the answering VMAC, then the
    // asker's.
    let expected = Bvll6::VirtualAddressResolutionAck {
        source: RANDOM,
        destination: DEVICE_1234,
    };
    check(
        &[0x82, 0x07, 0x00, 0x0A, 0x4A, 0x5B, 0x6C, 0x00, 0x04, 0xD2],
        expected,
    );
    assert_eq!(expected.destination_vmac(), Some(DEVICE_1234));
}

#[test]
fn forwarded_npdu() {
    // U.2.9.1: function 0x08, length 4 + 3 + 18 + 8 = 33, the originator's
    // VMAC and B/IPv6 address, then its NPDU.
    let expected = Bvll6::ForwardedNpdu {
        original_source: DEVICE_1234,
        original_address: DEVICE_A,
        npdu: &WHO_IS_NPDU,
    };
    check(
        &wire(
            [0x82, 0x08, 0x00, 0x21],
            &[&[0x00, 0x04, 0xD2], &DEVICE_A_OCTETS, &WHO_IS_NPDU],
        ),
        expected,
    );
    assert_eq!(expected.source_vmac(), DEVICE_1234);
    assert_eq!(expected.origin(), Some(DEVICE_A));
    assert_eq!(expected.npdu(), Some(&WHO_IS_NPDU[..]));

    // An empty NPDU decodes; the transport decides what to do with it.
    check(
        &wire(
            [0x82, 0x08, 0x00, 0x19],
            &[&[0x00, 0x04, 0xD2], &DEVICE_A_OCTETS],
        ),
        Bvll6::ForwardedNpdu {
            original_source: DEVICE_1234,
            original_address: DEVICE_A,
            npdu: &[],
        },
    );
}

#[test]
fn register_foreign_device() {
    // U.2.10.1: function 0x09, length 9, the source VMAC, then the
    // time-to-live in seconds, high octet first.
    check(
        &[0x82, 0x09, 0x00, 0x09, 0x00, 0x04, 0xD2, 0x01, 0x2C],
        Bvll6::RegisterForeignDevice {
            source: DEVICE_1234,
            ttl: 300,
        },
    );
    check(
        &[0x82, 0x09, 0x00, 0x09, 0x00, 0x04, 0xD2, 0x00, 0x00],
        Bvll6::RegisterForeignDevice {
            source: DEVICE_1234,
            ttl: 0,
        },
    );
}

#[test]
fn delete_foreign_device_table_entry() {
    // U.2.11.1: function 0x0A, length 25, the source VMAC, then the entry's
    // B/IPv6 address.
    check(
        &wire(
            [0x82, 0x0A, 0x00, 0x19],
            &[&[0x00, 0x04, 0xD2], &DEVICE_A_OCTETS],
        ),
        Bvll6::DeleteForeignDeviceTableEntry {
            source: DEVICE_1234,
            entry: DEVICE_A,
        },
    );
}

#[test]
fn distribute_broadcast_to_network() {
    // U.2.13.1: function 0x0C, length 4 + 3 + 8 = 15, the foreign device's
    // VMAC, then the NPDU.
    let expected = Bvll6::DistributeBroadcastToNetwork {
        original_source: RANDOM,
        npdu: &WHO_IS_NPDU,
    };
    check(
        &wire(
            [0x82, 0x0C, 0x00, 0x0F],
            &[&[0x4A, 0x5B, 0x6C], &WHO_IS_NPDU],
        ),
        expected,
    );
    assert_eq!(expected.source_vmac(), RANDOM);
    assert_eq!(expected.npdu(), Some(&WHO_IS_NPDU[..]));
}

/// U.1.1.2: the IPv6 address, then the port, both high octet first.
#[test]
fn addresses_and_vmacs_keep_wire_order() {
    assert_eq!(NOTATION_EXAMPLE.to_octets(), NOTATION_EXAMPLE_OCTETS);
    assert_eq!(Bip6Address::from_octets(DEVICE_A_OCTETS), DEVICE_A);
    assert_eq!(Bip6Address::from_slice(&DEVICE_A_OCTETS[..]), Ok(DEVICE_A));
    assert_eq!(
        NOTATION_EXAMPLE.to_string(),
        "[2001:db8:1111:2222::cb0]:47808"
    );
    assert_eq!(DEVICE_1234.octets(), [0x00, 0x04, 0xD2]);
    assert_eq!(<[u8; 3]>::from(RANDOM), [0x4A, 0x5B, 0x6C]);
    assert_eq!((VMAC_LENGTH, BIP6_ADDRESS_LENGTH), (3, 18));

    // A socket address keeps the IPv6 address and port; its flow label and
    // scope ID don't travel.
    let scoped = core::net::SocketAddrV6::new(DEVICE_A.ip, DEVICE_A.port, 7, 4);
    assert_eq!(Bip6Address::from(scoped), DEVICE_A);
    assert_eq!(
        core::net::SocketAddrV6::from(DEVICE_A),
        core::net::SocketAddrV6::new(DEVICE_A.ip, DEVICE_A.port, 0, 0)
    );
}

#[test]
fn function_codes_follow_annex_u() {
    let expected = [
        ("BVLC_RESULT", 0x00),
        ("ORIGINAL_UNICAST_NPDU", 0x01),
        ("ORIGINAL_BROADCAST_NPDU", 0x02),
        ("ADDRESS_RESOLUTION", 0x03),
        ("FORWARDED_ADDRESS_RESOLUTION", 0x04),
        ("ADDRESS_RESOLUTION_ACK", 0x05),
        ("VIRTUAL_ADDRESS_RESOLUTION", 0x06),
        ("VIRTUAL_ADDRESS_RESOLUTION_ACK", 0x07),
        ("FORWARDED_NPDU", 0x08),
        ("REGISTER_FOREIGN_DEVICE", 0x09),
        ("DELETE_FOREIGN_DEVICE_TABLE_ENTRY", 0x0A),
        ("DISTRIBUTE_BROADCAST_TO_NETWORK", 0x0C),
    ];
    let named: Vec<_> = Bvlc6Function::ALL_NAMED
        .iter()
        .map(|(name, function)| (*name, function.to_raw()))
        .collect();
    assert_eq!(named, expected);
    for (name, raw) in expected {
        let function = Bvlc6Function::from_raw(raw);
        assert_eq!(function.name(), Some(name));
        // The Debug and Display output bacnet-types gave these codes.
        assert_eq!(function.to_string(), name);
        assert_eq!(format!("{function:?}"), format!("Bvlc6Function::{name}"));
        assert!(function.payload_shape().is_some());
    }
    // U.2.12, which once defined 0x0B, is deleted in this edition.
    for raw in (0x0B..=0xFF).filter(|raw| *raw != 0x0C) {
        let function = Bvlc6Function::from_raw(raw);
        assert!(!function.is_defined(), "{raw:#04X}");
        assert_eq!(function.payload_shape(), None);
        assert_eq!(function.to_string(), raw.to_string());
        assert_eq!(format!("{function:?}"), format!("Bvlc6Function({raw})"));
    }
}

#[test]
fn result_codes_follow_annex_u() {
    let expected = [
        ("SUCCESSFUL_COMPLETION", 0x0000),
        ("ADDRESS_RESOLUTION_NAK", 0x0030),
        ("VIRTUAL_ADDRESS_RESOLUTION_NAK", 0x0060),
        ("REGISTER_FOREIGN_DEVICE_NAK", 0x0090),
        ("DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK", 0x00A0),
        ("DISTRIBUTE_BROADCAST_TO_NETWORK_NAK", 0x00C0),
    ];
    let named: Vec<_> = Bvlc6ResultCode::ALL_NAMED
        .iter()
        .map(|(name, code)| (*name, code.to_raw()))
        .collect();
    assert_eq!(named, expected);
    for (name, raw) in expected {
        let code = Bvlc6ResultCode::from_raw(raw);
        assert_eq!(code.to_string(), name);
        assert_eq!(format!("{code:?}"), format!("Bvlc6ResultCode::{name}"));
    }
    // The B/IP codes in between are not B/IPv6 codes.
    for raw in [0x0010, 0x0020, 0x0040, 0x0050, 0x0001] {
        assert!(!Bvlc6ResultCode::from_raw(raw).is_defined(), "{raw:#06X}");
    }
    assert_eq!(
        format!("{:?}", Bvlc6ResultCode::from_raw(0x0010)),
        "Bvlc6ResultCode(16)"
    );
    assert_eq!(Bvlc6ResultCode::from_raw(0x0010).to_string(), "16");
}

/// U.2.1.1 names a NAK after five requests; nothing else has one.
#[test]
fn naks_pair_with_their_requests() {
    let pairs = [
        (
            Bvlc6Function::ADDRESS_RESOLUTION,
            Bvlc6ResultCode::ADDRESS_RESOLUTION_NAK,
        ),
        (
            Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
            Bvlc6ResultCode::VIRTUAL_ADDRESS_RESOLUTION_NAK,
        ),
        (
            Bvlc6Function::REGISTER_FOREIGN_DEVICE,
            Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK,
        ),
        (
            Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
        ),
        (
            Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK,
            Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
        ),
    ];
    for raw in 0..=0xFF {
        let function = Bvlc6Function::from_raw(raw);
        let expected = pairs
            .iter()
            .find(|(request, _)| *request == function)
            .map(|(_, nak)| *nak);
        assert_eq!(function.nak(), expected, "{function:?}");
    }
}
