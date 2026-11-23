//! The Device a builder builds keeps what peers write to it in the file
//! `device_storage_path` names (#2730), on the wire over the shared test
//! transport: a server built again on the path serves a written
//! Description and, with the zone opt-in, a written zone; with the opt-in
//! off the kept zone stays in the file, unapplied. Without a path such a
//! write lasts until the server stops. A missing directory starts fresh, a
//! damaged or newer file fails the build naming the path, and a write whose
//! save fails is refused.

use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::ReceivedNpdu;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use PropertyIdentifier as P;

const DEVICE: u32 = 2730;

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(DEVICE, 260).unwrap()
}

fn device() -> ObjectIdentifier {
    identity().device_oid()
}

fn zone(utc_offset_minutes: i16, daylight_savings: bool) -> ClockConfig {
    ClockConfig::new(utc_offset_minutes, daylight_savings).unwrap()
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

/// A directory of the test's own under the system's temporary one, not
/// made until a save makes it, and removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "rusty-device-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        )))
    }

    /// The Device file, two directories down, neither made yet.
    fn file(&self) -> PathBuf {
        self.0.join("state").join("device.tags")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A server for Device 2730 built by the generic builder, its Device kept
/// at `path` if given, in zone 300 minutes west, taking a peer's zone
/// writes when `writable`, and a peer at MAC 1 that sends it requests.
struct Peer {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    invoke: u8,
}

impl Peer {
    async fn build(path: Option<&Path>, writable: bool) -> Result<Self, Error> {
        let (tx, rx) = mpsc::channel(16);
        let transport = TestTransport::builder().local_mac(&[2]).inbound(rx).build();
        let sent = transport.handle().sent();
        let mut builder = BACnetServer::generic_builder()
            .device(identity())
            .transport(transport)
            .clock_zone(zone(300, false))
            .clock_zone_writable(writable);
        if let Some(path) = path {
            builder = builder.device_storage_path(path);
        }
        Ok(Self {
            server: builder.build().await?,
            tx,
            sent,
            invoke: 0,
        })
    }

    async fn start(path: Option<&Path>, writable: bool) -> Self {
        Self::build(path, writable).await.unwrap()
    }

    /// Send one confirmed request from the peer and return the server's
    /// answer to it.
    async fn request(&mut self, service: ConfirmedServiceChoice, body: Bytes) -> Apdu {
        self.invoke = self.invoke.wrapping_add(1);
        let id = self.invoke;
        let mut payload = BytesMut::new();
        encode_apdu(
            &mut payload,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: service,
                service_request: body,
            }),
        )
        .unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                expecting_reply: true,
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.tx
            .send(ReceivedNpdu::unverified(
                npdu.freeze(),
                MacAddr::from_slice(&[1]),
                false,
                vec![],
                None,
            ))
            .await
            .unwrap();
        let sent = self.sent.clone();
        tokio::time::timeout(Duration::from_secs(5), async move {
            loop {
                let seen = sent.len();
                let answer = sent.frames().into_iter().map(|f| f.apdu()).find(|apdu| {
                    let invoke = match apdu {
                        Apdu::ComplexAck(ack) => ack.invoke_id,
                        Apdu::SimpleAck(ack) => ack.invoke_id,
                        Apdu::Error(error) => error.invoke_id,
                        Apdu::Reject(reject) => reject.invoke_id,
                        Apdu::Abort(abort) => abort.invoke_id,
                        _ => return false,
                    };
                    invoke == id
                });
                if let Some(answer) = answer {
                    return answer;
                }
                sent.wait_for_len(seen + 1).await;
            }
        })
        .await
        .expect("an answer")
    }

    /// A WriteProperty of the Device's `property`: `Ok` for a SimpleACK,
    /// else the Error's class and code.
    async fn write(
        &mut self,
        property: P,
        value: PropertyValue,
    ) -> Result<(), (ErrorClass, ErrorCode)> {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value).unwrap();
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: device(),
            property_identifier: property,
            property_array_index: None,
            property_value: encoded.to_vec(),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        match self
            .request(ConfirmedServiceChoice::WRITE_PROPERTY, body.freeze())
            .await
        {
            Apdu::SimpleAck(_) => Ok(()),
            Apdu::Error(error) => Err((error.error_class, error.error_code)),
            other => panic!("WriteProperty answered {other:?}"),
        }
    }

    /// A ReadProperty of the Device's `property`.
    async fn read(&mut self, property: P) -> PropertyValue {
        let mut body = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: device(),
            property_identifier: property,
            property_array_index: None,
        }
        .encode(&mut body);
        match self
            .request(ConfirmedServiceChoice::READ_PROPERTY, body.freeze())
            .await
        {
            Apdu::ComplexAck(ack) => {
                let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
                decode_application_value(&ack.property_value, 0).unwrap().0
            }
            other => panic!("ReadProperty answered {other:?}"),
        }
    }

    async fn stop(mut self) {
        self.server.stop().await.unwrap();
    }
}

/// What the file at `path` holds for the Device, read as the next start
/// would.
fn kept(path: &Path) -> Option<TagsSnapshot> {
    FileTagsPersistence::new(path)
        .unwrap()
        .load(device())
        .unwrap()
}

/// A written Description and a written zone half survive a rebuild on the
/// same path, which the first save made, directories and all; with the
/// opt-in off the kept half stays in the file and the builder's zone runs.
/// The built Device has no Tags row to keep.
#[tokio::test(start_paused = true)]
async fn a_server_built_again_on_the_path_serves_what_peers_wrote() {
    let scratch = Scratch::new();
    let path = scratch.file();
    let mut peer = Peer::start(Some(&path), true).await;
    assert!(!path.exists(), "nothing is saved until a write");
    assert_eq!(peer.read(P::DESCRIPTION).await, text(""));
    peer.write(P::DESCRIPTION, text("Lobby")).await.unwrap();
    peer.write(P::UTC_OFFSET, PropertyValue::Signed(-60))
        .await
        .unwrap();
    assert_eq!(
        peer.write(P::TAGS, PropertyValue::List(Vec::new())).await,
        Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
    peer.stop().await;
    let saved = kept(&path).unwrap();
    assert_eq!(saved.description.as_deref(), Some("Lobby"));
    assert_eq!(saved.zone.utc_offset, Some(-60));
    assert_eq!(saved.tags, None);

    let mut peer = Peer::start(Some(&path), true).await;
    assert_eq!(peer.read(P::DESCRIPTION).await, text("Lobby"));
    assert_eq!(peer.read(P::UTC_OFFSET).await, PropertyValue::Signed(-60));
    assert_eq!(peer.server.clock_zone(), Some(zone(-60, false)));
    peer.stop().await;

    // The opt-in off: the half stays in the file, unapplied, and the
    // Description is still served.
    let before = std::fs::read(&path).unwrap();
    let mut peer = Peer::start(Some(&path), false).await;
    assert_eq!(peer.read(P::DESCRIPTION).await, text("Lobby"));
    assert_eq!(peer.read(P::UTC_OFFSET).await, PropertyValue::Signed(300));
    peer.stop().await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

/// Without a path the Device keeps a written Description until the server
/// stops; the next one serves the empty text it starts with.
#[tokio::test(start_paused = true)]
async fn without_a_path_a_written_description_lasts_until_stop() {
    let mut peer = Peer::start(None, true).await;
    peer.write(P::DESCRIPTION, text("Lobby")).await.unwrap();
    assert_eq!(peer.read(P::DESCRIPTION).await, text("Lobby"));
    peer.stop().await;
    let mut peer = Peer::start(None, true).await;
    assert_eq!(peer.read(P::DESCRIPTION).await, text(""));
    peer.stop().await;
}

/// A file the Device can't load fails the build with an error naming the
/// path and the reason, and is left as it was: damaged octets, a newer
/// release's version, and another Device's file, whose error names both
/// Devices and the way on (a Device instance changed on the same path).
#[tokio::test(start_paused = true)]
async fn a_file_that_does_not_load_fails_the_build_naming_the_path() {
    let scratch = Scratch::new();
    let path = scratch.file();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let other = ObjectIdentifier::new(ObjectType::DEVICE, DEVICE + 1).unwrap();
    let newer = [&b"RBNTAG09"[..], &device().encode(), &[0]].concat();
    let anothers = [&b"RBNTAG01"[..], &other.encode(), &[0]].concat();
    let belongs = format!("belongs to {other}, not {}", device());
    for (octets, reasons) in [
        (b"not a Tags file".to_vec(), vec!["no valid header"]),
        (newer, vec!["version 9"]),
        (anothers, vec![belongs.as_str(), "remove or move it"]),
    ] {
        std::fs::write(&path, &octets).unwrap();
        let Err(error) = Peer::build(Some(&path), true).await else {
            panic!("{reasons:?}: the build must fail");
        };
        let message = error.to_string();
        assert!(message.contains(&path.display().to_string()), "{message}");
        for reason in &reasons {
            assert!(message.contains(reason), "{message}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), octets, "{reasons:?}");
    }
}

/// A write whose save fails is refused with DEVICE / OPERATIONAL_PROBLEM,
/// the Description served before still served, and the file unchanged.
#[tokio::test(start_paused = true)]
async fn a_write_whose_save_fails_is_refused() {
    let scratch = Scratch::new();
    let path = scratch.file();
    let mut peer = Peer::start(Some(&path), true).await;
    peer.write(P::DESCRIPTION, text("Lobby")).await.unwrap();
    let before = std::fs::read(&path).unwrap();
    // A directory where the save writes its temporary file.
    let mut temporary = path.clone().into_os_string();
    temporary.push(".tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert_eq!(
        peer.write(P::DESCRIPTION, text("Atrium")).await,
        Err((ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM))
    );
    assert_eq!(peer.read(P::DESCRIPTION).await, text("Lobby"));
    peer.stop().await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

/// With the zone opt-in on, a Device without storage gets one warning at
/// start that a written zone lasts until the server stops; with a path, or
/// with the opt-in off, none.
#[tokio::test(start_paused = true)]
async fn a_writable_zone_without_storage_is_warned_about() {
    let scratch = Scratch::new();
    let path = scratch.file();
    let warned = |spy: &super::super::warn_spy::WarnSpy| {
        spy.warnings()
            .iter()
            .filter(|warning| warning.message.contains("lasts until the server stops"))
            .count()
    };
    for (stored, writable, expected) in [(false, true, 1), (true, true, 0), (false, false, 0)] {
        let (spy, _guard) = super::super::warn_spy::WarnSpy::install();
        let peer = Peer::start(stored.then_some(path.as_path()), writable).await;
        assert_eq!(
            warned(&spy),
            expected,
            "stored {stored}, writable {writable}"
        );
        peer.stop().await;
    }
}

/// A path beside a database the application gave, or in the configuration
/// a database is started with, is refused: that Device brings its own
/// storage.
#[tokio::test(start_paused = true)]
async fn a_path_beside_a_given_database_is_refused() {
    let scratch = Scratch::new();
    let given = || identity().build_database().unwrap();
    let Err(error) = BACnetServer::generic_builder()
        .device(identity())
        .database(given())
        .transport(TestTransport::new())
        .device_storage_path(scratch.file())
        .build()
        .await
    else {
        panic!("a builder with a database and a path must refuse");
    };
    // Refused as the builder prepares the database, before a start.
    let message = error.to_string();
    assert!(
        message.contains("the database the builder was given"),
        "{message}"
    );
    let config = ServerConfig {
        device: identity(),
        device_storage_path: Some(scratch.file()),
        ..ServerConfig::default()
    };
    let Err(error) = BACnetServer::start(config, given(), TestTransport::new()).await else {
        panic!("a start with a path must refuse");
    };
    let message = error.to_string();
    assert!(message.contains("a database a start is given"), "{message}");
    assert!(!scratch.0.exists());
}

/// Every builder takes the path into its configuration, a relative one
/// resolved against the current directory when it is given.
#[test]
fn the_builders_take_the_path() {
    let relative = PathBuf::from("device.tags");
    let path = std::env::current_dir().unwrap().join(&relative);
    assert_eq!(
        BACnetServer::<TestTransport>::generic_builder()
            .device_storage_path(&relative)
            .config
            .device_storage_path,
        Some(path.clone())
    );
    assert_eq!(
        BACnetServer::bip_builder()
            .device_storage_path(&relative)
            .config
            .device_storage_path,
        Some(path.clone())
    );
    #[cfg(feature = "sc-tls")]
    assert_eq!(
        BACnetServer::sc_builder()
            .device_storage_path(&relative)
            .config
            .device_storage_path,
        Some(path)
    );
    let _: fn() -> ObjectDatabase = ObjectDatabase::new;
}
