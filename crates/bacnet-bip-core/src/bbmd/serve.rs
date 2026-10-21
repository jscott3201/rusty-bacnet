//! The management requests a BBMD answers from its tables (Annex J.4.4.1,
//! J.5.2.1.1, J.5.2.1.2, J.5.2.2).

use core::fmt;
use core::net::SocketAddrV4;

use bacnet_bip_codec::{BvlcResultCode, Bvll, FdtEntry};
use bacnet_time::Instant;

use super::fdt::{RegisterOutcome, RegistrationLimits, Reservation, StorageFull};
use super::forward::Tables;
use crate::node::Permissive;

/// A request for a BBMD's tables, which [`Tables::serve`] answers. The
/// node hands it over as
/// [`ForBbmd::Management`](crate::node::ForBbmd::Management), once every
/// receive rule has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// A Register-Foreign-Device.
    Register {
        /// The registrant.
        sender: SocketAddrV4,
        /// The time-to-live it asks for, in seconds.
        ttl: u16,
    },
    /// A Read-Broadcast-Distribution-Table.
    ReadBdt {
        /// The requester.
        sender: SocketAddrV4,
    },
    /// A Read-Foreign-Device-Table.
    ReadFdt {
        /// The requester.
        sender: SocketAddrV4,
    },
    /// Delete-Foreign-Device-Table-Entry, naming a registration to
    /// remove.
    DeleteFdtEntry {
        /// The requester.
        sender: SocketAddrV4,
        /// The registration to delete.
        entry: SocketAddrV4,
    },
}

impl Request {
    /// Who sent it, and gets the answer.
    #[must_use]
    pub const fn sender(&self) -> SocketAddrV4 {
        match *self {
            Self::Register { sender, .. }
            | Self::ReadBdt { sender }
            | Self::ReadFdt { sender }
            | Self::DeleteFdtEntry { sender, .. } => sender,
        }
    }
}

/// The host's own rules for the requests [`Tables::serve`] answers: which
/// limits a registration is held to, and who may register or delete.
///
/// Every method has a default that admits, so a host implements only the
/// rules it has. [`Permissive`] keeps every default: for firmware and
/// tests.
pub trait Rules {
    /// The limits a Register-Foreign-Device is held to:
    /// [`RegistrationLimits::DEFAULT`] unless the host sets its own.
    fn registration_limits(&mut self) -> RegistrationLimits {
        RegistrationLimits::DEFAULT
    }

    /// Whether a registration from `source` may fill the slots
    /// [`RegistrationLimits::reserved_capacity`] keeps back.
    fn reservation(&mut self, source: SocketAddrV4) -> Reservation {
        let _ = source;
        Reservation::Ordinary
    }

    /// The host's own admission check for a registration from `source`:
    /// an access list, a rate limit. It is asked only once the
    /// time-to-live has passed the limits, as
    /// [`Fdt::register`](super::Fdt::register) asks its `admit`; `false`
    /// refuses the registration.
    fn admit_registration(&mut self, source: SocketAddrV4, now: Instant) -> bool {
        let _ = (source, now);
        true
    }

    /// Whether `sender` may delete `entry`'s registration; `false` answers
    /// with the Delete-Foreign-Device-Table-Entry NAK and changes nothing.
    ///
    /// The default admits every requester, where the hosted BBMD of
    /// `bacnet-transport` denies every one its management access list
    /// doesn't name, an empty list included. A firmware BBMD that wants to
    /// fail closed overrides this.
    fn admit_delete(&mut self, sender: SocketAddrV4, entry: SocketAddrV4) -> bool {
        let _ = (sender, entry);
        true
    }
}

impl Rules for Permissive {}

/// Why Delete-Foreign-Device-Table-Entry deleted nothing (Annex J.5.2.1.2).
///
/// It is `#[non_exhaustive]`, so a new reason isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeleteRefusal {
    /// The table holds no live registration for the entry. A lapsed one
    /// counts as none, since the BBMD would have purged it already.
    NotRegistered,
    /// [`Rules::admit_delete`] said no.
    NotAdmitted,
}

impl DeleteRefusal {
    /// The BVLC-Result to answer the requester with: every refusal is the
    /// Delete-Foreign-Device-Table-Entry NAK.
    #[must_use]
    pub const fn result_code(self) -> BvlcResultCode {
        BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
    }
}

impl fmt::Display for DeleteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotRegistered => "no live registration for that entry",
            Self::NotAdmitted => "deletion not admitted",
        })
    }
}

impl core::error::Error for DeleteRefusal {}

/// What a request came to, besides the answer [`Tables::serve`] built.
///
/// It is `#[non_exhaustive]`, so a new request isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Served {
    /// A Register-Foreign-Device, accepted or refused, and the lapsed
    /// registrations purged before it.
    Register(RegisterOutcome),
    /// A Read-Broadcast-Distribution-Table, answered with every row.
    ReadBdt,
    /// A Read-Foreign-Device-Table, answered with this many live
    /// registrations, or refused because the row storage couldn't hold
    /// them.
    ReadFdt(Result<usize, StorageFull>),
    /// Delete-Foreign-Device-Table-Entry: the registration deleted, or
    /// why not.
    DeleteFdtEntry(Result<(), DeleteRefusal>),
}

/// The answer [`Tables::serve`] built, for the caller to encode and send
/// from the BBMD's socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Answer<'t> {
    /// Where it goes: the requester.
    pub to: SocketAddrV4,
    /// A BVLC-Result, or a table ACK that borrows the tables or the row
    /// storage.
    pub message: Bvll<'t>,
    /// What the request came to.
    pub served: Served,
}

/// The answer to a request that drew `code`.
const fn result(to: SocketAddrV4, code: BvlcResultCode, served: Served) -> Answer<'static> {
    Answer {
        to,
        message: Bvll::BvlcResult(code),
        served,
    }
}

impl<const B: usize, const F: usize> Tables<B, F> {
    /// Answer a management request at `now`, as `rules` allow, writing a
    /// Read-Foreign-Device-Table-Ack's rows into `rows`; the caller sends
    /// the [`Answer`] to the requester.
    ///
    /// - **Register-Foreign-Device** (Annex J.5.2.2, J.5.2.3): registered
    ///   through [`Fdt::register`](super::Fdt::register), held to
    ///   `rules`' limits, reservation and admission check. Success answers
    ///   with Successful completion, which restarts the registration's
    ///   timer of its time-to-live plus the 30-second grace; any refusal,
    ///   a full table included, with the Register-Foreign-Device NAK.
    /// - **Read-Broadcast-Distribution-Table** (Annex J.4.4.1): a
    ///   Read-Broadcast-Distribution-Table-Ack with every row, this BBMD's
    ///   own included. A table never set has no rows, and its ACK an empty
    ///   list (Annex J.2.4).
    /// - **Read-Foreign-Device-Table** (Annex J.5.2.1.1): a
    ///   Read-Foreign-Device-Table-Ack with each live registration, its
    ///   time-to-live and the seconds it has left, grace included (Annex
    ///   J.2.8.1). When `rows` can't hold them the read fails, and the
    ///   answer is the Read-Foreign-Device-Table NAK: storage of `F` rows
    ///   never fails.
    /// - **Delete-Foreign-Device-Table-Entry** (Annex J.5.2.1.2): a live
    ///   registration for the entry is removed, with Successful completion;
    ///   with none, or when [`Rules::admit_delete`] says no, the answer is
    ///   the Delete-Foreign-Device-Table-Entry NAK. A lapsed registration
    ///   counts as none, and is removed too.
    pub fn serve<'t>(
        &'t mut self,
        request: Request,
        now: Instant,
        rules: &mut impl Rules,
        rows: &'t mut [FdtEntry],
    ) -> Answer<'t> {
        match request {
            Request::Register { sender, ttl } => {
                let limits = rules.registration_limits();
                let reservation = rules.reservation(sender);
                let outcome = self
                    .fdt
                    .register(sender, ttl, now, &limits, reservation, || {
                        rules.admit_registration(sender, now)
                    });
                let code = match outcome.result {
                    Ok(_) => BvlcResultCode::SUCCESSFUL_COMPLETION,
                    Err(refusal) => refusal.result_code(),
                };
                result(sender, code, Served::Register(outcome))
            }
            Request::ReadBdt { sender } => {
                let tables: &'t Self = self;
                Answer {
                    to: sender,
                    message: Bvll::ReadBroadcastDistributionTableAck(tables.bdt.entries()),
                    served: Served::ReadBdt,
                }
            }
            Request::ReadFdt { sender } => match self.fdt.wire_entries(now, rows) {
                Ok(entries) => Answer {
                    to: sender,
                    message: Bvll::ReadForeignDeviceTableAck(entries),
                    served: Served::ReadFdt(Ok(entries.len())),
                },
                Err(full) => result(
                    sender,
                    BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
                    Served::ReadFdt(Err(full)),
                ),
            },
            Request::DeleteFdtEntry { sender, entry } => {
                let deleted = if rules.admit_delete(sender, entry) {
                    let live = self.fdt.is_registered_at(entry, now);
                    // A lapsed registration goes too: it was due for purging.
                    let _ = self.fdt.delete(entry);
                    if live {
                        Ok(())
                    } else {
                        Err(DeleteRefusal::NotRegistered)
                    }
                } else {
                    Err(DeleteRefusal::NotAdmitted)
                };
                let code = match deleted {
                    Ok(()) => BvlcResultCode::SUCCESSFUL_COMPLETION,
                    Err(refusal) => refusal.result_code(),
                };
                result(sender, code, Served::DeleteFdtEntry(deleted))
            }
        }
    }
}
