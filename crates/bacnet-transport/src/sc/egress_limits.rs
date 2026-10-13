//! What the hub connection lets this node send: the APDU limit the network
//! layer budgets with, and the NPDU limit a router reads (#1665).
//!
//! Moved out of the transport file to keep that file within the
//! repository file-size cap.

use std::sync::atomic::{AtomicU16, Ordering};

use super::{ScConnection, ScTransport, WebSocketPort};
use crate::port::NpduLimits;
use crate::sc_limits::LOCAL_NPDU_HEADER_LEN;

/// The BVLC octets around an NPDU sent to the hub with no options: the
/// header and the destination VMAC.
const SC_ENCAPSULATED_NPDU_BASE_HEADER_LEN: u16 = 10;

/// The longest NPDU `conn`'s hub takes from this node: its Max-NPDU-Length,
/// within its Max-BVLC-Length less the encapsulation.
fn effective_max_npdu_length(conn: &ScConnection) -> u16 {
    let bvlc_npdu_budget = conn
        .hub_max_bvlc_length
        .saturating_sub(SC_ENCAPSULATED_NPDU_BASE_HEADER_LEN);
    conn.hub_max_npdu_length.min(bvlc_npdu_budget)
}

/// Store the APDU limit of `conn`'s hub: its NPDU limit less the 2-octet
/// NPDU header.
pub(super) fn publish_effective_max_apdu_length(store: &AtomicU16, conn: &ScConnection) {
    let apdu = effective_max_npdu_length(conn).saturating_sub(LOCAL_NPDU_HEADER_LEN);
    store.store(apdu, Ordering::Relaxed);
}

impl<W: WebSocketPort> ScTransport<W> {
    /// [`TransportPort::max_npdu`](crate::port::TransportPort::max_npdu):
    /// once started, what the hub connection carries, the published APDU
    /// limit with its NPDU header back on. Both ways, since a broadcast goes
    /// through the hub too. A reconnect or failover can change it, and a
    /// direct connection's own limit isn't known here. Before start, the
    /// default.
    pub(super) fn hub_npdu_limits(&self) -> NpduLimits {
        if self.connection.is_none() {
            return NpduLimits::default();
        }
        let apdu = self.effective_max_apdu_length.load(Ordering::Relaxed);
        NpduLimits::both(apdu.saturating_add(LOCAL_NPDU_HEADER_LEN))
    }
}
