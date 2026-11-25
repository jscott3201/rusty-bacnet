//! The transport's counters, read: moved out of the transport file to keep
//! that file within the repository file-size cap.

use super::*;

impl BipTransport {
    /// Return the operational BBMD management counters.
    pub fn management_counters(&self) -> ManagementCounters {
        self.management_limiter.locked().counters()
    }

    /// Return operational Foreign Device Table counters if BBMD mode is enabled.
    pub fn fdt_counters(&self) -> Option<FdtCounters> {
        self.bbmd
            .as_ref()
            .map(|bbmd| bbmd::lock(bbmd).fdt_counters())
    }

    /// Return the operational broadcast forwarding fanout counters.
    pub fn fanout_counters(&self) -> FanoutCounters {
        self.fanout_counters.snapshot()
    }

    /// Return the datagrams the receive path dropped since this transport
    /// was created, by reason (see [`ReceiveCounters`]).
    pub fn receive_counters(&self) -> ReceiveCounters {
        self.receive_drops.snapshot()
    }

    /// Return the NPDUs this transport refused to send since it was
    /// created, by reason (see [`SendCounters`]).
    pub fn send_counters(&self) -> SendCounters {
        self.send_refusals.snapshot()
    }
}
