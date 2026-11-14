//! TUI-attached mode (#2185; design §8.1, §9): the terminal UI listens on
//! a local socket in its profile's workspace, and each agent that
//! connects gets an MCP session of its own over the TUI's engine, so the
//! agent's requests share the TUI's client, transport and request limits.
//! The sessions share one watch hub and one principal bucket, as they are
//! one principal, and while any is attached the profile's `[mcp]` limits
//! join the engine's. `bacnet mcp connect` bridges an agent host's stdio
//! to that socket ([`bridge()`]).
//!
//! Only this OS user may connect. On Unix, checked three ways: the
//! workspace directory must be the user's own (it is narrowed to 0700
//! when looser), the socket file is made 0600, and every connection's
//! peer credentials must name the listener's own user before any MCP
//! traffic flows. On Windows the listener is a named pipe whose name comes
//! from the workspace's path, a secret salt kept in it (#2219) and a nonce
//! each run publishes beside the salt (#2237), both files this user's
//! alone, in a workspace other users may not list.
//! The pipe's security descriptor grants access to this user's SID
//! alone, it takes no remote clients, its first instance
//! claims the name, and every client process's token must name this user
//! before any MCP traffic flows. A refused peer is closed and audited with
//! what the OS reports of it.
//!
//! A listener holds an exclusive lock on [`LOCK_FILE`] in the workspace for
//! its whole life, taken before the socket path is touched (or the pipe
//! made): a second listener is refused, and a socket found by the lock's
//! holder is stale and removed. The socket file is never followed through
//! a symbolic link.
//!
//! The sessions run on a runtime of the listener's own
//! ([`SessionRuntime::Own`]), every task an agent's request starts included,
//! on threads named [`SESSION_THREADS`]: a host whose panic hook restores
//! its terminal (the TUI's) leaves a panic on those threads alone, and the
//! session it hit ends, audited, while the host and other agents carry on
//! (#2203). The engine, its client and transport stay on the host's
//! runtime; the sessions reach them through Tokio's synchronization, which
//! works across runtimes, and their own timers and socket I/O run on their
//! runtime.
//!
//! Sessions over the socket are read-only unless the profile offers
//! writes (`mcp_writes = "elicit"`) and the TUI lends its write desk
//! ([`DeskJob`]): then the person at the TUI approves every write, at
//! every tier, from its queue of [`AgentApproval`]s, and the client's own
//! confirmation, `mcp_arm` and any plan grant play no part (#2185).
//! The sessions serve any byte stream; the socket (`socket.rs`) and the
//! pipe (`pipe.rs`) each bring their own listener, peer check and bridge
//! connection.

mod bridge;
#[cfg(unix)]
mod guard;
mod lock;
#[cfg(any(windows, all(unix, test)))]
mod nonce;
#[cfg(windows)]
mod pipe;
#[cfg(any(windows, all(unix, test)))]
mod salt;
mod sessions;
#[cfg(unix)]
mod socket;
#[cfg(windows)]
mod win32;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bacnet_ops::Client;
use bacnet_transport::port::TransportPort;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use crate::board::{Board, CALLS, CallOutcome, CallRow, Calls};
pub use crate::tools::write_attached::DeskJob;
pub use crate::writes::tui::{
    AgentApproval, Declined, Gone, LIFETIME, MAX_QUEUED, Refusal, WAIT_AT_MOST,
};
pub use bridge::bridge;
#[cfg(feature = "stdio")]
pub use bridge::bridge_stdio;
pub use lock::LOCK_FILE;
#[cfg(all(test, windows))]
pub(crate) use nonce::{NONCE_FILE, Nonce, publish as publish_nonce};
#[cfg(windows)]
use pipe as platform;
#[cfg(all(test, windows))]
pub(crate) use pipe::{ListenerCredentials, ListenerId, Sid, Unidentified};
#[cfg(windows)]
pub use pipe::{connect, socket_path};
#[cfg(test)]
pub(crate) use platform::connect_with;
use platform::os_credentials;
pub(crate) use platform::{Credentials, PeerId, VIA};
#[cfg(all(test, windows))]
pub(crate) use salt::{SALT_FILE, Salt, pipe_name};
#[cfg(unix)]
use socket as platform;
#[cfg(all(test, unix))]
pub(crate) use socket::own_uid;
#[cfg(unix)]
pub use socket::{SOCKET_FILE, connect, socket_path};
#[cfg(all(test, windows))]
pub(crate) use win32::{Descriptor, create_dir, own_sid, security_of};

use crate::policy::{ArmMode, Policy, Workspace, WritePolicy};
use crate::server::Shared;
use sessions::Sessions;

/// How many agents may be attached at once; another is refused.
pub const MAX_AGENTS: usize = 8;

/// The name of every thread of a listener's own runtime begins with this,
/// so a panic hook can tell a session's panic from its host's.
pub const SESSION_THREADS: &str = "bacnet-mcp-agent";

/// The worker threads of a listener's own runtime.
const SESSION_WORKERS: usize = 2;

/// How long a listener's shutdown waits for its runtime's threads to stop,
/// once every session has ended; and how long a listener dropped without
/// its shutdown waits for its sessions to end.
const RUNTIME_WAIT: Duration = Duration::from_secs(5);

/// Where a listener runs its agents' sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRuntime {
    /// On a runtime of the listener's own, whose threads are named
    /// [`SESSION_THREADS`]; it ends with the listener. What the TUI uses.
    Own,
    /// On the caller's runtime, sharing its clock: for tests on a paused
    /// clock, which a second runtime doesn't share. A session's panic
    /// can't be told apart by its thread then.
    Callers,
}

/// The listener's own runtime ([`SessionRuntime::Own`]). Dropped, it
/// stops without waiting for its threads, which any context allows: a
/// runtime dropped the usual way inside another's task panics.
struct OwnRuntime(Option<Runtime>);

impl Drop for OwnRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

/// Stop the sessions' own runtime and wait for its threads, off the async
/// context, where a runtime may not block.
async fn stop_runtime(mut runtime: OwnRuntime) {
    let stopped = tokio::task::spawn_blocking(move || {
        if let Some(runtime) = runtime.0.take() {
            runtime.shutdown_timeout(RUNTIME_WAIT);
        }
    });
    if let Err(e) = stopped.await {
        tracing::warn!("the MCP sessions' runtime didn't stop cleanly: {e}");
    }
}

/// Build the listener's own runtime.
fn session_runtime() -> Result<OwnRuntime, AttachError> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder
        .worker_threads(SESSION_WORKERS)
        .thread_name_fn(|| {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("{SESSION_THREADS}-{n}")
        })
        .enable_all();
    #[cfg(test)]
    builder
        .on_thread_start(|| {
            threads::LIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
        .on_thread_stop(|| {
            threads::LIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        });
    builder
        .build()
        .map(|runtime| OwnRuntime(Some(runtime)))
        .map_err(|e| AttachError(format!("cannot start the MCP sessions' runtime: {e}")))
}

/// The threads of listeners' own runtimes alive now, and how long a
/// session takes to end, for tests.
#[cfg(test)]
pub(crate) mod threads {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    /// Started and not yet stopped.
    pub(crate) static LIVE: AtomicUsize = AtomicUsize::new(0);

    /// Milliseconds each session waits before its close is recorded.
    pub(crate) static END_DELAY_MS: AtomicU64 = AtomicU64::new(0);

    /// Wait out [`END_DELAY_MS`].
    pub(crate) async fn ending() {
        let ms = END_DELAY_MS.load(Ordering::SeqCst);
        if ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        }
    }
}

/// Why the listener didn't start, or the bridge couldn't connect.
#[derive(Debug)]
pub struct AttachError(String);

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AttachError {}

impl From<String> for AttachError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

/// What the bridge says when nothing listens at `path`.
fn not_listening(path: &Path) -> AttachError {
    AttachError(format!(
        "no TUI is listening for MCP at {}: open the profile in `bacnet tui` first (its MCP \
         authorization must stand, and its [mcp] table must not set tui_listener = false)",
        path.display()
    ))
}

/// What a listener does after its sessions end: write the count of
/// refusals left out of the record, and shut the shared hub down.
type Finish = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Whether a TUI is listening in the workspace `dir`: its listener's lock
/// is held. A socket file alone may be a stale one.
pub fn listening(dir: &Path) -> bool {
    lock::held(dir)
}

/// The TUI's listener: an accept loop on the workspace's socket (or named
/// pipe) and the sessions it started. [`shutdown`](Self::shutdown) ends
/// them all and removes the socket file; dropping it stops accepting and
/// removes the file without waiting.
pub struct Listener {
    endpoint: platform::Endpoint,
    stop: CancellationToken,
    accept: Option<JoinHandle<()>>,
    sessions: tokio_util::task::TaskTracker,
    agents: watch::Receiver<usize>,
    finish: Option<Finish>,
    /// The sessions' own runtime, if they have one: shut down once they
    /// have all ended.
    runtime: Option<OwnRuntime>,
    /// The agents' write requests, until the TUI takes them.
    approvals: Option<mpsc::Receiver<AgentApproval>>,
    /// The agents' calls and the TUI's armed state, for the agent pane.
    board: Board,
    /// The listener's lock, held for its life and released last.
    _lock: std::fs::File,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("path", &self.endpoint.path())
            .field("agents", &*self.agents.borrow())
            .finish_non_exhaustive()
    }
}

impl Listener {
    /// Listen on [`socket_path`] of `workspace.dir` for agents of this OS
    /// user, each served by a session over `engine` under `policy` as
    /// `principal`, audited in the workspace, the sessions run on
    /// `runtime`. Read-only unless the policy offers writes and the TUI
    /// lends its write `desk`: then the TUI takes the agents' write
    /// requests from [`approvals`](Self::approvals). Needs a Tokio
    /// runtime, which must outlive the listener: the engine's client and
    /// transport run on it.
    ///
    /// Refused when another listener holds the workspace's lock. On Unix,
    /// the workspace must be this user's own directory, not a symbolic
    /// link, and a looser mode than 0700 is narrowed; refused too when the
    /// socket path is too long, a symbolic link or some other file, or
    /// when the TUI runs as the overflow uid. On Windows, refused when the
    /// pipe's name is already taken, by anyone; when the workspace lets
    /// other users list it; when its salt file isn't this user's alone
    /// or is corrupt; or when the run's nonce can't be published.
    pub async fn bind<T: TransportPort + 'static>(
        engine: Client<T>,
        policy: Policy,
        principal: impl Into<String>,
        workspace: Workspace,
        runtime: SessionRuntime,
        desk: Option<mpsc::Sender<DeskJob<T>>>,
    ) -> Result<Self, AttachError> {
        let principal = principal.into();
        Self::bind_with(
            engine,
            policy,
            principal,
            workspace,
            (os_credentials, runtime),
            desk,
        )
        .await
    }

    /// [`bind`](Self::bind), identifying peers with `credentials`.
    pub(crate) async fn bind_with<T: TransportPort + 'static>(
        engine: Client<T>,
        mut policy: Policy,
        principal: String,
        workspace: Workspace,
        (credentials, runtime): (Credentials, SessionRuntime),
        desk: Option<mpsc::Sender<DeskJob<T>>>,
    ) -> Result<Self, AttachError> {
        // The listener always has an engine, so a caller's passive policy
        // is overridden. Writes go over the socket only with the TUI's
        // desk to write through, and only as the TUI approves them: not by
        // the client's confirmation, an armed MCP session or a plan grant.
        policy.passive = false;
        let desk = desk.filter(|_| policy.writes.offered());
        if desk.is_some() {
            policy.writes.arm = ArmMode::Never;
            policy.writes.plan = None;
        } else {
            policy.writes = WritePolicy::default();
        }
        policy
            .validate()
            .map_err(|e| format!("the MCP policy: {e}"))?;
        let preflight = platform::preflight(&workspace.dir)?;
        // Taken before the socket path is touched: holding it, any socket
        // there is a stale one.
        let lock = lock::lock(&workspace.dir).await?;
        let runtime = match runtime {
            SessionRuntime::Own => Some(session_runtime()?),
            SessionRuntime::Callers => None,
        };
        let handle = match runtime.as_ref().and_then(|own| own.0.as_ref()) {
            Some(own) => own.handle().clone(),
            None => Handle::current(),
        };
        // The socket, its connections and the watch hub's tasks belong to
        // the sessions' runtime.
        let (mut endpoint, incoming) = preflight.bind(&workspace.dir, &handle)?;
        let entered = handle.enter();
        let shared = match Shared::new(Some(&engine), &policy) {
            Ok(shared) => shared,
            Err(error) => {
                endpoint.remove();
                return Err(AttachError(error.to_string()));
            }
        };
        drop(entered);
        let stop = CancellationToken::new();
        let (agents, seen) = watch::channel(0);
        let tracker = tokio_util::task::TaskTracker::new();
        let refusals = Arc::new(std::sync::Mutex::new(sessions::Refusals::default()));
        let finish = finish(&shared, &workspace, &principal, &refusals);
        let (lending, approvals) = match desk {
            Some(desk) => {
                let (to_tui, approvals) = mpsc::channel(MAX_AGENTS);
                (Some((to_tui, desk)), Some(approvals))
            }
            None => (None, None),
        };
        let board = Board::default();
        let sessions = Sessions {
            board: board.clone(),
            lending,
            overlay: policy.overlay(),
            engine,
            policy,
            shared,
            principal,
            workspace,
            stop: stop.clone(),
            tracker: tracker.clone(),
            agents,
            next: std::sync::atomic::AtomicU64::new(1),
            refusals,
            runtime: handle.clone(),
        };
        let accept = handle.spawn(platform::accept_loop(incoming, credentials, sessions));
        Ok(Self {
            endpoint,
            stop,
            accept: Some(accept),
            sessions: tracker,
            agents: seen,
            finish: Some(finish),
            runtime,
            approvals,
            board,
            _lock: lock,
        })
    }

    /// The agents' write requests, for the TUI to queue and answer: once,
    /// and only when it lent its desk to a policy that offers writes.
    /// Dropping it (or each request unanswered) declines them.
    pub fn approvals(&mut self) -> Option<mpsc::Receiver<AgentApproval>> {
        self.approvals.take()
    }

    /// The agents' calls, for the TUI's agent pane, and where the TUI says
    /// whether its write session is armed, which `bacnet://session` reports
    /// to the agents.
    pub fn board(&self) -> Board {
        self.board.clone()
    }

    /// The socket's path, or on Windows the pipe's name.
    pub fn path(&self) -> &Path {
        self.endpoint.path()
    }

    /// The number of agents attached, as it changes.
    pub fn agents(&self) -> watch::Receiver<usize> {
        self.agents.clone()
    }

    /// Stop accepting, end every session gracefully (listen streams
    /// complete, writes in flight settle), cancel the COV subscriptions
    /// through the sessions' shared watch hub, stop the sessions' own
    /// runtime, then remove the socket file (on Windows, withdraw the
    /// run's nonce) and release the lock.
    pub async fn shutdown(mut self) {
        self.stop.cancel();
        // The run's name goes first: a connect from now on hears that no
        // TUI is listening. The lock is held until the end.
        self.endpoint.withdraw();
        if let Some(accept) = self.accept.take()
            && let Err(e) = accept.await
            && e.is_panic()
        {
            tracing::warn!("the MCP accept loop panicked: {e}");
        }
        self.sessions.close();
        self.sessions.wait().await;
        if let Some(finish) = self.finish.take() {
            finish.await;
        }
        if let Some(runtime) = self.runtime.take() {
            stop_runtime(runtime).await;
        }
        self.remove();
    }

    fn remove(&mut self) {
        self.endpoint.remove();
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
        // The ordered path is the graceful `shutdown()`. Dropped without it,
        // a task of its own waits a while for the sessions to end, then
        // shuts the hub down and stops the sessions' runtime.
        let runtime = self.runtime.take();
        let sessions = self.sessions.clone();
        sessions.close();
        match (self.finish.take(), Handle::try_current()) {
            (Some(finish), Ok(current)) => {
                current.spawn(async move {
                    // Sessions still ending keep their close on record:
                    // stopping the runtime would drop them.
                    if tokio::time::timeout(RUNTIME_WAIT, sessions.wait())
                        .await
                        .is_err()
                    {
                        tracing::warn!("MCP sessions were still ending as the listener stopped");
                    }
                    finish.await;
                    if let Some(runtime) = runtime {
                        stop_runtime(runtime).await;
                    }
                });
            }
            // Stopped without waiting, as it drops.
            _ => drop(runtime),
        }
        self.remove();
    }
}

/// What a listener does once its sessions have ended.
fn finish<T: TransportPort + 'static>(
    shared: &Shared<T>,
    workspace: &Workspace,
    principal: &str,
    refusals: &Arc<std::sync::Mutex<sessions::Refusals>>,
) -> Finish {
    let (hub, workspace, principal) =
        (shared.hub.clone(), workspace.clone(), principal.to_string());
    let refusals = Arc::clone(refusals);
    Box::pin(async move {
        let left_out = std::mem::take(
            &mut refusals
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .suppressed,
        );
        if left_out > 0 {
            sessions::record_left_out(&workspace, &principal, left_out).await;
        }
        if let Some(hub) = hub {
            hub.shutdown(crate::server::cancel_wait(hub.stats().tasks))
                .await;
        }
    })
}

/// How long the loop rests after a failed accept (too many open files,
/// say) before trying again.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
