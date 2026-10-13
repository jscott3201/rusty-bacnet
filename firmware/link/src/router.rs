//! The router image's scenario (#1679).
//!
//! [`run`] routes between two networks the way router firmware would, with
//! no allocator: `bacnet-router-core`'s [`Router`] joins a B/IP port and an
//! MS/TP port through the core's `bip` and `mstp` glue.
//!
//! - **Port 0, network 1:** B/IP, where the router is also the BBMD
//!   ([`BbmdPort`] over [`BbmdTables`]), with one peer in its BDT, reached
//!   by two-hop distribution. A BBMD and a router may share a device
//!   (Annex J.7.1).
//! - **Port 1, network 2:** MS/TP with extended frames on, which a routing
//!   node needs (Clause 9.3), so both ports carry NPDUs of up to 1497
//!   octets (Table 6-1). The router is station 1 and passes the token to
//!   station 2, a master; station 5, which never holds the token, takes
//!   what the router routes to it.
//!
//! Time is one injected `bacnet-time` clock that all three cores share.
//! Every action is carried out: each router send goes to its port's glue,
//! each datagram through the codec into a buffer, and each MS/TP frame onto
//! the line, with its octets' time; MS/TP frames come in octet by octet.
//! Then [`run`] returns a [`Summary`]. The steps:
//!
//! 1. **Start:** a Network-Number-Is and an I-Am-Router on each port. On
//!    B/IP each goes out on the subnet, and the BBMD forwards it to its
//!    peer; on MS/TP both leave on the first token.
//! 2. **B/IP to MS/TP:** a node's Original-Unicast for station 5 on
//!    network 2 leaves on the next token without its destination, and with
//!    SNET 1 and the node's six-octet address as SADR.
//! 3. **A forwarded broadcast (#1678):** the peer's Forwarded-NPDU, at
//!    life-safety priority, for station 5: the BBMD broadcasts it on its
//!    subnet, and it reaches MS/TP with the origin as its SADR and its
//!    priority kept.
//! 4. **MS/TP to B/IP:** station 2's request for the node on network 1,
//!    which expects a reply, goes out as an Original-Unicast, and the MS/TP
//!    side answers Reply Postponed as soon as the line allows: one
//!    T_turnaround after the request, not at the core's own deadline.
//! 5. **An unknown network:** the node's message for network 9 starts a
//!    search, a Who-Is-Router on MS/TP, and draws reject reason 1 back on
//!    B/IP. A second one within the search's cooldown draws only the
//!    reject.
//! 6. **Too long for MS/TP (#1665):** the rewrite takes out DNET, DLEN,
//!    DADR and the hop count (5 octets) and puts in SNET, SLEN and a
//!    six-octet SADR (9), so
//!    an NPDU of [`REFUSED_LENGTH`] octets would leave at 1498, one past
//!    MS/TP's limit: it draws reason 4 and isn't queued. One of
//!    [`QUEUED_LENGTH`] leaves at exactly 1497, in an extended frame.
//! 7. **Route questions from MS/TP:** a Who-Is-Router naming no network is
//!    answered with an I-Am-Router for network 1 alone, and an
//!    Initialize-Routing-Table query with one Ack listing both networks.
//! 8. **A global broadcast from MS/TP** goes out on B/IP as an
//!    Original-Broadcast, which the BBMD forwards to its peer, and reaches
//!    the router's own application too.
//!
//! The summary holds what the router made of each frame, every send it
//! asked for, the events it reported, its counters, and every datagram and
//! frame sent, with a hash over each port's octets. `tests/router.rs` runs
//! it natively and compares the whole summary with [`expected()`]; the
//! `router` image (`src/bin/router.rs`) links it, and its emulation build
//! checks the emulated CPU's summary against the same.
//!
//! [`Memory`] holds what the router, the MS/TP core and the BBMD port
//! borrow, and the B/IP socket's two datagram buffers. The router, the
//! core, the port with its node, and the BBMD's tables are built on the
//! stack.
//!
//! # What it isn't
//!
//! The image routes, and has no application layer. A product router also
//! needs a Device object: Clause 6.6 sends routers to the device
//! requirements of Clause 22.1.5. The global broadcast of step 8 reaches
//! [`Received::Local`], where such a device's application would take it.

mod expected;

pub use expected::expected;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcFunction, Bvll};
use bacnet_bip_core::bbmd::{Tables, Target, TWO_HOP_MASK};
use bacnet_bip_core::node::{self, Addressed, Datagram, Link, Mode, Node, MAX_DATAGRAM};
use bacnet_bip_core::LocalSubnet;
use bacnet_mstp_codec::{Frame, FrameType, BROADCAST_MAC};
use bacnet_mstp_core::codec::{MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA};
use bacnet_mstp_core::{
    Action as MstpAction, Duration, ExtendedActionBuffers, HeaplessQueues, MasterCore,
    MstpCoreConfig, RequestId, TransmitId,
};
use bacnet_router_core::bip::{self, BbmdPort, Datagrams, Ingress};
use bacnet_router_core::mstp::{self, MstpPort};
use bacnet_router_core::router::{
    Action, Counters, Dest, Discard, Event, Group, Host, Kind, Options, Received, SendRefusal,
};
use bacnet_router_core::{Instant, PortId, Router, Storage, MAX_NPDU};

use crate::tally::Sent;

/// Ports the router has room for.
pub const PORTS: usize = 2;
/// Routes its table has room for, direct ones included.
pub const ROUTES: usize = 8;
/// Reject hold-down records.
pub const HOLDS: usize = 8;
/// Searches for unknown networks it remembers at once.
pub const SEARCHES: usize = 4;

/// The router's storage: its ports, table, searches and an output scratch
/// of [`MAX_NPDU`] octets.
pub type RouterStorage = Storage<PORTS, ROUTES, HOLDS, SEARCHES>;

/// The router as the image builds it, over storage it borrows.
pub type RouterEngine<'m> = Router<&'m mut RouterStorage, PORTS, ROUTES, HOLDS, SEARCHES>;

/// Unicast slots in the MS/TP transmit queue. Each step's sends leave on
/// the next token, so one is enough.
pub const UNICAST_SLOTS: usize = 1;
/// Broadcast slots: two, for the startup's Network-Number-Is and
/// I-Am-Router, which the router queues together.
pub const BROADCAST_SLOTS: usize = 2;

/// The MS/TP core's transmit queues, with slots for extended frames.
pub type Queues = HeaplessQueues<UNICAST_SLOTS, BROADCAST_SLOTS, MAX_EXTENDED_MPDU_DATA>;

/// The MS/TP core's action buffers, sized for extended frames, with one
/// delivery slot: the scenario polls every action after each call.
pub type Actions = ExtendedActionBuffers<1>;

/// The MS/TP master core, borrowing its queues as it does its receive
/// storage and action buffers.
pub type Core<'m> = MasterCore<'m, &'m mut Queues, MAX_EXTENDED_MPDU_DATA>;

/// BDT rows the BBMD has room for, its own included.
pub const BDT_ROWS: usize = 2;
/// Foreign devices it has room for.
pub const FDT_ENTRIES: usize = 2;

/// The BBMD's tables.
pub type BbmdTables = Tables<BDT_ROWS, FDT_ENTRIES>;

/// The B/IP port: a BBMD's node with one exchange slot, which it never
/// uses (a BBMD registers nowhere), over [`BbmdTables`].
pub type BipPort<'m> = BbmdPort<'m, BDT_ROWS, FDT_ENTRIES, 1>;

/// The B/IP port's network.
pub const BIP_NETWORK: u16 = 1;
/// The MS/TP port's network.
pub const MSTP_NETWORK: u16 = 2;
/// The B/IP port.
const BIP: PortId = PortId(0);
/// The MS/TP port.
const MSTP: MstpPort = MstpPort::new(PortId(1));

/// The router, and the BBMD, on the B/IP subnet.
pub const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 1), 0xBAC0);
/// The subnet, a /24: the broadcast address and the mask.
pub const SUBNET: LocalSubnet = {
    let mut subnet = LocalSubnet::new(SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), 0xBAC0));
    subnet.mask = Some(Ipv4Addr::new(255, 255, 255, 0));
    subnet
};
/// The BBMD's peer on another subnet, reached by two-hop distribution: it
/// forwards to this BBMD alone, which broadcasts on its subnet.
pub const PEER_BBMD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 1), 0xBAC0);
/// A node on the subnet, on network 1.
pub const NODE: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 0xBAC0);
/// Where the peer's forwarded broadcast started.
pub const ORIGIN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 77), 0xBAC0);

/// The BDT the BBMD is given: the peer, without its own row, which the
/// table appends.
const PEERS: [BdtEntry; 1] = [BdtEntry::new(PEER_BBMD, TWO_HOP_MASK)];

/// The B/IP link: in BBMD mode, with [`Link::new`]'s send budget, B/IP's
/// largest datagram, so NPDUs of up to 1497 octets go out either way. It is
/// checked when the image is built.
pub const LINK: Link = match Link::new(OWN, SUBNET, Mode::Bbmd) {
    Ok(link) => link,
    Err(_) => panic!("the scenario's link is invalid"),
};

/// The router's MS/TP station.
pub const STATION: u8 = 1;
/// The next master, which passes the router the token.
pub const PEER_STATION: u8 = 2;
/// A node on MS/TP that the router reaches with frames that need no reply.
pub const TARGET_STATION: u8 = 5;

/// The MS/TP port: station 1 of masters 0-2 at 9600 baud, sending up to
/// two frames each time it holds the token, with extended frames on.
pub const CONFIG: MstpCoreConfig = {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = PEER_STATION;
    config.max_info_frames = 2;
    config.with_extended_frames()
};

/// The hosted router's timers and policy, with a route cap of the table's
/// size.
pub const OPTIONS: Options = {
    let mut options = Options::HOSTED;
    options.route_cap = ROUTES as u16;
    options
};

/// Step 2: a Who-Is for station 5 on network 2, from the B/IP node.
pub const TO_STATION_5: [u8; 9] = [0x01, 0x20, 0x00, 0x02, 0x01, 0x05, 0xFF, 0x10, 0x08];
/// Step 3: the same, at life-safety priority (3), forwarded by the peer.
pub const LIFE_SAFETY: [u8; 9] = [0x01, 0x23, 0x00, 0x02, 0x01, 0x05, 0xFF, 0x10, 0x08];
/// Step 4: station 2's ReadProperty of a device's Object_Name, for the B/IP
/// node on network 1, expecting a reply.
pub const READ_PROPERTY: [u8; 23] = [
    0x01, 0x24, 0x00, 0x01, 0x06, 192, 168, 1, 20, 0xBA, 0xC0, 0xFF, 0x00, 0x05, 0x01, 0x0C, 0x0C,
    0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
];
/// Step 5: a Who-Is for every node on network 9, which no route reaches.
pub const TO_NETWORK_9: [u8; 8] = [0x01, 0x20, 0x00, 0x09, 0x00, 0xFF, 0x10, 0x08];
/// Step 7: a Who-Is-Router naming no network.
pub const WHO_IS_ROUTER: [u8; 3] = [0x01, 0x80, 0x00];
/// Step 7: an Initialize-Routing-Table with no entries: a query.
pub const INIT_RT_QUERY: [u8; 4] = [0x01, 0x80, 0x06, 0x00];
/// Step 8: a global-broadcast Who-Is.
pub const GLOBAL_WHO_IS: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// Step 6: the B/IP node's NPDU too long for MS/TP once rewritten.
pub const REFUSED_LENGTH: usize = MAX_NPDU - 3;
/// Step 6: the longest of its NPDUs that MS/TP carries once rewritten.
pub const QUEUED_LENGTH: usize = MAX_NPDU - 4;

/// The NPCI and APDU header of step 6's NPDUs: for station 5 on network 2,
/// an Unconfirmed-Request (UnconfirmedPrivateTransfer).
const LONG_HEADER: [u8; 9] = [0x01, 0x20, 0x00, 0x02, 0x01, 0x05, 0xFF, 0x10, 0x04];

/// Octet `index` of step 6's NPDUs: [`LONG_HEADER`], then service data
/// that steps through a pattern with a zero every 251 octets, which the
/// extended frame's COBS encoding has to remove.
const fn long_octet(index: usize) -> u8 {
    if index < LONG_HEADER.len() {
        LONG_HEADER[index]
    } else {
        // Below 251, so the cast keeps every bit.
        (index.wrapping_mul(7) % 251) as u8
    }
}

/// What the router, the MS/TP core and the BBMD port borrow, and the B/IP
/// socket's buffers: the datagram received, which the router reads in
/// place, and the one being sent, which the port encodes into.
///
/// [`Memory::new`] is all zero bits, so the image keeps it in a static
/// that the linker places in `.bss`.
pub struct Memory {
    /// The router's storage.
    router: RouterStorage,
    /// The MS/TP core's receive storage: the longest extended frame.
    rx: [u8; MAX_EXTENDED_FRAME_LENGTH],
    /// The MS/TP core's transmit queues.
    queues: Queues,
    /// The MS/TP core's action buffers.
    actions: Actions,
    /// Where the BBMD writes one decision's destinations.
    targets: [Target; BbmdTables::MAX_TARGETS],
    /// The datagram the B/IP socket received.
    datagram_in: [u8; MAX_DATAGRAM],
    /// The datagram it sends.
    datagram_out: [u8; MAX_DATAGRAM],
}

impl Memory {
    /// Zeroed storage.
    pub const fn new() -> Self {
        Self {
            router: RouterStorage::new(),
            rx: [0; MAX_EXTENDED_FRAME_LENGTH],
            queues: Queues::new(),
            actions: Actions::new(),
            targets: [Target::UNUSED; BbmdTables::MAX_TARGETS],
            datagram_in: [0; MAX_DATAGRAM],
            datagram_out: [0; MAX_DATAGRAM],
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// What the router made of a frame it took ([`Received`], without the
/// borrow of the frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Routed, answered or relayed.
    Handled,
    /// For its own application: whether sent to a group, and whether a
    /// global broadcast.
    Local {
        /// To a group.
        group: bool,
        /// A global broadcast.
        global: bool,
    },
    /// A reject addressed to the router.
    ToRouter,
    /// Discarded, for this reason.
    Discarded(Discard),
}

impl From<Received<'_>> for Outcome {
    fn from(received: Received<'_>) -> Self {
        match received {
            Received::Handled => Self::Handled,
            Received::Local(local) => Self::Local {
                group: local.group,
                global: local.global,
            },
            Received::ToRouter(_) => Self::ToRouter,
            Received::Discarded(why) => Self::Discarded(why),
        }
    }
}

/// A send the router asked for: its port, why it sends, and whether to
/// every node there.
pub type RouterSend = (u8, Kind, bool);
/// A B/IP datagram sent: its function, destination and length.
pub type SentDatagram = (BvlcFunction, SocketAddrV4, u16);
/// An MS/TP frame sent: its type, destination and length.
pub type SentFrame = (FrameType, u8, u16);

/// Room for the frames the router takes.
pub const RECEIVES: usize = 12;
/// Room for the sends it asks for.
pub const SENDS: usize = 20;
/// Room for the events it reports.
pub const EVENTS: usize = 4;
/// Room for the datagrams the B/IP port sends.
pub const DATAGRAMS: usize = 12;
/// Room for the frames the MS/TP core sends.
pub const FRAMES: usize = 20;

/// What [`run`] saw. Every field is plain data, so the host test can
/// compare the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// What the router made of each frame it took, in order.
    pub received: [Option<Outcome>; RECEIVES],
    /// Each send it asked for, in order.
    pub sends: [Option<RouterSend>; SENDS],
    /// What it reported to its host, in order.
    pub events: [Option<Event>; EVENTS],
    /// Its counters at the end.
    pub counters: Counters,
    /// Routes in its table at the end.
    pub routes: usize,
    /// Each datagram the B/IP port sent, in order.
    pub datagrams: [Option<SentDatagram>; DATAGRAMS],
    /// All of them, with a hash over each one's destination and octets.
    pub bip: Sent,
    /// How many of the B/IP port's own datagrams its socket refused.
    pub bip_refused: u32,
    /// Each frame the MS/TP core sent, in order.
    pub frames: [Option<SentFrame>; FRAMES],
    /// All of them, with a hash over their octets.
    pub mstp: Sent,
    /// MS/TP core counter: valid frames received.
    pub frames_received: u32,
    /// MS/TP core counter: frames sent.
    pub frames_transmitted: u32,
    /// MS/TP core counter: NPDUs delivered to the router.
    pub npdus_delivered: u32,
    /// MS/TP core counter: invalid frames received.
    pub parser_errors: u32,
    /// MS/TP queue counters: NPDUs queued for one station, and for all.
    pub queued: (u32, u32),
    /// How long after station 2's request the router's Reply Postponed
    /// started.
    pub postponed_after: Option<Duration>,
    /// Calls that failed, or answered otherwise than the scenario expects.
    /// Zero in a good run.
    pub errors: u8,
}

impl Summary {
    fn new() -> Self {
        Self {
            received: [None; RECEIVES],
            sends: [None; SENDS],
            events: [None; EVENTS],
            counters: Counters::default(),
            routes: 0,
            datagrams: [None; DATAGRAMS],
            bip: Sent::new(),
            bip_refused: 0,
            frames: [None; FRAMES],
            mstp: Sent::new(),
            frames_received: 0,
            frames_transmitted: 0,
            npdus_delivered: 0,
            parser_errors: 0,
            queued: (0, 0),
            postponed_after: None,
            errors: 0,
        }
    }

    /// Count a call that failed, or answered otherwise than expected.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// Run the whole scenario in `memory`. The router, the MS/TP core, the
/// BBMD port and its tables are built on this function's stack.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on, or returns early when it can't.
pub fn run(memory: &mut Memory) -> Summary {
    let Memory {
        router,
        rx,
        queues,
        actions,
        targets,
        datagram_in,
        datagram_out,
    } = memory;
    let mut summary = Summary::new();
    let mut tables = BbmdTables::new(OWN);
    if tables.bdt.set(BdtEntries::from_entries(&PEERS)).is_err() {
        summary.error();
    }
    // One at a time, so no two results to unpack share the stack.
    let Ok(node) = Node::new(LINK, Instant::ZERO) else {
        summary.error();
        return summary;
    };
    let Ok(mut bbmd) = BipPort::new(BIP, node, &mut tables, targets) else {
        summary.error();
        return summary;
    };
    let Ok(mstp_port) = MstpPort::config(&CONFIG, MSTP_NETWORK) else {
        summary.error();
        return summary;
    };
    let ports = [bbmd.config(BIP_NETWORK), mstp_port];
    let Ok(mut router) = RouterEngine::new(router, &ports, OPTIONS, Instant::ZERO) else {
        summary.error();
        return summary;
    };
    let Ok(mut core) = MasterCore::new(CONFIG, queues, rx, actions, Instant::ZERO) else {
        summary.error();
        return summary;
    };
    Device {
        router: &mut router,
        bbmd: &mut bbmd,
        core: &mut core,
        host: Hooks {
            link: LINK,
            events: [None; EVENTS],
        },
        datagram_in,
        datagram_out,
        now: Instant::ZERO,
        octet: CONFIG.octet_time(),
        last_frame: None,
        last_sent_at: Instant::ZERO,
        summary: &mut summary,
    }
    .run();
    summary
}

/// The router's host: each port's group addresses, and a record of the
/// events the router reports.
struct Hooks {
    link: Link,
    events: [Option<Event>; EVENTS],
}

impl Host for Hooks {
    fn group(&self, port: PortId, mac: &[u8]) -> Group {
        if port == BIP {
            bip::group(&self.link, mac)
        } else {
            mstp::group(mac)
        }
    }

    fn observe(&mut self, event: Event) {
        // A full record shows as a summary that differs.
        if let Some(slot) = self.events.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(event);
        }
    }
}

/// The B/IP port's socket: it encodes each datagram into the buffer, and
/// the summary keeps a record of it.
struct Socket<'w> {
    buffer: &'w mut [u8],
    summary: &'w mut Summary,
}

impl Datagrams for Socket<'_> {
    fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal> {
        let Some(datagram) = self.summary.bip.send(self.buffer, to, message) else {
            self.summary.error();
            return Err(SendRefusal::Other);
        };
        let length = u16::try_from(datagram.len()).unwrap_or(u16::MAX);
        let entry = (message.function(), to, length);
        record(&mut self.summary.datagrams, entry, &mut self.summary.errors);
        Ok(())
    }
}

/// The most actions one call may queue, or deadlines one step may wait
/// through, before the scenario counts an error and moves on, so a runaway
/// core can't hang it.
const MAX_ACTIONS: usize = 16;

/// The device around the router: it carries out what the router and both
/// ports ask, plays the B/IP network and the MS/TP line, and keeps the
/// time.
struct Device<'d, 'm> {
    router: &'d mut RouterEngine<'m>,
    bbmd: &'d mut BipPort<'m>,
    core: &'d mut Core<'m>,
    host: Hooks,
    datagram_in: &'d mut [u8],
    datagram_out: &'d mut [u8],
    now: Instant,
    /// One octet's time on the MS/TP line.
    octet: Duration,
    /// The type of the last frame the MS/TP core sent in this step.
    last_frame: Option<FrameType>,
    /// When that frame started.
    last_sent_at: Instant,
    summary: &'d mut Summary,
}

/// What one polled MS/TP action leaves to do once its borrow of the core
/// has ended.
enum Then {
    Nothing,
    Send { id: TransmitId, octets: usize },
    Release(Option<RequestId>),
}

impl Device<'_, '_> {
    fn run(&mut self) {
        // 1. The startup announcements, and the first token.
        self.router.start();
        self.carry();
        self.token();

        // 2. B/IP to MS/TP.
        self.at(1_000);
        self.bip_in(&Bvll::OriginalUnicastNpdu(&TO_STATION_5), NODE, false);
        self.token();

        // 3. The peer's Forwarded-NPDU, by unicast, at life-safety priority.
        self.at(2_000);
        let forwarded = Bvll::ForwardedNpdu {
            origin: ORIGIN,
            npdu: &LIFE_SAFETY,
        };
        self.bip_in(&forwarded, PEER_BBMD, false);
        self.token();

        // 4. MS/TP to B/IP, and Reply Postponed.
        self.at(3_000);
        self.mstp_in(
            FrameType::BACNET_DATA_EXPECTING_REPLY,
            STATION,
            &READ_PROPERTY,
        );
        let asked = self.now;
        self.until(FrameType::REPLY_POSTPONED);
        self.summary.postponed_after = Some(self.last_sent_at.saturating_duration_since(asked));

        // 5. An unknown network, twice within the search's cooldown.
        self.at(4_000);
        self.bip_in(&Bvll::OriginalUnicastNpdu(&TO_NETWORK_9), NODE, false);
        self.token();
        self.at(5_000);
        self.bip_in(&Bvll::OriginalUnicastNpdu(&TO_NETWORK_9), NODE, false);

        // 6. One octet too long for MS/TP once rewritten, then the longest
        // that fits.
        self.at(6_000);
        self.bip_long(REFUSED_LENGTH);
        self.bip_long(QUEUED_LENGTH);
        self.token();

        // 7. Route questions from MS/TP.
        self.at(7_000);
        let data = FrameType::BACNET_DATA_NOT_EXPECTING_REPLY;
        self.mstp_in(data, BROADCAST_MAC, &WHO_IS_ROUTER);
        self.mstp_in(data, STATION, &INIT_RT_QUERY);
        self.token();

        // 8. A global broadcast from MS/TP.
        self.at(8_000);
        self.mstp_in(data, BROADCAST_MAC, &GLOBAL_WHO_IS);

        self.finish();
    }

    /// Take what the router, the port and the core counted, once, at the
    /// end.
    fn finish(&mut self) {
        let summary = &mut *self.summary;
        summary.events = self.host.events;
        summary.counters = counters(self.router);
        summary.routes = self.router.table().len();
        summary.bip_refused = self.bbmd.take_counters().refused;
        let counters = self.core.take_counters();
        summary.frames_received = counters.frames_received;
        summary.frames_transmitted = counters.frames_transmitted;
        summary.npdus_delivered = counters.npdus_delivered;
        summary.parser_errors = counters.parser_errors;
        summary.queued = (counters.unicast.enqueued, counters.broadcast.enqueued);
    }

    /// Let the clock run on to `millis` milliseconds into the timeline.
    fn at(&mut self, millis: u64) {
        self.now = self.now.max(Instant::from_millis(millis));
        self.last_frame = None;
    }

    /// The B/IP socket receives `message` from `source`, sent to the
    /// router's own address, or to the subnet's broadcast address.
    fn bip_in(&mut self, message: &Bvll<'_>, source: SocketAddrV4, broadcast: bool) {
        match message.encode_into(self.datagram_in) {
            Ok(length) => self.bip_datagram(length, source, broadcast),
            Err(_) => self.summary.error(),
        }
    }

    /// The B/IP node sends an Original-Unicast of step 6's NPDU of
    /// `length` octets: built in the outgoing buffer, which is free between
    /// sends, and encoded into the incoming one, so no NPDU-sized buffer
    /// goes on the stack.
    fn bip_long(&mut self, length: usize) {
        let Some(npdu) = self.datagram_out.get_mut(..length) else {
            self.summary.error();
            return;
        };
        for (index, octet) in npdu.iter_mut().enumerate() {
            *octet = long_octet(index);
        }
        match Bvll::OriginalUnicastNpdu(npdu).encode_into(self.datagram_in) {
            Ok(encoded) => self.bip_datagram(encoded, NODE, false),
            Err(_) => self.summary.error(),
        }
    }

    /// Hand the BBMD port the first `length` octets received, and the
    /// router the NPDU the port delivers; then carry out every send.
    fn bip_datagram(&mut self, length: usize, source: SocketAddrV4, broadcast: bool) {
        let Some(payload) = self.datagram_in.get(..length) else {
            self.summary.error();
            return;
        };
        let to = Addressed {
            node: !broadcast,
            broadcast,
        };
        let datagram = Datagram::new(payload, source, Some(to));
        let mut socket = Socket {
            buffer: &mut *self.datagram_out,
            summary: &mut *self.summary,
        };
        let ingress = self
            .bbmd
            .receive(&datagram, self.now, &mut node::Permissive, &mut socket);
        match ingress {
            Ingress::Npdu(arrived) => {
                let received = self
                    .router
                    .receive(arrived.frame(), self.now, &mut self.host);
                note(self.summary, received.into());
            }
            _ => self.summary.error(),
        }
        self.carry();
        self.drain();
    }

    /// Poll the router until it is idle, handing each send to its port's
    /// glue and each refusal back to the router.
    fn carry(&mut self) {
        for _ in 0..MAX_ACTIONS {
            let send = match self.router.poll_action() {
                Action::Idle => return,
                Action::Send(send) => send,
            };
            let entry = (send.port.0, send.kind, send.to == Dest::Broadcast);
            record(&mut self.summary.sends, entry, &mut self.summary.errors);
            let result = if send.port == BIP {
                let mut socket = Socket {
                    buffer: &mut *self.datagram_out,
                    summary: &mut *self.summary,
                };
                self.bbmd.send(&send, self.now, &mut socket)
            } else {
                MSTP.send(self.core, &send, self.now)
            };
            if let Err(why) = result {
                self.router.refused(why);
            }
        }
        self.summary.error();
    }

    /// Station 2 sends a frame of `frame_type` to `destination`, carrying
    /// `data`: encoded into a buffer that lasts only for this call and fed
    /// to the core octet by octet, then every action carried out.
    fn mstp_in(&mut self, frame_type: FrameType, destination: u8, data: &[u8]) {
        let mut wire = [0; MSTP_WIRE];
        let frame = Frame::new(frame_type, destination, PEER_STATION, data);
        let Ok(length) = frame.encode_into(&mut wire) else {
            self.summary.error();
            return;
        };
        for &octet in wire.iter().take(length) {
            self.now += self.octet;
            if self.core.on_octet(octet, self.now).is_err() {
                self.summary.error();
            }
        }
        self.drain();
    }

    /// Station 2 passes the router the token, and the core sends what it
    /// has queued, then passes the token back: at the first token, after a
    /// poll for the next master that station 2 answers.
    fn token(&mut self) {
        self.last_frame = None;
        self.mstp_in(FrameType::TOKEN, STATION, &[]);
        for _ in 0..MAX_ACTIONS {
            match self.last_frame {
                Some(FrameType::TOKEN) => return,
                Some(FrameType::POLL_FOR_MASTER) => {
                    self.last_frame = None;
                    self.mstp_in(FrameType::REPLY_TO_POLL_FOR_MASTER, STATION, &[]);
                }
                _ => self.wait(),
            }
        }
        self.summary.error();
    }

    /// Wait through the core's deadlines until it has sent a frame of
    /// `frame_type`.
    fn until(&mut self, frame_type: FrameType) {
        for _ in 0..MAX_ACTIONS {
            if self.last_frame == Some(frame_type) {
                return;
            }
            self.wait();
        }
        self.summary.error();
    }

    /// Let time run to the core's next deadline, service it, and carry out
    /// what it asks.
    fn wait(&mut self) {
        let Some(deadline) = self.core.next_deadline() else {
            self.summary.error();
            return;
        };
        self.now = self.now.max(deadline);
        if self.core.on_deadline(self.now).is_err() {
            self.summary.error();
        }
        self.drain();
    }

    /// Poll the core until it has no action, carrying out each: a frame
    /// goes on the line at once and takes its octets' time; an NPDU goes
    /// to the router, whose sends are carried out, and a request in it is
    /// released, for the core to answer Reply Postponed.
    fn drain(&mut self) {
        for _ in 0..MAX_ACTIONS {
            let then = match self.core.poll_action() {
                MstpAction::Idle => return,
                MstpAction::SetDirection(_) => Then::Nothing,
                MstpAction::Transmit { id, frame } => {
                    wired(self.summary, frame);
                    self.last_frame = frame.get(2).copied().map(FrameType::from_raw);
                    self.last_sent_at = self.now;
                    Then::Send {
                        id,
                        octets: frame.len(),
                    }
                }
                MstpAction::DeliverNpdu {
                    source,
                    destination,
                    npdu,
                    request_id,
                } => {
                    let arrived = MSTP.arrived(source, destination, npdu);
                    let received = self
                        .router
                        .receive(arrived.frame(), self.now, &mut self.host);
                    note(self.summary, received.into());
                    Then::Release(request_id)
                }
                MstpAction::DeliverFrame { .. } => {
                    self.summary.error();
                    Then::Nothing
                }
            };
            match then {
                Then::Nothing => {}
                Then::Send { id, octets } => {
                    if self.core.on_transmit_started(id, self.now).is_err() {
                        self.summary.error();
                    }
                    let octets = u32::try_from(octets).unwrap_or(u32::MAX);
                    self.now += self.octet.saturating_mul(octets);
                    if self.core.on_transmit_complete(id, self.now).is_err() {
                        self.summary.error();
                    }
                }
                Then::Release(request) => {
                    if MstpPort::release(self.core, request, self.now).is_err() {
                        self.summary.error();
                    }
                    self.carry();
                }
            }
        }
        self.summary.error();
    }
}

/// Room for the frames station 2 sends, all short.
const MSTP_WIRE: usize = 64;

/// The router's counters, returned from a function kept out of line. A
/// copy straight from the router's storage into the summary, which the
/// compiler can't prove apart from it, links a memmove call; the value this
/// returns lands in a slot of the caller's that overlaps nothing.
#[inline(never)]
fn counters(router: &RouterEngine<'_>) -> Counters {
    router.counters()
}

/// Note what the router made of a frame.
fn note(summary: &mut Summary, outcome: Outcome) {
    record(&mut summary.received, outcome, &mut summary.errors);
}

/// Note a frame the MS/TP core sends: its type, destination and length,
/// and its octets in the tally.
fn wired(summary: &mut Summary, frame: &[u8]) {
    let (Some(&raw_type), Some(&destination)) = (frame.get(2), frame.get(3)) else {
        summary.error();
        return;
    };
    summary.mstp.frame(frame);
    let length = u16::try_from(frame.len()).unwrap_or(u16::MAX);
    let entry = (FrameType::from_raw(raw_type), destination, length);
    record(&mut summary.frames, entry, &mut summary.errors);
}

/// Put `value` in the first free slot of `slots`, or count an error when
/// none is free.
fn record<T: Copy>(slots: &mut [Option<T>], value: T, errors: &mut u8) {
    match slots.iter_mut().find(|slot| slot.is_none()) {
        Some(slot) => *slot = Some(value),
        None => *errors = errors.saturating_add(1),
    }
}
