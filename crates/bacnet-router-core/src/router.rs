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
//! router's MAC on it, its group-address rule, and its NPDU limits.
//! [`Router::set_port_max_npdu`] replaces those for a link whose limit changes
//! while it runs. The link type never matters, so firmware joins B/IP and
//! MS/TP ports through it, and the hosted router any transport it has.
//!
//! [`receive`](Router::receive) follows the hosted router's order step for
//! step, so each frame is dropped, answered or relayed for the same reason
//! and counted the same way:
//!
//! 1. The link source and the NPDU's addresses are bounded to
//!    [`MAX_MAC_LEN`](crate::MAX_MAC_LEN); an over-long DLEN or SLEN with a
//!    DNET draws reject reason 6.
//! 2. A DADR beside DNET 0xFFFF, and a broadcast carrying anything but an
//!    Unconfirmed-Request, are dropped. Steps 1 and 2 are the checks a
//!    [`Node`](crate::node::Node) makes too, written once in
//!    [`node`](crate::node).
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
//! The [`Host`] authorizes protected network messages and observes events.
//! Each port supplies its own group-address rule through [`PortConfig`].

mod config;
mod congestion;
mod control;
mod counters;
mod duplicates;
mod forward;
mod groups;
mod host;
mod ingress;
mod init;
mod originate;
mod output;
mod reject;
mod reply;
mod tasks;

use core::borrow::BorrowMut;
use core::fmt;

pub use config::{ConfigError, LimitError, Options, PortConfig, MIN_PORT_LIMIT};
pub use congestion::{Backlog, Congestion};
pub use counters::Counters;
pub use duplicates::Duplicates;
pub use groups::{Group, GroupClassifier, Groups};
pub use host::{Claim, ControlClass, Event, Host, Permissive, Targets};
pub use ingress::{Discard, Frame, Local, Received};
pub use originate::{OriginateError, Request, SourceSelection};
pub use output::{Action, Cause, Dest, Kind, Send, SendRefusal};
pub use reject::RejectOrigin;
pub use reply::{ReplyPlan, ReplyPlanError};

use congestion::Flow;
pub(crate) use counters::bump;
use duplicates::Recent;
use output::Outputs;
use tasks::Task;

use crate::discovery::Discovery;
use crate::npdu::{NpduLimits, GLOBAL_NETWORK};
use crate::table::Table;
use crate::{Instant, PortId, MAX_NPDU};

/// Everything large a [`Router`] keeps: up to `P` ports, a table of `R`
/// routes and `H` hold-down records, `D` searches, an output scratch of
/// `N` octets, and a record of the last `B` global broadcasts for
/// [`Options::duplicates`] (#1698), 24 octets each, kept whether or not
/// that is on (`B` of 0 keeps none, and so suppresses nothing).
///
/// With the default [`Groups`], [`new`](Self::new) is a `const fn` whose
/// result is all zero bits. Firmware keeps it in a zero-initialized static
/// (`.bss`) and lends it to
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
    G = Groups,
    const B: usize = 32,
> {
    ports: [PortConfig<G>; P],
    /// Each port's congestion state (#1697).
    flows: [Flow; P],
    port_count: usize,
    table: Table<R, H>,
    discovery: Discovery<D>,
    scratch: [u8; N],
    out: Outputs,
    counters: Counters,
    /// The last global broadcasts seen (#1698).
    recent: Recent<B>,
    /// When the next sweep is due. Kept here rather than in the
    /// [`Router`], with the latest time, so the router itself stays small
    /// on a firmware stack.
    next_sweep: Instant,
    /// The latest time any call has passed in.
    latest: Instant,
}

impl<
        const P: usize,
        const R: usize,
        const H: usize,
        const D: usize,
        const N: usize,
        G: GroupClassifier,
        const B: usize,
    > Storage<P, R, H, D, N, G, B>
{
    /// Empty storage: all zero bits with the default [`Groups`] rule.
    /// Custom rules supply their own unused-slot value through [`GroupClassifier::NONE`].
    #[must_use]
    pub const fn new() -> Self {
        const { assert!(P <= 255, "a router has at most 255 ports") };
        const { assert!(N >= MAX_NPDU, "the scratch holds at least MAX_NPDU") };
        Self {
            ports: [const { PortConfig::ZERO }; P],
            flows: [Flow::CLEAR; P],
            port_count: 0,
            table: Table::new(),
            discovery: Discovery::new(),
            scratch: [0; N],
            out: Outputs::NEW,
            counters: Counters::ZERO,
            recent: Recent::NEW,
            next_sweep: Instant::ZERO,
            latest: Instant::ZERO,
        }
    }
}

impl<
        const P: usize,
        const R: usize,
        const H: usize,
        const D: usize,
        const N: usize,
        G: GroupClassifier,
        const B: usize,
    > Default for Storage<P, R, H, D, N, G, B>
{
    fn default() -> Self {
        Self::new()
    }
}

/// The ports and state kept, not the scratch.
impl<
        const P: usize,
        const R: usize,
        const H: usize,
        const D: usize,
        const N: usize,
        G: GroupClassifier + fmt::Debug,
        const B: usize,
    > fmt::Debug for Storage<P, R, H, D, N, G, B>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Storage")
            .field("ports", &&self.ports[..self.port_count])
            .field("table", &self.table)
            .field("discovery", &self.discovery)
            .field("pending", &self.out.pending(self.port_count))
            .field("counters", &self.counters)
            .field("next_sweep", &self.next_sweep)
            .field("latest", &self.latest)
            .finish_non_exhaustive()
    }
}

/// A sans-IO BACnet router over the [`Storage`] it borrows or owns (`S` is
/// `&mut Storage<..>` in firmware, a `Box<Storage<..>>` on a host). `D` and
/// `N` default as [`Storage`]'s do. `G` is the ports' owned group rule,
/// [`Groups`] unless a custom link supplies another [`GroupClassifier`],
/// and `B`, the broadcasts remembered, defaults as [`Storage`]'s does.
pub struct Router<
    S,
    const P: usize,
    const R: usize,
    const H: usize,
    const D: usize = 16,
    const N: usize = MAX_NPDU,
    G = Groups,
    const B: usize = 32,
> {
    storage: S,
    groups: core::marker::PhantomData<G>,
    options: Options,
}

/// A [`Router`] over [`Storage`] it borrows, as firmware runs one: the
/// storage's sizes are said once, in the alias.
///
/// ```
/// use bacnet_router_core::npdu::{Mac, NpduLimits};
/// use bacnet_router_core::router::{Groups, Options, PortConfig};
/// use bacnet_router_core::{Instant, RouterIn, Storage};
///
/// /// Two ports, 8 routes and 8 hold-down records.
/// type Engine<'s> = RouterIn<'s, 2, 8, 8>;
///
/// let ports = [
///     PortConfig::new(1, Mac::from_array([1]), NpduLimits::both(501), Groups::None),
///     PortConfig::new(2, Mac::from_array([2]), NpduLimits::both(501), Groups::None),
/// ];
/// let mut storage = Storage::new();
/// let router = Engine::new(&mut storage, &ports, Options::STANDARD, Instant::ZERO)?;
/// assert_eq!(router.table().len(), 2);
/// # Ok::<(), bacnet_router_core::router::ConfigError>(())
/// ```
pub type RouterIn<
    's,
    const P: usize,
    const R: usize,
    const H: usize,
    const D: usize = 16,
    const N: usize = MAX_NPDU,
    const B: usize = 32,
> = Router<&'s mut Storage<P, R, H, D, N, Groups, B>, P, R, H, D, N, Groups, B>;

impl<
        S,
        const P: usize,
        const R: usize,
        const H: usize,
        const D: usize,
        const N: usize,
        G: GroupClassifier + fmt::Debug,
        const B: usize,
    > fmt::Debug for Router<S, P, R, H, D, N, G, B>
where
    S: BorrowMut<Storage<P, R, H, D, N, G, B>>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Router")
            .field("storage", self.storage.borrow())
            .field("options", &self.options)
            .finish()
    }
}

impl<
        S,
        const P: usize,
        const R: usize,
        const H: usize,
        const D: usize,
        const N: usize,
        G: GroupClassifier,
        const B: usize,
    > Router<S, P, R, H, D, N, G, B>
where
    S: BorrowMut<Storage<P, R, H, D, N, G, B>>,
{
    /// A router on `ports`, in that order (port `n` is [`PortId`]`(n)`),
    /// with each port's network installed as a direct route, the table's
    /// convergence set from `options`, and the first sweep due
    /// `options.sweep` after `now`. Whatever `storage` held is cleared.
    ///
    /// Refused: no port; more than `P` or 255; a port numbered 0 or 0xFFFF
    /// ([`PortConfig::network`]); two ports with one network number; a
    /// limit under 64 octets or over the scratch; a route cap over `R`; a
    /// table too small for every port's network; a zero sweep; duplicate
    /// suppression on storage that records none (`B` of 0); a congestion
    /// policy [`Congestion::is_valid`] refuses.
    pub fn new(
        mut storage: S,
        ports: &[PortConfig<G>],
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
            // At most 255 ports.
            let id = PortId(index as u8);
            if port.network == 0 || port.network == GLOBAL_NETWORK {
                return Err(ConfigError::ReservedNetwork {
                    port: id,
                    network: port.network,
                });
            }
            if ports[..index]
                .iter()
                .any(|other| other.network == port.network)
            {
                return Err(ConfigError::DuplicateNetwork(port.network));
            }
            if !config::limits_fit(port.max_npdu, N) {
                return Err(ConfigError::Limit(id));
            }
        }
        if let Some(cap) = options.route_cap {
            if usize::from(cap) > R {
                return Err(ConfigError::RouteCap { cap, capacity: R });
            }
        }
        if options.sweep.as_nanos() == 0 {
            return Err(ConfigError::ZeroSweep);
        }
        if options.duplicates.is_some() && B == 0 {
            return Err(ConfigError::NoDuplicateRecord);
        }
        if options.congestion.is_some_and(|policy| !policy.is_valid()) {
            return Err(ConfigError::Congestion);
        }

        let store = storage.borrow_mut();
        store.table.clear();
        store.table.set_convergence(options.convergence);
        store.discovery.clear();
        store.out = Outputs::NEW;
        store.counters = Counters::ZERO;
        store.recent = Recent::NEW;
        store.next_sweep = now + options.sweep;
        store.latest = now;
        store.ports[..ports.len()].clone_from_slice(ports);
        // Release custom rules from ports a prior router used in this storage.
        store.ports[ports.len()..].fill_with(|| PortConfig::ZERO);
        store.port_count = ports.len();
        store.flows = [Flow::CLEAR; P];
        for (index, port) in ports.iter().enumerate() {
            if !store.table.add_direct(port.network, PortId(index as u8)) {
                return Err(ConfigError::TableTooSmall);
            }
        }
        Ok(Self {
            storage,
            groups: core::marker::PhantomData,
            options,
        })
    }

    fn store(&mut self) -> &mut Storage<P, R, H, D, N, G, B> {
        self.storage.borrow_mut()
    }

    /// `now`, or the latest time passed in when that is later, which it
    /// then stays (the clock rule in the module docs).
    fn clock(&mut self, now: Instant) -> Instant {
        let store = self.store();
        store.latest = store.latest.max(now);
        store.latest
    }

    /// Queue the startup announcements, port by port: a Network-Number-Is
    /// with the port's number, then an I-Am-Router listing every other
    /// port's network (none on a one-port router). They carry no ingress
    /// data.
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

    /// Send a request from the router's own application to another
    /// network, through the routing table (#1699): toward the next router
    /// with its DNET, DADR and a full hop count, or onto a directly attached
    /// network to the DADR, from the address [`Request::source`] selects.
    /// The send is polled as any is, with [`Kind::Originate`] and
    /// [`Cause::Local`], and carries no reject origin: a later refusal
    /// answers no one.
    ///
    /// Refused, sending nothing, for the cases [`OriginateError`] lists: a
    /// network with no route also starts a search for one, polled as a
    /// Who-Is-Router on every port, and a busy or failed route is refused
    /// as the router refuses others' traffic for it. Like
    /// [`receive`](Self::receive), it drops what an earlier call left
    /// unpolled.
    ///
    /// A global broadcast (DNET 0xFFFF) is refused as
    /// [`OriginateError::ReservedNetwork`]: there is no engine path for one
    /// yet. To reach every network, send a remote broadcast (an empty
    /// DADR) to each network in the table, the attached ones included,
    /// where it leaves as that port's local broadcast.
    pub fn originate(&mut self, request: &Request<'_>, now: Instant) -> Result<(), OriginateError> {
        let now = self.clock(now);
        let options = self.options;
        self.store().originate(request, now, &options)
    }

    /// The next thing to do. Its NPDU stays in the router's scratch until
    /// the next poll.
    pub fn poll_action(&mut self) -> Action<'_> {
        self.store().poll()
    }

    /// Say that the port refused the send just polled. It is counted, and
    /// it ends a table answer (Initialize-Routing-Table-Ack) that still has
    /// Acks to send, as a full queue does in the hosted router.
    ///
    /// With [`Options::congestion`] set, a [`SendRefusal::Full`] of a send
    /// that would have left on a clogged port, routed toward a network for
    /// someone else, also queues reject reason 2 to that send's origin
    /// (Clause 6.6.3.6).
    pub fn refused(&mut self, why: SendRefusal) {
        self.refused_later(why, None);
        let congestion = self.options.congestion.is_some();
        let store = self.store();
        store.end_refused_answer();
        if congestion && why == SendRefusal::Full {
            store.refused_while_clogged();
        }
    }

    /// Report port `port`'s egress backlog at `now`, for the router's own
    /// Router-Busy and Router-Available (#1697). Without
    /// [`Options::congestion`] it does nothing.
    ///
    /// Report it whenever the backlog may have crossed the policy's lines:
    /// each time the queue refuses or admits a send, and, while
    /// [`port_congested`](Self::port_congested) says so, each time it lets
    /// one go. A backlog at capacity starts the wait; one still at capacity
    /// [`Congestion::clog_after`] later clogs the port; one under the
    /// low-water mark clears it. Changing state may queue an announcement
    /// for the other ports, polled as any output is.
    ///
    /// Unlike [`receive`](Self::receive) it drops nothing still waiting,
    /// so a caller may report between polls.
    pub fn egress_backlog(&mut self, port: PortId, backlog: Backlog, now: Instant) {
        let now = self.clock(now);
        if let Some(policy) = self.options.congestion {
            self.store().egress_backlog(port, backlog, now, &policy);
        }
    }

    /// Whether port `port` is clogged, or at capacity and waiting to be:
    /// while it is, report its backlog as it drains too.
    #[must_use]
    pub fn port_congested(&self, port: PortId) -> bool {
        let store = self.storage.borrow();
        store.flows[..store.port_count]
            .get(usize::from(port.0))
            .is_some_and(Flow::congested)
    }

    /// Report one definite refusal of a previously polled, queued send.
    /// `origin` is that send's [`Send::reject_origin`], from this router.
    /// Every refusal increments [`Counters::refused`]; [`SendRefusal::TooLong`]
    /// also increments [`Counters::egress_too_long`] and, when eligible,
    /// queues reason 4 back to the retained origin with the original DNET.
    ///
    /// Existing outputs, including a table answer, remain ahead of the new
    /// reject. Poll until [`Action::Idle`] before reporting another refusal.
    /// Sends with no origin record, including forwarded rejects and fan-outs,
    /// produce no reply. A generated reject likewise carries no record, so
    /// its own refusal cannot cause a rejection loop.
    pub fn refused_later(&mut self, why: SendRefusal, origin: Option<RejectOrigin>) {
        let store = self.store();
        if why == SendRefusal::TooLong {
            bump(&mut store.counters.egress_too_long);
            if let Some(origin) = origin {
                store.queue_reject(origin, crate::npdu::RejectMessageReason::MESSAGE_TOO_LONG);
            }
        }
        bump(&mut store.counters.refused);
    }

    /// Replace port `port`'s NPDU limits while the router runs: for a link
    /// whose limit changes after start, such as a BACnet/SC port that fails
    /// over to a hub that carries more or less (#1696).
    ///
    /// Every routing decision after the call goes by the new limits. An
    /// output still waiting to be polled is measured against them when it
    /// is polled, as every send is: one that no longer fits is left out
    /// and counted in [`Counters::egress_too_long`], with no reject. The
    /// NPDUs already polled are the caller's; a link that then refuses one
    /// as too long says so with [`refused_later`](Self::refused_later). The
    /// table, the searches, the time and [`next_deadline`](Self::next_deadline)
    /// don't change.
    ///
    /// Refused, changing nothing, for a port the router doesn't have, or a
    /// limit under [`MIN_PORT_LIMIT`] or over the scratch, as
    /// [`new`](Self::new) refuses one.
    pub fn set_port_max_npdu(
        &mut self,
        port: PortId,
        limits: NpduLimits,
    ) -> Result<(), LimitError> {
        let store = self.store();
        let count = store.port_count;
        let Some(config) = store.ports[..count].get_mut(usize::from(port.0)) else {
            return Err(LimitError::UnknownPort);
        };
        if !config::limits_fit(limits, N) {
            return Err(LimitError::OutOfRange);
        }
        config.max_npdu = limits;
        Ok(())
    }

    /// When [`on_deadline`](Self::on_deadline) next has work: the next
    /// sweep.
    ///
    /// Only `on_deadline` moves it, so a host may sleep until it without
    /// any other call waking that sleep (the hosted router's sweep task
    /// does). A change that lets another call move it must say so here,
    /// since such a host would then need a wake after that call.
    pub fn next_deadline(&self) -> Option<Instant> {
        Some(self.storage.borrow().next_sweep)
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
        if now < self.storage.borrow().next_sweep {
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
        store.next_sweep += options.sweep;
        if store.next_sweep <= now {
            store.next_sweep = now + options.sweep;
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

    /// The counts since the counters were last taken, which then start
    /// again from zero: add them to your own totals, as wide as you need,
    /// as the hosted router does. Each is a `u32` that saturates between
    /// takes.
    #[must_use = "the counts are gone once taken; add them to your totals"]
    pub fn take_counters(&mut self) -> Counters {
        core::mem::replace(&mut self.store().counters, Counters::ZERO)
    }

    /// The counts since the counters were last taken, as
    /// [`take_counters`](Self::take_counters) would return them, without
    /// taking them: for a look between takes. A router whose counters are
    /// never taken counts from its start.
    #[must_use]
    pub fn counters(&self) -> Counters {
        self.storage.borrow().counters
    }

    /// The ports, by index.
    pub fn ports(&self) -> &[PortConfig<G>] {
        let store = self.storage.borrow();
        &store.ports[..store.port_count]
    }

    /// The options.
    pub const fn options(&self) -> &Options {
        &self.options
    }
}
