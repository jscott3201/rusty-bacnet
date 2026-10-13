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
//! Each writes its destinations into storage the caller passes and returns
//! the Forwarded-NPDU to send them ([`Distribution`]), or, for a received
//! Forwarded-NPDU, which goes on unchanged, only where ([`Fanout`]). The tables read and
//! write through the codec's views: [`Bdt::set`] takes a
//! [`BdtEntries`](bacnet_bip_codec::BdtEntries), [`Bdt::entries`] gives one
//! for a Read-Broadcast-Distribution-Table-Ack, and [`Fdt::wire_entries`]
//! gives the rows of a Read-Foreign-Device-Table-Ack.
//!
//! The caller keeps everything else: the socket, delivering the NPDU to its
//! network layer, its own echoes, access lists and rate limits (through
//! [`Fdt::register`]'s `admit`), counters, and persistence. Registrations
//! expire on the caller's clock: pass `now` to each call, and call
//! [`Fdt::purge`] now and then to free the slots of lapsed ones.
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

pub use bdt::{
    canonicalize, distribution_address, validate_entry, Bdt, BdtError, EntryError, TWO_HOP_MASK,
};
pub use fdt::{
    Fdt, RegisterOutcome, Registered, Registration, RegistrationLimits, RegistrationRefusal,
    Reservation, StorageFull, GRACE_PERIOD,
};
pub use forward::{
    Context, DistributeRefusal, Distribution, Fanout, ForwardRefusal, Tables, Target, Via,
};
