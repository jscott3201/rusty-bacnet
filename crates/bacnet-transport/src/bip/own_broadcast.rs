//! Forwarding of a BBMD's own broadcasts (Annex J.4.5, #937).
//!
//! A BBMD forwards each broadcast that starts on its subnet to the other BDT
//! subnets and to its registered foreign devices. The BBMD's own device is a
//! node on that subnet, but the receive loop discards the transport's own
//! broadcast echo, so `send_broadcast` hands each NPDU it broadcasts to this
//! forwarder instead.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use tracing::debug;

use crate::bbmd::{self, BbmdState};

use super::fanout::FanoutDispatcher;

/// Owned handle the send path uses to forward this BBMD's own broadcasts
/// through the receive loop's fanout dispatcher, so the same budgets, queue
/// and counters apply. The per-origin budget is keyed on the origin IP, so
/// these forwards share it with everything forwarded with this BBMD's IP as
/// origin, including the broadcasts it routes.
pub(super) struct OwnBroadcastForwarder {
    bbmd: Arc<std::sync::Mutex<BbmdState>>,
    fanout: FanoutDispatcher,
}

impl OwnBroadcastForwarder {
    pub(super) fn new(bbmd: Arc<std::sync::Mutex<BbmdState>>, fanout: FanoutDispatcher) -> Self {
        Self { bbmd, fanout }
    }

    /// Queue `npdu` as a Forwarded-NPDU whose originating address is this
    /// BBMD's own B/IP address, for every BDT entry but its own (inverted mask
    /// ORed with the entry address) and the registered foreign devices, at
    /// most `ForeignDevicePolicy::max_fdt_fanout` of them.
    ///
    /// Nothing is returned to the caller, as for an inbound
    /// Original-Broadcast-NPDU. Throttled targets, a full queue and failed
    /// sends are counted in the fanout counters and logged; a closed fanout
    /// worker or a Forwarded-NPDU that cannot be encoded is only logged.
    pub(super) async fn forward(&self, npdu: &[u8]) {
        let ((origin_ip, origin_port), targets, dedup_count) = {
            let mut state = bbmd::lock(&self.bbmd);
            let origin = state.local_address();
            let before = state.fdt_counters().destinations_deduplicated;
            // The self BDT row is always skipped; excluding the origin as well
            // only drops a foreign device registered from this exact address.
            let targets = state.forwarding_targets(origin.0, origin.1);
            let after = state.fdt_counters().destinations_deduplicated;
            (origin, targets, after.saturating_sub(before))
        };
        let addrs = targets
            .into_iter()
            .map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port))
            .collect();
        if !self
            .fanout
            .dispatch_forwarded_npdu(origin_ip, origin_port, npdu, addrs, dedup_count)
        {
            debug!("BIP BBMD: own broadcast was not forwarded to any BDT peer or foreign device");
        }
    }
}
