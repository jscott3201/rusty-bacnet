//! The MS/TP screen's messages and the worker side of it (#969): the
//! station a session joined as, and samples of its diagnostics handle.
//!
//! The transport keeps the counters and the node's state; the worker reads
//! them once a second through the handle it took before the client owned
//! the transport. Reading takes two short `std` locks inside the handle and
//! never awaits while holding one.

use std::time::Duration;

use bacnet_transport::mstp::{
    MasterState, MstpDiagnostics, MstpDiagnosticsSnapshot, MstpNodeState,
};

use super::message::WorkerEvent;
use super::worker::EventSink;
use crate::mstp::MstpArgs;

/// How often the worker samples the diagnostics handle.
pub(crate) const SAMPLE_EVERY: Duration = Duration::from_secs(1);

/// The node's place in the ring, ready to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeView {
    /// The master state's Clause 9.5.6 name.
    pub(crate) state: &'static str,
    /// The station holds the token (using it, polling or awaiting a reply).
    pub(crate) holds_token: bool,
    /// The station it passes the token to.
    pub(crate) next_station: u8,
    /// The station it polls next for a new master.
    pub(crate) poll_station: u8,
    /// It believes it is the only master.
    pub(crate) sole_master: bool,
}

impl From<MstpNodeState> for NodeView {
    fn from(node: MstpNodeState) -> Self {
        let (state, holds_token) = match node.state {
            MasterState::Idle => ("IDLE", false),
            MasterState::NoToken => ("NO_TOKEN", false),
            MasterState::UseToken => ("USE_TOKEN", true),
            MasterState::DoneWithToken => ("DONE_WITH_TOKEN", true),
            MasterState::PassToken => ("PASS_TOKEN", false),
            MasterState::WaitForReply => ("WAIT_FOR_REPLY", true),
            MasterState::PollForMaster => ("POLL_FOR_MASTER", true),
            MasterState::AnswerDataRequest => ("ANSWER_DATA_REQUEST", false),
        };
        Self {
            state,
            holds_token,
            next_station: node.next_station,
            poll_station: node.poll_station,
            sole_master: node.sole_master,
        }
    }
}

/// One reading of the diagnostics handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sample {
    /// `None` until the transport first publishes it.
    pub(crate) node: Option<NodeView>,
    /// The counters since the transport started.
    pub(crate) counters: MstpDiagnosticsSnapshot,
}

impl Sample {
    /// Read `handle` now.
    pub(crate) fn of(handle: &MstpDiagnostics) -> Self {
        Self {
            node: handle.node_state().map(NodeView::from),
            counters: handle.snapshot(),
        }
    }
}

/// What the MS/TP worker reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MstpReport {
    /// The session joined the trunk with these settings; the text says
    /// how its loop runs, as the transport read it back
    /// ([`crate::mstp_thread::status`]).
    Joined(MstpArgs, String),
    /// A new reading.
    Sample(Box<Sample>),
}

/// Report a sample of `handle` every [`SAMPLE_EVERY`] until the session is
/// cancelled; then wait for good, for the caller's `select!` to drop.
/// Samples are offered, never awaited: a full channel drops one.
pub(crate) async fn sample(handle: MstpDiagnostics, sink: EventSink) {
    let mut every = tokio::time::interval(SAMPLE_EVERY);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        let report = WorkerEvent::Mstp(MstpReport::Sample(Box::new(Sample::of(&handle))));
        if let super::worker::Offer::Closed = sink.offer(report) {
            return std::future::pending().await;
        }
    }
}
