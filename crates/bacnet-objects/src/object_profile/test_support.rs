use super::*;
use crate::durable::{PendingWrite, SaveWait, StageStep};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub(super) const WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct Memory {
    pub saved: Mutex<Option<TagsSnapshot>>,
    pub fail_load: AtomicBool,
    pub fail_save: AtomicBool,
    pub attempts: AtomicUsize,
    hold: Mutex<Option<(mpsc::Sender<TagsSnapshot>, mpsc::Receiver<()>)>>,
}

impl Memory {
    pub fn hold(&self) -> (mpsc::Receiver<TagsSnapshot>, mpsc::Sender<()>) {
        let (started, rx) = mpsc::channel();
        let (go, release) = mpsc::channel();
        *self.hold.lock().unwrap() = Some((started, release));
        (rx, go)
    }

    pub fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
    pub fn saved(&self) -> Option<TagsSnapshot> {
        self.saved.lock().unwrap().clone()
    }
}

impl TagsPersistence for Memory {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        if self.fail_load.load(Ordering::SeqCst) {
            return Err(Error::Encoding("load refused".into()));
        }
        Ok(self.saved())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if let Some((started, go)) = &*self.hold.lock().unwrap() {
            let _ = started.send(snapshot.clone());
            let _ = go.recv();
        }
        if self.fail_save.load(Ordering::SeqCst) {
            return Err(Error::Encoding("save refused".into()));
        }
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

pub(super) fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap()
}
pub(super) fn tags(name: &str) -> Vec<BACnetNameValue> {
    vec![BACnetNameValue::semantic(name)]
}
pub(super) fn profile(name: &str) -> ObjectProfile {
    ObjectProfile {
        tags: Some(tags(name)),
        ..ObjectProfile::default()
    }
}
pub(super) fn snapshot(name: &str) -> TagsSnapshot {
    TagsSnapshot {
        tags: Some(tags(name)),
    }
}
pub(super) fn framed(tags: &[BACnetNameValue]) -> PropertyValue {
    let mut bytes = BytesMut::new();
    for tag in tags {
        encode_name_value(&mut bytes, tag).unwrap();
    }
    PropertyValue::ApplicationData(bytes.to_vec())
}
pub(super) fn write(index: Option<u32>, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property: P::TAGS,
        array_index: index,
        value,
    }
}
pub(super) fn whole(name: &str) -> PendingWrite {
    write(None, framed(&tags(name)))
}
pub(super) fn state(store: &Arc<Memory>) -> ProfileState {
    let mut state = ProfileState::persistent(oid(), store.clone()).unwrap();
    state.provision(profile("configured")).unwrap();
    state
}
pub(super) fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected stage: {other:?}"),
    }
}
pub(super) fn apply(state: &mut ProfileState, write: &PendingWrite) -> Result<(), Error> {
    state
        .write(write.property, write.array_index, &write.value)
        .unwrap()
}
pub(super) fn assert_code<T>(result: Result<T, Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { code, .. }) if code == u32::from(expected.to_raw()))
    );
}
