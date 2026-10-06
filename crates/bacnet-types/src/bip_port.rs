//! The B/IP operating mode a transport lends the Network Port object.
//!
//! A B/IP port runs in NORMAL, FOREIGN or BBMD mode (Clause 12.56.21), and
//! which Network Port properties exist depends on the mode (Table 12-71,
//! footnotes 11 to 13). The transport layer owns that state and the object
//! layer serves it, and neither crate depends on the other, so the contract
//! between them lives here.
//!
//! A BBMD's tables change at run time, so they are lent as a live view,
//! [`BbmdTables`], which the object reads on each property read. Nothing is
//! copied, so the object cannot drift from the transport.

use core::fmt;

#[cfg(not(feature = "std"))]
use alloc::{sync::Arc, vec::Vec};
#[cfg(feature = "std")]
use std::sync::Arc;

use crate::constructed::{BACnetBDTEntry, BACnetFDTEntry, BACnetHostNPort};
use crate::enums::IPMode;

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
}

/// A B/IP port's mode, with what that mode's Network Port properties need.
#[derive(Clone)]
pub enum BipPortMode {
    /// Plain B/IP: the port relays no broadcasts and joins no BBMD.
    Normal,
    /// The port joins a remote BBMD's network by registering with it.
    Foreign {
        /// The BBMD it registers with: FD_BBMD_Address.
        bbmd: BACnetHostNPort,
        /// The time to live, in seconds, it registers for:
        /// FD_Subscription_Lifetime.
        subscription_lifetime: u16,
    },
    /// The port relays broadcasts for its subnet as a BBMD.
    Bbmd {
        /// The BBMD's tables. `None` while a transport is configured but not
        /// yet started, since starting creates them.
        tables: Option<Arc<dyn BbmdTables>>,
    },
}

impl BipPortMode {
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
            Self::Foreign {
                bbmd,
                subscription_lifetime,
            } => f
                .debug_struct("Foreign")
                .field("bbmd", bbmd)
                .field("subscription_lifetime", subscription_lifetime)
                .finish(),
            Self::Bbmd { tables } => f
                .debug_struct("Bbmd")
                .field("started", &tables.is_some())
                .finish(),
        }
    }
}
