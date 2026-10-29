//! A router between B/IP and MS/TP, with the router as the B/IP network's
//! BBMD, run the way firmware runs it, on a host (#1771).
//!
//! ```text
//! cargo run -p bacnet-router-core --features bip,mstp --example bip_mstp_router
//! ```
//!
//! Three portable cores share one injected clock: the router, the MS/TP
//! master core behind its MS/TP port, and the BBMD's tables behind its
//! B/IP port. The loop is the one a firmware main loop runs:
//!
//! 1. Merge every deadline with `bacnet_time::earliest`, which skips a
//!    `None`. `Option::min` would pick the `None` of a core with nothing
//!    due, and every timer would stall.
//! 2. Move the clock there (here the clock is virtual, so the example
//!    jumps; firmware sleeps until a timer interrupt or a received octet).
//! 3. Hand each core whose deadline came its `on_deadline`, then carry out
//!    whatever every core asks for, until all are idle.
//!
//! The BBMD's deadline is the host's: its tables keep no timers, so the
//! loop computes when the next foreign-device registration lapses and
//! purges then. The BBMD answers management requests itself: its port
//! passes each to `Tables::serve` and sends the answer.
//!
//! The rest of the world is simulated: a foreign device that registers and
//! reads the BDT, a B/IP node, and MS/TP station 2, which answers polls and
//! passes the token back.

#![allow(clippy::print_stdout)] // an example demonstrates itself by printing

use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_router_core::bip::BipPort;
use bacnet_router_core::bip_core::LocalSubnet;
use bacnet_router_core::bip_core::bbmd::{TWO_HOP_MASK, Tables, Target};
use bacnet_router_core::bip_core::codec::{BdtEntries, BdtEntry, Bvll, FdtEntry};
use bacnet_router_core::bip_core::node::{
    self, Addressed, Datagram, Link, MAX_DATAGRAM, Mode, SendRefusal,
};
use bacnet_router_core::bip_core::port::{BbmdPort, Datagrams, Ingress};
use bacnet_router_core::mstp::{MstpPort, PortAction};
use bacnet_router_core::mstp_core::codec::{
    Frame, FrameType, MAX_EXTENDED_MPDU_DATA, decode_frame,
};
use bacnet_router_core::mstp_core::{
    Action as MstpAction, ExtendedStorage, FixedQueues, MasterCore, MstpCoreConfig,
};
use bacnet_router_core::router::{Action, Event, Host, Options, Received};
use bacnet_router_core::{Duration, Instant, PortId, RouterIn, Storage};
use bacnet_time::earliest;

/// The router's storage: 2 ports, 8 routes, 8 reject hold-downs and 4
/// searches. On firmware it lives in a static.
type RouterStorage = Storage<2, 8, 8, 4>;
/// The router over that storage.
type Router<'m> = RouterIn<'m, 2, 8, 8, 4>;
/// The MS/TP core's storage: one unicast and two broadcast slots sized for
/// extended frames, which Clause 9.3 asks of a router's MS/TP port.
type MstpStorage = ExtendedStorage<1, 2, 1>;
/// The router's MS/TP port, which owns its core.
type Mstp<'m> = MstpPort<'m, &'m mut FixedQueues<1, 2, MAX_EXTENDED_MPDU_DATA>>;
/// The BBMD's tables: 2 BDT rows, its own included, and 2 foreign devices.
type BbmdTables = Tables<2, 2>;

/// The router, and the BBMD, on its subnet.
const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 1), 0xBAC0);
/// The subnet's broadcast address.
const SUBNET_BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), 0xBAC0);
/// A peer BBMD on another subnet, reached by two-hop distribution.
const PEER_BBMD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 1), 0xBAC0);
/// A B/IP node on the subnet.
const NODE: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 0xBAC0);
/// A foreign device, which registers with the BBMD.
const FOREIGN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(172, 16, 0, 5), 0xBAC0);

/// Port 0, B/IP, network 1; port 1, MS/TP, network 2.
const BIP: BipPort = BipPort::new(PortId(0));
const MSTP: PortId = PortId(1);
const BIP_NETWORK: u16 = 1;
const MSTP_NETWORK: u16 = 2;

/// MS/TP station 1 of masters 0 to 2 at 9600 baud, with extended frames.
const CONFIG: MstpCoreConfig = {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = 1;
    config.max_master = 2;
    config.with_extended_frames()
};
/// The other master on the line.
const STATION_2: u8 = 2;

/// The node's Who-Is for station 5 on network 2.
const WHO_IS_STATION_5: [u8; 9] = [0x01, 0x20, 0x00, 0x02, 0x01, 0x05, 0xFF, 0x10, 0x08];
/// Station 2's Who-Is for every device on every network.
const GLOBAL_WHO_IS: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// How long the example runs: past the foreign device's lapse.
const RUN_FOR: Duration = Duration::from_secs(40);

/// What the simulated world does, and when.
enum Step {
    /// The foreign device registers for 5 seconds; with the 30-second grace
    /// period of Annex J.5.2.3 the BBMD purges it 35 seconds later.
    Register,
    /// It reads the BDT.
    ReadBdt,
    /// The B/IP node looks for station 5 on network 2.
    NodeWhoIs,
    /// Station 2 has a global Who-Is to send when it next holds the token.
    StationWhoIs,
}

const SCRIPT: [(u64, Step); 4] = [
    (1_000, Step::Register),
    (1_500, Step::ReadBdt),
    (2_000, Step::NodeWhoIs),
    (3_000, Step::StationWhoIs),
];

/// The router's host: it prints what the router reports.
struct Log;

impl Host for Log {
    fn observe(&mut self, event: Event) {
        println!("          router event: {event:?}");
    }
}

/// The B/IP socket: it encodes each datagram and prints where it goes.
struct Socket<'b> {
    now: Instant,
    buffer: &'b mut [u8],
}

impl Datagrams for Socket<'_> {
    fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal> {
        let length = message
            .encode_into(self.buffer)
            .map_err(|_| SendRefusal::SocketError)?;
        println!(
            "{:>9} B/IP   -> {to} {:?} ({length} octets)",
            stamp(self.now),
            message.function()
        );
        Ok(())
    }
}

/// MS/TP station 2, simulated: the frame it sends next and when, and a
/// data frame it holds until it has the token.
struct Station2 {
    next: Option<(Instant, FrameType, &'static [u8])>,
    outbox: Option<&'static [u8]>,
    tokens: u32,
}

/// Everything the loop drives.
struct World<'m> {
    now: Instant,
    router: Router<'m>,
    bbmd: BbmdPort<'m, 2, 2>,
    mstp: Mstp<'m>,
    station_2: Station2,
    datagram_in: [u8; MAX_DATAGRAM],
    datagram_out: [u8; MAX_DATAGRAM],
}

impl World<'_> {
    /// When the BBMD should next purge its foreign-device table: just past
    /// the earliest lapse, since a registration is live at exactly its
    /// lifetime.
    fn next_purge(&self) -> Option<Instant> {
        let registrations = self.bbmd.tables().fdt.registrations();
        earliest(registrations.iter().map(|registration| {
            Some(registration.registered_at + registration.lifetime() + Duration::from_nanos(1))
        }))
    }

    /// Run until the end, one deadline at a time.
    fn run(&mut self) {
        self.router.start();
        self.carry();
        let mut script = SCRIPT.iter().peekable();
        let end = Instant::ZERO + RUN_FOR;
        loop {
            let step_at = script.peek().map(|(at, _)| Instant::from_millis(*at));
            let deadlines = [
                self.router.next_deadline(),
                self.mstp.core().next_deadline(),
                self.next_purge(),
                self.station_2.next.map(|(at, ..)| at),
                step_at,
            ];
            let Some(next) = earliest(deadlines).filter(|&next| next < end) else {
                break;
            };
            self.now = self.now.max(next);
            if step_at.is_some_and(|at| at <= self.now)
                && let Some((_, step)) = script.next()
            {
                self.step(step);
            }
            if self.station_2.next.is_some_and(|(at, ..)| at <= self.now) {
                self.station_2_sends();
            }
            if self
                .mstp
                .core()
                .next_deadline()
                .is_some_and(|at| at <= self.now)
            {
                self.mstp.core_mut().on_deadline(self.now);
                self.drain();
            }
            if self.router.next_deadline().is_some_and(|at| at <= self.now) {
                self.router.on_deadline(self.now, &mut Log);
                self.carry();
            }
            if self.next_purge().is_some_and(|at| at <= self.now) {
                let purged = self.bbmd.tables_mut().fdt.purge(self.now);
                println!(
                    "{:>9} BBMD purged {purged} lapsed registration(s)",
                    stamp(self.now)
                );
            }
        }
        let counters = self.mstp.core_mut().take_counters();
        println!(
            "MS/TP: {} frames sent, {} received, {} tokens from station 2; router: {} routes",
            counters.frames_transmitted,
            counters.frames_received,
            self.station_2.tokens,
            self.router.table().len()
        );
    }

    /// One step of the script.
    fn step(&mut self, step: &Step) {
        match step {
            Step::Register => self.bip_in(&Bvll::RegisterForeignDevice { ttl: 5 }, FOREIGN),
            Step::ReadBdt => self.bip_in(&Bvll::ReadBroadcastDistributionTable, FOREIGN),
            Step::NodeWhoIs => self.bip_in(&Bvll::OriginalUnicastNpdu(&WHO_IS_STATION_5), NODE),
            Step::StationWhoIs => self.station_2.outbox = Some(&GLOBAL_WHO_IS),
        }
    }

    /// The B/IP socket receives `message` from `source`, sent to the
    /// router's own address: the BBMD port takes it first, and an NPDU in
    /// it goes on to the router.
    fn bip_in(&mut self, message: &Bvll<'_>, source: SocketAddrV4) {
        let Ok(length) = message.encode_into(&mut self.datagram_in) else {
            return;
        };
        println!(
            "{:>9} B/IP   <- {source} {:?}",
            stamp(self.now),
            message.function()
        );
        let to = Addressed {
            node: true,
            broadcast: false,
        };
        let datagram = Datagram::new(&self.datagram_in[..length], source, Some(to));
        let mut socket = Socket {
            now: self.now,
            buffer: &mut self.datagram_out,
        };
        match self
            .bbmd
            .receive(&datagram, self.now, &mut node::Permissive, &mut socket)
        {
            Ingress::Npdu(delivery) => {
                let arrived = BIP.arrived(&delivery);
                let received = self.router.receive(arrived.frame(), self.now, &mut Log);
                report(self.now, &received);
            }
            Ingress::Served(served) => {
                println!("          BBMD answered from its tables: {served:?}");
            }
            other => println!("          B/IP port: {other:?}"),
        }
        self.carry();
        self.drain();
    }

    /// Poll the router until it is idle, handing each send to its port.
    fn carry(&mut self) {
        loop {
            let send = match self.router.poll_action() {
                Action::Idle => return,
                Action::Send(send) => send,
            };
            let result = if send.port == BIP.port() {
                let mut socket = Socket {
                    now: self.now,
                    buffer: &mut self.datagram_out,
                };
                BIP.send(&mut self.bbmd, &send, self.now, &mut socket)
            } else {
                self.mstp.send(&send, self.now)
            };
            if let Err(why) = result {
                self.router.refused(why);
            }
        }
    }

    /// Poll the MS/TP port until its core is idle: a frame goes on the line
    /// at once, taking its octets' time, and an NPDU goes to the router.
    fn drain(&mut self) {
        loop {
            match self.mstp.poll_action() {
                PortAction::Core(MstpAction::Idle) => return,
                PortAction::Core(MstpAction::Transmit { id, .. }) => {
                    // The octets come with the confirmed start.
                    let Ok(frame) = self.mstp.core_mut().on_transmit_started(id, self.now) else {
                        continue;
                    };
                    let length = frame.len();
                    let header = decode_frame(frame).ok().map(|(frame, _)| frame.header);
                    let octets = u32::try_from(length).unwrap_or(u32::MAX);
                    self.now += CONFIG.octet_time().saturating_mul(octets);
                    self.mstp.core_mut().on_transmit_complete(id, self.now);
                    if let Some(header) = header {
                        self.station_2_hears(header.frame_type, header.destination, length);
                    }
                }
                PortAction::Arrived { arrived, request } => {
                    let received = self.router.receive(arrived.frame(), self.now, &mut Log);
                    report(self.now, &received);
                    let _ = self.mstp.release(request, self.now);
                    self.carry();
                }
                PortAction::Core(_) => {}
            }
        }
    }

    /// Station 2 hears a frame the router sent, and answers one meant for
    /// it after the turnaround the line needs.
    fn station_2_hears(&mut self, frame_type: FrameType, destination: u8, length: usize) {
        if frame_type != FrameType::TOKEN {
            println!(
                "{:>9} MS/TP  -> {destination:<3} {frame_type:?} ({length} octets)",
                stamp(self.now)
            );
        }
        if destination != STATION_2 {
            return;
        }
        let answer = if frame_type == FrameType::POLL_FOR_MASTER {
            Some((FrameType::REPLY_TO_POLL_FOR_MASTER, &[][..]))
        } else if frame_type == FrameType::TOKEN {
            self.station_2.tokens += 1;
            Some(match self.station_2.outbox.take() {
                Some(npdu) => (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, npdu),
                None => (FrameType::TOKEN, &[][..]),
            })
        } else {
            None
        };
        self.station_2.next =
            answer.map(|(kind, data)| (self.now + CONFIG.t_turnaround, kind, data));
    }

    /// Station 2 sends its frame, octet by octet. After a data frame it
    /// passes the token back.
    fn station_2_sends(&mut self) {
        let Some((_, frame_type, data)) = self.station_2.next.take() else {
            return;
        };
        let destination = if frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
            println!("{:>9} MS/TP  <- station 2 {frame_type:?}", stamp(self.now));
            0xFF
        } else {
            CONFIG.station
        };
        let mut wire = [0u8; 64];
        let frame = Frame::new(frame_type, destination, STATION_2, data);
        let Ok(length) = frame.encode_into(&mut wire) else {
            return;
        };
        for &octet in &wire[..length] {
            self.now += CONFIG.octet_time();
            self.mstp.core_mut().on_octet(octet, self.now);
        }
        self.drain();
        if frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
            self.station_2.next = Some((self.now + CONFIG.t_turnaround, FrameType::TOKEN, &[]));
        }
    }
}

/// Print what the router made of a frame, when it is more than routing.
fn report(now: Instant, received: &Received<'_>) {
    match received {
        Received::Handled => {}
        Received::Local(local) => println!(
            "{:>9} router: for its own application (global broadcast: {})",
            stamp(now),
            local.global
        ),
        other => println!("{:>9} router: {other:?}", stamp(now)),
    }
}

/// `at` in seconds, to the millisecond, for the log.
fn stamp(at: Instant) -> String {
    let millis = at.as_nanos() / 1_000_000;
    format!("{}.{:03} s", millis / 1_000, millis % 1_000)
}

fn main() {
    let mut subnet = LocalSubnet::new(SUBNET_BROADCAST);
    subnet.mask = Some(Ipv4Addr::new(255, 255, 255, 0));
    let link = Link::new(OWN, subnet, Mode::Bbmd).expect("a valid link");

    let mut tables = BbmdTables::new(OWN);
    let peers = [BdtEntry::new(PEER_BBMD, TWO_HOP_MASK)];
    tables
        .bdt
        .set(BdtEntries::from_entries(&peers))
        .expect("room in the BDT");
    let mut targets = [Target::UNUSED; BbmdTables::MAX_TARGETS];
    let mut rows = [FdtEntry::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), 0, 0); 2];
    let bbmd = BbmdPort::new(link, &mut tables, &mut targets, &mut rows).expect("a BBMD's link");

    let mut mstp_storage = MstpStorage::new();
    let core = MasterCore::new(CONFIG, &mut mstp_storage, Instant::ZERO).expect("a valid config");
    let mstp = MstpPort::new(MSTP, core).expect("extended frames are on");

    let ports = [
        BipPort::config(&link, BIP_NETWORK),
        mstp.port_config(MSTP_NETWORK),
    ];
    let mut router_storage = RouterStorage::new();
    let router = Router::new(
        &mut router_storage,
        &ports,
        Options::STANDARD,
        Instant::ZERO,
    )
    .expect("valid ports");

    World {
        now: Instant::ZERO,
        router,
        bbmd,
        mstp,
        station_2: Station2 {
            next: None,
            outbox: None,
            tokens: 0,
        },
        datagram_in: [0; MAX_DATAGRAM],
        datagram_out: [0; MAX_DATAGRAM],
    }
    .run();
}
