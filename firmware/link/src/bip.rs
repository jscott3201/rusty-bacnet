//! The B/IP image's scenario.
//!
//! [`run`] drives `bacnet-bip-codec`, with default features plus its
//! `heapless` sink, through every message it knows: the B/IPv4 messages of
//! Annex J here, then the B/IPv6 messages of Annex U in [`ipv6`], in the
//! same storage. It returns a [`Summary`] of what it saw.
//!
//! - **Messages:** encode one message of each [`Bvll`] variant into a slice
//!   and into a `heapless::Vec` sink, and decode each back.
//! - **Golden octets:** decode three datagrams written by hand from the
//!   Annex J.2 layouts, so a fault that encode and decode share (a field in
//!   the wrong byte order, say) can't cancel out.
//! - **Tables:** read the decoded BDT and FDT acknowledgements through their
//!   borrowed [`Entries`] views, from both ends.
//! - **Forwarded-NPDU:** take its originator's address and its NPDU.
//! - **Functions:** the payload shape of a few functions, and which NAK
//!   answers a few others.
//! - **Rejections:** a Length field that disagrees with the datagram, a table
//!   that isn't whole entries, an undefined result code on encode, an
//!   unknown function, and a message too long for a fixed slice and for a
//!   nearly full sink.
//!
//! The B/IPv6 part ([`ipv6::run`]) does the same for its twelve messages and
//! its own rejections.
//!
//! `tests/bip.rs` runs it natively and compares the whole summary with
//! [`EXPECTED`]; the `bip` image (`src/bin/bip.rs`) links it, and its
//! emulation build checks the emulated CPU's summary against the same.

mod expected;
pub mod ipv6;

pub use expected::EXPECTED;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{
    BdtEntries, BdtEntry, BvlcFunction, BvlcResultCode, Bvll, BvllError, Entries, FdtEntries,
    FdtEntry, PayloadShape, TableEntry, decode,
};

use crate::tally::{FNV_OFFSET, fnv1a};

/// Octets the scenario encodes into at once; its longest message, the
/// B/IPv6 Forwarded-NPDU, takes 33.
const WIRE_LENGTH: usize = 64;

/// The growable-sink path: a `heapless::Vec`, whose capacity the codec checks
/// before it writes.
pub type Sink = heapless::Vec<u8, WIRE_LENGTH>;

/// Storage the scenario works in besides its stack: an encode buffer and
/// the sink.
///
/// The image keeps it in a static, so the linker counts it in `.bss`.
pub struct Memory {
    /// Encoded messages on their way to the decoder.
    wire: [u8; WIRE_LENGTH],
    /// The same messages encoded through the sink.
    sink: Sink,
}

impl Memory {
    /// Empty storage.
    pub const fn new() -> Self {
        Self {
            wire: [0; WIRE_LENGTH],
            sink: Sink::new(),
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// A BBMD on this subnet, reached by two-hop distribution.
pub const LOCAL_BBMD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 10), 0xBAC0);
/// A BBMD on another subnet, reached by one-hop distribution.
pub const REMOTE_BBMD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 0xBAC0);
/// A registered foreign device, on a non-default port.
pub const FOREIGN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 2, 20), 47809);
/// A second foreign device, close to expiry.
pub const EXPIRING: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(172, 16, 0, 5), 0xBAC0);
/// Where the forwarded NPDU started.
pub const ORIGIN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 77), 0xBAC0);

/// A broadcast distribution table of two entries.
pub const BDT: [BdtEntry; 2] = [
    BdtEntry::new(LOCAL_BBMD, Ipv4Addr::new(255, 255, 255, 255)),
    BdtEntry::new(REMOTE_BBMD, Ipv4Addr::new(255, 255, 255, 0)),
];
/// A foreign device table of two entries.
pub const FDT: [FdtEntry; 2] = [
    FdtEntry::new(FOREIGN, 300, 312),
    FdtEntry::new(EXPIRING, 60, 4),
];
/// A global-broadcast Who-Is in an NPDU.
pub const NPDU: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// One message of each variant, in function-code order.
pub const MESSAGES: [Bvll<'static>; 12] = [
    Bvll::BvlcResult(BvlcResultCode::SUCCESSFUL_COMPLETION),
    Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&BDT)),
    Bvll::ReadBroadcastDistributionTable,
    Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&BDT)),
    Bvll::ForwardedNpdu {
        origin: ORIGIN,
        npdu: &NPDU,
    },
    Bvll::RegisterForeignDevice { ttl: 300 },
    Bvll::ReadForeignDeviceTable,
    Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(&FDT)),
    Bvll::DeleteForeignDeviceTableEntry(FOREIGN),
    Bvll::DistributeBroadcastToNetwork(&NPDU),
    Bvll::OriginalUnicastNpdu(&NPDU),
    Bvll::OriginalBroadcastNpdu(&NPDU),
];

/// Datagrams written by hand from the format subclauses of Annex J.2,
/// rather than by the encoder: the 4-octet BVLL header (type 0x81, the
/// function, then the whole message's length, high octet first), then the
/// function's fields. Field values whose two octets differ catch a swapped
/// byte order.
pub const GOLDEN: [&[u8]; 3] = [
    // Register-Foreign-Device (J.2.6.1): a time-to-live of 300 s, 0x012C.
    &[0x81, 0x05, 0x00, 0x06, 0x01, 0x2C],
    // Forwarded-NPDU (J.2.5.1): the originator's address, 192.168.1.20 on
    // UDP port 47809 (0xBAC1), then the NPDU 01 00.
    &[
        0x81, 0x04, 0x00, 0x0C, 192, 168, 1, 20, 0xBA, 0xC1, 0x01, 0x00,
    ],
    // Read-Foreign-Device-Table-Ack (J.2.8.1) with one entry: 10.0.0.5 on
    // port 47808, registered for 60 s (0x003C) with 45 s left (0x002D).
    &[
        0x81, 0x07, 0x00, 0x0E, 10, 0, 0, 5, 0xBA, 0xC0, 0x00, 0x3C, 0x00, 0x2D,
    ],
];

/// Functions whose payload shape the scenario asks for; 0x0C is unknown.
pub const SHAPED: [BvlcFunction; 4] = [
    BvlcFunction::BVLC_RESULT,
    BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
    BvlcFunction::FORWARDED_NPDU,
    BvlcFunction::from_raw(0x0C),
];
/// Functions whose NAK the scenario asks for.
pub const NAKED: [BvlcFunction; 3] = [
    BvlcFunction::REGISTER_FOREIGN_DEVICE,
    BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
    BvlcFunction::ORIGINAL_UNICAST_NPDU,
];
/// The rejection paths the scenario takes.
pub const REJECTIONS: usize = 7;

/// What [`run`] saw. Every field is plain data, so the host test can compare
/// the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Messages encoded into a slice.
    pub encoded: u8,
    /// Octets those messages took, in all.
    pub encoded_octets: u16,
    /// 32-bit FNV-1a over those octets in order, so any change to what the
    /// encoder writes changes it.
    pub encoded_fnv: u32,
    /// Messages the sink took with the same octets as the slice.
    pub sink_matches: u8,
    /// Messages that decoded back equal to what was encoded.
    pub round_trips: u8,
    /// One bit per function code decoded: bit `n` for code `n`.
    pub functions: u16,
    /// What each datagram in [`GOLDEN`] decodes to.
    pub goldens: [Option<Bvll<'static>>; GOLDEN.len()],
    /// The decoded BDT acknowledgement's first and last entries, read from
    /// the front and from the back.
    pub bdt_ends: [Option<BdtEntry>; 2],
    /// The decoded FDT acknowledgement's first and last entries, likewise.
    pub fdt_ends: [Option<FdtEntry>; 2],
    /// The decoded Forwarded-NPDU's originator.
    pub origin: Option<SocketAddrV4>,
    /// The decoded Forwarded-NPDU's NPDU length.
    pub forwarded_npdu: Option<usize>,
    /// The payload shape of each function in [`SHAPED`].
    pub shapes: [Option<PayloadShape>; SHAPED.len()],
    /// The NAK of each function in [`NAKED`].
    pub naks: [Option<BvlcResultCode>; NAKED.len()],
    /// The error each rejection path produced: a trailing octet, a table
    /// payload of 9 octets (decoded, then read directly), the result code
    /// 0x0070 on encode, function 0x0C, and a 24-octet message encoded into
    /// a 10-octet slice and into a sink with 14 octets left.
    pub rejections: [Option<BvllError>; REJECTIONS],
    /// Calls that failed where the scenario expects success. Zero in a good
    /// run.
    pub errors: u8,
    /// What the B/IPv6 part saw.
    pub ipv6: ipv6::Summary,
}

impl Summary {
    const fn new() -> Self {
        Self {
            encoded: 0,
            encoded_octets: 0,
            encoded_fnv: FNV_OFFSET,
            sink_matches: 0,
            round_trips: 0,
            functions: 0,
            goldens: [None; GOLDEN.len()],
            bdt_ends: [None; 2],
            fdt_ends: [None; 2],
            origin: None,
            forwarded_npdu: None,
            shapes: [None; SHAPED.len()],
            naks: [None; NAKED.len()],
            rejections: [None; REJECTIONS],
            errors: 0,
            ipv6: ipv6::Summary::new(),
        }
    }

    /// Count a call that failed where the scenario expects success.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// Run the whole scenario in `memory`.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on.
pub fn run(memory: &mut Memory) -> Summary {
    let Memory { wire, sink } = memory;
    let mut summary = Summary::new();
    for message in &MESSAGES {
        round_trip(message, wire, sink, &mut summary);
    }
    for (golden, datagram) in summary.goldens.iter_mut().zip(GOLDEN) {
        *golden = decode(datagram).ok();
    }
    for (shape, function) in summary.shapes.iter_mut().zip(SHAPED) {
        *shape = function.payload_shape();
    }
    for (nak, function) in summary.naks.iter_mut().zip(NAKED) {
        *nak = function.nak();
    }
    summary.rejections = rejections(wire, sink);
    summary.ipv6 = ipv6::run(wire, sink);
    summary
}

/// Encode `message` into `wire` and into `sink`, decode it from `wire`, and
/// read what the decoded message carries.
fn round_trip(message: &Bvll<'_>, wire: &mut [u8], sink: &mut Sink, summary: &mut Summary) {
    let Ok(length) = message.encode_into(wire) else {
        summary.error();
        return;
    };
    let Some(octets) = wire.get(..length) else {
        summary.error();
        return;
    };
    summary.encoded += 1;
    summary.encoded_octets = summary
        .encoded_octets
        .saturating_add(u16::try_from(length).unwrap_or(u16::MAX));
    summary.encoded_fnv = fnv1a(summary.encoded_fnv, octets);
    sink.clear();
    match message.encode_to_sink(sink) {
        Ok(written) if written == length && sink.as_slice() == octets => {
            summary.sink_matches += 1;
        }
        _ => summary.error(),
    }
    let Ok(decoded) = decode(octets) else {
        summary.error();
        return;
    };
    if decoded == *message {
        summary.round_trips += 1;
    } else {
        summary.error();
    }
    summary.functions |= 1u16
        .checked_shl(u32::from(decoded.function().to_raw()))
        .unwrap_or(0);
    match decoded {
        Bvll::ReadBroadcastDistributionTableAck(table) => summary.bdt_ends = ends(table),
        Bvll::ReadForeignDeviceTableAck(table) => summary.fdt_ends = ends(table),
        Bvll::ForwardedNpdu { .. } => {
            summary.origin = decoded.origin();
            summary.forwarded_npdu = decoded.npdu().map(<[u8]>::len);
        }
        _ => {}
    }
}

/// A table's first entry, read from the front, and its last, from the back.
fn ends<T: TableEntry>(table: Entries<'_, T>) -> [Option<T>; 2] {
    let mut entries = table.iter();
    [entries.next(), entries.next_back()]
}

/// Take each rejection path and return the errors, `None` where a call
/// unexpectedly succeeded.
fn rejections(wire: &mut [u8], sink: &mut Sink) -> [Option<BvllError>; REJECTIONS] {
    // A Register-Foreign-Device followed by one octet its Length field
    // doesn't count.
    let trailing = [0x81, 0x05, 0x00, 0x06, 0x01, 0x2C, 0x00];
    // A Read-Foreign-Device-Table-Ack whose 9-octet payload is not a whole
    // 10-octet entry.
    let partial_table = [
        0x81, 0x07, 0x00, 0x0D, 192, 168, 2, 20, 0xBA, 0xC1, 0x01, 0x2C, 0x01,
    ];
    let undefined_result = Bvll::BvlcResult(BvlcResultCode::from_raw(0x0070));
    // Function 0x0C, whose clause this edition deletes.
    let unknown = [0x81, 0x0C, 0x00, 0x04];
    // A Read-Broadcast-Distribution-Table-Ack of two entries: 24 octets.
    let table = Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&BDT));
    // The sink, 50 octets into its 64.
    sink.clear();
    let crowded = sink.extend_from_slice(&[0; WIRE_LENGTH - 14]).is_ok();
    [
        decode(&trailing).err(),
        decode(&partial_table).err(),
        partial_table
            .get(4..)
            .and_then(|payload| FdtEntries::from_wire(payload).err()),
        undefined_result.encode_into(wire).err(),
        decode(&unknown).err(),
        wire.get_mut(..10)
            .and_then(|short| table.encode_into(short).err()),
        table.encode_to_sink(sink).err().filter(|_| crowded),
    ]
}
