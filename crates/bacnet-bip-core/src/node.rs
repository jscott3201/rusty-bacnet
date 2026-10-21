//! A sans-IO B/IP node (Annex J) in normal, foreign-device and BBMD mode.
//!
//! Two layers:
//!
//! - [`Link`] is the fixed configuration: the node's own B/IP address, its
//!   [`LocalSubnet`](crate::LocalSubnet), its [`Mode`] and the largest
//!   datagram it sends. Its decisions are pure. [`Link::receive`] applies
//!   every receive rule to one datagram and says what to do with it,
//!   borrowing the caller's buffer; [`Link::unicast`] and
//!   [`Link::broadcast`] say what to send for an NPDU. A host can share
//!   one copy and decide without a lock.
//! - [`Node`] is the mutable part: this node's own management exchanges,
//!   matched by peer and operation since a BVLC-Result carries no invoke
//!   ID; a foreign device's registration lifecycle and the broadcasts it
//!   holds before its first attempt resolves; and the timers. After every
//!   call the caller polls [`Node::poll_action`] until [`Action::Idle`].
//!
//! There are no buffers and no allocator: a delivery borrows the datagram,
//! and every send is a [`Bvll`](bacnet_bip_codec::Bvll) the caller encodes.
//!
//! **The host's side.** Each [`Datagram`] carries what the host's socket
//! says about it: the UDP source, whether the read was cut short, which
//! socket read it, and where it was sent ([`Datagram::to`], built with
//! [`Addressed::classify`]). The last decides whether a message came to
//! this node alone or to every node on the subnet, which the receive
//! rules depend on. A host whose IP stack can't report the destination of
//! each datagram (lwIP without per-packet destination information, say)
//! passes `None`, and the node delivers nothing from it and drops it as
//! [`Discard::UnknownDestination`], never guessing (#1678).
//! In [`Mode::Bbmd`], once every node rule has passed, [`Link::receive`]
//! hands the BBMD's share of a message over as [`ForBbmd`], for the caller
//! to run through [`bbmd::Tables`](crate::bbmd::Tables); a node that is not
//! a BBMD links none of that code.
//!
//! ```
//! use core::net::{Ipv4Addr, SocketAddrV4};
//! use bacnet_bip_core::codec::{BvlcResultCode, Bvll};
//! use bacnet_bip_core::node::{
//!     Action, Addressed, Datagram, Foreign, Inbound, Link, Mode, Node, Permissive,
//! };
//! use bacnet_bip_core::{Instant, LocalSubnet};
//!
//! let own = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 47808);
//! let bbmd = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 1), 47808);
//! let subnet = LocalSubnet::new(SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 47808));
//! let link = Link::new(own, subnet, Mode::Foreign(Foreign::new(bbmd, 60)))?;
//! let start = Instant::from_secs(1);
//! let mut node: Node = Node::new(link, start)?;
//!
//! // The first registration is due at once.
//! let Action::Register { exchange, to, ttl } = node.poll_action() else { panic!() };
//! assert_eq!((to, ttl), (bbmd, 60));
//! // ... encode Bvll::RegisterForeignDevice { ttl } and send it to `to` ...
//! assert!(!node.exchange_sent(exchange, start));
//!
//! // The BBMD's success comes back.
//! let mut wire = [0u8; 6];
//! Bvll::BvlcResult(BvlcResultCode::SUCCESSFUL_COMPLETION).encode_into(&mut wire)?;
//! let datagram = Datagram::new(&wire, bbmd, Some(Addressed { node: true, broadcast: false }));
//! let now = Instant::from_millis(1_010);
//! assert!(matches!(
//!     node.receive(&datagram, now, &mut Permissive),
//!     Inbound::Matched { settled: true, .. }
//! ));
//! assert!(matches!(node.poll_action(), Action::Registration(_)));
//! assert!(node.registration(now).is_some_and(|snapshot| snapshot.registered));
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! # Annex J, for a node that is not a BBMD
//!
//! Each obligation Annex J places on a node that is not a BBMD, and one
//! recommendation, in our own words, with the test in
//! `crates/bacnet-bip-core/tests/` that covers it.
//!
//! | Obligation | Clause | Test |
//! | --- | --- | --- |
//! | A B/IP address is an IPv4 address and a UDP port; a peer may use any port, and is answered at the one it used | J.1.2 | `node_receive.rs` `a_peer_on_another_port_is_delivered_and_answered_there` |
//! | A local broadcast goes to the subnet's broadcast IP at the network's port | J.1.2, J.4.1 | `node_send.rs` `normal_broadcast_is_original_broadcast_at_the_network_port` |
//! | Every message has BVLC type 0x81, and a Length that counts the whole message | J.2 | `node_receive.rs` `header_failures_are_discarded_by_reason` |
//! | Each request only a BBMD serves, reaching a non-BBMD by unicast, is answered with that request's own NAK: Read-BDT 0x0020, Read-FDT 0x0040, Delete-FDT-Entry 0x0050, Register-Foreign-Device 0x0030, Distribute-Broadcast-To-Network 0x0060 | J.2.1, J.4.4.1, J.4.5, J.5.2.1.1, J.5.2.1.2, J.5.2.2 | `node_naks.rs` `each_bbmd_only_request_draws_its_own_nak` |
//! | Write-BDT is refused with 0x0010 by every device, a BBMD included, and changes nothing | J.4.4.2 | `node_naks.rs` `write_bdt_is_refused_in_every_mode` |
//! | A BVLC-Result reports on one of the node's own requests; nothing answers it | J.2.1 | `node_exchanges.rs` `a_result_is_matched_and_never_answered` |
//! | A Forwarded-NPDU's originating address, not its sender, is the NPDU's source | J.2.5 | `node_receive.rs` `forwarded_npdu_is_delivered_from_its_origin` |
//! | A directed NPDU goes to its recipient's B/IP address as an Original-Unicast-NPDU | J.2.11, J.3 | `node_send.rs` `unicast_is_original_unicast` |
//! | An Original-Unicast-NPDU must reach the node at its own address, an Original-Broadcast-NPDU at the broadcast address | J.2.11, J.2.12, J.4.1 | `node_receive.rs` `original_functions_must_fit_their_destination` |
//! | A foreign device broadcasts through its BBMD with Distribute-Broadcast-To-Network, never with Original-Broadcast-NPDU | J.2.10, J.2.12 | `node_send.rs` `foreign_broadcast_is_always_dbtn_to_the_bbmd` |
//! | A device that can't act as a BBMD can register as a foreign device; a success result means it is registered | J.4.3, J.5.2 | `node_registration.rs` `success_registers` |
//! | The time-to-live bounds when the device must register again; the 30-second grace belongs to the BBMD, so the device doesn't count on it. The lease the node reports never outlasts the BBMD's entry, for any time-to-live, while every result and copy of one arrives less than the answer window ([`Lease`]: three response timeouts, at most the renewal interval, never under one timeout) after its request | J.2.6.1, J.5.2.1, J.5.2.3 | `registration_traces.rs`; `node_properties.rs` `lease_never_outlives_the_bbmd_entry_without_grace` (time-to-live 2, 20 and 60 s); `node_registration.rs` `a_copy_of_an_answer_never_stretches_a_short_lease` |
//! | A BBMD that won't register the device answers with the Register-Foreign-Device NAK | J.5.2.2 | `node_registration.rs` `a_refusal_keeps_earlier_evidence` |
//! | A BBMD answers Distribute-Broadcast-To-Network with its NAK when the sender isn't registered, and also when forwarding fails; that NAK from the device's own BBMD is taken as a hint to register again, at most once per response timeout, when retries back off | J.4.5 | `node_registration.rs` `distribute_nak_pulls_the_renewal_forward`, `at_renewal_takes_no_hint_from_a_distribute_nak` |
//! | A recommendation, not an obligation: behind NAT, a foreign device registers often enough to keep the router's mapping open, whatever its time-to-live | J.7.5 (e) | `node_registration.rs` `renewal_is_independent_of_ttl` |
//! | A function the annex doesn't define, 0x0C among them (its clause is deleted), is dropped unanswered | J.2.13 | `node_receive.rs` `unknown_functions_are_dropped_unanswered` |
//! | A B/IP-M node sends its broadcasts to its multicast group, which stands in for the subnet's broadcast address, and directed messages by unicast as before | J.8.1, J.8.2 | `node_multicast.rs` `a_member_broadcasts_to_the_group_and_never_unicasts_there` |
//! | A B/IP-M node takes Original-Broadcast-NPDU and Forwarded-NPDU from its group, and not from the subnet's broadcast address: members and plain devices don't share a subnet, so that broadcast belongs to another network | J.8.1 | `node_multicast.rs` `a_member_takes_broadcast_functions_from_the_group_and_unicast_ones_only_by_unicast` |
//! | A foreign device broadcasts through its BBMD, so it joins no group; a BBMD may serve one (its rules are in [`bbmd`](crate::bbmd)) | J.8.1, J.8.3 | `node_multicast.rs` `a_foreign_device_takes_no_group` |
//!
//! Local rules, beyond the annex:
//!
//! | Rule | Test |
//! | --- | --- |
//! | A BBMD-only request sent to a broadcast address draws nothing: J.2.1 asks for the NAK when the request came by unicast, read here as leaving broadcast copies unanswered | `node_naks.rs` `a_broadcast_bbmd_request_draws_nothing` |
//! | A datagram whose UDP source is a group address is dropped before its frame is read (#1504) | `node_receive.rs` `group_sources_and_origins_are_dropped` |
//! | A B/IP-M node's subnet directed broadcast stays a group address, as a UDP source and as a Forwarded-NPDU origin | `node_multicast.rs` `a_members_directed_broadcast_stays_a_group` |
//! | A Forwarded-NPDU whose origin is a group address is dropped (#1493) | `node_receive.rs` `group_sources_and_origins_are_dropped` |
//! | The node's own frames, and Forwarded-NPDUs naming it as the origin, are dropped | `node_receive.rs` `own_echoes_are_dropped` |
//! | A broadcast listener keeps only this link's broadcasts, from its own interface (#1538) | `node_receive.rs` `listeners_keep_only_this_links_broadcasts` |
//! | A wildcard bind takes unicast only at the host's own addresses (#952) | `node_receive.rs` `wildcard_binds_take_unicast_only_at_own_addresses` |
//! | The host's rate limit sees the four covered requests before their payload | `node_receive.rs` `the_policy_sees_covered_requests_before_their_payload` |
//! | No NAK goes to an address a reply can't reach | `node_naks.rs` `no_nak_to_an_unanswerable_source` |
//! | A malformed BBMD-only request still draws its NAK | `node_naks.rs` `a_malformed_request_still_draws_its_nak` |
//! | A truncated datagram is dropped first | `node_receive.rs` `truncation_is_checked_first` |
//! | A datagram whose destination the host can't report is dropped, never guessed at (#1678) | `node_receive.rs` `ingress_fails_closed_for_destination_mismatch_missing_metadata_and_truncation` |
//! | A foreign device takes Forwarded-NPDUs only from its BBMD by unicast | `node_receive.rs` `foreign_trust_takes_forwards_only_from_the_bbmd_by_unicast` |
//! | Broadcasts are held, bounded, until the first registration attempt resolves | `node_registration.rs` `broadcasts_are_held_until_the_first_attempt_resolves` |
//! | A foreign device refuses manual requests a success answers to its own BBMD, whose registration it owns | `node_registration.rs` `a_foreign_device_refuses_manual_requests_to_its_bbmd` |
//! | NPDUs longer than the datagram budget allows are refused | `node_send.rs` `the_budget_caps_each_cast` |
//! | A unicast to one of the link's group addresses is refused before anything is encoded (#1677) | `node_send.rs` `unicast_group_destinations_are_rejected_before_encoding_or_send` |

mod actions;
mod addressing;
mod config;
mod counters;
mod engine;
mod exchange;
mod receive;
mod registration;
mod send;

pub use actions::Action;
pub use addressing::{Addressed, Groups, LocalAddresses};
pub use config::{
    ConfigError, Foreign, ForwardedTrust, Link, Mode, Retry, DEFAULT_RESPONSE_TIMEOUT,
    MAX_DATAGRAM, MIN_BUDGET, MIN_RENEWAL,
};
pub use counters::{ExchangeCounters, NodeCounters, RegisterAttempt};
pub use engine::{Inbound, Node};
pub use exchange::{ExchangeId, ExchangeRefusal, LocalFailure, Operation};
pub use receive::{
    Datagram, Delivery, Discard, ForBbmd, Path, Permissive, Policy, Received, Response,
};
pub use registration::{Admission, AttemptOutcome, Lease, RegistrationEvent, RegistrationSnapshot};
pub use send::{Cast, Outgoing, SendRefusal};
