//! What the hub connection lets this node send: the APDU limit the network
//! layer budgets with, and the NPDU limit a router reads (#1665) and
//! follows (#1696).
//!
//! Moved out of the transport file to keep that file within the
//! repository file-size cap.

use std::sync::atomic::{AtomicU16, Ordering};

use tokio::sync::watch;

use super::{DEFAULT_MAX_APDU_LENGTH, ScConnection, ScTransport, WebSocketPort};
use crate::sc_limits::LOCAL_NPDU_HEADER_LEN;
use bacnet_npdu::NpduLimits;

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

/// The limits of the hub connection, published each time one is
/// established: at start, on failover to the other hub, on reconnect and on
/// a primary restore.
#[derive(Debug)]
pub(super) struct HubLimits {
    /// The APDU limit: the NPDU limit less the 2-octet NPDU header.
    apdu: AtomicU16,
    /// The NPDU limit, both ways, for a router to follow.
    npdu: watch::Sender<NpduLimits>,
}

impl HubLimits {
    /// The limits before any hub connection: the default APDU limit and
    /// [`NpduLimits::STANDARD`].
    pub(super) fn new() -> Self {
        Self {
            apdu: AtomicU16::new(DEFAULT_MAX_APDU_LENGTH),
            npdu: watch::Sender::new(NpduLimits::default()),
        }
    }

    /// Publish what `conn`'s hub takes. The NPDU limit is sent on only when
    /// it changed, so a router hears of each change and of nothing else.
    pub(super) fn publish(&self, conn: &ScConnection) {
        let npdu = effective_max_npdu_length(conn);
        self.apdu.store(
            npdu.saturating_sub(LOCAL_NPDU_HEADER_LEN),
            Ordering::Relaxed,
        );
        let limits = NpduLimits::both(npdu);
        self.npdu.send_if_modified(|current| {
            let changed = *current != limits;
            *current = limits;
            changed
        });
    }

    /// The APDU limit now.
    pub(super) fn apdu(&self) -> u16 {
        self.apdu.load(Ordering::Relaxed)
    }

    /// Back to the default APDU limit, when the transport aborts. The NPDU
    /// limit stays as published: nothing routes through a stopped
    /// transport, and [`ScTransport::hub_npdu_limits`] reports the default
    /// for one.
    pub(super) fn reset_apdu(&self) {
        self.apdu.store(DEFAULT_MAX_APDU_LENGTH, Ordering::Relaxed);
    }

    /// The NPDU limit now.
    fn npdu(&self) -> NpduLimits {
        *self.npdu.borrow()
    }

    /// A receiver of the NPDU limit's changes.
    pub(super) fn npdu_changes(&self) -> watch::Receiver<NpduLimits> {
        self.npdu.subscribe()
    }
}

impl<W: WebSocketPort> ScTransport<W> {
    /// [`TransportPort::max_npdu`](crate::port::TransportPort::max_npdu):
    /// once started, what the hub connection carries, both ways, since a
    /// broadcast goes through the hub too. A reconnect or failover can
    /// change it, and a direct connection's own limit isn't known here.
    /// Before start, the default.
    pub(super) fn hub_npdu_limits(&self) -> NpduLimits {
        if self.connection.is_none() {
            return NpduLimits::default();
        }
        self.hub_limits.npdu()
    }
}
