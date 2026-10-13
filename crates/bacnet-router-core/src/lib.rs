//! Portable BACnet router behaviour (Clause 6), for firmware and for the
//! hosted router of `bacnet-network`.
//!
//! The crate is `no_std` and needs no allocator. NPDUs are read in place
//! from the caller's frame and written into the caller's buffer; the
//! routing table, the discovery state and the router's output scratch live
//! in arrays whose size the caller picks with const generics; and time is
//! an [`Instant`] the caller passes to each call that needs one. It reads
//! no clock and opens no socket, and nothing on a routing decision formats
//! text or divides a 64-bit value. None of that storage keeps timers or
//! defaults of its own, so it all starts as zero bits and can live in
//! `.bss`.
//!
//! [`Instant`] and [`Duration`] are `bacnet-time`'s, in nanoseconds,
//! re-exported here: the types `bacnet-mstp-core` and `bacnet-bip-core`
//! take too, so firmware that runs a router between their ports feeds all
//! three one clock.
//!
//! - [`npdu`]: a borrowed view of an NPDU's control information, held equal
//!   to `bacnet-encoding`'s decoder field for field by a property test, and
//!   a writer that refuses an NPDU too long for its buffer before writing an
//!   octet.
//! - [`message`]: the network-layer messages a router reads and sends:
//!   their type codes, the reject reasons, and allocation-free views and
//!   writers for their payloads.
//! - [`table`]: the routing table, sorted by network number, with
//!   learning, the convergence modes, busy and unreachable marks, the
//!   reject hold-down and aging.
//! - [`discovery`]: which unknown networks to look for, at most once per
//!   cooldown each, in bounded storage.
//! - [`router`]: the sans-IO engine that joins them: a [`Router`] over a
//!   [`Storage`] takes each frame a port receives, says what it means for
//!   the router's own application, and asks for the sends it calls for,
//!   which the caller polls. It takes the time with every call.
//!
//! A port is identified by its index, a [`PortId`]; its MAC is a [`Mac`] of
//! up to [`MAX_MAC_LEN`] octets. To the engine a port is data, so any link
//! plugs in; two features add the glue for the portable link cores, so
//! firmware can route between them with no allocator:
//!
//! - `bip`: B/IP ports over `bacnet-bip-core`'s node engine, in normal or
//!   foreign-device mode, or as a BBMD over its tables (module `bip`).
//! - `mstp`: MS/TP ports over `bacnet-mstp-core`'s master core, with
//!   extended frames (module `mstp`).
//!
//! Both hand the router each NPDU their link delivers as an `Arrived`,
//! whose frame borrows its link source, and carry out the router's sends on
//! their link. Each module documents its wiring, and the clauses its tests
//! cover.
//!
//! The types that may grow are `#[non_exhaustive]`. Start the
//! [`Options`](router::Options) from `HOSTED` and set the fields to change;
//! build a [`Route`](table::Route), for comparison, with its `direct` or
//! `learned`, and counters from `Default`; and give a `match` on an error,
//! a refusal, a [`Discard`](router::Discard), an
//! [`Event`](router::Event), a [`Kind`](router::Kind) or another of these
//! enums a wildcard arm. The enums a host acts on case by case,
//! [`Action`](router::Action), [`Received`](router::Received) and the B/IP
//! glue's `Ingress`, stay exhaustive, as do the closed sets such as
//! [`Dest`](router::Dest), [`Group`](router::Group) and
//! [`Reachability`](table::Reachability). The wire codes,
//! [`MessageType`](message::MessageType) and
//! [`RejectReason`](message::RejectReason), are open: any octet converts,
//! so a `match` on their constants needs a wildcard arm too.
//!
//! # Time
//!
//! A `now` earlier than the latest one passed in is taken as that latest
//! one: time never runs backwards, so a clock that steps back holds the
//! timers still instead of failing. Here that is the [`Router`]; the table
//! and discovery keep no clock of their own, and count a `now` before a
//! stamp as no time passed. The B/IP and MS/TP ports hand their `now` to
//! their link core, which keeps the same rule, and a BBMD port keeps the
//! latest time for its tables too.
//!
//! # Example
//!
//! A router between network 1, a B/IP-like link with six-octet MACs, and
//! network 2, an MS/TP-like one, passes a request from station 5 on network
//! 2 on to a node on network 1:
//!
//! ```
//! use bacnet_router_core::npdu::{Address, Npdu};
//! use bacnet_router_core::router::{
//!     Action, Dest, Frame, Limits, Options, Permissive, PortConfig, Received,
//! };
//! use bacnet_router_core::{Instant, Mac, PortId, Router, Storage};
//!
//! let ports = [
//!     PortConfig {
//!         network: 1,
//!         mac: Mac::from_array([192, 0, 2, 1, 0xBA, 0xC0]),
//!         max_npdu: Limits::both(1497),
//!     },
//!     PortConfig {
//!         network: 2,
//!         mac: Mac::from_array([1]),
//!         max_npdu: Limits::both(1497),
//!     },
//! ];
//! // Two ports, a table of 8 routes and 8 hold-down records; firmware
//! // keeps the storage in a static.
//! let mut storage = Storage::<2, 8, 8>::new();
//! let mut options = Options::HOSTED;
//! options.route_cap = 8;
//! let mut router = Router::new(&mut storage, &ports, options, Instant::ZERO)?;
//!
//! // A confirmed request for the node at 192.0.2.9 on network 1.
//! let node = [192, 0, 2, 9, 0xBA, 0xC0];
//! let mut request = Npdu::apdu(&[0x00, 0x05, 0x01, 0x0C]);
//! request.destination = Some(Address { network: 1, mac: &node });
//! request.expecting_reply = true;
//! let mut frame = [0u8; 32];
//! let len = request.write(&mut frame)?;
//!
//! let arrived = Frame { port: PortId(1), source: &[5], link_group: false, npdu: &frame[..len] };
//! let received = router.receive(arrived, Instant::from_secs(1), &mut Permissive);
//! assert_eq!(received, Received::Handled);
//!
//! // It goes to the node on port 0, with the station as its source.
//! let Action::Send(send) = router.poll_action() else {
//!     panic!("a send");
//! };
//! assert_eq!((send.port, send.to), (PortId(0), Dest::Unicast(&node[..])));
//! let relayed = Npdu::parse(send.npdu).expect("an NPDU");
//! assert_eq!(relayed.source, Some(Address { network: 2, mac: &[5] }));
//! assert_eq!(relayed.destination, None);
//! assert_eq!(router.poll_action(), Action::Idle);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! # Clause 6 coverage
//!
//! What the core is answerable for, in our words, with the tests that show
//! it. First its parts:
//!
//! | Obligation | Clause | Test |
//! | --- | --- | --- |
//! | The control octet says which NPCI fields follow, and they follow in one fixed order; a zero DNET, a zero or all-ones SNET and an empty SADR make the NPDU invalid | 6.2, 6.2.2, 6.2.2.1 | `tests/npdu_view.rs`: `view_agrees_with_decode_npdu` |
//! | A DLEN or SLEN past any data link's address width is told apart from other faults, carrying the DNET and the source a reason-6 reject needs | 6.2.2.2, 6.4.4 | `tests/npdu_view.rs`: `over_long_addresses_carry_what_a_reject_needs` |
//! | Each network-layer message's payload is read whole or not at all, and written in the layout the clause gives it | 6.4 | `tests/message.rs` |
//! | The table records, for each network it can reach, the port, the next router (none for a directly attached network), and a state that separates a lasting failure from congestion | 6.6.1 | `tests/table.rs` |
//! | A later claim of the route to a network supersedes the earlier one | 6.6.3.2 | `tests/table_claims.rs` |
//! | A reject with reason 1 marks a learned route unreachable, and one with reason 2 marks it busy for a while; no other reason changes the table | 6.6.3.5 | `src/table/tests/rejects.rs` (unit tests, as the hold-down records they read are the crate's own) |
//! | Flow control holds traffic for the networks named, or for every network the sending router serves when none are named, and lets it go again | 6.6.3.6, 6.6.3.7 | `tests/table_busy.rs` |
//! | A routing-table update replaces or adds a network's row, and Port ID 0 deletes it | 6.4.7, 6.6.3.8 | `tests/table.rs`: `management_updates_never_touch_direct_routes` |
//! | A router with no route to a network looks for one before it refuses the message | 6.6.3.1 | `tests/discovery.rs` |
//!
//! Then the engine, with the hosted router's options:
//!
//! | Obligation | Clause | Test |
//! | --- | --- | --- |
//! | An address length no link uses draws reason 6 when the NPDU names a network, and nothing when it is a global broadcast | 6.2.2.2, 6.4.4 | `tests/ingress.rs`: `over_long_addresses_reject_with_reason_6` |
//! | A message for the router's own address on any of its networks goes to its own application, not on a wire | 6.1, Figure 6-12 | `tests/ingress.rs`: `own_address_on_any_port_is_local`, `own_mac_on_a_remote_network_is_routed_on` |
//! | Only an Unconfirmed-Request may use a broadcast network address | 6.3 | `tests/ingress.rs`: `broadcast_addresses_carry_only_unconfirmed` |
//! | Missing SNET and SADR are filled in from the arrival network and the link sender; present ones travel unchanged | 6.5.4 | `tests/forward.rs`: `source_is_added_once_and_kept_after` |
//! | Onto an attached network the destination fields and hop count are dropped, and the frame goes to the DADR, or to every node for DLEN 0 | 6.5.4 | `tests/forward.rs`: `direct_delivery_strips_destination` |
//! | Toward another router the destination fields stay and the hop count falls by one; a count that would reach zero goes no further | 6.2.3, 6.5.4 | `tests/forward.rs`: `next_router_keeps_destination_and_counts_down`, `hop_count_reaching_zero_is_not_relayed`, `hop_count_reaching_zero_stops_routed_network_messages` |
//! | A global broadcast reaches every attached network but the one it came from, still global, and the router's own application too; one whose count would reach zero is only delivered | 6.3.2, 6.5.4 | `tests/forward.rs`: `global_broadcast_fans_out_and_delivers` |
//! | A network message for another network is routed like an APDU, a proprietary one included, vendor ID and all | 6.2.5, 6.5.4 | `tests/forward.rs`: `proprietary_messages_route_opaquely` |
//! | Lacking a route, the router searches for one and refuses the message with reason 1 | 6.6.3.1 | `tests/traces.rs`: `a_discovery_learning_and_aging` |
//! | A refusal goes to whoever first sent the refused message, as its source fields say | 6.4.4, 6.5.4 | `tests/reject.rs`: `reject_path_follows_the_origin` |
//! | A reject that arrives updates the table, for reasons 1 and 2 only, then travels on toward the node it names, whatever its reason | 6.6.3.5 | `tests/reject.rs`: `received_rejects_relay_by_their_destination`, `other_reasons_relay_and_keep_the_route`; `tests/traces.rs`: `c_received_rejects` |
//! | Traffic for a network on hold draws reason 2 until the hold lapses or is lifted | 6.6.3.6 | `tests/traces.rs`: `b_flow_control` |
//! | A message type the router doesn't know, sent to it, draws reason 3 | 6.6.3.5 | `tests/control.rs`: `unknown_types_draw_reason_3` |
//! | Route announcements teach the table and go on to the other ports | 6.6.3.2, 6.6.3.3 | `tests/control.rs`: `i_am_router_learns_then_rebroadcasts`; `tests/traces.rs`: `d_hardened_convergence` |
//! | A route question about one network is answered when the network lies beyond another port, searched further, with the asker as origin, when the table lacks it, and left alone when it lies beyond the arrival port | 6.6.3.2 | `tests/control.rs`: `scoped_who_is_router_cases` |
//! | A route question about every network is answered with those beyond the other ports, ascending, but for those failed for good | 6.6.3.2 | `tests/control.rs`: `unscoped_answer_lists_reachable_networks` |
//! | Flow control covers what its sender serves, named or not, and goes on to the other ports | 6.6.3.6, 6.6.3.7 | `tests/control.rs`: `flow_control_covers_what_the_sender_serves` |
//! | A table update applies entry by entry and gets an empty answer; a query gets the whole table, over several answers when needed, and changes nothing | 6.4.7, 6.6.3.8, 6.6.3.9 | `tests/init.rs` |
//! | The network-number messages stay on their link | 6.4.19, 6.4.20 | `tests/control.rs`: `network_number_messages_stay_local` |
//! | At start each port hears its own number, then which networks lie beyond the others | 6.4.20, 6.6.2, 6.6.3.3 | `tests/traces.rs`: `a_discovery_learning_and_aging`, `a_one_port_router_announces_only_its_number` |
//! | With no point-to-point links, connection messages change nothing | 6.6.3.10, 6.6.3.11 | `tests/control.rs`: `ptp_messages_change_nothing` |
//! | A message too long for its egress port draws reason 4, a remote broadcast included, and a fan-out leaves that port out | 6.4.4, 6.6.3.5, Table 6-1 | `tests/forward.rs`: `too_long_for_the_egress_port_draws_reason_4`, `a_fan_out_goes_by_the_broadcast_limit` |
//!
//! Rules of this stack beyond the clause, each tested in the same files:
//! the 18-octet address bound (#1141, #1198); a DADR beside DNET 0xFFFF
//! (#1379); group DADRs, where only an Unconfirmed-Request for a group of
//! the delivery port's own goes on, as that port's broadcast (#1504,
//! #1677); the relay back out the arrival port of a
//! message for the arrival network (a DLEN 0 broadcast at the port's limit
//! is delivered and also draws reason 4, as the relay gains source
//! fields); the RB-09 gate for each protected
//! message, silent when it refuses; direct routes that no learning, reject
//! or management write changes; a 30-second hold-down per network and
//! arrival port on reject-driven changes; hardened convergence, which
//! waits for a second claim before moving a route to another port; the
//! route cap; a discovery cooldown, cap and expiry; no reject for a global
//! broadcast, and none to the router itself.
//!
//! # Features
//!
//! None by default: the crate then depends only on `bacnet-time`. Neither
//! feature needs std or an allocator, and every combination builds on
//! bare-metal targets.
//!
//! - `bip`: the B/IP glue, which adds `bacnet-bip-core` and
//!   `bacnet-bip-codec`.
//! - `mstp`: the MS/TP glue, which adds `bacnet-mstp-core`.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_debug_implementations)]
#![warn(clippy::must_use_candidate)]

#[cfg(test)]
extern crate std;

#[cfg(any(feature = "bip", feature = "mstp"))]
mod arrived;
#[cfg(feature = "bip")]
pub mod bip;
pub mod discovery;
pub mod message;
#[cfg(feature = "mstp")]
pub mod mstp;
pub mod npdu;
pub mod router;
pub mod table;

#[cfg(any(feature = "bip", feature = "mstp"))]
pub use arrived::Arrived;
pub use bacnet_time::{Duration, Instant};
pub use npdu::{Mac, MacTooLong};
pub use router::{Router, Storage};

/// The largest NPDU a B/IP or MS/TP port carries, in octets (Table 6-1;
/// MS/TP needs extended frames for it, Clause 9.3).
///
/// With the glue features it is held equal to the link cores' own, so the
/// three can't drift apart.
pub const MAX_NPDU: usize = 1497;

#[cfg(feature = "bip")]
const _: () = assert!(
    MAX_NPDU == bacnet_bip_core::node::MAX_NPDU,
    "router-core's MAX_NPDU is bip-core's"
);

#[cfg(feature = "mstp")]
const _: () = assert!(
    MAX_NPDU == bacnet_mstp_core::codec::MAX_EXTENDED_MPDU_DATA,
    "router-core's MAX_NPDU is the MS/TP codec's extended-frame limit"
);

/// The longest DADR, SADR or link MAC the core accepts, in octets.
///
/// It is `bacnet-encoding`'s `NpduAddress::MAX_MAC_LEN` (#1141), so the
/// core refuses exactly the addresses the hosted codec refuses; a test holds
/// the two equal. No data link in Table 6-2 uses more than 7.
pub const MAX_MAC_LEN: usize = 18;

/// A router port, by its index in the router's port list.
///
/// On the wire, in a routing table's Port ID field, port `n` is `n + 1`:
/// Port ID 0 asks the receiver to delete a network's rows (Clause 6.4.7),
/// so it names no port. Index 255 has no wire Port ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(pub u8);

impl PortId {
    /// This port's Port ID in a routing table on the wire, or `None` for
    /// index 255, which the one-octet field can't name.
    #[must_use]
    pub const fn wire_id(self) -> Option<u8> {
        self.0.checked_add(1)
    }

    /// The port a wire Port ID names on a router with `port_count` ports:
    /// `None` for Port ID 0, the delete request, and for a Port ID past the
    /// last port.
    #[must_use]
    pub const fn from_wire_id(id: u8, port_count: usize) -> Option<Self> {
        match id.checked_sub(1) {
            Some(index) if (index as usize) < port_count => Some(Self(index)),
            _ => None,
        }
    }
}

/// Swap `slice[low]` and `slice[high]`.
///
/// The caller keeps `low <= high < slice.len()`, which a debug assertion
/// checks; `low == high` leaves the slice as it is, which a compaction
/// that has kept everything so far relies on. A release build given
/// anything else does nothing.
///
/// The table and its hold-down records, the searches, the set of networks
/// one message claims and the output queue shift and compact their arrays
/// with this, one element at a time, never with `copy_within` or a copy
/// from one element to another, which compile to a memmove call:
/// 0.8-1.4 KB of code on a microcontroller. `slice::swap` would link one
/// too for a large element, such as a route, since it has to allow equal
/// indices; two borrows of distinct elements can't overlap, so their swap
/// needs no memmove.
pub(crate) fn swap_apart<T>(slice: &mut [T], low: usize, high: usize) {
    debug_assert!(
        low <= high && high < slice.len(),
        "swap_apart needs low <= high < len"
    );
    if let Some((front, back)) = slice.split_at_mut_checked(high) {
        if let (Some(first), Some(second)) = (front.get_mut(low), back.first_mut()) {
            core::mem::swap(first, second);
        }
    }
}

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
