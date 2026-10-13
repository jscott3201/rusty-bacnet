//! The router engine: Clause 6 routing between ports, with no I/O.
//!
//! A [`Router`] owns the [`Table`], the [`Discovery`] state, the counters
//! and one output scratch, all in a [`Storage`] the caller provides. It
//! never reads a clock or touches a link: the caller hands it each frame a
//! port receives with [`Router::receive`], the time with every call, and
//! carries out what [`Router::poll_action`] asks for. After every call,
//! poll until [`Action::Idle`]; a call made with actions still waiting drops
//! them and counts them in [`Counters::undrained`].
//!
//! A `now` earlier than the latest one passed in is taken as that latest
//! one: time never runs backwards, so a clock that steps back holds the
//! timers still instead of failing. The router keeps that latest time, from
//! [`Router::new`] on, for [`receive`](Router::receive) and
//! [`on_deadline`](Router::on_deadline).
//!
//! To the router a port is data, a [`PortConfig`]: its network number, the
//! router's MAC on it and the longest NPDU it carries. The link type never
//! matters, so firmware joins B/IP and MS/TP ports through it, and the
//! hosted router any transport it has.
//!
//! [`receive`](Router::receive) follows the hosted router's order step for
//! step, so each frame is dropped, answered or relayed for the same reason
//! and counted the same way:
//!
//! 1. The link source and the NPDU's addresses are bounded to
//!    [`MAX_MAC_LEN`](crate::MAX_MAC_LEN); an over-long DLEN or SLEN with a
//!    DNET draws reject reason 6.
//! 2. A DADR beside DNET 0xFFFF, and a broadcast carrying anything but an
//!    Unconfirmed-Request, are dropped.
//! 3. A network message is routed first when it names another network,
//!    except the three that are always handled here (What-Is, Number-Is
//!    and Reject); otherwise its type's handler takes it.
//! 4. An APDU with no DNET is delivered locally. A global broadcast is
//!    passed on to every other port and delivered. Any other DNET is
//!    looked up: no route starts a search and draws reason 1, a busy route
//!    reason 2 and an unreachable one reason 1. A DADR that is the
//!    router's own address, a port's network with the router's MAC on that
//!    port, is delivered locally before any route is looked up. On the
//!    arrival network, a DLEN 0 broadcast is delivered and also sent back
//!    out the same port.
//! 5. The NPDU is rewritten (Clause 6.5.4) and checked against its egress
//!    port's limit, then sent: SNET and SADR added when missing; onto a
//!    directly attached network without its DNET, DADR and hop count, to
//!    the DADR or as a broadcast; toward the next router with its hop count
//!    one lower. One that arrives with a hop count of 0 goes nowhere, and
//!    toward the next router neither does one that arrives with 1.
//!
//! The [`Host`] answers what the router can't know itself: which MACs are
//! group addresses on a port ([`Group`]), and whether to act on a
//! protected network message.

mod config;
mod control;
mod counters;
mod forward;
mod host;
mod ingress;
mod init;
mod output;
mod reject;
mod tasks;

use core::borrow::BorrowMut;
use core::fmt;

pub use config::{ConfigError, Limits, Options, PortConfig};
pub use counters::Counters;
pub use host::{Claim, ControlClass, Event, Group, Host, Permissive, Targets};
pub use ingress::{Discard, Frame, Local, Received};
pub use output::{Action, Cause, Dest, Kind, Send, SendRefusal};

use counters::bump;
use output::Outputs;
use tasks::Task;

use crate::discovery::Discovery;
use crate::table::Table;
use crate::{Instant, PortId, MAX_NPDU};

/// The NPDU limits a port may have: at least this.
pub(crate) const MIN_LIMIT: u16 = Limits::MIN;

/// Everything large a [`Router`] keeps: up to `P` ports, a table of `R`
/// routes and `H` hold-down records, `D` searches, and an output scratch of
/// `N` octets.
///
/// [`new`](Self::new) is a `const fn` whose result is all zero bits, so
/// firmware keeps it in a zero-initialized static (`.bss`) and lends it to
/// the router; a host can box it. `P` is at most 255 and `N` at least
/// [`MAX_NPDU`], both checked when the program compiles:
///
/// ```
/// let _ = bacnet_router_core::Storage::<255, 8, 8, 4, 1497>::new();
/// ```
///
/// ```compile_fail
/// // 256 ports: a wire Port ID can't name the last.
/// let _ = bacnet_router_core::Storage::<256, 8, 8>::new();
/// ```
///
/// ```compile_fail
/// // A scratch too small for the largest B/IP or MS/TP NPDU.
/// let _ = bacnet_router_core::Storage::<2, 8, 8, 4, 1496>::new();
/// ```
pub struct Storage<
    const P: usize,
    const R: usize,
    const H: usize,
    const D: usize = 16,
    const N: usize = MAX_NPDU,
> {
    ports: [PortConfig; P],
    port_count: usize,
    table: Table<R, H>,
    discovery: Discovery<D>,
    scratch: [u8; N],
    out: Outputs,
    counters: Counters,
}

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Storage<P, R, H, D, N>
{
    /// Empty storage: all zero bits.
    #[must_use]
    pub const fn new() -> Self {
        const { assert!(P <= 255, "a router has at most 255 ports") };
        const { assert!(N >= MAX_NPDU, "the scratch holds at least MAX_NPDU") };
        Self {
            ports: [PortConfig::ZERO; P],
            port_count: 0,
            table: Table::new(),
            discovery: Discovery::new(),
            scratch: [0; N],
            out: Outputs::NEW,
            counters: Counters::ZERO,
        }
    }
}

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize> Default
    for Storage<P, R, H, D, N>
{
    fn default() -> Self {
        Self::new()
    }
}

/// The ports and state kept, not the scratch.
impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize> fmt::Debug
    for Storage<P, R, H, D, N>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Storage")
            .field("ports", &&self.ports[..self.port_count])
            .field("table", &self.table)
            .field("discovery", &self.discovery)
            .field("pending", &self.out.pending(self.port_count))
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

/// A sans-IO BACnet router over the [`Storage`] it borrows or owns (`S` is
/// `&mut Storage<..>` in firmware, a `Box<Storage<..>>` on a host). `D` and
/// `N` default as [`Storage`]'s do.
pub struct Router<
    S,
    const P: usize,
    const R: usize,
    const H: usize,
    const D: usize = 16,
    const N: usize = MAX_NPDU,
> {
    storage: S,
    options: Options,
    next_sweep: Instant,
    /// The latest time any call has passed in.
    latest: Instant,
}

impl<S, const P: usize, const R: usize, const H: usize, const D: usize, const N: usize> fmt::Debug
    for Router<S, P, R, H, D, N>
where
    S: BorrowMut<Storage<P, R, H, D, N>>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Router")
            .field("storage", self.storage.borrow())
            .field("options", &self.options)
            .field("next_sweep", &self.next_sweep)
            .field("latest", &self.latest)
            .finish()
    }
}

impl<S, const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Router<S, P, R, H, D, N>
where
    S: BorrowMut<Storage<P, R, H, D, N>>,
{
    /// A router on `ports`, in that order (port `n` is [`PortId`]`(n)`),
    /// with each port's network installed as a direct route, the table's
    /// convergence set from `options`, and the first sweep due
    /// `options.sweep` after `now`. Whatever `storage` held is cleared.
    ///
    /// Refused: no port; more than `P` or 255; two ports with one network
    /// number; a limit under 64 octets or over the scratch; a route cap
    /// over `R`; a table too small for every port's network; a zero sweep.
    /// A port numbered 0 or 0xFFFF is taken, with no direct route, as the
    /// hosted router takes one.
    pub fn new(
        mut storage: S,
        ports: &[PortConfig],
        options: Options,
        now: Instant,
    ) -> Result<Self, ConfigError> {
        if ports.is_empty() {
            return Err(ConfigError::NoPorts);
        }
        if ports.len() > P.min(255) {
            return Err(ConfigError::TooManyPorts {
                count: ports.len(),
                capacity: P.min(255),
            });
        }
        for (index, port) in ports.iter().enumerate() {
            if ports[..index]
                .iter()
                .any(|other| other.network == port.network)
            {
                return Err(ConfigError::DuplicateNetwork(port.network));
            }
            let limits = [port.max_npdu.unicast, port.max_npdu.broadcast];
            if limits
                .iter()
                .any(|&limit| limit < MIN_LIMIT || usize::from(limit) > N)
            {
                // At most 255 ports.
                return Err(ConfigError::Limit(PortId(index as u8)));
            }
        }
        if usize::from(options.route_cap) > R {
            return Err(ConfigError::RouteCap {
                cap: options.route_cap,
                capacity: R,
            });
        }
        if options.sweep.as_nanos() == 0 {
            return Err(ConfigError::ZeroSweep);
        }

        let store = storage.borrow_mut();
        store.table.clear();
        store.table.set_convergence(options.convergence);
        store.discovery.clear();
        store.out = Outputs::NEW;
        store.counters = Counters::ZERO;
        store.ports[..ports.len()].copy_from_slice(ports);
        store.port_count = ports.len();
        for (index, port) in ports.iter().enumerate() {
            let reserved = port.network == 0 || port.network == 0xFFFF;
            if !store.table.add_direct(port.network, PortId(index as u8)) && !reserved {
                return Err(ConfigError::TableTooSmall);
            }
        }
        Ok(Self {
            storage,
            options,
            next_sweep: now + options.sweep,
            latest: now,
        })
    }

    fn store(&mut self) -> &mut Storage<P, R, H, D, N> {
        self.storage.borrow_mut()
    }

    /// `now`, or the latest time passed in when that is later, which it
    /// then stays (the clock rule in the module docs).
    fn clock(&mut self, now: Instant) -> Instant {
        self.latest = self.latest.max(now);
        self.latest
    }

    /// Queue the startup announcements, port by port: a Network-Number-Is
    /// for a port with a usable number, then an I-Am-Router listing every
    /// other port's network (none on a one-port router). They carry no
    /// ingress data.
    pub fn start(&mut self) {
        let store = self.store();
        store.drop_undrained();
        store.out.push(Task::announce());
    }

    /// Take a frame that port `frame.port` received at `now`. What it means
    /// for the router's own application comes back at once; what to send is
    /// polled after. A `now` before the latest one passed in is taken as
    /// that one.
    pub fn receive<'f>(
        &mut self,
        frame: Frame<'f>,
        now: Instant,
        host: &mut impl Host,
    ) -> Received<'f> {
        let now = self.clock(now);
        let options = self.options;
        self.store().receive(frame, now, &options, host)
    }

    /// The next thing to do. Its NPDU stays in the router's scratch until
    /// the next poll.
    pub fn poll_action(&mut self) -> Action<'_> {
        self.store().poll()
    }

    /// Say that the port refused the send just polled. It is counted, and
    /// it ends a table answer (Initialize-Routing-Table-Ack) that still has
    /// Acks to send, as a full queue does in the hosted router.
    pub fn refused(&mut self, why: SendRefusal) {
        let store = self.store();
        if why == SendRefusal::TooLong {
            bump(&mut store.counters.egress_too_long);
        }
        store.refused();
    }

    /// When [`on_deadline`](Self::on_deadline) next has work: the next
    /// sweep.
    ///
    /// Only `on_deadline` moves it, so a host may sleep until it without
    /// any other call waking that sleep (the hosted router's sweep task
    /// does). A change that lets another call move it must say so here,
    /// since such a host would then need a wake after that call.
    pub fn next_deadline(&self) -> Option<Instant> {
        Some(self.next_sweep)
    }

    /// Run the sweep, if it is due: purge learned routes not confirmed
    /// within `learned_age`, clear lapsed busy marks, and drop pending
    /// hardened-mode claims and searches past their age, telling `host`
    /// about each. Busy marks are read at their deadline anyway, so when
    /// the sweep runs never changes a forwarding decision. The next sweep
    /// is due a period later; one that is already past is skipped. A `now`
    /// before the latest one passed in is taken as that one.
    pub fn on_deadline(&mut self, now: Instant, host: &mut impl Host) {
        let now = self.clock(now);
        if now < self.next_sweep {
            return;
        }
        let options = self.options;
        let store = self.storage.borrow_mut();
        store
            .table
            .purge_stale(now, options.learned_age, |network| {
                host.observe(Event::Purged { network });
            });
        store.table.clear_expired_busy(now);
        store.table.expire_pending(now, &options.timing, |network| {
            host.observe(Event::PendingExpired { network });
        });
        store
            .discovery
            .expire(now, options.discovery_age, |network| {
                host.observe(Event::DiscoveryExpired { network });
            });
        self.next_sweep += options.sweep;
        if self.next_sweep <= now {
            self.next_sweep = now + options.sweep;
        }
    }

    /// Stop looking for routes to unknown networks, for good: the router is
    /// stopping. Messages for them are still refused with reason 1.
    pub fn cancel_discovery(&mut self) {
        self.store().discovery.cancel();
    }

    /// The routing table.
    pub fn table(&self) -> &Table<R, H> {
        &self.storage.borrow().table
    }

    /// The routing table, to change locally. Changes made here aren't
    /// network messages, so no [`Host::authorize`] sees them.
    pub fn table_mut(&mut self) -> &mut Table<R, H> {
        &mut self.store().table
    }

    /// The searches for unknown networks.
    pub fn discovery(&self) -> &Discovery<D> {
        &self.storage.borrow().discovery
    }

    /// The router's counts so far.
    pub fn counters(&self) -> Counters {
        self.storage.borrow().counters
    }

    /// The ports, by index.
    pub fn ports(&self) -> &[PortConfig] {
        let store = self.storage.borrow();
        &store.ports[..store.port_count]
    }

    /// The options.
    pub const fn options(&self) -> &Options {
        &self.options
    }
}
