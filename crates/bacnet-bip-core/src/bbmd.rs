//! A BBMD's tables and forwarding rules (Annex J.4, J.5).
//!
//! [`Tables`] holds the broadcast distribution table ([`Bdt`]) and the
//! foreign device table ([`Fdt`]), each in an array whose size the caller
//! picks, and decides where each broadcast goes:
//!
//! - [`Tables::original_broadcast`]: a broadcast on this BBMD's subnet, to
//!   every peer subnet and foreign device.
//! - [`Tables::forwarded_npdu`]: a peer's Forwarded-NPDU, to the foreign
//!   devices and, when it came to this BBMD alone (its own BDT row is
//!   two-hop and it arrived by unicast), the local subnet.
//! - [`Tables::distribute_broadcast`]: a foreign device's broadcast, to the
//!   local subnet, the peer subnets and the other foreign devices.
//! - [`Tables::own_broadcast`]: a broadcast this BBMD's own device sends.
//!
//! A BBMD behind a NAT router ([`Tables::set_nat`], Annex J.7.5) runs under
//! its global address, and names it as the origin of the Forwarded-NPDUs it
//! builds ([`Nat`]).
//!
//! # A BBMD that serves a B/IP-M group (Annex J.8.3)
//!
//! A BBMD whose subnet is a multicast group ([`LocalSubnet::member`],
//! [`LocalSubnet::is_group`]) serves that group: it is a member, and the
//! group stands in for its subnet's broadcast address in every decision
//! above. Its link takes broadcasts only through the group, as a member's
//! does. Each rule, in our own words, with its test in
//! `crates/bacnet-bip-core/tests/port_bbmd_multicast.rs`:
//!
//! | Rule | Clause | Test |
//! | --- | --- | --- |
//! | An Original-Broadcast-NPDU that came through the group goes to the other BDT entries and the foreign devices | J.8.3, J.4.5 | `a_broadcast_from_the_group_goes_to_the_bdt_and_the_fdt` |
//! | A peer's Forwarded-NPDU goes to the group and the foreign devices (the subnet's mask isn't used, so a plain peer on the BBMD's IP subnet, which J.8.1 rules out, is tolerated in this direction only) | J.8.3 | `a_peers_forward_goes_to_the_group_and_the_fdt` |
//! | A Forwarded-NPDU that came through the group goes to the foreign devices only, never back to the group | J.4.5, J.8.3 | `a_forward_from_the_group_never_goes_back_to_it` |
//! | A foreign device's Distribute-Broadcast-To-Network goes to the group, the BDT and every other foreign device | J.8.3 | `a_foreign_devices_broadcast_goes_to_the_group_the_bdt_and_the_other_devices` |
//! | The BBMD's own broadcast goes to the group, then to the BDT and the foreign devices | J.8.2, J.4.5 | `the_bbmds_own_broadcast_goes_to_the_group_then_the_bdt_and_fdt` |
//! | Its own BDT row has the all-ones mask, so peers send it the network's broadcasts by unicast ([`Bdt::set_serves_group`], [`BdtError::OneHopForGroup`]) | J.8.3 | `a_bbmd_that_serves_a_group_needs_a_two_hop_own_row` |
//! | Local: it may also run behind a NAT router; both rules want a two-hop own row | J.7.5, J.8.3 | `a_group_and_nat_traversal_compose` |
//!
//! The group is not a BDT entry: [`validate_entry`] still refuses a
//! multicast address there. [`BbmdPort`](crate::port::BbmdPort) sets the
//! BDT's group rule from its link; a caller that drives [`Tables`] itself
//! calls [`Bdt::set_serves_group`] and passes the link's subnet to each
//! decision.
//!
//! [`LocalSubnet::member`]: crate::LocalSubnet::member
//! [`LocalSubnet::is_group`]: crate::LocalSubnet::is_group
//!
//! Each writes its destinations into storage the caller passes and returns
//! the Forwarded-NPDU to send them ([`Distribution`]), or, for a received
//! Forwarded-NPDU, which goes on unchanged, only where ([`Fanout`]). The tables read and
//! write through the codec's views: [`Bdt::set`] takes a
//! [`BdtEntries`](bacnet_bip_codec::BdtEntries), [`Bdt::entries`] gives one
//! for a Read-Broadcast-Distribution-Table-Ack, and [`Fdt::wire_entries`]
//! gives the rows of a Read-Foreign-Device-Table-Ack.
//!
//! [`Tables::serve`] answers the four management requests a BBMD serves
//! from its tables ([`Request`]): Register-Foreign-Device, the two table
//! reads and Delete-Foreign-Device-Table-Entry, each with the answer Annex
//! J gives it, under the host's own [`Rules`].
//!
//! The caller keeps everything else: the socket, delivering the NPDU to its
//! network layer, its own echoes, access lists and rate limits (through
//! [`Rules`], or [`Fdt::register`]'s `admit`), counters, and persistence.
//! Registrations expire on the caller's clock: pass `now` to each call, and
//! call [`Fdt::purge`] now and then to free the slots of lapsed ones.
//! [`port::BbmdPort`](crate::port::BbmdPort) does all of this but the
//! persistence for a BBMD on one socket.
//!
//! ```
//! use core::net::{Ipv4Addr, SocketAddrV4};
//! use bacnet_bip_core::codec::{BdtEntries, BdtEntry};
//! use bacnet_bip_core::bbmd::{Context, RegistrationLimits, Reservation, Tables, Target, Via};
//! use bacnet_bip_core::Instant;
//!
//! let own = SocketAddrV4::new(Ipv4Addr::new(10, 0, 1, 1), 47808);
//! let peer = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 1), 47808);
//! let mut tables: Tables<4, 4> = Tables::new(own);
//! // One peer, reached by one-hop distribution; the own row is added.
//! let bdt = [BdtEntry::new(peer, Ipv4Addr::new(255, 255, 255, 0))];
//! tables.bdt.set(BdtEntries::from_entries(&bdt))?;
//! assert_eq!(tables.bdt.len(), 2);
//!
//! let now = Instant::from_secs(100);
//! let device = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 7), 47808);
//! let limits = RegistrationLimits::default();
//! let outcome = tables.fdt.register(device, 60, now, &limits, Reservation::Ordinary, || true);
//! assert!(outcome.result.is_ok());
//!
//! let cx = Context::new(now, 4);
//! let source = SocketAddrV4::new(Ipv4Addr::new(10, 0, 1, 20), 47808);
//! let mut out = [Target::UNUSED; Tables::<4, 4>::MAX_TARGETS];
//! let sent = tables.original_broadcast(&cx, source, &[0x01, 0x00], &mut out);
//! assert_eq!(sent.origin, source);
//! let targets: Vec<_> = sent.fanout.targets.iter().map(|t| (t.destination, t.via)).collect();
//! assert_eq!(
//!     targets,
//!     [
//!         (SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 255), 47808), Via::Peer),
//!         (device, Via::ForeignDevice),
//!     ]
//! );
//! # Ok::<(), bacnet_bip_core::bbmd::BdtError>(())
//! ```

mod bdt;
mod fdt;
mod forward;
mod nat;
mod serve;

pub use bdt::{
    Bdt, BdtError, EntryError, TWO_HOP_MASK, canonicalize, distribution_address, validate_entry,
};
pub use fdt::{
    Fdt, GRACE_PERIOD, RegisterOutcome, Registered, Registration, RegistrationLimits,
    RegistrationRefusal, Reservation, StorageFull,
};
pub use forward::{
    Context, DistributeRefusal, Distribution, Fanout, ForwardRefusal, Tables, Target, Via,
};
pub use nat::{MAX_PRIVATE_RANGES, Nat, NatError, PrivateRange, is_rfc1918};
pub use serve::{Answer, DeleteRefusal, Request, Rules, Served};
