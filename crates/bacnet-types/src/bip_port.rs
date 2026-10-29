//! The B/IP operating mode a transport lends the Network Port object.
//!
//! A B/IP or B/IPv6 port runs in NORMAL, FOREIGN or BBMD mode (Clause
//! 12.56.21, 12.56.39: BACnet_IP_Mode or BACnet_IPv6_Mode), and the same
//! types serve both: a host, BDT row or FDT entry holds either IP version.
//! For either, which Network Port properties exist depends on the mode
//! (Table 12-71, footnotes 11 to 13, and the informative Table 12-72). The
//! footnotes tie the BBMD rows to a port able to act as a BBMD; this stack
//! reads that as a port configured as one, since a port's mode can't change
//! while it is registered. The transport layer owns that state and the
//! object layer serves it, and neither crate depends on the other, so the
//! contract between them lives here.
//!
//! A BBMD's tables change at run time, so they are lent as a live view,
//! [`BbmdTables`], which the object reads on each property read. Nothing is
//! copied, so the object cannot drift from the transport. The same view
//! checks a written BDT and puts written values into effect when they are
//! activated ([`BbmdChanges`]). A foreign device's registration is lent
//! the same way, [`ForeignRegistration`], so a written BBMD address and
//! time-to-live can be checked and activated ([`ForeignDeviceChanges`]).
//! A port that can use B/IP multicast lends its group as [`IpMulticast`],
//! in any mode.
//!
//! The module needs the `alloc` feature, and a target with atomic
//! compare-and-swap for the shared `Arc`, which bare-metal targets such as
//! ARMv6-M lack (#1604). So a no_std device on such a target can lend its
//! B/IP state to the Network Port object too (#1615).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use crate::constructed::{BACnetBDTEntry, BACnetFDTEntry, BACnetHostNPort};
use crate::enums::{ErrorClass, ErrorCode, IPMode};
use crate::error::Error;

/// A live view of one BBMD's BDT and FDT.
///
/// Each call reports the tables as they stand at that moment. The Network
/// Port object calls these while a reader holds the server's object
/// database, so an implementation must answer promptly: no I/O and no
/// waiting on anything but a short critical section.
pub trait BbmdTables: Send + Sync {
    /// The BDT, this BBMD's own row included, in table order: the value of
    /// BBMD_Broadcast_Distribution_Table.
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry>;

    /// Whether the BBMD accepts Register-Foreign-Device requests: the value
    /// of BBMD_Accept_FD_Registrations.
    fn accepts_foreign_device_registrations(&self) -> bool;

    /// The registrations that have not expired, each with the seconds it has
    /// left: the value of BBMD_Foreign_Device_Table.
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry>;

    /// Check a BDT written to BBMD_Broadcast_Distribution_Table, changing
    /// nothing: `Ok` when [`activate`](Self::activate) would take it. The
    /// default refuses every table, as for tables that can't be changed.
    fn check_broadcast_distribution_table(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        let _ = table;
        Err(write_access_denied())
    }

    /// Whether the BBMD runs behind a NAT router (Annex J.7.5): the value of
    /// BACnet_IP_NAT_Traversal. `None`, the default, for a BBMD that can't,
    /// whose port then has neither NAT row (Table 12-71, footnotes 9 and
    /// 10). An IPv4 BBMD only: B/IPv6 has no NAT rows.
    fn nat_traversal(&self) -> Option<bool> {
        None
    }

    /// The NAT router's public address and port that reach this BBMD: the
    /// value of BACnet_IP_Global_Address. The `None` host with port 0 when
    /// none is configured (Clause 12.56.33). `None`, the default, for a
    /// BBMD that can't run behind NAT.
    fn global_address(&self) -> Option<BACnetHostNPort> {
        None
    }

    /// Check one written value on its own, as a WriteProperty takes it:
    /// `changes` holds only that value. The default is [`check`](Self::check).
    /// A view whose values depend on one another checks only what the value
    /// alone decides here, and the rest in `check`, at activation.
    fn check_value(&self, changes: &BbmdChanges) -> Result<(), Error> {
        self.check(changes)
    }

    /// Check every written value waiting together, changing nothing: `Ok`
    /// when [`activate`](Self::activate) would take them. The default
    /// checks a BDT alone with
    /// [`check_broadcast_distribution_table`](Self::check_broadcast_distribution_table)
    /// and refuses the NAT values, as for a BBMD that can't run behind NAT.
    fn check(&self, changes: &BbmdChanges) -> Result<(), Error> {
        if changes.nat_traversal.is_some()
            || changes.global_address.is_some()
            || changes.multicast_group.is_some()
        {
            return Err(write_access_denied());
        }
        match &changes.broadcast_distribution_table {
            Some(table) => self.check_broadcast_distribution_table(table),
            None => Ok(()),
        }
    }

    /// Put written values into effect, as a ReinitializeDevice
    /// ACTIVATE_CHANGES or WARMSTART asks (Clause 12.56.13): all of them, or
    /// none and an error. The default refuses.
    fn activate(&self, changes: &BbmdChanges) -> Result<(), Error> {
        let _ = changes;
        Err(write_access_denied())
    }

    /// Whether written values can take effect at all, so the rows are
    /// writable: `true` for a view that implements
    /// [`check_broadcast_distribution_table`](Self::check_broadcast_distribution_table)
    /// and [`activate`](Self::activate). The default, `false`, lists the
    /// rows read-only and refuses writes to them. Override it together
    /// with those methods; a view that overrides them alone still reads as
    /// one that can't change.
    fn can_change(&self) -> bool {
        false
    }

    /// Whether this BBMD's B/IP multicast group (Annex J.8.3) changes
    /// through this view, together with its rows: a written
    /// BACnet_IP_Multicast_Address then waits in
    /// [`BbmdChanges::multicast_group`], is checked with the other values
    /// waiting, and takes effect in the same [`activate`](Self::activate),
    /// all or nothing (#1963). A BBMD that serves a group must give its own
    /// BDT row an all-ones mask, so the group and the table can only change
    /// as one. The default, `false`, leaves the row to the port's
    /// [`IpMulticast`] view.
    fn changes_multicast_group(&self) -> bool {
        false
    }
}

/// Values written to a BBMD's Network Port properties and not yet in effect
/// (Clause 12.56.26, 12.56.32 to 12.56.35): `None` leaves that setting as it
/// is.
///
/// It is `#[non_exhaustive]`: outside this crate, start from
/// [`BbmdChanges::default`] and set the fields that change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BbmdChanges {
    /// A new BDT: BBMD_Broadcast_Distribution_Table.
    pub broadcast_distribution_table: Option<Vec<BACnetBDTEntry>>,
    /// Whether to accept foreign-device registrations:
    /// BBMD_Accept_FD_Registrations.
    pub accept_foreign_device_registrations: Option<bool>,
    /// Whether to run behind a NAT router: BACnet_IP_NAT_Traversal.
    pub nat_traversal: Option<bool>,
    /// The NAT router's public address and port: BACnet_IP_Global_Address.
    /// The `None` host with port 0 clears it.
    pub global_address: Option<BACnetHostNPort>,
    /// A new B/IP multicast group, most significant octet first, or
    /// 0.0.0.0 to leave multicast: BACnet_IP_Multicast_Address, on a BBMD
    /// whose view takes it ([`BbmdTables::changes_multicast_group`]).
    pub multicast_group: Option<[u8; 4]>,
}

impl BbmdChanges {
    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.broadcast_distribution_table.is_none()
            && self.accept_foreign_device_registrations.is_none()
            && self.nat_traversal.is_none()
            && self.global_address.is_none()
            && self.multicast_group.is_none()
    }
}

/// A live view of a foreign device's registration: the BBMD it registers
/// with and the time-to-live it asks for.
///
/// As with [`BbmdTables`], the Network Port object calls these while a
/// reader holds the server's object database, so each must answer
/// promptly, with no I/O.
pub trait ForeignRegistration: Send + Sync {
    /// The BBMD it registers with: the value of FD_BBMD_Address.
    fn bbmd(&self) -> BACnetHostNPort;

    /// The time-to-live, in seconds, it registers for: the value of
    /// FD_Subscription_Lifetime.
    fn subscription_lifetime(&self) -> u16;

    /// Check written values, changing nothing: `Ok` when
    /// [`activate`](Self::activate) would take them. `changes` holds every
    /// value waiting, so a check sees the pair it would activate. The
    /// default refuses, as for a registration that can't change.
    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let _ = changes;
        Err(write_access_denied())
    }

    /// Put written values into effect, as a ReinitializeDevice
    /// ACTIVATE_CHANGES or WARMSTART asks (Clause 12.56.13): register with
    /// the new BBMD or time-to-live from now on, or change nothing and
    /// fail. The default refuses.
    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        let _ = changes;
        Err(write_access_denied())
    }

    /// Whether written values can take effect at all, so the rows are
    /// writable: `true` for a view that implements [`check`](Self::check)
    /// and [`activate`](Self::activate). The default, `false`, lists the
    /// rows read-only and refuses writes to them. Override it together
    /// with those methods; a view that overrides them alone still reads as
    /// one that can't change.
    fn can_change(&self) -> bool {
        false
    }
}

/// A registration that never changes: it reports `bbmd` and
/// `subscription_lifetime`, and refuses every written value. For a port
/// that can't register again while it runs, and for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedForeignRegistration {
    /// FD_BBMD_Address.
    pub bbmd: BACnetHostNPort,
    /// FD_Subscription_Lifetime, in seconds.
    pub subscription_lifetime: u16,
}

impl ForeignRegistration for FixedForeignRegistration {
    fn bbmd(&self) -> BACnetHostNPort {
        self.bbmd.clone()
    }

    fn subscription_lifetime(&self) -> u16 {
        self.subscription_lifetime
    }
}

/// Values written to a foreign device's Network Port properties and not
/// yet in effect (Clause 12.56.37, 12.56.38): `None` leaves that setting
/// as it is.
///
/// It is `#[non_exhaustive]`: outside this crate, start from
/// [`ForeignDeviceChanges::default`] and set the fields that change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ForeignDeviceChanges {
    /// A new BBMD to register with: FD_BBMD_Address.
    pub bbmd: Option<BACnetHostNPort>,
    /// A new time-to-live, in seconds: FD_Subscription_Lifetime.
    pub subscription_lifetime: Option<u16>,
}

impl ForeignDeviceChanges {
    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.bbmd.is_none() && self.subscription_lifetime.is_none()
    }
}

/// A live view of a B/IP port's multicast group (Annex J.8): the value of
/// BACnet_IP_Multicast_Address (Clause 12.56.26).
///
/// A transport lends it for a port that can use B/IP multicast, whatever
/// its mode, and the Network Port object then serves the row (Table 12-71,
/// footnote 7). As with [`BbmdTables`], the object calls these while a
/// reader holds the server's object database, so each must answer
/// promptly, with no I/O beyond a short, non-blocking socket option.
pub trait IpMulticast: Send + Sync {
    /// The group the port's broadcasts go to, most significant octet first,
    /// or 0.0.0.0 while the port doesn't use multicast.
    fn group(&self) -> [u8; 4];

    /// Check a written group, changing nothing: `Ok` when
    /// [`activate`](Self::activate) would take it. 0.0.0.0 turns multicast
    /// off. The default refuses, as for a group that can't change.
    fn check(&self, group: [u8; 4]) -> Result<(), Error> {
        let _ = group;
        Err(write_access_denied())
    }

    /// Put a written group into effect, as a ReinitializeDevice
    /// ACTIVATE_CHANGES or WARMSTART asks (Clause 12.56.13): join it and
    /// send broadcasts to it from now on, or leave multicast with 0.0.0.0.
    /// It changes nothing and fails when it can't. The default refuses.
    fn activate(&self, group: [u8; 4]) -> Result<(), Error> {
        let _ = group;
        Err(write_access_denied())
    }

    /// Whether a written group can take effect at all, so the row is
    /// writable: `true` for a view that implements [`check`](Self::check)
    /// and [`activate`](Self::activate). The default, `false`, lists the
    /// row read-only and refuses writes to it.
    fn can_change(&self) -> bool {
        false
    }
}

fn write_access_denied() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
}

/// A B/IP or B/IPv6 port's mode, with what that mode's Network Port
/// properties need.
#[derive(Clone)]
pub enum BipPortMode {
    /// Plain B/IP: the port relays no broadcasts and joins no BBMD.
    Normal,
    /// The port joins a remote BBMD's network by registering with it.
    Foreign {
        /// Its registration, read live: FD_BBMD_Address and
        /// FD_Subscription_Lifetime.
        registration: Arc<dyn ForeignRegistration>,
    },
    /// The port relays broadcasts for its subnet as a BBMD.
    Bbmd {
        /// The BBMD's tables. `None` while a transport is configured but not
        /// yet started, since starting creates them.
        tables: Option<Arc<dyn BbmdTables>>,
    },
}

impl BipPortMode {
    /// FOREIGN mode with a registration that never changes
    /// ([`FixedForeignRegistration`]).
    pub fn fixed_foreign(bbmd: BACnetHostNPort, subscription_lifetime: u16) -> Self {
        Self::Foreign {
            registration: Arc::new(FixedForeignRegistration {
                bbmd,
                subscription_lifetime,
            }),
        }
    }

    /// The BACnet_IP_Mode value for this mode.
    pub fn ip_mode(&self) -> IPMode {
        match self {
            Self::Normal => IPMode::NORMAL,
            Self::Foreign { .. } => IPMode::FOREIGN,
            Self::Bbmd { .. } => IPMode::BBMD,
        }
    }
}

impl fmt::Debug for BipPortMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => f.write_str("Normal"),
            Self::Foreign { registration } => f
                .debug_struct("Foreign")
                .field("bbmd", &registration.bbmd())
                .field(
                    "subscription_lifetime",
                    &registration.subscription_lifetime(),
                )
                .finish(),
            Self::Bbmd { tables } => f
                .debug_struct("Bbmd")
                .field("started", &tables.is_some())
                .finish(),
        }
    }
}
