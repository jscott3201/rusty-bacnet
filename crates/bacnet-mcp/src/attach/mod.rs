//! TUI-attached mode (#2185; design §8.1, §9): the terminal UI listens on
//! a local socket in its profile's workspace, and each agent that
//! connects gets an MCP session of its own over the TUI's engine, so the
//! agent's requests share the TUI's client, transport and request limits.
//! The sessions share one watch hub and one principal bucket, as they are
//! one principal, and while any is attached the profile's `[mcp]` limits
//! join the engine's. `bacnet mcp connect` bridges an agent host's stdio
//! to that socket ([`bridge()`]).
//!
//! Only this OS user may connect, checked three ways: the workspace
//! directory must be the user's own (it is narrowed to 0700 when looser),
//! the socket file is made 0600, and every connection's peer credentials
//! must name the listener's own user before any MCP traffic flows. A
//! refused peer is closed and audited with the uid and pid the OS reports.
//!
//! A listener holds an exclusive lock on [`LOCK_FILE`] in the workspace for
//! its whole life, taken before the socket path is touched: a second
//! listener is refused, and a socket found by the lock's holder is stale
//! and removed. The socket file is never followed through a symbolic link.
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
//! Sessions over the socket are read-only whatever the profile's write
//! policy: the write tools need the TUI's own approver, which comes later.
//! Unix only for now; Windows gets a named pipe with an owner-only DACL.
//! The sessions themselves serve any byte stream, so a named pipe plugs in
//! beside the socket.

mod bridge;
mod guard;
mod sessions;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bacnet_ops::engine::Engine;
use bacnet_transport::port::TransportPort;
use tokio::net::{UnixListener, UnixStream};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "stdio")]
pub use bridge::bridge_stdio;
#[cfg(test)]
pub(crate) use bridge::connect_with;
pub use bridge::{bridge, connect};
pub use guard::LOCK_FILE;

use crate::policy::{Policy, Workspace, WritePolicy};
use crate::server::Shared;
use sessions::Sessions;

/// The socket's file name in the profile's workspace.
pub const SOCKET_FILE: &str = "mcp.sock";

/// How many agents may be attached at once; another is refused.
pub const MAX_AGENTS: usize = 8;

/// The name of every thread of a listener's own runtime begins with this,
/// so a panic hook can tell a session's panic from its host's.
pub const SESSION_THREADS: &str = "bacnet-mcp-agent";

/// The worker threads of a listener's own runtime.
const SESSION_WORKERS: usize = 2;

/// How long a listener's shutdown waits for its runtime's threads to stop,
/// once every session has ended.
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

/// The threads of listeners' own runtimes alive now, for tests.
#[cfg(test)]
pub(crate) mod threads {
    use std::sync::atomic::AtomicUsize;

    /// Started and not yet stopped.
    pub(crate) static LIVE: AtomicUsize = AtomicUsize::new(0);
}

/// Where the TUI's socket for the workspace `dir` is.
pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_FILE)
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

/// Who is at the other end of a connection, as the OS reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PeerId {
    /// The peer's effective user id.
    pub(crate) uid: u32,
    /// Its process id, where the OS says.
    pub(crate) pid: Option<i32>,
}

/// How a connection's peer is identified: the OS's credentials, or a
/// test's stand-in.
type Credentials = fn(&UnixStream) -> io::Result<PeerId>;

/// The peer's credentials from the kernel (`SO_PEERCRED` or
/// `getpeereid`).
pub(crate) fn os_credentials(stream: &UnixStream) -> io::Result<PeerId> {
    let cred = stream.peer_cred()?;
    Ok(PeerId {
        uid: cred.uid(),
        pid: cred.pid(),
    })
}

/// This process's effective user id.
pub(crate) fn own_uid() -> u32 {
    nix::unistd::geteuid().as_raw()
}

/// The uid the kernel reports for a peer whose user isn't mapped into
/// this user namespace: a TUI running as it can't tell such peers apart.
const OVERFLOW_UID: u32 = 65_534;

/// What a listener does after its sessions end: write the count of
/// refusals left out of the record, and shut the shared hub down.
type Finish = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Whether a TUI is listening in the workspace `dir`: its listener's lock
/// is held. A socket file alone may be a stale one.
pub fn listening(dir: &Path) -> bool {
    guard::held(dir)
}

/// The TUI's listener: an accept loop on the workspace's socket and the
/// sessions it started. [`shutdown`](Self::shutdown) ends them all and
/// removes the socket file; dropping it stops accepting and removes the
/// file without waiting.
pub struct Listener {
    path: PathBuf,
    /// The socket file's device and inode, so only this file is removed.
    identity: (u64, u64),
    stop: CancellationToken,
    accept: Option<JoinHandle<()>>,
    sessions: tokio_util::task::TaskTracker,
    agents: watch::Receiver<usize>,
    finish: Option<Finish>,
    removed: bool,
    /// The sessions' own runtime, if they have one: shut down once they
    /// have all ended.
    runtime: Option<OwnRuntime>,
    /// The listener's lock, held for its life and released last.
    _lock: std::fs::File,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("path", &self.path)
            .field("agents", &*self.agents.borrow())
            .finish_non_exhaustive()
    }
}

impl Listener {
    /// Listen on [`socket_path`] of `workspace.dir` for agents of this OS
    /// user, each served by a read-only session over `engine` under
    /// `policy` as `principal`, audited in the workspace, the sessions run
    /// on `runtime`. Needs a Tokio runtime, which must outlive the
    /// listener: the engine's client and transport run on it.
    ///
    /// The workspace must be this user's own directory, not a symbolic
    /// link; a looser mode than 0700 is narrowed. Refused when another
    /// listener holds the workspace's lock, when the socket path is too
    /// long, a symbolic link or some other file, or when the TUI runs as
    /// the overflow uid.
    pub async fn bind<T: TransportPort + 'static>(
        engine: Arc<Engine<T>>,
        policy: Policy,
        principal: impl Into<String>,
        workspace: Workspace,
        runtime: SessionRuntime,
    ) -> Result<Self, AttachError> {
        let principal = principal.into();
        Self::bind_with(
            engine,
            policy,
            principal,
            workspace,
            os_credentials,
            runtime,
        )
        .await
    }

    /// [`bind`](Self::bind), identifying peers with `credentials`.
    pub(crate) async fn bind_with<T: TransportPort + 'static>(
        engine: Arc<Engine<T>>,
        mut policy: Policy,
        principal: String,
        workspace: Workspace,
        credentials: Credentials,
        runtime: SessionRuntime,
    ) -> Result<Self, AttachError> {
        // The listener always has an engine, so a caller's passive policy
        // is overridden; and no write tool goes over the socket, whatever
        // the profile allows.
        policy.passive = false;
        policy.writes = WritePolicy::default();
        policy
            .validate()
            .map_err(|e| format!("the MCP policy: {e}"))?;
        let uid = own_uid();
        if uid == OVERFLOW_UID {
            return Err(AttachError(format!(
                "this TUI runs as uid {OVERFLOW_UID}, which peers in other user namespaces \
                 report too, so it doesn't listen for MCP"
            )));
        }
        let path = socket_path(&workspace.dir);
        guard::check_length(&path)?;
        let dir = guard::open_workspace(&workspace.dir, uid)?;
        // Taken before the socket path is touched: holding it, any socket
        // there is a stale one.
        let lock = guard::lock(&workspace.dir).await?;
        let runtime = match runtime {
            SessionRuntime::Own => Some(session_runtime()?),
            SessionRuntime::Callers => None,
        };
        let handle = match runtime.as_ref().and_then(|own| own.0.as_ref()) {
            Some(own) => own.handle().clone(),
            None => Handle::current(),
        };
        // The socket, its connections and the watch hub's tasks belong to
        // the sessions' runtime. Nothing below awaits while it is entered.
        let entered = handle.enter();
        guard::clear_stale(&path)?;
        let listener = UnixListener::bind(&path)
            .map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
        // Bound with the process umask; the 0700 directory keeps others
        // out until this narrows the file too.
        let identity = guard::own_socket(&path)?;
        if let Err(error) = dir.recheck(&workspace.dir, uid) {
            guard::remove_own(&path, identity);
            return Err(error.into());
        }
        let shared = match Shared::new(Some(&engine), &policy) {
            Ok(shared) => shared,
            Err(error) => {
                guard::remove_own(&path, identity);
                return Err(AttachError(error.to_string()));
            }
        };
        drop(entered);
        let stop = CancellationToken::new();
        let (agents, seen) = watch::channel(0);
        let tracker = tokio_util::task::TaskTracker::new();
        let refusals = Arc::new(std::sync::Mutex::new(sessions::Refusals::default()));
        let finish = finish(&shared, &workspace, &principal, &refusals);
        let sessions = Sessions {
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
        let accept = handle.spawn(accept_loop(listener, uid, credentials, sessions));
        Ok(Self {
            path,
            identity,
            stop,
            accept: Some(accept),
            sessions: tracker,
            agents: seen,
            finish: Some(finish),
            removed: false,
            runtime,
            _lock: lock,
        })
    }

    /// The socket's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The number of agents attached, as it changes.
    pub fn agents(&self) -> watch::Receiver<usize> {
        self.agents.clone()
    }

    /// Stop accepting, end every session gracefully (listen streams
    /// complete, writes in flight settle), cancel the COV subscriptions
    /// through the sessions' shared watch hub, stop the sessions' own
    /// runtime, then remove the socket file and release the lock.
    pub async fn shutdown(mut self) {
        self.stop.cancel();
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
        if !self.removed {
            self.removed = true;
            guard::remove_own(&self.path, self.identity);
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
        // The ordered path is the graceful `shutdown()`. Dropped without it,
        // the hub's shutdown runs on a task of its own, racing the sessions
        // still ending, and the count of refusals left out of the record
        // may be lost. The sessions' runtime stops after it.
        let runtime = self.runtime.take();
        match (self.finish.take(), Handle::try_current()) {
            (Some(finish), Ok(current)) => {
                current.spawn(async move {
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

/// Accept connections until `sessions.stop` is cancelled: a peer of
/// `uid` gets a session, any other is refused and audited before a byte
/// of MCP is read. Runs on a task of the sessions' runtime, never the
/// TUI's loop.
async fn accept_loop<T: TransportPort + 'static>(
    listener: UnixListener,
    uid: u32,
    credentials: Credentials,
    sessions: Sessions<T>,
) {
    loop {
        // `accept` is cancel-safe: a dropped call loses no connection.
        let accepted = tokio::select! {
            biased;
            () = sessions.stop.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, _)) => match credentials(&stream) {
                Ok(peer) if peer.uid == uid => sessions.open(stream, peer),
                Ok(peer) => {
                    drop(stream);
                    sessions.refuse(Some(peer), "another-user");
                }
                // macOS can't read the credentials of a peer already gone.
                Err(error) if error.kind() == io::ErrorKind::NotConnected => {
                    drop(stream);
                    sessions.refuse(None, "peer-closed");
                }
                Err(error) => {
                    drop(stream);
                    tracing::warn!("MCP socket: no peer credentials: {error}");
                    sessions.refuse(None, "no-peer-credentials");
                }
            },
            Err(error) => {
                tracing::warn!("MCP socket: accept failed: {error}");
                tokio::select! {
                    biased;
                    () = sessions.stop.cancelled() => break,
                    () = tokio::time::sleep(ACCEPT_BACKOFF) => {}
                }
            }
        }
    }
}
