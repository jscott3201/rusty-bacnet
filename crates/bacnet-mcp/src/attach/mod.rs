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
    /// `policy` as `principal`, audited in the workspace. Needs a Tokio
    /// runtime.
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
    ) -> Result<Self, AttachError> {
        Self::bind_with(engine, policy, principal.into(), workspace, os_credentials).await
    }

    /// [`bind`](Self::bind), identifying peers with `credentials`.
    pub(crate) async fn bind_with<T: TransportPort + 'static>(
        engine: Arc<Engine<T>>,
        mut policy: Policy,
        principal: String,
        workspace: Workspace,
        credentials: Credentials,
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
        let dir = guard::open_workspace(&workspace.dir, uid)?;
        let path = socket_path(&workspace.dir);
        guard::check_length(&path)?;
        // Taken before the socket path is touched: holding it, any socket
        // there is a stale one.
        let lock = guard::lock(&workspace.dir)?;
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
        };
        let accept = tokio::spawn(accept_loop(listener, uid, credentials, sessions));
        Ok(Self {
            path,
            identity,
            stop,
            accept: Some(accept),
            sessions: tracker,
            agents: seen,
            finish: Some(finish),
            removed: false,
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
    /// through the sessions' shared watch hub, then remove the socket
    /// file and release the lock.
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
        // Without a graceful shutdown the hub still stops, on its own task.
        if let Some(finish) = self.finish.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(finish);
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
/// of MCP is read. Runs on a task of its own, never the TUI's loop.
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
