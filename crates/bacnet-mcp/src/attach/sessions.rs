//! The attached agents' sessions: one MCP server each over the shared
//! engine, all sharing one watch hub (so one point cap) and one principal
//! bucket, counted, and audited when they open and close and when a peer
//! is refused. While any is attached, the profile's `[mcp]` limits join
//! the engine's. They serve any byte stream, so the Windows named pipe
//! will use them as the Unix socket does.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_ops::engine::Engine;
use bacnet_ops::limits::Overlay;
use bacnet_ops::safety::audit::Audit;
use bacnet_ops::safety::{FrontEnd, Level};
use bacnet_transport::port::TransportPort;
use futures_util::FutureExt as _;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{MAX_AGENTS, PeerId};
use crate::McpServer;
use crate::policy::{Policy, Workspace};
use crate::server::Shared;

/// How long a connection has to send its first MCP message.
pub(super) const INITIALIZE_WAIT: Duration = Duration::from_secs(30);

/// Refusals recorded a minute; the rest are counted, and the count goes
/// with the next record and at shutdown.
const REFUSALS_PER_MINUTE: u32 = 10;

/// What every session of one listener shares.
pub(super) struct Sessions<T: TransportPort + 'static> {
    pub(super) engine: Arc<Engine<T>>,
    /// Read-only: the listener dropped the write policy.
    pub(super) policy: Policy,
    /// The `[mcp]` limits, joined to the engine's while an agent is in.
    pub(super) overlay: Overlay,
    /// The watch hub and principal bucket every session shares.
    pub(super) shared: Shared<T>,
    pub(super) principal: String,
    pub(super) workspace: Workspace,
    /// Cancelled at shutdown: every session then ends gracefully.
    pub(super) stop: CancellationToken,
    pub(super) tracker: TaskTracker,
    /// Agents attached now.
    pub(super) agents: watch::Sender<usize>,
    /// The next connection's number, for the audit log.
    pub(super) next: AtomicU64,
    pub(super) refusals: Arc<Mutex<Refusals>>,
}

/// The refusal records of the current minute, and those left out.
#[derive(Debug, Default)]
pub(super) struct Refusals {
    since: Option<Instant>,
    recorded: u32,
    pub(super) suppressed: u64,
}

impl Refusals {
    /// Whether a refusal now gets a record, and if so how many were left
    /// out before it (counted afresh from then).
    fn admit(&mut self, now: Instant) -> Option<u64> {
        if self
            .since
            .is_none_or(|since| now.saturating_duration_since(since) >= Duration::from_secs(60))
        {
            self.since = Some(now);
            self.recorded = 0;
        }
        if self.recorded >= REFUSALS_PER_MINUTE {
            self.suppressed += 1;
            return None;
        }
        self.recorded += 1;
        Some(std::mem::take(&mut self.suppressed))
    }
}

/// Join the `[mcp]` limits to the engine's, or take them off, and say so.
fn join_limits<T: TransportPort + 'static>(engine: &Engine<T>, overlay: Option<&Overlay>) {
    engine.limits().set_overlay(overlay);
    match overlay {
        Some(overlay) => tracing::info!(
            "MCP: an agent is attached, so the [mcp] limits join the session's: at most {} \
             requests a second overall (or the session's own cap if lower), {} a network, and \
             {:?} for the networks named",
            overlay.global,
            overlay.network,
            overlay.rates
        ),
        None => tracing::info!("MCP: no agent is attached; the session's own limits apply again"),
    }
}

/// An agent's place in the count, given back however its session ends;
/// the last one out takes the `[mcp]` limits off.
struct Seat<T: TransportPort + 'static> {
    agents: watch::Sender<usize>,
    engine: Arc<Engine<T>>,
}

impl<T: TransportPort + 'static> Drop for Seat<T> {
    fn drop(&mut self) {
        self.agents.send_modify(|n| {
            *n = n.saturating_sub(1);
            if *n == 0 {
                join_limits(&self.engine, None);
            }
        });
    }
}

impl<T: TransportPort + 'static> Sessions<T> {
    /// Serve an admitted peer on `stream` until it closes or the listener
    /// shuts down, unless [`MAX_AGENTS`] are attached already.
    pub(super) fn open<S>(&self, stream: S, peer: PeerId)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let mut admitted = false;
        // The count and the limits change together, under the count's lock.
        self.agents.send_if_modified(|n| {
            admitted = *n < MAX_AGENTS;
            if admitted {
                if *n == 0 {
                    join_limits(&self.engine, Some(&self.overlay));
                }
                *n += 1;
            }
            admitted
        });
        if !admitted {
            return self.refuse(Some(peer), "too-many-agents");
        }
        let seat = Seat {
            agents: self.agents.clone(),
            engine: Arc::clone(&self.engine),
        };
        let server = match McpServer::attached(
            Arc::clone(&self.engine),
            self.policy.clone(),
            self.principal.clone(),
            &self.shared,
            INITIALIZE_WAIT,
        ) {
            Ok(server) => server,
            Err(error) => {
                tracing::error!("an attached MCP session can't start: {error}");
                return self.refuse_with(Some(peer), "session-failed", Some(error.to_string()));
            }
        };
        let connection = self.next.fetch_add(1, Ordering::Relaxed);
        let (workspace, principal) = (self.workspace.clone(), self.principal.clone());
        let stop = self.stop.clone();
        self.tracker.spawn(async move {
            let mut fields = peer_fields(Some(peer), connection);
            record(&workspace, &principal, "mcp-connect", fields.clone()).await;
            tracing::info!("MCP agent {connection} attached (pid {:?})", peer.pid);
            let (reader, writer) = tokio::io::split(stream);
            let serving = server.serve_until(reader, writer, stop.cancelled_owned());
            // rmcp serves requests on tasks of its own, whose panics come
            // back as the session's error. This catches a panic polled on
            // this task: in the initialize handshake (where the server's
            // `initialize` handler runs) or the session's own clean-up, so
            // its close is still on record.
            let served = AssertUnwindSafe(serving).catch_unwind().await;
            drop(seat);
            match served {
                Ok(Ok(())) => {}
                Ok(Err(error)) => fields["error"] = json!(error.to_string()),
                Err(_) => fields["error"] = json!("the session panicked"),
            }
            tracing::info!("MCP agent {connection} detached");
            record(&workspace, &principal, "mcp-disconnect", fields).await;
        });
    }

    /// Audit a connection closed before any MCP traffic, and `why`, up to
    /// [`REFUSALS_PER_MINUTE`] records a minute.
    pub(super) fn refuse(&self, peer: Option<PeerId>, why: &'static str) {
        self.refuse_with(peer, why, None);
    }

    /// [`refuse`](Self::refuse), with the `error` behind it.
    fn refuse_with(&self, peer: Option<PeerId>, why: &'static str, error: Option<String>) {
        let connection = self.next.fetch_add(1, Ordering::Relaxed);
        let admitted = self
            .refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(Instant::now());
        let Some(left_out) = admitted else {
            return tracing::debug!("MCP socket: refused a connection ({why}), not recorded");
        };
        tracing::warn!(
            "MCP socket: refused a connection ({why}; peer uid {:?}, pid {:?})",
            peer.map(|p| p.uid),
            peer.and_then(|p| p.pid)
        );
        let (workspace, principal) = (self.workspace.clone(), self.principal.clone());
        self.tracker.spawn(async move {
            let mut fields = peer_fields(peer, connection);
            fields["outcome"] = json!({ "result": "refused", "why": why });
            if left_out > 0 {
                fields["refusals_not_recorded_before"] = json!(left_out);
            }
            if let Some(error) = error {
                fields["error"] = json!(error);
            }
            record(&workspace, &principal, "mcp-refused", fields).await;
        });
    }
}

/// The count of refusals left out of the record, as one record at
/// shutdown.
pub(super) async fn record_left_out(workspace: &Workspace, principal: &str, left_out: u64) {
    let fields = json!({
        "via": "local-socket",
        "outcome": { "result": "refused", "why": "not-recorded", "count": left_out },
    });
    record(workspace, principal, "mcp-refused", fields).await;
}

/// The fields every record of a connection carries.
fn peer_fields(peer: Option<PeerId>, connection: u64) -> Value {
    json!({
        "via": "local-socket",
        "connection": connection,
        "peer_uid": peer.map(|p| p.uid),
        "peer_pid": peer.and_then(|p| p.pid),
    })
}

/// Append one record to the workspace's audit log, as the stdio server's
/// write records are appended: without the workspace's lock.
async fn record(workspace: &Workspace, principal: &str, event: &str, fields: Value) {
    let context = crate::tools::write::context(workspace);
    let written = match Audit::open(&workspace.dir, context) {
        Ok(audit) => {
            audit
                .written_by(FrontEnd::Mcp, Some(principal.to_string()))
                .record(Level::ReadOnly, event, fields)
                .await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = written {
        tracing::error!("{error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten refusals a minute are recorded; the rest are counted and the
    /// count goes with the next record.
    #[tokio::test(start_paused = true)]
    async fn refusal_records_are_bounded() {
        let mut refusals = Refusals::default();
        let now = Instant::now();
        let recorded = (0..15).filter(|_| refusals.admit(now).is_some()).count();
        assert_eq!(recorded, 10);
        assert_eq!(refusals.suppressed, 5);
        let later = now + Duration::from_secs(60);
        assert_eq!(refusals.admit(later), Some(5), "the count comes along");
        assert_eq!(refusals.suppressed, 0);
    }
}
