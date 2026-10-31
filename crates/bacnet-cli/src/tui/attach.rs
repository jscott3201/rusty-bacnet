//! The TUI's MCP listener (#2185; design §9): while a profile is open,
//! agents attach through `bacnet mcp connect` to a local socket in its
//! workspace, and each gets a read-only MCP session on the session's own
//! engine, so their reads share the client, the transport and the rate
//! meter. The listener runs beside the session's workers and ends with
//! the session: every agent's session ends gracefully and the socket file
//! goes. Unix only for now.

use std::path::Path;
use std::sync::Arc;

use bacnet_mcp::Policy;
use bacnet_mcp::policy::Workspace;
use bacnet_ops::engine::Engine;
use bacnet_transport::port::TransportPort;
use tokio::sync::mpsc;

use super::profile::service::Prepared;
use super::worker::EventSink;
use crate::transport::TransportArgs;

/// What a profile's session listens with.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// The `[mcp]` table's reach, rates and caps; the listener drops any
    /// write policy.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) policy: Policy,
    /// The workspace that holds the socket and the audit log.
    pub(crate) workspace: Workspace,
    /// Whom the sessions run as: the TUI's OS user.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) principal: String,
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
    Ok(Some(Plan {
        policy: crate::mcp::policy(&section, false)?,
        workspace: Workspace {
            dir: dir.to_path_buf(),
            profile: Some(name.to_string()),
            transport,
            local,
        },
        principal: crate::mcp::principal(),
    }))
}

/// Run `plan`'s listener over `engine` until `stop` closes (the session
/// loop ended) or the session is cancelled, handing the UI the agents'
/// count; then end their sessions and remove the socket. While an agent
/// is attached the `[mcp]` limits join the engine's, the listener's doing.
pub(crate) async fn run<T: TransportPort + 'static>(
    engine: &Arc<Engine<T>>,
    plan: Option<&Plan>,
    sink: &EventSink,
    stop: mpsc::Receiver<()>,
) {
    let Some(plan) = plan else {
        return;
    };
    #[cfg(unix)]
    {
        use super::message::{McpListener, WorkerEvent};
        use bacnet_mcp::attach::Listener;
        let mut stop = stop;
        let bound = Listener::bind(
            Arc::clone(engine),
            plan.policy.clone(),
            plan.principal.clone(),
            plan.workspace.clone(),
        )
        .await;
        let listener = match bound {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!("MCP: agents can't attach: {error}");
                sink.deliver(WorkerEvent::Mcp(McpListener::Off)).await;
                return;
            }
        };
        let profile = plan.workspace.profile.as_deref().unwrap_or_default();
        tracing::info!(
            "MCP: agents can attach with `bacnet mcp connect --profile {profile}` (read-only; {})",
            listener.path().display()
        );
        // The bar reads the count from the listener's own channel, so it
        // always shows the latest.
        let agents = McpListener::Agents(listener.agents());
        sink.deliver(WorkerEvent::Mcp(agents)).await;
        tokio::select! {
            biased;
            _ = stop.recv() => {}
            () = sink.cancelled() => {}
        }
        listener.shutdown().await;
    }
    #[cfg(not(unix))]
    {
        let _ = (engine, sink, stop);
        tracing::info!(
            "MCP: attaching agents to the TUI isn't available on Windows yet (profile {:?}); \
             use `bacnet mcp serve --stdio`",
            plan.workspace.profile
        );
    }
}
