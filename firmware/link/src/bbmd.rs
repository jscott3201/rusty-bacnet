//! The BBMD image's scenario (#1667, #1766).
//!
//! [`run`] serves as a BBMD the way firmware would: `bacnet-bip-core`'s
//! [`BbmdPort`], over a [`Link`] in [`Mode::Bbmd`] and [`Tables`] (four BDT
//! rows and four FDT entries), takes each datagram, forwards what the
//! tables say, and answers each management request from them itself. Every
//! datagram leaves through the scenario's socket, a [`Datagrams`] that
//! encodes it with `bacnet-bip-codec`. Time is a fixed timeline of
//! `bacnet-time` instants, which the bip-core re-exports. It returns a
//! [`Summary`] of what it saw.
//!
//! Its BDT lists two peers ([`PEERS`]), one reached by one-hop
//! distribution and one by two-hop; the table appends this BBMD's own row.
//! Then it serves [`STEPS`], one datagram each:
//!
//! - **A registration:** a foreign device registers, and the BBMD answers
//!   with success; a Read-FDT then lists it, with the seconds it has left.
//! - **An Original-Broadcast fan-out:** a broadcast on the BBMD's subnet is
//!   delivered, and forwarded to each peer and the foreign device.
//! - **A Forwarded-NPDU:** the two-hop peer's is delivered and broadcast on
//!   the subnet, which the peer couldn't reach itself, and passed to the
//!   foreign device; a stranger's is refused.
//! - **A Distribute-Broadcast-To-Network:** the foreign device's is
//!   delivered and forwarded to the subnet and both peers; an unregistered
//!   sender's draws the NAK.
//! - **A Read-BDT:** the table, its own row included.
//! - **A Delete-FDT-Entry:** the foreign device's registration is deleted,
//!   with success; a second request for it draws the Delete NAK.
//!
//! `tests/bbmd.rs` runs it natively and compares the whole summary with
//! [`EXPECTED`]; the `bbmd` image (`src/bin/bbmd.rs`) links it, and its
//! emulation build checks the emulated CPU's summary against the same.

mod expected;

pub use expected::EXPECTED;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{
    BdtEntries, BdtEntry, BvlcResultCode, Bvll, Entries, FdtEntry, TableEntry, decode,
};
use bacnet_bip_core::bbmd::{
    DeleteRefusal, ForwardRefusal, Registered, RegistrationRefusal, Served, TWO_HOP_MASK, Tables,
    Target,
};
use bacnet_bip_core::node::{Addressed, Datagram, Link, Mode, Permissive, SendRefusal};
use bacnet_bip_core::port::{BbmdPort, Datagrams, Ingress};
use bacnet_bip_core::{Instant, LocalSubnet};

use crate::tally::Sent;

/// BDT rows the BBMD has room for, its own included.
pub const BDT_ROWS: usize = 4;
/// Foreign devices it has room for.
pub const FDT_ENTRIES: usize = 4;

/// The BBMD's tables as the image builds them.
pub type BbmdTables = Tables<BDT_ROWS, FDT_ENTRIES>;

/// The most destinations one forwarding decision writes.
pub const MAX_TARGETS: usize = BbmdTables::MAX_TARGETS;

/// Where the Forwarded-NPDUs of one step went, in order.
pub type Fan = [Option<SocketAddrV4>; MAX_TARGETS];

/// This BBMD.
pub const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 1), 0xBAC0);
/// Its subnet, a /24: the broadcast address and the mask.
pub const SUBNET: LocalSubnet = {
    let mut subnet = LocalSubnet::new(SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), 0xBAC0));
    subnet.mask = Some(Ipv4Addr::new(255, 255, 255, 0));
    subnet
};
/// A peer BBMD reached by one-hop distribution: its subnet's directed
/// broadcast address.
pub const ONE_HOP_PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 1), 0xBAC0);
/// A peer BBMD reached by two-hop distribution: the peer itself.
pub const TWO_HOP_PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 3, 1), 0xBAC0);
/// A foreign device, on a port other than the network's.
pub const FOREIGN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(172, 16, 0, 5), 47809);
/// A device on the BBMD's subnet.
pub const LOCAL: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 0xBAC0);
/// Where the two-hop peer's forwarded broadcast started.
pub const REMOTE: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 3, 77), 0xBAC0);
/// A sender that is neither a peer nor registered.
pub const STRANGER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(172, 16, 0, 9), 0xBAC0);

/// The time-to-live the foreign device registers with, in seconds.
pub const TTL: u16 = 60;

/// The configured BDT: the two peers, without this BBMD's own row, which the
/// table appends.
pub const PEERS: [BdtEntry; 2] = [
    BdtEntry::new(ONE_HOP_PEER, Ipv4Addr::new(255, 255, 255, 0)),
    BdtEntry::new(TWO_HOP_PEER, TWO_HOP_MASK),
];

/// A global-broadcast Who-Is in an NPDU: every broadcast in the scenario.
pub const WHO_IS: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// One datagram the BBMD receives.
#[derive(Debug, Clone, Copy)]
pub struct Step {
    /// When.
    pub at: Instant,
    /// From whom.
    pub source: SocketAddrV4,
    /// Whether it was sent to the subnet's broadcast address rather than
    /// to the BBMD.
    pub broadcast: bool,
    /// The message.
    pub message: Bvll<'static>,
}

/// The datagrams the BBMD receives, in order.
pub const STEPS: [Step; 10] = [
    step(1_000, FOREIGN, Bvll::RegisterForeignDevice { ttl: TTL }),
    step(2_000, LOCAL, Bvll::ReadForeignDeviceTable),
    Step {
        broadcast: true,
        ..step(3_000, LOCAL, Bvll::OriginalBroadcastNpdu(&WHO_IS))
    },
    step(4_000, TWO_HOP_PEER, FORWARDED),
    step(4_000, STRANGER, FORWARDED),
    step(5_000, FOREIGN, Bvll::DistributeBroadcastToNetwork(&WHO_IS)),
    step(5_000, STRANGER, Bvll::DistributeBroadcastToNetwork(&WHO_IS)),
    step(6_000, LOCAL, Bvll::ReadBroadcastDistributionTable),
    step(7_000, LOCAL, Bvll::DeleteForeignDeviceTableEntry(FOREIGN)),
    step(7_000, LOCAL, Bvll::DeleteForeignDeviceTableEntry(FOREIGN)),
];

/// The two-hop peer's forwarded broadcast, and the stranger's copy of it.
const FORWARDED: Bvll<'static> = Bvll::ForwardedNpdu {
    origin: REMOTE,
    npdu: &WHO_IS,
};

/// A datagram sent to the BBMD's own address `millis` milliseconds into the
/// timeline.
const fn step(millis: u64, source: SocketAddrV4, message: Bvll<'static>) -> Step {
    Step {
        at: Instant::from_millis(millis),
        source,
        broadcast: false,
        message,
    }
}

/// Octets the scenario encodes into at once; its longest message, the
/// Read-BDT-Ack of three rows, takes 34.
const WIRE_LENGTH: usize = 64;

/// An unused row of the FDT the BBMD reports: all zero bits.
const NO_ROW: FdtEntry = FdtEntry::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), 0, 0);

/// Storage the scenario works in besides its stack: a buffer for the
/// datagram coming in, which the BBMD's decisions borrow, one for the
/// datagram going out, and the storage the port lends the tables for a
/// decision's destinations and a Read-FDT-Ack's rows. The tables
/// themselves, a few hundred octets, are built on the stack.
///
/// [`Memory::new`] is all zero bits, so the image keeps it in a static that
/// the linker places in `.bss`.
pub struct Memory {
    /// The datagram received.
    rx: [u8; WIRE_LENGTH],
    /// The datagram sent.
    tx: [u8; WIRE_LENGTH],
    /// A decision's destinations.
    targets: [Target; MAX_TARGETS],
    /// A Read-FDT-Ack's rows.
    rows: [FdtEntry; FDT_ENTRIES],
}

impl Memory {
    /// Zeroed storage.
    pub const fn new() -> Self {
        Self {
            rx: [0; WIRE_LENGTH],
            tx: [0; WIRE_LENGTH],
            targets: [Target::UNUSED; MAX_TARGETS],
            rows: [NO_ROW; FDT_ENTRIES],
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// Room for the NPDUs the BBMD delivers to its own network layer.
pub const DELIVERIES: usize = 4;
/// Room for the BVLC-Results it sends.
pub const RESULTS: usize = 4;

/// What [`run`] saw. Every field is plain data, so the host test can
/// compare the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// What the FDT made of the registration.
    pub registration: Option<Result<Registered, RegistrationRefusal>>,
    /// The FDT, as the Read-FDT-Ack the BBMD sent decodes.
    pub fdt: [Option<FdtEntry>; FDT_ENTRIES],
    /// Where the Original-Broadcast was forwarded.
    pub original: Fan,
    /// Where the peer's Forwarded-NPDU went on to.
    pub forwarded: Fan,
    /// Why the stranger's Forwarded-NPDU was refused.
    pub not_from_peer: Option<ForwardRefusal>,
    /// Where the foreign device's Distribute-Broadcast-To-Network was
    /// forwarded.
    pub distributed: Fan,
    /// The BDT, as the Read-BDT-Ack the BBMD sent decodes.
    pub bdt: [Option<BdtEntry>; BDT_ROWS],
    /// What the two Delete-FDT-Entry requests came to.
    pub deleted: [Option<Result<(), DeleteRefusal>>; 2],
    /// Each BVLC-Result the BBMD sent, in order: where, and its code.
    pub results: [Option<(SocketAddrV4, BvlcResultCode)>; RESULTS],
    /// The source of each NPDU the BBMD delivered to its own network layer,
    /// in order.
    pub delivered: [Option<SocketAddrV4>; DELIVERIES],
    /// Every datagram the BBMD sent.
    pub sent: Sent,
    /// Datagrams of the port's own its socket refused.
    pub refused: u32,
    /// Calls that failed or answered otherwise than the scenario expects.
    /// Zero in a good run.
    pub errors: u8,
}

impl Summary {
    const fn new() -> Self {
        Self {
            registration: None,
            fdt: [None; FDT_ENTRIES],
            original: [None; MAX_TARGETS],
            forwarded: [None; MAX_TARGETS],
            not_from_peer: None,
            distributed: [None; MAX_TARGETS],
            bdt: [None; BDT_ROWS],
            deleted: [None; 2],
            results: [None; RESULTS],
            delivered: [None; DELIVERIES],
            sent: Sent::new(),
            refused: 0,
            errors: 0,
        }
    }

    /// Count a call that failed, or answered otherwise than expected.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// Run the whole scenario in `memory`. The link and the tables are built on
/// this function's stack.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on, or returns early when it can't.
pub fn run(memory: &mut Memory) -> Summary {
    let Memory {
        rx,
        tx,
        targets,
        rows,
    } = memory;
    let mut summary = Summary::new();
    let Ok(link) = Link::new(OWN, SUBNET, Mode::Bbmd) else {
        summary.error();
        return summary;
    };
    let mut tables = BbmdTables::new(OWN);
    if tables.bdt.set(BdtEntries::from_entries(&PEERS)).is_err() {
        summary.error();
    }
    let Ok(mut port) = BbmdPort::new(link, &mut tables, targets, rows) else {
        summary.error();
        return summary;
    };
    let mut socket = Socket {
        tx,
        summary: &mut summary,
        fan: [None; MAX_TARGETS],
    };
    for step in &STEPS {
        let Ok(length) = step.message.encode_into(rx) else {
            socket.summary.error();
            continue;
        };
        let Some(payload) = rx.get(..length) else {
            socket.summary.error();
            continue;
        };
        let to = Addressed {
            node: !step.broadcast,
            broadcast: step.broadcast,
        };
        let datagram = Datagram::new(payload, step.source, Some(to));
        socket.fan = [None; MAX_TARGETS];
        let ingress = port.receive(&datagram, step.at, &mut Permissive, &mut socket);
        note(socket.summary, step, ingress, socket.fan);
    }
    socket.summary.refused = port.take_counters().refused;
    summary
}

/// Note what the port made of `step`: what it delivered, and where it
/// forwarded the step's broadcast, or what it answered.
fn note(summary: &mut Summary, step: &Step, ingress: Ingress<'_>, fan: Fan) {
    match ingress {
        Ingress::Npdu(delivery) => {
            deliver(summary, delivery.source, delivery.npdu);
            let slot = match step.message {
                Bvll::OriginalBroadcastNpdu(_) => &mut summary.original,
                Bvll::ForwardedNpdu { .. } => &mut summary.forwarded,
                Bvll::DistributeBroadcastToNetwork(_) => &mut summary.distributed,
                _ => {
                    summary.error();
                    return;
                }
            };
            *slot = fan;
        }
        // Nothing answers a refused Forwarded-NPDU.
        Ingress::NotForwarded(refusal) => summary.not_from_peer = Some(refusal),
        Ingress::Served(Served::Register(outcome)) => summary.registration = Some(outcome.result),
        Ingress::Served(Served::DeleteFdtEntry(deleted)) => {
            record(&mut summary.deleted, deleted, &mut summary.errors);
        }
        // The socket notes the table ACKs and the NAK.
        Ingress::Served(Served::ReadBdt | Served::ReadFdt(Ok(_))) | Ingress::Handled => {}
        _ => summary.error(),
    }
}

/// The BBMD's socket: it encodes each datagram into `tx`, keeps the
/// summary's tally of it, and decodes it again for the summary.
struct Socket<'m> {
    tx: &'m mut [u8],
    summary: &'m mut Summary,
    /// Where this step's Forwarded-NPDUs went.
    fan: Fan,
}

impl Datagrams for Socket<'_> {
    fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal> {
        let summary = &mut *self.summary;
        let Some(datagram) = summary.sent.send(self.tx, to, message) else {
            summary.error();
            return Err(SendRefusal::SocketError);
        };
        match decode(datagram) {
            Ok(Bvll::ForwardedNpdu { .. }) => record(&mut self.fan, to, &mut summary.errors),
            Ok(Bvll::BvlcResult(code)) => {
                record(&mut summary.results, (to, code), &mut summary.errors)
            }
            Ok(Bvll::ReadForeignDeviceTableAck(sent)) => {
                summary.fdt = rows_of(sent, &mut summary.errors);
            }
            Ok(Bvll::ReadBroadcastDistributionTableAck(sent)) => {
                summary.bdt = rows_of(sent, &mut summary.errors);
            }
            _ => summary.error(),
        }
        Ok(())
    }
}

/// Hand `npdu`, from `source`, to the BBMD's own network layer.
fn deliver(summary: &mut Summary, source: SocketAddrV4, npdu: &[u8]) {
    if npdu != WHO_IS {
        summary.error();
    }
    record(&mut summary.delivered, source, &mut summary.errors);
}

/// Put `value` in the first free slot of `slots`, or count an error when
/// none is free.
fn record<T: Copy>(slots: &mut [Option<T>], value: T, errors: &mut u8) {
    match slots.iter_mut().find(|slot| slot.is_none()) {
        Some(slot) => *slot = Some(value),
        None => *errors = errors.saturating_add(1),
    }
}

/// A table's rows, from the front; an error when there are more than `N`.
fn rows_of<T: TableEntry, const N: usize>(
    entries: Entries<'_, T>,
    errors: &mut u8,
) -> [Option<T>; N] {
    let mut rows = [None; N];
    let mut iter = entries.iter();
    for slot in &mut rows {
        *slot = iter.next();
    }
    if iter.next().is_some() {
        *errors = errors.saturating_add(1);
    }
    rows
}
