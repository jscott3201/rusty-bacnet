//! BBMD table reads through the engine, under the rate cap: the read side
//! of BVLL only (Annex J.2.3 and J.2.7), never a table write or a
//! registration.

use std::any::Any;
use std::net::SocketAddrV4;

use bacnet_transport::bip::{BipTransport, BvlcExchangeError};
use bacnet_transport::port::TransportPort;

use super::Engine;
use crate::bbmd::{BdtRow, FdtRow, Outcome, Probe, mac};

impl<T: TransportPort + 'static> Engine<T> {
    /// The client's B/IP transport, when it has one: the BBMD reads need it.
    pub fn bip(&self) -> Option<&BipTransport> {
        (self.client.transport() as &dyn Any).downcast_ref()
    }

    /// The BBMD the client last tried to register with as a foreign device,
    /// if any. Reads the transport's registration state and sends nothing.
    pub fn registration_bbmd(&self) -> Option<SocketAddrV4> {
        self.bip()?
            .bvlc_client_snapshot()
            .foreign_registration
            .last_bbmd
    }

    /// Read `bbmd`'s BDT, after a slot under the local network's rate and
    /// the global one. On a client without B/IP it sends nothing.
    pub async fn read_bdt(&self, bbmd: SocketAddrV4) -> Probe {
        let Some(bip) = self.bip() else {
            return Outcome::Unreachable(NOT_BIP.into());
        };
        self.limits.slot(Some(0)).await;
        settle(bip.read_bdt(&mac(bbmd)).await, |e| BdtRow::from(e))
    }

    /// Read `bbmd`'s FDT, as [`read_bdt`](Self::read_bdt).
    pub async fn read_fdt(&self, bbmd: SocketAddrV4) -> Outcome<Vec<FdtRow>> {
        let Some(bip) = self.bip() else {
            return Outcome::Unreachable(NOT_BIP.into());
        };
        self.limits.slot(Some(0)).await;
        settle(bip.read_fdt(&mac(bbmd)).await, |e| FdtRow::from(e))
    }
}

/// Why a client without B/IP reads no BBMD.
const NOT_BIP: &str = "BBMD tables need BACnet/IP";

/// A table read's outcome. A request the node refused before sending
/// never left, so the call isn't charged for it.
fn settle<E, R>(read: Result<Vec<E>, BvlcExchangeError>, row: fn(&E) -> R) -> Outcome<Vec<R>> {
    match read {
        Ok(entries) => Outcome::Table(entries.iter().map(row).collect()),
        Err(error) => {
            if matches!(error, BvlcExchangeError::Refused(_)) {
                crate::limits::refund();
            }
            Outcome::from_error(&error)
        }
    }
}
