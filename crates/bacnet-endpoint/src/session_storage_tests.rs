//! The Device a session builds keeps what peers write to it in the file
//! `with_device_storage_path` names (#2730): a session started again on the
//! path serves a written Description to a peer on the in-memory link. The
//! path is refused, before the link starts, beside a database of the
//! session's, on a client-only session, and on its server's builder, which
//! takes none of its own; a bare B/IP transport refuses it.
use super::*;
use crate::session::server_seat::tests::wire::Wire;
use crate::session::{SessionConfig, SessionRole};
use bacnet_client::client::BACnetClient;
use bacnet_objects::object_profile::{FileTagsPersistence, TagsPersistence};
use bacnet_server::server::{BACnetServer, ServerBuilder};
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const NODE: u8 = 1;
const PEER: u8 = 2;
const NODE_DEVICE: u32 = 2730;

fn identity() -> crate::DeviceIdentity {
    crate::DeviceIdentity::new(NODE_DEVICE, 42).unwrap()
}

/// A directory of the test's own, not made until a save makes it, and
/// removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "rusty-session-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn file(&self) -> PathBuf {
        self.0.join("node").join("device.tags")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A session of both roles on node [`NODE`] of `wire`, with Device
/// [`NODE_DEVICE`] and no database, a server that lets peers write, and
/// its Device kept at `path`.
fn session(wire: &Wire, path: &Path) -> EndpointSession<TestTransport> {
    EndpointSession::new(
        wire.port(NODE).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .device(identity())
    .with_server(BACnetServer::generic_builder())
    .with_device_storage_path(path)
}

/// The node's Description as a standalone client on node [`PEER`] reads
/// it, after writing `write` there first if given.
async fn description(wire: &Wire, write: Option<&str>) -> String {
    let peer = BACnetClient::generic_builder()
        .transport(wire.port(PEER).build())
        .apdu_timeout_ms(1_000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    let node = Wire::mac(NODE);
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    let property = PropertyIdentifier::DESCRIPTION;
    if let Some(text) = write {
        // An application-tagged character string, UTF-8.
        let mut value = vec![0x75, text.len() as u8 + 1, 0x00];
        value.extend_from_slice(text.as_bytes());
        peer.write_property(&node, device, property, None, value, None)
            .await
            .unwrap();
    }
    let ack = peer
        .read_property(&node, device, property, None)
        .await
        .unwrap();
    String::from_utf8(ack.property_value[3..].to_vec()).unwrap()
}

/// A peer's written Description is saved before it is answered, and a
/// session started again on the path serves it.
#[tokio::test(start_paused = true)]
async fn a_session_started_again_on_the_path_serves_the_written_description() {
    let scratch = Scratch::new();
    let path = scratch.file();
    let wire = Wire::new();
    let mut node = session(&wire, &path);
    node.start().await.unwrap();
    assert_eq!(description(&wire, Some("Lobby")).await, "Lobby");
    let device = ObjectIdentifier::new(ObjectType::DEVICE, NODE_DEVICE).unwrap();
    let saved = FileTagsPersistence::new(&path)
        .unwrap()
        .load(device)
        .unwrap();
    assert_eq!(saved.unwrap().description.as_deref(), Some("Lobby"));
    node.stop().await.unwrap();
    drop(node);

    let wire = Wire::new();
    let mut node = session(&wire, &path);
    node.start().await.unwrap();
    assert_eq!(description(&wire, None).await, "Lobby");
    node.stop().await.unwrap();
}

/// The error `session`'s start fails with, leaving it ready to start.
async fn refused(session: &mut EndpointSession<TestTransport>) -> String {
    let error = session.start().await.unwrap_err().to_string();
    assert_eq!(
        session.lifecycle.load(Ordering::Acquire),
        crate::session::Lifecycle::Ready as u8,
        "{error}"
    );
    error
}

/// The path is refused at start, before the link starts, where the
/// session builds no Device for a server: beside its own database, on a
/// client-only session, and named on the server's builder rather than the
/// session.
#[tokio::test(start_paused = true)]
async fn a_path_the_session_cannot_use_is_refused_at_start() {
    let scratch = Scratch::new();
    let path = scratch.file();
    let wire = Wire::new();

    let mut given = session(&wire, &path).with_database(identity().build_database().unwrap());
    let error = refused(&mut given).await;
    assert!(error.contains("given a database"), "{error}");

    // A client-only session runs no server, so nothing could write the
    // Device it would build.
    let mut client_only = EndpointSession::new(
        wire.port(NODE).build(),
        SessionRole::ClientOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .device(identity())
    .with_device_storage_path(&path);
    let error = refused(&mut client_only).await;
    assert!(
        error.contains("client-only session runs no server"),
        "{error}"
    );

    // A serving session with no identity never starts, so a path on one is
    // never dropped without a word.
    let mut nameless = EndpointSession::new(
        wire.port(NODE).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .with_device_storage_path(&path);
    let error = refused(&mut nameless).await;
    assert!(
        error.contains("needs the node's Device identity"),
        "{error}"
    );
    assert!(!scratch.0.exists());

    let builder: ServerBuilder<TestTransport> =
        BACnetServer::generic_builder().device_storage_path(&path);
    let mut on_builder = EndpointSession::new(
        wire.port(NODE).build(),
        SessionRole::Both,
        SessionConfig::default(),
    )
    .unwrap()
    .device(identity())
    .with_server(builder);
    let error = refused(&mut on_builder).await;
    assert!(error.contains("with_device_storage_path"), "{error}");
    assert!(!scratch.0.exists());
}

/// The B/IP builder hands the path to the session it builds, and a bare
/// transport, which would drop it, refuses it.
#[test]
fn the_bip_builder_hands_the_path_to_its_session() {
    let builder = || {
        crate::bip::BipEndpointBuilder::new(
            std::net::Ipv4Addr::LOCALHOST,
            0,
            std::net::Ipv4Addr::BROADCAST,
        )
        .device_storage_path("device.tags")
    };
    let Err(error) = builder().build_transport() else {
        panic!("a bare transport must refuse the path");
    };
    assert!(error.to_string().contains("Device storage path"), "{error}");
    // Resolved against the current directory when it is given.
    let session = builder().build_session().unwrap();
    assert_eq!(
        session.device_storage_path,
        Some(std::env::current_dir().unwrap().join("device.tags"))
    );
}
