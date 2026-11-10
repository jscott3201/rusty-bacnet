//! What the tests of the COV-multiple path share: a database of Analog
//! Values with a hand-set Device clock, one subscriber, its requests, and
//! instants set by hand on the host's timeline.
use std::sync::{Arc, Mutex};

use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_time::{Duration, Instant};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use super::timed::{TimedChange, TimedClaim, TimedHistories};
use super::{
    CovRecipient, CovSubscriptionKey, CovSubscriptionSnapshot, CovTable, MultipleContextKey,
    MultipleRefusal, SubscriberEndpoint, subscribe_cov_property_multiple,
};

pub(super) const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
pub(super) const FLAGS: PropertyIdentifier = PropertyIdentifier::STATUS_FLAGS;

/// The instant the tests begin at.
pub(super) const T0: Instant = Instant::from_secs(1_000);

/// `seconds` after [`T0`].
pub(super) fn at(seconds: u64) -> Instant {
    T0 + Duration::from_secs(seconds)
}

/// A Device clock a test sets by hand.
pub(super) struct HandClock(Mutex<Option<ClockFrame>>);

impl HandClock {
    /// A clock showing [`frame`]`(second)`.
    pub(super) fn showing(second: u8) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(frame(second)))))
    }

    pub(super) fn set(&self, frame: Option<ClockFrame>) {
        *self.0.lock().unwrap() = frame;
    }
}

impl ClockReader for HandClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        *self.0.lock().unwrap()
    }
}

/// A valid Device time, `second` past 14:00 on a Tuesday.
pub(super) fn frame(second: u8) -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 9,
            day: 29,
            day_of_week: 2,
        },
        local_time: Time {
            hour: 14,
            minute: 0,
            second,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

pub(super) fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

/// Analog Values 1 to `count`, each at zero, read by `clock`.
pub(super) fn database(clock: Option<&Arc<HandClock>>, count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    for instance in 1..=count {
        let object = AnalogValueObject::with_access(
            instance,
            format!("AV-{instance}"),
            EngineeringUnits::NO_UNITS,
            PresentValueAccess::Writable,
        )
        .unwrap();
        db.add(Box::new(object)).unwrap();
    }
    db.set_clock_reader(clock.map(|clock| Arc::clone(clock) as Arc<dyn ClockReader>));
    db
}

/// Write Analog Value `instance`'s Present_Value, as a write request does.
pub(super) fn set(db: &mut ObjectDatabase, instance: u32, value: f32) {
    let target = crate::write::WriteTarget {
        oid: av(instance),
        property: PV,
        array_index: None,
        priority: None,
        value: &[],
    };
    crate::write::commit_attempt(db, None, target, PropertyValue::Real(value), None, None).unwrap();
}

pub(super) fn peer() -> SubscriberEndpoint {
    SubscriberEndpoint::new(&[10, 0, 0, 5, 0xBA, 0xC0], None)
}

/// The unconfirmed context of [`peer`]'s process `process`.
pub(super) fn context(process: u32) -> MultipleContextKey {
    let peer = peer();
    MultipleContextKey {
        recipient: CovRecipient::from_endpoint(&peer.mac, peer.network.as_ref()),
        process_id: process,
        confirmed: false,
    }
}

pub(super) fn key(process: u32, instance: u32, property: PropertyIdentifier) -> CovSubscriptionKey {
    CovSubscriptionKey::Multiple {
        context: context(process),
        object: av(instance),
        property,
        index: None,
    }
}

pub(super) fn reference(property: PropertyIdentifier, timestamped: bool) -> COVReference {
    COVReference {
        monitored_property: PropertyReference {
            property_identifier: property,
            property_array_index: None,
        },
        cov_increment: None,
        timestamped,
    }
}

/// An unconfirmed request of process `process` naming, for each Analog
/// Value instance, its references.
pub(super) fn request(
    process: u32,
    lifetime: Option<u32>,
    delay: Option<u32>,
    objects: &[(u32, &[COVReference])],
) -> SubscribeCOVPropertyMultipleRequest {
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: process,
        issue_confirmed_notifications: false,
        lifetime,
        max_notification_delay: delay,
        list_of_cov_subscription_specifications: objects
            .iter()
            .map(|(instance, references)| COVSubscriptionSpecification {
                monitored_object_identifier: av(*instance),
                list_of_cov_references: references.to_vec(),
            })
            .collect(),
    }
}

/// The request from [`peer`], at `now`.
pub(super) fn subscribe(
    table: &mut CovTable,
    histories: &mut TimedHistories,
    db: &ObjectDatabase,
    request: SubscribeCOVPropertyMultipleRequest,
    now: Instant,
) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
    subscribe_cov_property_multiple(table, histories, db, &peer(), None, request, now)
}

/// Take a reference's pending changes, oldest first, and retire them as a
/// delivered notification would.
pub(super) fn deliver(
    histories: &mut TimedHistories,
    snapshot: &CovSubscriptionSnapshot,
) -> Vec<TimedChange> {
    let (incarnation, changes) = histories.drain(snapshot.key(), snapshot.generation());
    let mut claim = TimedClaim::new();
    claim.add(snapshot.key().clone(), incarnation, changes.clone());
    histories.commit(claim);
    changes
}

/// Each change's Device second and the REAL its Present_Value carries.
pub(super) fn seconds_and_values(changes: &[TimedChange]) -> Vec<(u8, f32)> {
    changes
        .iter()
        .map(|change| {
            let value = change
                .values()
                .iter()
                .find(|value| value.property_identifier == PV)
                .expect("a Present_Value");
            // An application-tagged REAL: its tag, then four octets.
            let octets: [u8; 4] = value.value[1..5].try_into().unwrap();
            (change.frame().local_time.second, f32::from_be_bytes(octets))
        })
        .collect()
}
