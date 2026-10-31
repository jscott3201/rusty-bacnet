//! The attached agents' sessions: one MCP server each over the shared
//! engine, counted, and audited when they open and close and when a peer
//! is refused. They serve any byte stream, so the Windows named pipe will
//! use them as the Unix socket does.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_ops::engine::Engine;
use bacnet_ops::safety::audit::Audit;
use bacnet_ops::safety::{FrontEnd, Level};
use bacnet_transport::port::TransportPort;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{MAX_AGENTS, PeerId};
use crate::McpServer;
use crate::policy::{Policy, Workspace};

/// What every session of one listener shares.
pub(super) struct Sessions<T: TransportPort + 'static> {
    pub(super) engine: Arc<Engine<T>>,
    /// Read-only: the listener dropped the write policy.
    pub(super) policy: Policy,
    pub(super) principal: String,
    pub(super) workspace: Workspace,
    /// Cancelled at shutdown: every session then ends gracefully.
    pub(super) stop: CancellationToken,
    pub(super) tracker: TaskTracker,
    /// Agents attached now.
    pub(super) agents: watch::Sender<usize>,
    /// The next connection's number, for the audit log.
    pub(super) next: AtomicU64,
}

/// An agent's place in the count, given back however its session ends.
struct Seat(watch::Sender<usize>);

impl Drop for Seat {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n = n.saturating_sub(1));
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
        self.agents.send_if_modified(|n| {
            admitted = *n < MAX_AGENTS;
            if admitted {
                *n += 1;
            }
            admitted
        });
        if !admitted {
            return self.refuse(Some(peer), "too-many-agents");
        }
        let seat = Seat(self.agents.clone());
        let server = match McpServer::new(
            Some(Arc::clone(&self.engine)),
            self.policy.clone(),
            self.principal.clone(),
        ) {
            Ok(server) => server,
            Err(error) => return tracing::error!("an attached MCP session can't start: {error}"),
        };
        let connection = self.next.fetch_add(1, Ordering::Relaxed);
        let (workspace, principal) = (self.workspace.clone(), self.principal.clone());
        let stop = self.stop.clone();
        self.tracker.spawn(async move {
            let mut fields = peer_fields(Some(peer), connection);
            record(&workspace, &principal, "mcp-connect", fields.clone()).await;
            tracing::info!("MCP agent {connection} attached (pid {:?})", peer.pid);
            let (reader, writer) = tokio::io::split(stream);
            let served = server
                .serve_until(reader, writer, stop.cancelled_owned())
                .await;
            drop(seat);
            if let Err(error) = &served {
                fields["error"] = json!(error.to_string());
            }
            tracing::info!("MCP agent {connection} detached");
            record(&workspace, &principal, "mcp-disconnect", fields).await;
        });
    }

    /// Audit a connection closed before any MCP traffic, and `why`.
    pub(super) fn refuse(&self, peer: Option<PeerId>, why: &'static str) {
        let connection = self.next.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            "MCP socket: refused a connection ({why}; peer uid {:?}, pid {:?})",
            peer.map(|p| p.uid),
            peer.and_then(|p| p.pid)
        );
        let (workspace, principal) = (self.workspace.clone(), self.principal.clone());
        self.tracker.spawn(async move {
            let mut fields = peer_fields(peer, connection);
            fields["outcome"] = json!({ "result": "refused", "why": why });
            record(&workspace, &principal, "mcp-refused", fields).await;
        });
    }
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
