//! Application work waits while the host task awaits its serial write.
use bacnet_mstp_core::{MasterState, RequestId};

use super::state::{Decision, WireRequest};
use crate::{sim::Sim, ViolationKind};

/// Same finite number of deferred replies as the production host adapter.
const DECISION_BOUND: usize = 8;

impl Sim {
    /// The event is ready, but the single adapter task cannot poll it while
    /// transmit/drain/echo is awaiting. State dies with this incarnation.
    pub(crate) fn host_defer_decision(
        &mut self,
        node: usize,
        request: RequestId,
        requester: u8,
        postpone: bool,
        payload: Vec<u8>,
    ) {
        let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
        if host.decisions.len() == DECISION_BOUND {
            self.violation(
                ViolationKind::EventGrowth,
                "host ready application decisions exceeded 8".into(),
            );
            return;
        }
        host.decisions.push_back(Decision {
            request,
            requester,
            postpone,
            payload,
        });
    }

    pub(super) fn host_decisions(&mut self, node: usize) {
        loop {
            let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
            if host.write.is_some() {
                return;
            }
            let Some(decision) = host.decisions.pop_front() else {
                return;
            };
            self.decide(
                node,
                decision.request,
                decision.requester,
                decision.postpone,
                decision.payload,
            );
        }
    }

    /// Only an actual request-bearing delivery establishes an association.
    /// The core sees the read timestamp; the oracle retains the physical
    /// final stop bit captured before buffering and echo processing.
    pub(crate) fn host_accept_request(&mut self, node: usize, id: RequestId, source: u8) {
        let master = self.master_mut(node).unwrap();
        let Some(host) = master.host.as_mut() else {
            return;
        };
        let end = host.receiving.expect("request delivered by a host octet");
        host.request = Some(WireRequest {
            id,
            source,
            incarnation: master.incarnation,
            end,
        });
    }

    /// The core clears its application RequestId as soon as it queues an
    /// answer, possibly before turnaround permits a Transmit action. Keep
    /// provenance through that wait, admission retries and write completion.
    pub(crate) fn host_finish_request(&mut self, node: usize) {
        let master = self.master_mut(node).unwrap();
        let answering = master.core.as_ref().unwrap().state() == MasterState::AnswerDataRequest;
        if let Some(host) = master.host.as_mut() {
            if !answering {
                host.request = None;
            }
        }
    }
}
