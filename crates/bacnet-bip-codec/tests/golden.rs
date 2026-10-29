//! Golden vectors for every BVLL message Annex J.2 defines, written out by
//! hand from the format clauses (J.2.1.1-J.2.12.1), not produced by the
//! encoder. Each one must decode to the expected message and encode back to
//! the same octets.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{
    BdtEntries, BdtEntry, BvlcFunction, BvlcResultCode, Bvll, BvllFrame, FdtEntries, FdtEntry,
    decode,
};

const ALL_ONES: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 255);

fn address(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

/// The two-row BDT that BR1 holds for B/IP network 1 in the NAT example of
/// Annex J.7.5 (Figure J-7): both rows are global NAT addresses on port
/// 47809 with all-ones (two-hop) masks.
fn nat_example_bdt() -> [BdtEntry; 2] {
    [
        BdtEntry::new(address(201, 1, 1, 1, 47809), ALL_ONES),
        BdtEntry::new(address(237, 2, 2, 2, 47809), ALL_ONES),
    ]
}

/// The same table as wire octets: per entry, the IPv4 address, the port
/// (0xBAC1) and the mask, most significant octet first.
const NAT_EXAMPLE_BDT_OCTETS: [u8; 20] = [
    201, 1, 1, 1, 0xBA, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF, //
    237, 2, 2, 2, 0xBA, 0xC1, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// A global Who-Is NPDU: version 1, a destination (DNET 0xFFFF, no DADR),
/// hop count 255, then the unconfirmed Who-Is APDU.
const WHO_IS_NPDU: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// Decode `wire`, compare with `expected`, and encode `expected` back.
fn check(wire: &[u8], expected: Bvll<'_>) {
    let decoded = decode(wire).unwrap_or_else(|error| panic!("{expected:?}: {error}"));
    assert_eq!(decoded, expected);
    let frame = BvllFrame::decode(wire).unwrap();
    assert_eq!(frame.function, expected.function());
    assert_eq!(frame.payload, &wire[4..]);

    let mut out = vec![0u8; wire.len()];
    assert_eq!(expected.encode_into(&mut out), Ok(wire.len()));
    assert_eq!(out, wire, "{expected:?}");
    assert_eq!(expected.encoded_len(), Ok(wire.len()));
}

#[test]
fn bvlc_result_codes() {
    // J.2.1.1: type, function 0x00, length 6, then the 2-octet result code.
    let codes = [
        (BvlcResultCode::SUCCESSFUL_COMPLETION, [0x00, 0x00]),
        (
            BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
            [0x00, 0x10],
        ),
        (
            BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK,
            [0x00, 0x20],
        ),
        (BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK, [0x00, 0x30]),
        (BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK, [0x00, 0x40]),
        (
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
            [0x00, 0x50],
        ),
        (
            BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
            [0x00, 0x60],
        ),
    ];
    for (code, [high, low]) in codes {
        check(&[0x81, 0x00, 0x00, 0x06, high, low], Bvll::BvlcResult(code));
    }
}

#[test]
fn write_broadcast_distribution_table() {
    // J.2.2.1: function 0x01, length 4 + 2 * 10 = 24.
    let mut wire = vec![0x81, 0x01, 0x00, 0x18];
    wire.extend_from_slice(&NAT_EXAMPLE_BDT_OCTETS);
    let rows = nat_example_bdt();
    check(
        &wire,
        Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&rows)),
    );
}

#[test]
fn read_broadcast_distribution_table() {
    // J.2.3.1: the header alone.
    check(
        &[0x81, 0x02, 0x00, 0x04],
        Bvll::ReadBroadcastDistributionTable,
    );
}

#[test]
fn read_broadcast_distribution_table_ack() {
    // J.2.4.1: function 0x03 and the table; J.2.4 says an empty table is a
    // zero-length list.
    let mut wire = vec![0x81, 0x03, 0x00, 0x18];
    wire.extend_from_slice(&NAT_EXAMPLE_BDT_OCTETS);
    let rows = nat_example_bdt();
    check(
        &wire,
        Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&rows)),
    );
    check(
        &[0x81, 0x03, 0x00, 0x04],
        Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&[])),
    );

    let Ok(Bvll::ReadBroadcastDistributionTableAck(table)) = decode(&wire) else {
        panic!("not a Read-BDT-Ack");
    };
    assert_eq!(table.len(), 2);
    assert_eq!(table.iter().collect::<Vec<_>>(), rows);
    assert_eq!(table.iter().next_back(), Some(rows[1]));
}

/// Wire and caller-built tables iterate alike from both ends, and know how
/// many entries are left at every step.
#[test]
fn table_iterators_count_down_from_both_ends() {
    let rows = [
        FdtEntry::new(address(10, 0, 0, 1, 47808), 60, 90),
        FdtEntry::new(address(10, 0, 0, 2, 47809), 120, 150),
        FdtEntry::new(address(10, 0, 0, 3, 47810), 300, 330),
    ];
    let octets: Vec<u8> = rows.iter().flat_map(|row| row.to_octets()).collect();
    let tables = [
        FdtEntries::from_wire(&octets).unwrap(),
        FdtEntries::from_entries(&rows),
    ];
    for table in tables {
        let mut entries = table.iter();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.size_hint(), (3, Some(3)));
        assert_eq!(entries.next(), Some(rows[0]));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.next_back(), Some(rows[2]));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.next_back(), Some(rows[1]));
        assert_eq!(entries.len(), 0);
        assert_eq!(entries.next(), None);
        assert_eq!(entries.next_back(), None);
        assert_eq!(
            table.iter().rev().collect::<Vec<_>>(),
            [rows[2], rows[1], rows[0]]
        );
    }
    assert_eq!(tables[0], tables[1]);
    assert_eq!(format!("{:?}", tables[0]), format!("{:?}", rows));
}

#[test]
fn forwarded_npdu() {
    // J.2.5.1: function 0x04, length 4 + 6 + 8 = 18, the originator's B/IP
    // address (192.168.1.10:47808), then its NPDU.
    let wire = [
        0x81, 0x04, 0x00, 0x12, 192, 168, 1, 10, 0xBA, 0xC0, 0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF,
        0x10, 0x08,
    ];
    let expected = Bvll::ForwardedNpdu {
        origin: address(192, 168, 1, 10, 47808),
        npdu: &WHO_IS_NPDU,
    };
    check(&wire, expected);
    assert_eq!(expected.origin(), Some(address(192, 168, 1, 10, 47808)));
    assert_eq!(expected.npdu(), Some(&WHO_IS_NPDU[..]));

    // The NPDU may be empty: the codec leaves its content to the network
    // layer.
    check(
        &[0x81, 0x04, 0x00, 0x0A, 10, 0, 0, 1, 0xBA, 0xC0],
        Bvll::ForwardedNpdu {
            origin: address(10, 0, 0, 1, 47808),
            npdu: &[],
        },
    );
}

#[test]
fn register_foreign_device() {
    // J.2.6.1: function 0x05, length 6, the time-to-live in seconds.
    check(
        &[0x81, 0x05, 0x00, 0x06, 0x00, 0x3C],
        Bvll::RegisterForeignDevice { ttl: 60 },
    );
    check(
        &[0x81, 0x05, 0x00, 0x06, 0xFF, 0xFF],
        Bvll::RegisterForeignDevice { ttl: u16::MAX },
    );
}

#[test]
fn read_foreign_device_table() {
    // J.2.7.1: the header alone.
    check(&[0x81, 0x06, 0x00, 0x04], Bvll::ReadForeignDeviceTable);
}

#[test]
fn read_foreign_device_table_ack() {
    // J.2.8.1: per entry, the registrant's address (192.168.1.10:47809), its
    // time-to-live (60) and the seconds left (90).
    let wire = [
        0x81, 0x07, 0x00, 0x0E, 192, 168, 1, 10, 0xBA, 0xC1, 0x00, 0x3C, 0x00, 0x5A,
    ];
    let rows = [FdtEntry::new(address(192, 168, 1, 10, 47809), 60, 90)];
    check(
        &wire,
        Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(&rows)),
    );
    check(
        &[0x81, 0x07, 0x00, 0x04],
        Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(&[])),
    );
}

#[test]
fn delete_foreign_device_table_entry() {
    // J.2.9.1: function 0x08, length 10, the entry's B/IP address.
    check(
        &[0x81, 0x08, 0x00, 0x0A, 192, 168, 1, 10, 0xBA, 0xC1],
        Bvll::DeleteForeignDeviceTableEntry(address(192, 168, 1, 10, 47809)),
    );
}

#[test]
fn distribute_broadcast_to_network() {
    // J.2.10.1: function 0x09, then the NPDU.
    let mut wire = vec![0x81, 0x09, 0x00, 0x0C];
    wire.extend_from_slice(&WHO_IS_NPDU);
    check(&wire, Bvll::DistributeBroadcastToNetwork(&WHO_IS_NPDU));
}

#[test]
fn original_unicast_npdu() {
    // J.2.11.1: function 0x0A, then the NPDU (here a local Who-Is).
    check(
        &[0x81, 0x0A, 0x00, 0x08, 0x01, 0x00, 0x10, 0x08],
        Bvll::OriginalUnicastNpdu(&[0x01, 0x00, 0x10, 0x08]),
    );
}

#[test]
fn original_broadcast_npdu() {
    // J.2.12.1: function 0x0B, then the NPDU.
    let mut wire = vec![0x81, 0x0B, 0x00, 0x0C];
    wire.extend_from_slice(&WHO_IS_NPDU);
    check(&wire, Bvll::OriginalBroadcastNpdu(&WHO_IS_NPDU));
    check(&[0x81, 0x0B, 0x00, 0x04], Bvll::OriginalBroadcastNpdu(&[]));
}

#[test]
fn function_codes_follow_annex_j() {
    let expected = [
        ("BVLC_RESULT", 0x00),
        ("WRITE_BROADCAST_DISTRIBUTION_TABLE", 0x01),
        ("READ_BROADCAST_DISTRIBUTION_TABLE", 0x02),
        ("READ_BROADCAST_DISTRIBUTION_TABLE_ACK", 0x03),
        ("FORWARDED_NPDU", 0x04),
        ("REGISTER_FOREIGN_DEVICE", 0x05),
        ("READ_FOREIGN_DEVICE_TABLE", 0x06),
        ("READ_FOREIGN_DEVICE_TABLE_ACK", 0x07),
        ("DELETE_FOREIGN_DEVICE_TABLE_ENTRY", 0x08),
        ("DISTRIBUTE_BROADCAST_TO_NETWORK", 0x09),
        ("ORIGINAL_UNICAST_NPDU", 0x0A),
        ("ORIGINAL_BROADCAST_NPDU", 0x0B),
    ];
    let named: Vec<_> = BvlcFunction::ALL_NAMED
        .iter()
        .map(|(name, function)| (*name, function.to_raw()))
        .collect();
    assert_eq!(named, expected);
    for (name, raw) in expected {
        let function = BvlcFunction::from_raw(raw);
        assert_eq!(function.name(), Some(name));
        assert_eq!(function.to_string(), name);
        assert_eq!(format!("{function:?}"), format!("BvlcFunction::{name}"));
        assert!(function.payload_shape().is_some());
    }
    // J.2.13, which once defined 0x0C, is deleted in this edition.
    for raw in 0x0C..=0xFF {
        let function = BvlcFunction::from_raw(raw);
        assert!(!function.is_defined(), "{raw:#04X}");
        assert_eq!(function.payload_shape(), None);
        assert_eq!(function.to_string(), raw.to_string());
        assert_eq!(format!("{function:?}"), format!("BvlcFunction({raw})"));
    }
}

#[test]
fn result_codes_follow_annex_j() {
    let named: Vec<_> = BvlcResultCode::ALL_NAMED
        .iter()
        .map(|(_, code)| code.to_raw())
        .collect();
    assert_eq!(named, [0x00, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
    assert!(!BvlcResultCode::from_raw(0x0070).is_defined());
    assert!(!BvlcResultCode::from_raw(0x0001).is_defined());
    assert_eq!(
        format!("{:?}", BvlcResultCode::from_raw(0x0070)),
        "BvlcResultCode(112)"
    );
}

/// J.2.1 lists the requests only a BBMD carries out; each has its own NAK.
#[test]
fn naks_pair_with_bbmd_only_requests() {
    let pairs = [
        (
            BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
            BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK,
        ),
        (
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK,
        ),
        (
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
        ),
        (
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
            BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
        ),
        (
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
        ),
        (
            BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
            BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
        ),
    ];
    for raw in 0..=0xFF {
        let function = BvlcFunction::from_raw(raw);
        let expected = pairs
            .iter()
            .find(|(request, _)| *request == function)
            .map(|(_, nak)| *nak);
        assert_eq!(function.nak(), expected, "{function:?}");
    }
}
