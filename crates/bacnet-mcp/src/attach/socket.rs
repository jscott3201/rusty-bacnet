//! The Unix side of the attached mode: the workspace's socket, its peers'
//! credentials from the kernel, and the bridge's connection to it.

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use bacnet_transport::port::TransportPort;
use serde_json::{Value, json};
use tokio::net::{UnixListener, UnixStream};
use tokio::runtime::Handle;

use super::sessions::Sessions;
use super::{ACCEPT_BACKOFF, AttachError, guard, not_listening};

/// The socket's file name in the profile's workspace.
pub const SOCKET_FILE: &str = "mcp.sock";

/// How the audit records name the way a connection came in.
pub(crate) const VIA: &str = "local-socket";

/// Where the TUI's socket for the workspace `dir` is. Never fails on
/// Unix; the Windows pipe's name needs the workspace's salt and nonce
/// files.
pub fn socket_path(dir: &Path) -> Result<PathBuf, AttachError> {
    Ok(dir.join(SOCKET_FILE))
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
pub(crate) type Credentials = fn(&UnixStream) -> io::Result<PeerId>;

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

/// The fields every record of a connection carries.
pub(super) fn peer_fields(peer: Option<&PeerId>, connection: u64) -> Value {
    json!({
        "via": VIA,
        "connection": connection,
        "peer_uid": peer.map(|p| p.uid),
        "peer_pid": peer.and_then(|p| p.pid),
    })
}

/// The peer, for a log line.
pub(super) fn describe(peer: Option<&PeerId>) -> String {
    format!(
        "peer uid {:?}, pid {:?}",
        peer.map(|p| p.uid),
        peer.and_then(|p| p.pid)
    )
}

/// What the listener checked before taking its lock.
pub(super) struct Preflight {
    uid: u32,
    path: PathBuf,
    dir: guard::WorkspaceDir,
}

/// Check that this user may listen in the workspace `dir`: not the
/// overflow uid, a socket path short enough, and a private workspace of
/// this user's (narrowed when looser).
pub(super) fn preflight(dir: &Path) -> Result<Preflight, AttachError> {
    let uid = own_uid();
    if uid == OVERFLOW_UID {
        return Err(AttachError(format!(
            "this TUI runs as uid {OVERFLOW_UID}, which peers in other user namespaces \
             report too, so it doesn't listen for MCP"
        )));
    }
    let path = dir.join(SOCKET_FILE);
    guard::check_length(&path)?;
    let dir = guard::open_workspace(dir, uid)?;
    Ok(Preflight { uid, path, dir })
}

/// The bound socket file, removed only while it is still the one bound.
pub(super) struct Endpoint {
    path: PathBuf,
    /// The socket file's device and inode, so only this file is removed.
    identity: (u64, u64),
    removed: bool,
}

impl Endpoint {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Nothing to do: the Windows pipe's nonce is withdrawn here.
    pub(super) fn withdraw(&mut self) {}

    /// Remove the socket file, once, if it is still this listener's.
    pub(super) fn remove(&mut self) {
        if !self.removed {
            self.removed = true;
            guard::remove_own(&self.path, self.identity);
        }
    }
}

/// What the accept loop takes connections from.
pub(super) struct Incoming {
    listener: UnixListener,
    uid: u32,
}

impl Preflight {
    /// With the listener's lock held, so any socket at the path is a stale
    /// one: bind the socket on `runtime`, narrow it to its owner, and check
    /// that the workspace `dir` is still the one checked.
    pub(super) fn bind(
        self,
        dir: &Path,
        runtime: &Handle,
    ) -> Result<(Endpoint, Incoming), AttachError> {
        let Self {
            uid,
            path,
            dir: held,
        } = self;
        let _entered = runtime.enter();
        guard::clear_stale(&path)?;
        let listener = UnixListener::bind(&path)
            .map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
        // Bound with the process umask; the 0700 directory keeps others
        // out until this narrows the file too.
        let identity = guard::own_socket(&path)?;
        if let Err(error) = held.recheck(dir, uid) {
            guard::remove_own(&path, identity);
            return Err(error.into());
        }
        let endpoint = Endpoint {
            path,
            identity,
            removed: false,
        };
        Ok((endpoint, Incoming { listener, uid }))
    }
}

/// Accept connections until `sessions.stop` is cancelled: a peer of this
/// user gets a session, any other is refused and audited before a byte
/// of MCP is read. Runs on a task of the sessions' runtime, never the
/// TUI's loop.
pub(super) async fn accept_loop<T: TransportPort + 'static>(
    incoming: Incoming,
    credentials: Credentials,
    sessions: Sessions<T>,
) {
    let Incoming { listener, uid } = incoming;
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

/// Connect to the TUI's socket at `path`, checking that the listener is
/// this OS user's own.
pub async fn connect(path: &Path) -> Result<UnixStream, AttachError> {
    connect_with(path, os_credentials).await
}

/// [`connect`], identifying the listener with `credentials`.
pub(crate) async fn connect_with(
    path: &Path,
    credentials: Credentials,
) -> Result<UnixStream, AttachError> {
    let shown = path.display();
    let stream = match UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            return Err(not_listening(path));
        }
        Err(e) => return Err(AttachError(format!("cannot connect to {shown}: {e}"))),
    };
    let peer = credentials(&stream)
        .map_err(|e| AttachError(format!("cannot identify the listener at {shown}: {e}")))?;
    let uid = own_uid();
    if peer.uid != uid {
        return Err(AttachError(format!(
            "the listener at {shown} runs as uid {}, not this user (uid {uid}); not connecting",
            peer.uid
        )));
    }
    Ok(stream)
}
