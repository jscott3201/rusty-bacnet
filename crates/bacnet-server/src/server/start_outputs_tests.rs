//! A start that doesn't finish releases what its outputs began (#2479),
//! before and after the server shares the database.

use super::*;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::load_control::{
    LoadControlObject, LoadShedder, ShedProgress, ShedRequest, ShedStatus,
};
use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// A shedder that complies at once and counts its stops.
#[derive(Default)]
struct Stops(AtomicUsize);

impl LoadShedder for Stops {
    fn shed(&self, request: &ShedRequest) -> ShedStatus {
        let level = request.requested.clone();
        ShedStatus::new(ShedProgress::Compliant, level.clone(), level)
    }

    fn stop(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Stops {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// A Device clock fixed at 10:00 on 2026-11-12.
struct TenOClock;

const DAY: Date = Date {
    year: 126,
    month: 11,
    day: 12,
    day_of_week: 4,
};
const TEN: Time = Time {
    hour: 10,
    minute: 0,
    second: 0,
    hundredths: 0,
};

impl ClockReader for TenOClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: DAY,
            local_time: TEN,
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

/// A database whose LC-1 sheds for `stops` from 10:00, so ending its
/// request tells `stops`.
fn shedding(stops: &Arc<Stops>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(TenOClock)));
    let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
    lc.set_shedder(stops.clone());
    lc.set_requested_shed_level(BACnetShedLevel::Level(2))
        .unwrap();
    lc.set_shed_duration(30);
    db.add(Box::new(lc)).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 1).unwrap();
    let start = PropertyValue::List(vec![PropertyValue::Date(DAY), PropertyValue::Time(TEN)]);
    db.get_mut(&oid)
        .unwrap()
        .write_property(PropertyIdentifier::START_TIME, None, start, None)
        .unwrap();
    db
}

#[tokio::test]
async fn a_start_that_completes_releases_nothing() {
    let stops = Arc::new(Stops::default());
    let mut outputs = StartedOutputs::start(shedding(&stops));
    let db = outputs.share();
    outputs.complete();
    assert_eq!(stops.count(), 0);
    drop(db);
}

#[tokio::test]
async fn a_start_dropped_before_sharing_releases_the_outputs() {
    let stops = Arc::new(Stops::default());
    drop(StartedOutputs::start(shedding(&stops)));
    assert_eq!(stops.count(), 1);
}

/// The second failure path: the database shared, its guard free.
#[tokio::test]
async fn a_start_dropped_after_sharing_releases_the_outputs() {
    let stops = Arc::new(Stops::default());
    let mut outputs = StartedOutputs::start(shedding(&stops));
    let db = outputs.share();
    drop(outputs);
    assert_eq!(stops.count(), 1);
    drop(db);
}

/// Shared and locked: the drop neither waits for the guard nor takes it
/// from its holder; a task releases the outputs once the guard is free.
#[tokio::test]
async fn a_release_behind_a_held_guard_waits_for_it_in_a_task() {
    let stops = Arc::new(Stops::default());
    let mut outputs = StartedOutputs::start(shedding(&stops));
    let db = outputs.share();
    let held = db.write().await;
    drop(outputs);
    assert_eq!(stops.count(), 0);
    drop(held);
    tokio::time::timeout(Duration::from_secs(5), async {
        while stops.count() == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the task released the outputs");
    assert_eq!(stops.count(), 1);
}
