//! The endpoint's own Device keeps a peer-written Description (#2364): a
//! Device built with Tags storage saves it before the write is answered,
//! with the database guard dropped, serves it after a restart over the
//! configured one, and refuses a write it can't save.

use super::*;
use crate::session::bus_port::BusPort;
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use std::net::Ipv4Addr;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock, Weak};

const DESCRIPTION: PropertyIdentifier = PropertyIdentifier::DESCRIPTION;

/// Tags storage in memory. Each save notes whether the session's database
/// stayed locked for as long as the save ran: whether something held the
/// guard and waited for the save.
#[derive(Default)]
struct Storage {
    saved: Mutex<Option<TagsSnapshot>>,
    saves: AtomicUsize,
    fail: AtomicBool,
    database: OnceLock<Weak<RwLock<ObjectDatabase>>>,
    saved_under_the_guard: AtomicBool,
}

/// How long a save waits for the database lock to come free before it
/// counts as made under the guard. Only a failing run waits this long.
const HELD: Duration = Duration::from_secs(2);

impl Storage {
    fn description(&self) -> Option<String> {
        self.saved.lock().unwrap().clone()?.description
    }
}

impl TagsPersistence for Storage {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.saved.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        // The save runs on the Device's writer thread. A lock held at one
        // instant proves nothing: the request that staged this save may
        // still hold the guard it staged under, and other tasks read the
        // database, yet none of them waits for the save, so the lock comes
        // free at once. A write that saves where it is holds the guard
        // until this call returns, so for it the lock never comes free.
        if let Some(database) = self.database.get().and_then(Weak::upgrade) {
            let deadline = std::time::Instant::now() + HELD;
            while database.try_write().is_err() {
                if std::time::Instant::now() >= deadline {
                    self.saved_under_the_guard.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::yield_now();
            }
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A session serving Device 123, built on `storage` and configured with a
/// Description as an application does at each start.
async fn session_on(storage: &Arc<Storage>) -> (EndpointSession<BusPort>, [u8; 6]) {
    let config = DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(123, 42)
            .unwrap()
            .with_name("device-123"),
        ..Default::default()
    };
    let mut device = DeviceObject::with_tags_persistence(config, storage.clone()).unwrap();
    device.set_description("configured");
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    let mut session =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(db)
            .device_writes(Arc::new(|_| true))
            .build_session_over(BusPort::new())
            .unwrap();
    session.start().await.unwrap();
    let database = Arc::downgrade(session.database.as_ref().unwrap());
    let _ = storage.database.set(database);
    let port = session.bip_local_address().unwrap().port();
    let mac = bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], port);
    (session, mac)
}

fn text(value: &str) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut encoded, value).unwrap();
    encoded.to_vec()
}

fn served(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

#[tokio::test(start_paused = true)]
async fn a_description_written_through_the_endpoint_survives_a_restart() {
    let storage = Arc::new(Storage::default());
    let (mut session, mac) = session_on(&storage).await;
    let mut client = bus_client().await;
    assert_eq!(read(&session, DESCRIPTION).await, served("configured"));
    client
        .write_property(&mac, oid(123), DESCRIPTION, None, text("lobby"), None)
        .await
        .unwrap();
    // Saved by the time the write is answered, and off the database guard.
    assert_eq!(storage.description().as_deref(), Some("lobby"));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    assert!(!storage.saved_under_the_guard.load(Ordering::SeqCst));
    assert_eq!(read(&session, DESCRIPTION).await, served("lobby"));
    // A NULL changes nothing and saves nothing.
    client
        .write_property(&mac, oid(123), DESCRIPTION, None, vec![0], None)
        .await
        .unwrap();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    session.stop().await.unwrap();
    drop(session);

    // The restarted endpoint serves the written text over the configured.
    let (mut session, mac) = session_on(&storage).await;
    let ack = client
        .read_property(&mac, oid(123), DESCRIPTION, None)
        .await
        .unwrap();
    assert_eq!(ack.property_value, text("lobby"));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    client.stop().await.unwrap();
    session.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_description_the_endpoint_cannot_save_is_refused_with_the_old_one_served() {
    let storage = Arc::new(Storage::default());
    let (mut session, mac) = session_on(&storage).await;
    let mut client = bus_client().await;
    client
        .write_property(&mac, oid(123), DESCRIPTION, None, text("lobby"), None)
        .await
        .unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    let refused = client
        .write_property(&mac, oid(123), DESCRIPTION, None, text("hall"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            refused,
            Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::DEVICE && code == ErrorCode::OPERATIONAL_PROBLEM
        ),
        "{refused:?}"
    );
    let ack = client
        .read_property(&mac, oid(123), DESCRIPTION, None)
        .await
        .unwrap();
    assert_eq!(ack.property_value, text("lobby"));
    assert_eq!(storage.description().as_deref(), Some("lobby"));
    assert!(!storage.saved_under_the_guard.load(Ordering::SeqCst));
    client.stop().await.unwrap();
    session.stop().await.unwrap();
}
