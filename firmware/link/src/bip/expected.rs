//! What the B/IP scenario reports in a good run, its B/IPv6 part included.
//! `tests/bip.rs` compares the native run's summary with it, and the
//! emulation build of the `bip` image the emulated CPU's, so the two can't
//! drift apart.

use core::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4};

use bacnet_bip_codec::ipv6::{
    Bip6Address, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6, Bvll6Error,
};
use bacnet_bip_codec::{
    BdtEntry, BvlcFunction, BvlcResultCode, Bvll, BvllError, FdtEntries, FdtEntry, PayloadShape,
};

use super::{ipv6, Summary};

const fn address(octets: [u8; 4], port: u16) -> SocketAddrV4 {
    let [a, b, c, d] = octets;
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

/// The FNV-1a hash of every octet the scenario encodes. It was worked out
/// apart from the codec, from the twelve datagrams written out by hand from
/// the Annex J.2 layouts (156 octets).
const ENCODED_FNV: u32 = 0xCDF7_F4BE;

/// The same hash over the B/IPv6 part's encodings, worked out the same way
/// from its twelve datagrams written out by hand from the Annex U.2 layouts
/// (189 octets).
const ENCODED_FNV6: u32 = 0xFBBD_B05C;

const VMAC_1234: Bip6Vmac = Bip6Vmac::new([0x00, 0x04, 0xD2]);
const VMAC_5678: Bip6Vmac = Bip6Vmac::new([0x00, 0x16, 0x2E]);

/// What the B/IPv6 part reports.
const IPV6: ipv6::Summary = ipv6::Summary {
    // One message of each of the twelve variants, through a slice and
    // through the sink, each decoding back to itself.
    encoded: 12,
    // Virtual-Address-Resolution 7; the result and the registration 9; the
    // three two-VMAC messages 10; the two NPDU carriers with one VMAC 15 and
    // the unicast 18; the deletion 25, Forwarded-Address-Resolution 28 and
    // Forwarded-NPDU 33.
    encoded_octets: 7 + 9 * 2 + 10 * 3 + 15 * 2 + 18 + 25 + 28 + 33,
    encoded_fnv: ENCODED_FNV6,
    sink_matches: 12,
    round_trips: 12,
    // Function codes 0x00 through 0x0A, and 0x0C.
    functions: 0x17FF,
    // Original-Unicast-NPDU and the two ACKs.
    destinations: 3,
    // The hand-written datagrams decode to the field values their comments
    // give.
    goldens: [
        Some(Bvll6::RegisterForeignDevice {
            source: VMAC_1234,
            ttl: 300,
        }),
        Some(Bvll6::ForwardedNpdu {
            original_source: VMAC_1234,
            original_address: Bip6Address::new(
                Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 0x14),
                47809,
            ),
            npdu: &[0x01, 0x00],
        }),
        Some(Bvll6::AddressResolutionAck {
            source: VMAC_5678,
            destination: VMAC_1234,
        }),
    ],
    origin: Some(Bip6Address::new(
        Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 0x77),
        47808,
    )),
    forwarded_npdu: Some(8),
    shapes: [
        Some(PayloadShape::Fixed(3)),
        Some(PayloadShape::Npdu { prefix: 21 }),
        Some(PayloadShape::Fixed(21)),
        None,
    ],
    naks: [
        Some(Bvlc6ResultCode::ADDRESS_RESOLUTION_NAK),
        Some(Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK),
        None,
    ],
    rejections: [
        Some(Bvll6Error::LengthMismatch {
            declared: 9,
            received: 10,
        }),
        Some(Bvll6Error::NotBip6 { bvlc_type: 0x81 }),
        Some(Bvll6Error::PayloadLength {
            function: Bvlc6Function::REGISTER_FOREIGN_DEVICE,
            length: 4,
        }),
        Some(Bvll6Error::UnknownFunction { function: 0x0B }),
        Some(Bvll6Error::UndefinedResultCode { code: 0x0010 }),
        Some(Bvll6Error::AddressLength { length: 6 }),
        // The 33-octet Forwarded-NPDU into 10 octets of slice, and into the
        // sink with 14 of its 64 octets free.
        Some(Bvll6Error::BufferExhausted {
            needed: 33,
            available: 10,
        }),
        Some(Bvll6Error::BufferExhausted {
            needed: 33,
            available: 14,
        }),
    ],
    errors: 0,
};

/// The one entry of the golden FDT acknowledgement.
const GOLDEN_FDT: [FdtEntry; 1] = [FdtEntry::new(address([10, 0, 0, 5], 47808), 60, 45)];

/// The summary [`run`](super::run) returns in a good run.
pub const EXPECTED: Summary = Summary {
    // One message of each of the twelve variants, through a slice and
    // through the sink, each decoding back to itself.
    encoded: 12,
    // Header-only reads 4 octets each, the result and the registration 6,
    // the deletion 10, the three tables 24, the Forwarded-NPDU 18 and the
    // three other NPDU carriers 12.
    encoded_octets: 4 * 2 + 6 * 2 + 10 + 24 * 3 + 18 + 12 * 3,
    // Locks every octet the encoder wrote, in order. The golden datagrams
    // below check the layout itself.
    encoded_fnv: ENCODED_FNV,
    sink_matches: 12,
    round_trips: 12,
    // Function codes 0x00 through 0x0B.
    functions: 0x0FFF,
    // The hand-written datagrams decode to the field values their comments
    // give.
    goldens: [
        Some(Bvll::RegisterForeignDevice { ttl: 300 }),
        Some(Bvll::ForwardedNpdu {
            origin: address([192, 168, 1, 20], 47809),
            npdu: &[0x01, 0x00],
        }),
        Some(Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(
            &GOLDEN_FDT,
        ))),
    ],
    bdt_ends: [
        Some(BdtEntry::new(
            address([192, 168, 1, 10], 47808),
            Ipv4Addr::new(255, 255, 255, 255),
        )),
        Some(BdtEntry::new(
            address([10, 0, 0, 1], 47808),
            Ipv4Addr::new(255, 255, 255, 0),
        )),
    ],
    fdt_ends: [
        Some(FdtEntry::new(address([192, 168, 2, 20], 47809), 300, 312)),
        Some(FdtEntry::new(address([172, 16, 0, 5], 47808), 60, 4)),
    ],
    origin: Some(address([10, 0, 0, 77], 47808)),
    forwarded_npdu: Some(8),
    shapes: [
        Some(PayloadShape::Fixed(2)),
        Some(PayloadShape::Table { entry_length: 10 }),
        Some(PayloadShape::Npdu { prefix: 6 }),
        None,
    ],
    naks: [
        Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK),
        Some(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK),
        None,
    ],
    rejections: [
        Some(BvllError::LengthMismatch {
            declared: 6,
            received: 7,
        }),
        Some(BvllError::PayloadLength {
            function: BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
            length: 9,
        }),
        Some(BvllError::TableLength {
            length: 9,
            entry_length: 10,
        }),
        Some(BvllError::UndefinedResultCode { code: 0x0070 }),
        Some(BvllError::UnknownFunction { function: 0x0C }),
        // The 24-octet BDT acknowledgement into 10 octets of slice, and into
        // the sink with 14 of its 64 octets free.
        Some(BvllError::BufferExhausted {
            needed: 24,
            available: 10,
        }),
        Some(BvllError::BufferExhausted {
            needed: 24,
            available: 14,
        }),
    ],
    errors: 0,
    ipv6: IPV6,
};
