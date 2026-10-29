//! Storage stand-ins and helpers for the Access Rights persistence tests
//! (#1392).

use super::*;
use crate::durable::SaveWait;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::RemoteError;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Storage in memory. A save can be made to fail, or to wait until the test
/// lets it go.
#[derive(Default)]
pub(super) struct MemoryPersistence {
    saved: Mutex<Option<(ObjectIdentifier, AccessRightsSnapshot)>>,
    pub(super) fail: AtomicBool,
    saves: AtomicU64,
    hold: Mutex<Option<Hold>>,
}

struct Hold {
    started: mpsc::Sender<AccessRightsSnapshot>,
    go: mpsc::Receiver<()>,
}

/// The test's side of a held storage: each save reports on `started` and
/// then waits for one message on `go`. Dropping `go` lets every save through.
#[cfg(feature = "std")]
pub(super) struct Held {
    pub(super) started: mpsc::Receiver<AccessRightsSnapshot>,
    pub(super) go: mpsc::Sender<()>,
}

/// Long enough for any wait on another thread in these tests.
pub(super) const WAIT: Duration = Duration::from_secs(10);

impl MemoryPersistence {
    /// Make every later save wait for the test.
    #[cfg(feature = "std")]
    pub(super) fn hold(&self) -> Held {
        let (started, started_rx) = mpsc::channel();
        let (go_tx, go) = mpsc::channel();
        *self.hold.lock().unwrap() = Some(Hold { started, go });
        Held {
            started: started_rx,
            go: go_tx,
        }
    }

    /// What storage holds; `None` before any save.
    pub(super) fn snapshot(&self) -> Option<AccessRightsSnapshot> {
        self.saved
            .lock()
            .unwrap()
            .clone()
            .map(|(_, snapshot)| snapshot)
    }

    /// Saves that succeeded.
    pub(super) fn saves(&self) -> u64 {
        self.saves.load(Ordering::SeqCst)
    }

    /// Put `snapshot` in storage as if an object had saved it.
    pub(super) fn preload(&self, rights: ObjectIdentifier, snapshot: AccessRightsSnapshot) {
        *self.saved.lock().unwrap() = Some((rights, snapshot));
    }
}

impl AccessRightsPersistence for MemoryPersistence {
    fn load(&self, rights: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error> {
        Ok(self
            .saved
            .lock()
            .unwrap()
            .clone()
            .filter(|(oid, _)| *oid == rights)
            .map(|(_, snapshot)| snapshot))
    }

    fn save(&self, rights: ObjectIdentifier, snapshot: &AccessRightsSnapshot) -> Result<(), Error> {
        if let Some(hold) = &*self.hold.lock().unwrap() {
            let _ = hold.started.send(snapshot.clone());
            let _ = hold.go.recv();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        *self.saved.lock().unwrap() = Some((rights, snapshot.clone()));
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Access Rights 1, kept in `storage`.
pub(super) fn persistent(storage: &Arc<MemoryPersistence>) -> AccessRightsObject {
    AccessRightsObject::with_persistence(
        1,
        "AR",
        Arc::clone(storage) as Arc<dyn AccessRightsPersistence>,
    )
    .unwrap()
}

/// Block until `wait` is over, failing after [`WAIT`].
pub(super) fn block_on(wait: &SaveWait) {
    let (done, finished) = mpsc::channel();
    let blocking = wait.clone();
    std::thread::spawn(move || {
        blocking.block();
        let _ = done.send(());
    });
    finished
        .recv_timeout(WAIT)
        .expect("the wait did not end in time");
}

/// A rule that applies at any time in Access Zone `instance` of this device.
#[cfg(feature = "std")]
pub(super) fn zone_rule(instance: u32) -> BACnetAccessRule {
    let zone = ObjectIdentifier::new(ObjectType::ACCESS_ZONE, instance).unwrap();
    BACnetAccessRule::new(None, Some(zone.into()), true)
}

/// A rule whose location is an Access Door, which every write refuses.
#[cfg(feature = "std")]
pub(super) fn door_rule() -> BACnetAccessRule {
    let door = ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 1).unwrap();
    BACnetAccessRule::new(None, Some(door.into()), true)
}

/// `rules` back to back, as the server hands a written array to the object.
#[cfg(feature = "std")]
pub(super) fn octets(rules: &[BACnetAccessRule]) -> PropertyValue {
    let mut buf = BytesMut::new();
    for rule in rules {
        encode_access_rule(&mut buf, rule);
    }
    PropertyValue::ApplicationData(buf.to_vec())
}

/// Write `value` to `property` at `index`, as the handlers do.
pub(super) fn write(
    rights: &mut AccessRightsObject,
    property: P,
    index: Option<u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    rights.write_property(property, index, value, None)
}

/// Write the whole of Positive_Access_Rules.
#[cfg(feature = "std")]
pub(super) fn write_positive(
    rights: &mut AccessRightsObject,
    rules: &[BACnetAccessRule],
) -> Result<(), Error> {
    write(rights, P::POSITIVE_ACCESS_RULES, None, octets(rules))
}

/// What storage holds with only Positive_Access_Rules written, as `rules`.
#[cfg(feature = "std")]
pub(super) fn positive_only(rules: &[BACnetAccessRule]) -> AccessRightsSnapshot {
    AccessRightsSnapshot {
        positive_access_rules: Some(rules.to_vec()),
        ..AccessRightsSnapshot::default()
    }
}

pub(super) fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class: got_class,
            code: got_code,
        })) => assert_eq!(
            (got_class, got_code),
            (class, code),
            "expected {class:?} / {code:?}"
        ),
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}
