//! The TUI's MCP listener (#2185; design §9): while a profile is open,
//! agents attach through `bacnet mcp connect` to a local socket in its
//! workspace (on Windows, a named pipe named after it), and each gets an
//! MCP session on the session's own `bacnet_ops` client, so their reads
//! share the low-level client, the transport and the rate meter. With
//! `mcp_writes = "elicit"` the agents may write too: the write worker
//! lends them its desk, and each write waits for the person here to
//! confirm it in the preview dialog (`app::agents`); this loop hands the
//! requests to the app. The listener runs beside the session's workers
//! and ends with the session: every agent's session ends gracefully and
//! the socket file goes. The agents' sessions run on the listener's own
//! runtime, whose threads the panic hook leaves alone (#2203).

use std::path::Path;

use bacnet_mcp::Policy;
use bacnet_mcp::attach::DeskJob;
use bacnet_mcp::policy::Workspace;
use bacnet_ops::Client;
use bacnet_transport::port::TransportPort;
use tokio::sync::mpsc;

use super::profile::service::Prepared;
use super::worker::EventSink;
use crate::transport::TransportArgs;

/// What a profile's session listens with.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// The `[mcp]` table's reach, rates and caps, and whether it offers
    /// writes, which the person here then approves.
    pub(crate) policy: Policy,
    /// The workspace that holds the socket and the audit log.
    pub(crate) workspace: Workspace,
    /// Whom the sessions run as: the TUI's OS user.
    pub(crate) principal: String,
    /// Whether the sessions run on the listener's own runtime, so a panic
    /// in one leaves the terminal alone (#2203): always, but for tests on
    /// a paused clock, which that runtime doesn't share.
    pub(crate) own_runtime: bool,
}

/// The listener the profile `name` asks for, as stored and as resolved
/// with the command line's overrides: none unless MCP is authorized on it
/// and `tui_listener` isn't false, nor in a passive session, which has no
/// client to share. Overrides that change the transport the authorization
/// covered are refused, as `bacnet mcp serve` refuses transport flags.
pub(crate) fn plan(
    name: &str,
    prepared: &Prepared,
    workspace: Option<&Path>,
    args: &TransportArgs,
    passive: bool,
) -> Result<Option<Plan>, String> {
    let section = prepared.stored.mcp.as_deref().cloned().unwrap_or_default();
    if !section.listens() {
        if section.tui_listener == Some(true) {
            tracing::warn!(
                "profile {name} sets tui_listener = true but isn't authorized for MCP: run \
                 `bacnet mcp authorize --profile {name}` to let agents attach"
            );
        }
        return Ok(None);
    }
    if passive {
        tracing::info!("passive: no MCP listener, as there is no client to share");
        return Ok(None);
    }
    if prepared.resolved.link() != prepared.stored.link() {
        return Err(format!(
            "the command line changes profile {name}'s transport, which its MCP authorization \
             didn't cover"
        ));
    }
    let Some(dir) = workspace else {
        return Err(format!(
            "profile {name} has no workspace for the MCP socket"
        ));
    };
    let (transport, local) = crate::mcp::local(args);
    let workspace = Workspace {
        dir: dir.to_path_buf(),
        profile: Some(name.to_string()),
        transport,
        local,
    };
    let mut policy = crate::mcp::policy(&section, false)?;
    // Only `mcp_writes` counts here: the person at the TUI approves every
    // write, so `mcp_arm` and a plan grant play no part.
    if section.mcp_writes == crate::tui::profile::mcp::McpChoice::Elicit {
        policy.writes = bacnet_mcp::policy::WritePolicy {
            mode: bacnet_mcp::policy::WriteMode::Elicit,
            workspace: Some(workspace.clone()),
            ..bacnet_mcp::policy::WritePolicy::default()
        };
    }
    Ok(Some(Plan {
        policy,
        workspace,
        principal: crate::mcp::principal(),
        own_runtime: true,
    }))
}

/// Run `plan`'s listener over `client` until `stop` closes (the session
/// loop ended) or the session is cancelled, handing the UI the agents'
/// count and their write requests, their writes run on the write worker's
/// `desk`; then end their sessions and remove the socket. While an agent
/// is attached the `[mcp]` limits join the client's, the listener's doing.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &Client<T>,
    plan: Option<&Plan>,
    sink: &EventSink,
    mut stop: mpsc::Receiver<()>,
    desk: mpsc::Sender<DeskJob<T>>,
) {
    use super::message::{McpListener, WorkerEvent};
    use bacnet_mcp::attach::{Listener, SessionRuntime};
    let Some(plan) = plan else {
        return;
    };
    let runtime = if plan.own_runtime {
        SessionRuntime::Own
    } else {
        SessionRuntime::Callers
    };
    let bound = Listener::bind(
        client.clone(),
        plan.policy.clone(),
        plan.principal.clone(),
        plan.workspace.clone(),
        runtime,
        Some(desk),
    )
    .await;
    let mut listener = match bound {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!("MCP: agents can't attach: {error}");
            sink.deliver(WorkerEvent::Mcp(McpListener::Off)).await;
            return;
        }
    };
    let profile = plan.workspace.profile.as_deref().unwrap_or_default();
    let mut approvals = listener.approvals();
    let reach = if approvals.is_some() {
        "writes confirmed here"
    } else {
        "read-only"
    };
    tracing::info!(
        "MCP: agents can attach with `bacnet mcp connect --profile {profile}` ({reach}; {})",
        listener.path().display()
    );
    // The bar reads the count from the listener's own channel, so it
    // always shows the latest.
    let agents = McpListener::Agents(listener.agents());
    sink.deliver(WorkerEvent::Mcp(McpListener::Board(listener.board())))
        .await;
    sink.deliver(WorkerEvent::Mcp(agents)).await;
    loop {
        tokio::select! {
            biased;
            _ = stop.recv() => break,
            () = sink.cancelled() => break,
            request = next(&mut approvals) => match request {
                // The app queues it; a request it never answers is
                // declined once dropped.
                Some(request) => {
                    sink.deliver(WorkerEvent::Mcp(McpListener::Request(request))).await;
                }
                None => approvals = None,
            },
        }
    }
    // Requests still waiting here are dropped, and so declined.
    drop(approvals);
    listener.shutdown().await;
}

/// The next write request, or never without a queue.
async fn next(
    approvals: &mut Option<mpsc::Receiver<bacnet_mcp::attach::AgentApproval>>,
) -> Option<bacnet_mcp::attach::AgentApproval> {
    match approvals {
        Some(approvals) => approvals.recv().await,
        None => std::future::pending().await,
    }
}
