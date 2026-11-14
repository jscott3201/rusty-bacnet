//! TUI-attached mode on Windows (#2201): the named pipe's peer checks on
//! both ends, its single listener and claimed name, the descriptor its
//! instances carry. The reads, the read-only tool list, the shutdown, the
//! seats, the shared budget and the initialize wait run on every platform
//! (`attach.rs`, `attach_more.rs`, `attach_shared.rs`, `attach_runtime.rs`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

use super::attach::{audit, bridged, events, lab, private_dir, workspace};
use crate::Policy;
use crate::attach::{
    self, Listener, ListenerId, PeerId, SessionRuntime, Sid, Unidentified, pipe_name, security_of,
    socket_path,
};

/// The local SYSTEM account's SID: a user other than the test's.
fn system() -> Sid {
    Sid(vec![1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0])
}

/// A listener on `dir` over the lab's client, its clients identified by
/// `credentials`.
async fn listener_with(lab: &super::Lab, dir: &Path, credentials: attach::Credentials) -> Listener {
    Listener::bind_with(
        lab.client(),
        Policy::default(),
        "tester".into(),
        workspace(dir),
        (credentials, SessionRuntime::Callers),
        None,
    )
    .await
    .unwrap()
}

/// Open the pipe `name` as a plain client, as an agent's host would.
fn client(name: &Path) -> NamedPipeClient {
    ClientOptions::new().open(name).unwrap()
}

/// Send `client` an initialize request and expect the pipe closed with
/// nothing written back.
async fn closed_unanswered(mut client: NamedPipeClient) {
    let hello = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": { "name": "x", "version": "1" } } });
    // The listener may close before the request is written.
    let _ = client.write_all(format!("{hello}\n").as_bytes()).await;
    let mut answer = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut answer));
    assert!(
        read.await.is_ok(),
        "still open: {}",
        String::from_utf8_lossy(&answer)
    );
    assert!(answer.is_empty(), "{}", String::from_utf8_lossy(&answer));
}

/// A client whose process runs as another user (a stand-in for the
/// token's answer: a test can't run as SYSTEM) is closed before any MCP
/// traffic and audited with its SID and pid.
#[tokio::test(start_paused = true)]
async fn another_users_client_is_refused_and_audited() {
    fn stranger(_: &NamedPipeServer) -> Result<PeerId, Unidentified> {
        Ok(PeerId {
            sid: Some(system()),
            pid: Some(4242),
        })
    }
    let lab = lab().await;
    let dir = private_dir();
    let listener = listener_with(&lab, dir.path(), stranger).await;
    closed_unanswered(client(listener.path())).await;
    assert_eq!(*listener.agents().borrow(), 0);
    listener.shutdown().await;
    let records = audit(dir.path());
    assert_eq!(events(dir.path()), ["mcp-refused"], "{records:?}");
    assert_eq!(records[0]["via"], json!("named-pipe"));
    assert_eq!(records[0]["peer_sid"], json!("S-1-5-18"));
    assert_eq!(records[0]["peer_pid"], json!(4242));
    assert_eq!(records[0]["outcome"]["why"], json!("another-user"));
    lab.stop().await;
}

/// A client whose user can't be found (its process gone, or not open to
/// this user) is refused too, with the pid when the pipe gave one.
#[tokio::test(start_paused = true)]
async fn an_unidentified_client_is_refused() {
    fn unidentified(_: &NamedPipeServer) -> Result<PeerId, Unidentified> {
        Err(Unidentified {
            pid: Some(77),
            error: std::io::Error::other("access denied"),
        })
    }
    let lab = lab().await;
    let dir = private_dir();
    let listener = listener_with(&lab, dir.path(), unidentified).await;
    closed_unanswered(client(listener.path())).await;
    listener.shutdown().await;
    let records = audit(dir.path());
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["outcome"]["why"], json!("no-peer-credentials"));
    assert_eq!(records[0]["peer_pid"], json!(77));
    assert_eq!(records[0]["peer_sid"], json!(null));
    lab.stop().await;
}

/// The bridge to an impostor pipe identified by `credentials`: its
/// refusal, after checking the impostor was written nothing.
async fn refused_by_the_bridge(tag: &str, credentials: attach::ListenerCredentials) -> String {
    let dir = private_dir();
    let mut salt = [0; 32];
    getrandom::fill(&mut salt).unwrap();
    let name = pipe_name(&attach::Salt(salt), &attach::Nonce(salt), dir.path());
    let name = PathBuf::from(format!("{}-{tag}", name.display()));
    let mut impostor = ServerOptions::new().create(&name).unwrap();
    let connecting = tokio::spawn({
        let name = name.clone();
        async move { attach::connect_with(&name, credentials).await }
    });
    impostor.connect().await.unwrap();
    let error = connecting.await.unwrap().unwrap_err().to_string();
    let mut written = Vec::new();
    impostor.read_to_end(&mut written).await.unwrap();
    assert!(written.is_empty(), "{written:?}");
    error
}

/// The bridge refuses a pipe whose server runs as another user (a
/// stand-in for the token's answer) and writes it nothing.
#[tokio::test]
async fn the_bridge_refuses_a_listener_of_another_user() {
    fn stranger(_: &NamedPipeClient) -> std::io::Result<ListenerId> {
        Ok(ListenerId {
            process: PeerId {
                sid: Some(system()),
                pid: Some(4),
            },
            owner: attach::own_sid().unwrap(),
        })
    }
    let error = refused_by_the_bridge("impostor", stranger).await;
    assert!(error.contains("not this user"), "{error}");
    assert!(error.contains("S-1-5-18"), "{error}");
}

/// The bridge refuses a pipe owned by another user even when the process
/// at its server end is this user's (as a reused pid could make it look).
#[tokio::test]
async fn the_bridge_refuses_a_pipe_another_user_owns() {
    fn foreign(_: &NamedPipeClient) -> std::io::Result<ListenerId> {
        let own = attach::own_sid().unwrap();
        Ok(ListenerId {
            process: PeerId {
                sid: Some(own),
                pid: Some(4242),
            },
            owner: system(),
        })
    }
    let error = refused_by_the_bridge("foreign", foreign).await;
    assert!(error.contains("is owned by S-1-5-18"), "{error}");
}

/// Acceptance: a second listener on the workspace finds the first's lock
/// and is refused while the first keeps serving; once the first is gone a
/// listener starts again at once, under a new name (#2237), so a pipe
/// made under a name seen during an earlier run doesn't keep it from
/// listening, and `connect` reaches the new one.
#[tokio::test(start_paused = true)]
async fn a_second_listener_and_a_taken_name_are_refused() {
    let lab = lab().await;
    let dir = private_dir();
    let first = lab.attach(dir.path(), Policy::default()).await.unwrap();
    assert!(attach::listening(dir.path()));
    let error = lab.attach(dir.path(), Policy::default()).await.unwrap_err();
    assert!(
        error.to_string().contains("another TUI is listening"),
        "{error}"
    );
    // Refused by the lock, it published no nonce of its own.
    assert_eq!(socket_path(dir.path()).unwrap(), first.path());
    let (mut agent, bridge) = bridged(first.path());
    let hello = agent.initialize().await;
    assert!(hello.get("result").is_some(), "the first serves: {hello}");
    agent.close().await;
    bridge.await.unwrap().unwrap();
    let seen = first.path().to_path_buf();
    first.shutdown().await;
    assert!(!attach::listening(dir.path()), "released at shutdown");

    // Someone who saw the name takes it while no TUI runs.
    let squatter = ServerOptions::new().create(&seen).unwrap();
    let again = lab.attach(dir.path(), Policy::default()).await.unwrap();
    assert_ne!(again.path(), seen, "a new name");
    let path = socket_path(dir.path()).unwrap();
    assert_eq!(path, again.path(), "the nonce read anew");
    let (mut agent, bridge) = bridged(&path);
    let hello = agent.initialize().await;
    assert!(hello.get("result").is_some(), "the next serves: {hello}");
    agent.close().await;
    bridge.await.unwrap().unwrap();
    again.shutdown().await;
    drop(squatter);
    lab.stop().await;
}

/// Acceptance: the listener's instances, the first and the one it makes
/// once an agent connects, carry the owner-only descriptor: owned by this
/// user, with one entry, allowing this user, and the medium label.
#[tokio::test(start_paused = true)]
async fn the_listeners_instances_grant_only_this_user() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    let own = attach::own_sid().unwrap();
    let first = client(listener.path());
    let mut count = listener.agents();
    count.wait_for(|n| *n == 1).await.unwrap();
    let later = client(listener.path());
    count.wait_for(|n| *n == 2).await.unwrap();
    for end in [&first, &later] {
        let security = security_of(end).unwrap();
        assert_eq!(security.owner, own);
        let entries = &security.access;
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!((entries[0].0, &entries[0].2), (0, &own), "{entries:?}");
        assert_eq!(security.label.len(), 1, "{security:?}");
        assert_eq!(security.label[0].0, 0x11, "{security:?}");
    }
    drop((first, later));
    listener.shutdown().await;
    lab.stop().await;
}

/// Acceptance (#2219, #2237): the listener names its pipe from the
/// workspace's salt and its run's nonce, both files owned by and open to
/// this user alone, so the path alone doesn't give the name; `connect`'s
/// lookup finds the same name, and none once the listener has stopped
/// and withdrawn its nonce. Each run has a new name. A workspace other
/// users may list is refused before any pipe, salt or nonce is made.
#[tokio::test(start_paused = true)]
async fn the_pipe_name_needs_the_private_salt() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    assert_eq!(listener.path(), socket_path(dir.path()).unwrap());
    let own = attach::own_sid().unwrap();
    for file in [attach::SALT_FILE, attach::NONCE_FILE] {
        let opened = std::fs::File::open(dir.path().join(file)).unwrap();
        let security = security_of(&opened).unwrap();
        assert_eq!(security.owner, own, "{file}");
        let entries = &security.access;
        assert_eq!(entries.len(), 1, "{file}: {entries:?}");
        assert_eq!((entries[0].0, &entries[0].2), (0, &own), "{entries:?}");
    }
    let other = attach::Salt([0; 32]);
    let nonce = attach::Nonce([0; 32]);
    assert_ne!(listener.path(), pipe_name(&other, &nonce, dir.path()));
    let first = listener.path().to_path_buf();
    listener.shutdown().await;
    let error = socket_path(dir.path()).unwrap_err().to_string();
    assert!(error.contains("no TUI is listening"), "{error}");
    // A nonce left by a TUI that crashed names a pipe nobody holds.
    attach::publish_nonce(dir.path(), &attach::Nonce([7; 32])).unwrap();
    let (_agent, bridge) = bridged(&socket_path(dir.path()).unwrap());
    let error = bridge.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("no TUI is listening"), "{error}");
    let again = lab.attach(dir.path(), Policy::default()).await.unwrap();
    assert_ne!(again.path(), first, "a new name each run");
    again.shutdown().await;

    let open = dir.path().join("open");
    let sddl = format!("D:P(A;OICI;FA;;;{own})(A;;FR;;;WD)");
    attach::create_dir(&open, &attach::Descriptor::from_sddl(&sddl).unwrap()).unwrap();
    let error = lab.attach(&open, Policy::default()).await.unwrap_err();
    assert!(error.to_string().contains("open to other users"), "{error}");
    assert!(!open.join(attach::SALT_FILE).exists());
    assert!(!open.join(attach::NONCE_FILE).exists());
    lab.stop().await;
}
