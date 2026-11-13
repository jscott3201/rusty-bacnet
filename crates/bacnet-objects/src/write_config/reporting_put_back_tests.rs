//! Where storage goes when a kept-row write made where it is supersedes
//! another request's staged write and its own save fails (#2526). The
//! write is worked out on the rows served, taken rather than copied, so the
//! rows storage goes back to are worked out again from the object, which
//! hasn't made the write yet.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::reporting_tests::refused_with;
use super::{ReportingWriteConfig, WriteConfigPersistence, WriteConfigSnapshot};
use crate::analog::AnalogInputObject;
use crate::durable::{PendingWrite, StageStep};
use crate::traits::BACnetObject;

/// Storage that takes `allowed` more saves and refuses the rest.
struct Rationed {
    saved: Mutex<Option<WriteConfigSnapshot>>,
    allowed: AtomicUsize,
}

impl WriteConfigPersistence for Rationed {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
        Ok(self.saved.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error> {
        let left = self.allowed.load(Ordering::SeqCst);
        if left == 0 {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        self.allowed.store(left - 1, Ordering::SeqCst);
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

#[test]
fn a_superseding_write_whose_save_fails_leaves_storage_with_the_served_rows() {
    let storage = Arc::new(Rationed {
        saved: Mutex::new(None),
        allowed: AtomicUsize::new(usize::MAX),
    });
    let mut analog = AnalogInputObject::new(1, "AI", EngineeringUnits::NO_UNITS)
        .unwrap()
        .with_reporting_persistence(Arc::clone(&storage) as Arc<dyn WriteConfigPersistence>)
        .unwrap();
    let staged = PendingWrite {
        property: P::TIME_DELAY,
        array_index: None,
        value: PropertyValue::Unsigned(5),
    };
    let StageStep::Staged(wait) = analog
        .durable_writes()
        .unwrap()
        .stage_writes(core::slice::from_ref(&staged))
    else {
        panic!("the delay stages");
    };
    wait.block();
    // The put-back save lands; the superseding write's own save doesn't.
    storage.allowed.store(1, Ordering::SeqCst);
    let class = PropertyValue::Unsigned(7);
    let refused = analog.write_property(P::NOTIFICATION_CLASS, None, class, None);
    assert!(refused_with(
        &refused,
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM
    ));
    analog.wait_for_reporting_saves();
    let nothing_written = WriteConfigSnapshot::Reporting(ReportingWriteConfig::default());
    assert_eq!(*storage.saved.lock().unwrap(), Some(nothing_written));
    analog.durable_writes().unwrap().release_staged_write(&wait);
}
