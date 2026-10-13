//! Encode an arbitrary B/IP message (Annex J) through every sink, then
//! decode it.
//!
//! Oracle: each message has the octets its Annex J.2 layout gives, built
//! here field by field: type 0x81, the function, a Length field counting the
//! whole message, then the payload. Encoding refuses a BVLC-Result whose
//! code J.2.1.1 doesn't define, then a message longer than the Length field
//! can state; inputs reach that limit from both sides. Every sink with room
//! holds exactly the expected octets, a fixed sink without room refuses with
//! `BufferExhausted`, and a refused encode leaves every sink as it was. Each
//! successful encode decodes back to an equal message, and a table built
//! from its wire octets equals the same table built from entries.

#![no_main]

use std::net::{Ipv4Addr, SocketAddrV4};

use arbitrary::Arbitrary;
use bacnet_bip_codec::{
    decode, BdtEntries, BdtEntry, BvlcResultCode, Bvll, BvllError, BvllFrame, BvllSink, FdtEntries,
    FdtEntry,
};
use bip_codec_fuzz::{bvll_octets, check_sinks, Npdu, SinkKnobs, HEADER, MAX_LENGTH, TYPE_BIP};
use libfuzzer_sys::fuzz_target;

/// The result codes Annex J.2.1.1 defines.
const RESULT_CODES: [u16; 7] = [0x0000, 0x0010, 0x0020, 0x0030, 0x0040, 0x0050, 0x0060];

#[derive(Debug, Arbitrary)]
enum Spec {
    Result(u16),
    WriteBdt(Table),
    ReadBdt,
    ReadBdtAck(Table),
    Forwarded([u8; 6], Npdu),
    Register(u16),
    ReadFdt,
    ReadFdtAck(Table),
    Delete([u8; 6]),
    Distribute(Npdu),
    Unicast(Npdu),
    Broadcast(Npdu),
}

/// A BDT or FDT, as 10-octet rows. A long one has 6,550 to 6,557 rows
/// repeating `rows`, which takes the message across the Length field's
/// limit (6,553 rows still fit). With `from_wire`, the message borrows the
/// rows' octets rather than entries.
#[derive(Debug, Arbitrary)]
struct Table {
    rows: Vec<[u8; 10]>,
    /// Under 8, a long table with this many rows past 6,550; most inputs
    /// leave long tables out, as with NPDUs.
    long: Option<u8>,
    from_wire: bool,
}

impl Table {
    fn rows(&self) -> Vec<[u8; 10]> {
        match self.long {
            Some(extra) if extra < 8 => {
                let rows = if self.rows.is_empty() {
                    &[[0; 10]][..]
                } else {
                    &self.rows
                };
                let count = 6550 + usize::from(extra);
                rows.iter().copied().cycle().take(count).collect()
            }
            _ => self.rows.clone(),
        }
    }
}

#[derive(Debug, Arbitrary)]
struct Input {
    spec: Spec,
    sinks: SinkKnobs,
}

fn address(octets: [u8; 6]) -> SocketAddrV4 {
    let [a, b, c, d, high, low] = octets;
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), u16::from_be_bytes([high, low]))
}

fn bdt_entry(row: [u8; 10]) -> BdtEntry {
    let [a, b, c, d, high, low, m0, m1, m2, m3] = row;
    BdtEntry::new(
        address([a, b, c, d, high, low]),
        Ipv4Addr::new(m0, m1, m2, m3),
    )
}

fn fdt_entry(row: [u8; 10]) -> FdtEntry {
    let [a, b, c, d, high, low, t0, t1, r0, r1] = row;
    FdtEntry::new(
        address([a, b, c, d, high, low]),
        u16::from_be_bytes([t0, t1]),
        u16::from_be_bytes([r0, r1]),
    )
}

/// Everything a message borrows, built from the spec.
struct Owned {
    payload: Vec<u8>,
    npdu: Vec<u8>,
    bdt: Vec<BdtEntry>,
    fdt: Vec<FdtEntry>,
}

/// The function code and payload a spec gives, and what it borrows.
fn layout(spec: &Spec) -> (u8, Owned) {
    let mut owned = Owned {
        payload: Vec::new(),
        npdu: Vec::new(),
        bdt: Vec::new(),
        fdt: Vec::new(),
    };
    let function = match spec {
        Spec::Result(code) => {
            owned.payload = code.to_be_bytes().to_vec();
            0x00
        }
        Spec::WriteBdt(table) | Spec::ReadBdtAck(table) => {
            let rows = table.rows();
            owned.payload = rows.concat();
            owned.bdt = rows.into_iter().map(bdt_entry).collect();
            if matches!(spec, Spec::WriteBdt(_)) {
                0x01
            } else {
                0x03
            }
        }
        Spec::ReadBdt => 0x02,
        Spec::Forwarded(origin, npdu) => {
            owned.npdu = npdu.octets(6);
            owned.payload = [&origin[..], &owned.npdu].concat();
            0x04
        }
        Spec::Register(ttl) => {
            owned.payload = ttl.to_be_bytes().to_vec();
            0x05
        }
        Spec::ReadFdt => 0x06,
        Spec::ReadFdtAck(table) => {
            let rows = table.rows();
            owned.payload = rows.concat();
            owned.fdt = rows.into_iter().map(fdt_entry).collect();
            0x07
        }
        Spec::Delete(entry) => {
            owned.payload = entry.to_vec();
            0x08
        }
        Spec::Distribute(npdu) | Spec::Unicast(npdu) | Spec::Broadcast(npdu) => {
            owned.npdu = npdu.octets(0);
            owned.payload = owned.npdu.clone();
            match spec {
                Spec::Distribute(_) => 0x09,
                Spec::Unicast(_) => 0x0A,
                _ => 0x0B,
            }
        }
    };
    (function, owned)
}

/// The message a spec describes, borrowing from `owned`; tables come from
/// their wire octets when `from_wire` is set.
fn build<'a>(spec: &Spec, owned: &'a Owned) -> Bvll<'a> {
    let bdt = |table: &Table| {
        if table.from_wire {
            BdtEntries::from_wire(&owned.payload).expect("whole entries")
        } else {
            BdtEntries::from_entries(&owned.bdt)
        }
    };
    match spec {
        Spec::Result(code) => Bvll::BvlcResult(BvlcResultCode::from_raw(*code)),
        Spec::WriteBdt(table) => Bvll::WriteBroadcastDistributionTable(bdt(table)),
        Spec::ReadBdt => Bvll::ReadBroadcastDistributionTable,
        Spec::ReadBdtAck(table) => Bvll::ReadBroadcastDistributionTableAck(bdt(table)),
        Spec::Forwarded(origin, _) => Bvll::ForwardedNpdu {
            origin: address(*origin),
            npdu: &owned.npdu,
        },
        Spec::Register(ttl) => Bvll::RegisterForeignDevice { ttl: *ttl },
        Spec::ReadFdt => Bvll::ReadForeignDeviceTable,
        Spec::ReadFdtAck(table) => Bvll::ReadForeignDeviceTableAck(if table.from_wire {
            FdtEntries::from_wire(&owned.payload).expect("whole entries")
        } else {
            FdtEntries::from_entries(&owned.fdt)
        }),
        Spec::Delete(entry) => Bvll::DeleteForeignDeviceTableEntry(address(*entry)),
        Spec::Distribute(_) => Bvll::DistributeBroadcastToNetwork(&owned.npdu),
        Spec::Unicast(_) => Bvll::OriginalUnicastNpdu(&owned.npdu),
        Spec::Broadcast(_) => Bvll::OriginalBroadcastNpdu(&owned.npdu),
    }
}

fuzz_target!(|input: Input| {
    let (function, owned) = layout(&input.spec);
    let message = build(&input.spec, &owned);
    assert_eq!(message.function().to_raw(), function);
    let wire = bvll_octets(TYPE_BIP, function, &owned.payload);
    let expected = match input.spec {
        Spec::Result(code) if !RESULT_CODES.contains(&code) => {
            Err(BvllError::UndefinedResultCode { code })
        }
        _ if wire.len() > MAX_LENGTH => Err(BvllError::TooLong { length: wire.len() }),
        _ => Ok(wire.clone()),
    };
    assert_eq!(
        message.encoded_len(),
        expected.as_ref().map(Vec::len).map_err(|e| *e)
    );

    // A table read from wire octets equals the same table built from entries.
    if let Spec::WriteBdt(table) | Spec::ReadBdtAck(table) | Spec::ReadFdtAck(table) = &input.spec {
        let flipped = Table {
            rows: Vec::new(),
            long: None,
            from_wire: !table.from_wire,
        };
        let other = match &input.spec {
            Spec::WriteBdt(_) => Spec::WriteBdt(flipped),
            Spec::ReadBdtAck(_) => Spec::ReadBdtAck(flipped),
            _ => Spec::ReadFdtAck(flipped),
        };
        assert_eq!(build(&other, &owned), message);
    }

    let outputs = check_sinks(
        &expected,
        wire.len(),
        &input.sinks,
        |needed, available| BvllError::BufferExhausted { needed, available },
        |output| message.encode_into(output),
        |sink: &mut dyn BvllSink| message.encode_to_sink(sink),
    );
    for output in outputs {
        assert_eq!(decode(&output), Ok(message));
        let frame = BvllFrame::decode(&output).expect("an encoded message decodes");
        assert_eq!(frame.function.to_raw(), function);
        assert_eq!(frame.payload, &output[HEADER..]);
    }
});
