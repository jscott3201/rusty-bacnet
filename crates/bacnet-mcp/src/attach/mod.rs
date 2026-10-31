//! TUI-attached mode (#2185; design §8.1, §9): the terminal UI listens on
//! a local socket in its profile's workspace, and each agent that
//! connects gets an MCP session of its own over the TUI's engine, so the
//! agent's requests share the TUI's client, transport and request limits.
//! `bacnet mcp connect` bridges an agent host's stdio to that socket
//! ([`bridge()`]).
//!
//! Only this OS user may connect, checked three ways: the workspace
//! directory must be the user's own with mode 0700, the socket file is
//! made mode 0600, and every connection's peer credentials must name the
//! listener's own user before any MCP traffic flows. A refused peer is
//! closed and audited with the uid and pid the OS reports.
//!
//! The socket file is never followed through a symbolic link, and an
//! existing one is removed only once a connection to it is refused, so a
//! live listener (another TUI on the same profile) is never displaced.
//!
//! Sessions over the socket are read-only whatever the profile's write
//! policy: the write tools need the TUI's own approver, which comes later.
//! Unix only for now; Windows gets a named pipe with an owner-only DACL.
//! The sessions themselves serve any byte stream, so a
//! named pipe plugs in beside the socket.

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
pub use bridge::{bridge, connect};

use crate::policy::{Policy, Workspace, WritePolicy};
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
    removed: bool,
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
    /// Refused when the workspace isn't this user's own directory with
    /// mode 0700, when the socket path is a symbolic link or not a
    /// socket, or when another process is listening on it.
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
        // No write tool over the socket, whatever the profile allows.
        policy.passive = false;
        policy.writes = WritePolicy::default();
        policy
            .validate()
            .map_err(|e| format!("the MCP policy: {e}"))?;
        let uid = own_uid();
        guard::check_workspace(&workspace.dir, uid)?;
        let path = socket_path(&workspace.dir);
        guard::clear_stale(&path).await?;
        let listener = UnixListener::bind(&path)
            .map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
        // Bound with the process umask; the 0700 directory keeps others
        // out until this narrows the file too.
        let identity = match guard::own_socket(&path) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                return Err(error.into());
            }
        };
        let stop = CancellationToken::new();
        let (agents, seen) = watch::channel(0);
        let tracker = tokio_util::task::TaskTracker::new();
        let sessions = Sessions {
            engine,
            policy,
            principal,
            workspace,
            stop: stop.clone(),
            tracker: tracker.clone(),
            agents,
            next: std::sync::atomic::AtomicU64::new(1),
        };
        let accept = tokio::spawn(accept_loop(listener, uid, credentials, sessions));
        Ok(Self {
            path,
            identity,
            stop,
            accept: Some(accept),
            sessions: tracker,
            agents: seen,
            removed: false,
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
    /// complete, COV subscriptions are cancelled through each session's
    /// watch hub, writes in flight settle), then remove the socket file.
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
        self.remove();
    }
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
