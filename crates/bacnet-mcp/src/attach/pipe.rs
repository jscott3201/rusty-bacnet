//! The Windows side of the attached mode (#2201): a named pipe whose name
//! comes from a secret salt, a nonce of the run and the workspace's path,
//! open to this user's
//! SID alone and to local clients only, its peers identified by their
//! process's token, and the bridge's connection to it.
//!
//! The pipe's name is global, so it is guarded four ways. It can't be
//! worked out without the salt (`salt.rs`, #2219), a file in the
//! workspace only this user may read; the workspace itself must not be
//! listable by other users. Anyone may see it listed while the TUI runs,
//! but each run's nonce (`nonce.rs`, #2237) gives it a new one. The first instance claims the name
//! (`FILE_FLAG_FIRST_PIPE_INSTANCE`): if anyone holds it already, the
//! listener is refused rather than joining their pipe. Every instance
//! after it carries the same owner-only descriptor and refuses remote
//! clients, and the next instance is always made before a connected one
//! is handed on, so the name never goes free while the listener runs.
//! And either end checks the other's user before any MCP byte: the
//! listener each client's, the bridge the listener's, so an impostor pipe
//! gets nothing from it.
//!
//! The descriptor grants nobody but this user: not SYSTEM, not the
//! Administrators group. The agents are this user's own processes; SYSTEM
//! and administrators can take any object over through their privileges,
//! so naming them would only widen the door for everyone else they cover.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use serde_json::{Value, json};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::runtime::Handle;

use super::sessions::Sessions;
use super::win32::{self, Descriptor};
use super::{ACCEPT_BACKOFF, AttachError, not_listening};
use super::{nonce, salt};

/// How the audit records name the way a connection came in.
pub(crate) const VIA: &str = "named-pipe";

/// A security identifier, as the system lays it out; shown in its usual
/// `S-1-…` form.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Sid(pub(crate) Vec<u8>);

impl fmt::Display for Sid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Revision, sub-authority count, a 48-bit big-endian authority,
        // then 32-bit little-endian sub-authorities.
        let Some((head, rest)) = self.0.split_at_checked(8) else {
            return f.write_str("S-?");
        };
        let authority = head[2..].iter().fold(0u64, |a, &b| (a << 8) | u64::from(b));
        if authority >> 32 == 0 {
            write!(f, "S-{}-{authority}", head[0])?;
        } else {
            write!(f, "S-{}-0x{authority:012X}", head[0])?;
        }
        for sub in rest.as_chunks::<4>().0 {
            write!(f, "-{}", u32::from_le_bytes(*sub))?;
        }
        Ok(())
    }
}

impl fmt::Debug for Sid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The pipe of the TUI listening in the workspace `dir`: a name from a
/// hash of the workspace's salt, the running listener's nonce and the
/// workspace's canonical path, so each profile has its own, `bacnet mcp
/// connect` finds it with no setting, nobody who can't read both files
/// can work it out, and a TUI started again has a new one. The nonce is
/// read anew on each call. Fails when the workspace has no salt (no TUI
/// has listened on it) or no nonce (none listens now), or when either
/// file isn't this user's alone or is corrupt.
pub fn socket_path(dir: &Path) -> Result<PathBuf, AttachError> {
    let salt = salt::load(dir)?;
    let nonce = nonce::load(dir)?;
    Ok(salt::pipe_name(&salt, &nonce, dir))
}

/// Who is at the other end of a connection, as the OS reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerId {
    /// The user its process runs as, once known.
    pub(crate) sid: Option<Sid>,
    /// Its process id, where the OS says.
    pub(crate) pid: Option<u32>,
}

/// A peer whose user couldn't be found, and what is known of it.
#[derive(Debug)]
pub(crate) struct Unidentified {
    pub(crate) pid: Option<u32>,
    pub(crate) error: io::Error,
}

/// How a client of the pipe is identified: the OS's answer, or a test's
/// stand-in.
pub(crate) type Credentials = fn(&NamedPipeServer) -> Result<PeerId, Unidentified>;

/// What the bridge learns of the listener: the process at the server end,
/// and who owns the pipe object itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListenerId {
    /// The server's process and the user its token names.
    pub(crate) process: PeerId,
    /// The pipe's owner, as its descriptor names it.
    pub(crate) owner: Sid,
}

/// How the bridge identifies the listener at the server end.
pub(crate) type ListenerCredentials = fn(&NamedPipeClient) -> io::Result<ListenerId>;

/// The client's process, then the user its token names.
pub(crate) fn os_credentials(pipe: &NamedPipeServer) -> Result<PeerId, Unidentified> {
    let pid = win32::client_pid(pipe).map_err(|error| Unidentified { pid: None, error })?;
    let sid = win32::process_sid(pid).map_err(|error| Unidentified {
        pid: Some(pid),
        error,
    })?;
    Ok(PeerId {
        sid: Some(sid),
        pid: Some(pid),
    })
}

/// The listener's process and the user its token names, and the pipe
/// object's owner: a pid alone could name a process that has since gone
/// and been replaced, the owner can't change under the open handle.
fn os_listener(pipe: &NamedPipeClient) -> io::Result<ListenerId> {
    let pid = win32::server_pid(pipe)?;
    let process = PeerId {
        sid: Some(win32::process_sid(pid)?),
        pid: Some(pid),
    };
    Ok(ListenerId {
        process,
        owner: win32::owner_of(pipe)?,
    })
}

/// The fields every record of a connection carries.
pub(super) fn peer_fields(peer: Option<&PeerId>, connection: u64) -> Value {
    json!({
        "via": VIA,
        "connection": connection,
        "peer_sid": peer.and_then(|p| p.sid.as_ref()).map(ToString::to_string),
        "peer_pid": peer.and_then(|p| p.pid),
    })
}

/// The peer, for a log line.
pub(super) fn describe(peer: Option<&PeerId>) -> String {
    format!(
        "peer SID {:?}, pid {:?}",
        peer.and_then(|p| p.sid.as_ref()),
        peer.and_then(|p| p.pid)
    )
}

/// The options of every instance: byte mode, both ways, no remote
/// clients; the first also claims the name.
pub(super) fn options(first: bool) -> ServerOptions {
    let mut options = ServerOptions::new();
    options
        .reject_remote_clients(true)
        .first_pipe_instance(first);
    options
}

/// What the listener found before taking its lock.
pub(super) struct Preflight {
    owner: Sid,
    salt: salt::Salt,
    descriptor: Descriptor,
}

/// This user's SID, the workspace's salt and the pipe's owner-only
/// descriptor.
/// Refused when other users may list the workspace, or when its salt
/// file is not this user's alone or is corrupt; the salt is made when
/// the workspace has none.
pub(super) fn preflight(dir: &Path) -> Result<Preflight, AttachError> {
    let owner =
        win32::own_sid().map_err(|e| format!("cannot tell which user this TUI runs as: {e}"))?;
    win32::check_private_dir(dir, &owner).map_err(|e| {
        format!(
            "the workspace {shown} {e}; not listening for MCP. Only your account, SYSTEM \
             and Administrators may have access to it: `icacls \"{shown}\"` shows who does, \
             and `icacls \"{shown}\" /inheritance:r /grant:r \"%USERNAME%:(OI)(CI)F\"` \
             leaves only you",
            shown = dir.display()
        )
    })?;
    let descriptor = Descriptor::owner_only(&owner)
        .map_err(|e| format!("cannot make the MCP pipe's security descriptor: {e}"))?;
    let salt = salt::load_or_create(dir)?;
    Ok(Preflight {
        owner,
        salt,
        descriptor,
    })
}

/// The pipe's name, and the workspace whose nonce gave it. The name goes
/// with the pipe's last instance.
pub(super) struct Endpoint {
    name: PathBuf,
    /// Taken as the nonce is withdrawn, once.
    workspace: Option<PathBuf>,
}

impl Endpoint {
    pub(super) fn path(&self) -> &Path {
        &self.name
    }

    /// Withdraw the run's nonce, with the listener's lock still held: a
    /// connect after this finds no TUI listening rather than a stale name.
    pub(super) fn remove(&mut self) {
        if let Some(dir) = self.workspace.take() {
            nonce::withdraw(&dir);
        }
    }
}

/// What the accept loop takes connections from: the instance waiting for
/// a client, and what makes the next one.
pub(super) struct Incoming {
    waiting: NamedPipeServer,
    name: PathBuf,
    descriptor: Descriptor,
    owner: Sid,
}

impl Preflight {
    /// With the listener's lock held: pick the run's nonce, create the
    /// pipe's first instance on `runtime`, claiming the name it gives, and
    /// then publish the nonce for `connect`. A name nobody could know yet
    /// that is taken all the same is refused, never joined.
    pub(super) async fn bind(
        self,
        dir: &Path,
        runtime: &Handle,
    ) -> Result<(Endpoint, Incoming), AttachError> {
        let Self {
            owner,
            salt,
            descriptor,
        } = self;
        let nonce = nonce::fresh()?;
        let name = salt::pipe_name(&salt, &nonce, dir);
        let created = {
            let _entered = runtime.enter();
            descriptor.create(&options(true), &name)
        };
        let waiting = created.map_err(|e| {
            if e.kind() == io::ErrorKind::PermissionDenied {
                AttachError(format!(
                    "the MCP pipe {} is taken by another process; not listening",
                    name.display()
                ))
            } else {
                AttachError(format!("cannot make the MCP pipe {}: {e}", name.display()))
            }
        })?;
        nonce::publish(dir, &nonce)?;
        let endpoint = Endpoint {
            name: name.clone(),
            workspace: Some(dir.to_path_buf()),
        };
        let incoming = Incoming {
            waiting,
            name,
            descriptor,
            owner,
        };
        Ok((endpoint, incoming))
    }
}

/// Accept clients until `sessions.stop` is cancelled: a client of this
/// user gets a session, any other is refused and audited before a byte of
/// MCP is read. Runs on a task of the sessions' runtime.
pub(super) async fn accept_loop<T: TransportPort + 'static>(
    incoming: Incoming,
    credentials: Credentials,
    sessions: Sessions<T>,
) {
    let Incoming {
        mut waiting,
        name,
        descriptor,
        owner,
    } = incoming;
    loop {
        // `connect` is cancel-safe: a dropped call loses no client.
        let connected = tokio::select! {
            biased;
            () = sessions.stop.cancelled() => break,
            connected = waiting.connect() => connected,
        };
        if let Err(error) = connected {
            // A client that came and went: ready the instance again.
            tracing::debug!("MCP pipe: a client didn't stay: {error}");
            if !rest(&waiting, &sessions).await {
                break;
            }
            continue;
        }
        let peer = credentials(&waiting);
        // The next instance first, so the name always has one waiting
        // and never goes free for someone else to take.
        let next = match descriptor.create(&options(false), &name) {
            Ok(next) => next,
            Err(error) => {
                tracing::warn!("MCP pipe: cannot make another instance: {error}");
                sessions.refuse(Some(known(&peer)), "no-pipe-instance");
                if !rest(&waiting, &sessions).await {
                    break;
                }
                continue;
            }
        };
        let client = std::mem::replace(&mut waiting, next);
        match peer {
            Ok(peer) if peer.sid.as_ref() == Some(&owner) => sessions.open(client, peer),
            Ok(peer) => {
                drop(client);
                sessions.refuse(Some(peer), "another-user");
            }
            Err(unknown) => {
                drop(client);
                tracing::warn!("MCP pipe: no peer credentials: {}", unknown.error);
                let peer = PeerId {
                    sid: None,
                    pid: unknown.pid,
                };
                sessions.refuse(Some(peer), "no-peer-credentials");
            }
        }
    }
}

/// What is known of a peer, identified or not.
fn known(peer: &Result<PeerId, Unidentified>) -> PeerId {
    match peer {
        Ok(peer) => peer.clone(),
        Err(unknown) => PeerId {
            sid: None,
            pid: unknown.pid,
        },
    }
}

/// Drop the client of the instance `waiting` and rest a moment before it
/// waits again; false once the listener stops meanwhile.
async fn rest<T: TransportPort + 'static>(
    waiting: &NamedPipeServer,
    sessions: &Sessions<T>,
) -> bool {
    if let Err(error) = waiting.disconnect() {
        tracing::debug!("MCP pipe: disconnect: {error}");
    }
    tokio::select! {
        biased;
        () = sessions.stop.cancelled() => false,
        () = tokio::time::sleep(ACCEPT_BACKOFF) => true,
    }
}

/// How many times the bridge tries a pipe whose instances are all busy,
/// and how long apart: the listener makes a new one at once.
const BUSY_TRIES: u32 = 40;
const BUSY_RETRY: Duration = Duration::from_millis(50);

/// Connect to the TUI's pipe `path`, checking that the listener is this
/// OS user's own.
pub async fn connect(path: &Path) -> Result<NamedPipeClient, AttachError> {
    connect_with(path, os_listener).await
}

/// [`connect`], identifying the listener with `credentials`.
pub(crate) async fn connect_with(
    path: &Path,
    credentials: ListenerCredentials,
) -> Result<NamedPipeClient, AttachError> {
    let shown = path.display();
    let busy = Some(windows_sys::Win32::Foundation::ERROR_PIPE_BUSY.cast_signed());
    let mut tries = 0;
    // The default quality of service lets the listener only identify the
    // bridge, never act as it.
    let client = loop {
        match ClientOptions::new().open(path) {
            Ok(client) => break client,
            Err(e) if e.raw_os_error() == busy && tries < BUSY_TRIES => {
                tries += 1;
                tokio::time::sleep(BUSY_RETRY).await;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(not_listening(path)),
            Err(e) => return Err(AttachError(format!("cannot connect to {shown}: {e}"))),
        }
    };
    let listener = credentials(&client)
        .map_err(|e| AttachError(format!("cannot identify the listener at {shown}: {e}")))?;
    let own = win32::own_sid()
        .map_err(|e| AttachError(format!("cannot tell which user this is: {e}")))?;
    let peer = listener.process.sid;
    if peer.as_ref() != Some(&own) {
        return Err(AttachError(format!(
            "the listener at {shown} runs as {}, not this user ({own}); not connecting",
            peer.map_or_else(|| "an unknown user".to_string(), |sid| sid.to_string())
        )));
    }
    if listener.owner != own {
        return Err(AttachError(format!(
            "the pipe {shown} is owned by {}, not this user ({own}); not connecting",
            listener.owner
        )));
    }
    Ok(client)
}

#[cfg(test)]
#[path = "pipe_tests.rs"]
mod tests;
