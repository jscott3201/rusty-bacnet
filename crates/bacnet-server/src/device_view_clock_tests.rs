//! Availability is a lazy service-profile snapshot, not a snapshot of time.
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_types::bitstring::ServicesSupported;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::{Date, Time};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

struct CountingClock {
    reads: AtomicUsize,
    frame: Mutex<Option<ClockFrame>>,
}
impl CountingClock {
    fn new(available: bool) -> Arc<Self> {
        Arc::new(Self {
            reads: AtomicUsize::new(0),
            frame: Mutex::new(available.then_some(frame())),
        })
    }
    fn set(&self, available: bool) {
        *self.frame.lock().unwrap() = available.then_some(frame());
    }
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}
impl ClockReader for CountingClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        *self.frame.lock().unwrap()
    }
}
fn frame() -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 10,
            day: 17,
            day_of_week: 6,
        },
        local_time: Time {
            hour: 12,
            minute: 30,
            second: 15,
            hundredths: 25,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    }
}
fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap()
}
fn database(clock: Option<Arc<dyn ClockReader>>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(77, 0)
                .unwrap()
                .with_name("clock test"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.set_clock_reader(clock);
    db
}
fn supported(value: PropertyValue) -> ServicesSupported {
    let PropertyValue::BitString { unused_bits, data } = value else {
        panic!("expected supported-service bits");
    };
    assert_eq!((unused_bits, data.len()), (7, 7));
    ServicesSupported::from_bacnet(&data)
}
fn assert_clock_services(value: PropertyValue, expected: bool) {
    let services = supported(value);
    assert!(services.contains(ServiceSupported::READ_PROPERTY));
    for service in [
        ServiceSupported::TIME_SYNCHRONIZATION,
        ServiceSupported::UTC_TIME_SYNCHRONIZATION,
    ] {
        assert_eq!(services.contains(service), expected);
    }
}
fn services(context: &DeviceReadContext<'_>, db: &ObjectDatabase) -> PropertyValue {
    context
        .object(db.get(&device()).unwrap())
        .read_property(P::PROTOCOL_SERVICES_SUPPORTED, None)
        .unwrap()
}

#[test]
fn context_and_ordinary_reads_do_not_sample_the_clock() {
    let clock = CountingClock::new(true);
    let db = database(Some(clock.clone()));
    let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    assert_eq!(clock.reads(), 0, "construction does not need availability");
    let analog = AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    context
        .object(&analog)
        .read_property(P::PRESENT_VALUE, None)
        .unwrap();
    context
        .object(db.get(&device()).unwrap())
        .read_property(P::OBJECT_NAME, None)
        .unwrap();
    assert_eq!(
        clock.reads(),
        0,
        "ordinary properties do not need availability"
    );
}

#[test]
fn endpoint_services_do_not_sample_the_clock() {
    let clock = CountingClock::new(true);
    let db = database(Some(clock.clone()));
    for writes in [false, true] {
        for reinitialize in [false, true] {
            let context = DeviceReadContext::new(
                &db,
                DeviceExecution::Endpoint {
                    writes,
                    reinitialize,
                },
            );
            let value = services(&context, &db);
            assert_clock_services(value.clone(), false);
            let supported = supported(value);
            assert_eq!(supported.contains(ServiceSupported::WRITE_PROPERTY), writes);
            assert_eq!(
                supported.contains(ServiceSupported::REINITIALIZE_DEVICE),
                reinitialize
            );
        }
    }
    assert_eq!(clock.reads(), 0);
}

#[test]
fn service_availability_is_lazy_and_cached_through_live_attachment() {
    for available in [false, true] {
        let clock = CountingClock::new(!available);
        let db = database(Some(clock.clone()));
        let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
        clock.set(available);
        assert_clock_services(services(&context, &db), available);
        assert_eq!(clock.reads(), 1);
        clock.set(!available);
        assert_clock_services(services(&context, &db), available);
        let context = context.with_live(None);
        assert_clock_services(services(&context, &db), available);
        assert_eq!(
            clock.reads(),
            1,
            "cache both availability and unavailability"
        );
        let next = DeviceReadContext::new(&db, DeviceExecution::FullServer);
        assert_clock_services(services(&next, &db), !available);
        assert_eq!(
            clock.reads(),
            2,
            "a later request observes recovery or loss"
        );
    }
}

#[test]
fn later_requests_observe_missing_and_rebound_readers() {
    let mut db = database(None);
    let read = |db: &ObjectDatabase| {
        services(&DeviceReadContext::new(db, DeviceExecution::FullServer), db)
    };
    assert_clock_services(read(&db), false);
    let first = CountingClock::new(false);
    db.set_clock_reader(Some(first.clone()));
    assert_clock_services(read(&db), false);
    first.set(true);
    assert_clock_services(read(&db), true);
    let second = CountingClock::new(true);
    db.set_clock_reader(Some(second.clone()));
    assert_clock_services(read(&db), true);
    db.set_clock_reader(None);
    assert_clock_services(read(&db), false);
    assert_eq!(first.reads(), 2);
    assert_eq!(second.reads(), 1);
}

#[test]
fn rpm_repeated_service_rows_share_one_availability_sample() {
    use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
    use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
    for available in [false, true] {
        let clock = CountingClock::new(available);
        let db = database(Some(clock.clone()));
        let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
        let request = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: device(),
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: P::PROTOCOL_SERVICES_SUPPORTED,
                        property_array_index: None,
                    };
                    3
                ],
            }],
        };
        let plan = crate::handlers::RpmPlan::new(&db, &request, 3, Some(&context)).unwrap();
        assert_eq!(clock.reads(), 0, "explicit-row planning needs no clock");
        let context = context.with_live(None);
        let mut out = bytes::BytesMut::new();
        let mut observed = 0;
        plan.read_observed(&db, Some(&context), &mut out, 4096, |_, _, _, _| {
            observed += 1;
            clock.set(!available);
        })
        .unwrap();
        assert_eq!(observed, 3);
        assert_eq!(clock.reads(), 1);
        let ack = ReadPropertyMultipleACK::decode(&out).unwrap();
        let rows = &ack.list_of_read_access_results[0].list_of_results;
        assert_eq!(rows.len(), 3);
        for row in rows {
            let bytes = row.property_value.as_ref().unwrap();
            let (value, end) =
                bacnet_encoding::primitives::decode_application_value(bytes, 0).unwrap();
            assert_eq!(end, bytes.len());
            assert_clock_services(value, available);
        }
    }
}

#[test]
fn cached_service_availability_does_not_freeze_time_or_metadata() {
    let clock = CountingClock::new(true);
    let db = database(Some(clock.clone()));
    let context = DeviceReadContext::new(&db, DeviceExecution::FullServer);
    assert_clock_services(services(&context, &db), true);
    let view = context.object(db.get(&device()).unwrap());
    assert_eq!(
        view.read_property(P::LOCAL_TIME, None).unwrap(),
        PropertyValue::Time(frame().local_time)
    );
    let mut changed = frame();
    changed.local_time.second += 1;
    *clock.frame.lock().unwrap() = Some(changed);
    assert_eq!(
        view.read_property(P::LOCAL_TIME, None).unwrap(),
        PropertyValue::Time(changed.local_time)
    );
    clock.set(false);
    assert!(!view.property_list().contains(&P::LOCAL_TIME));
    assert!(view.read_property(P::LOCAL_TIME, None).is_err());
    assert_clock_services(services(&context, &db), true);
    clock.set(true);
    assert!(view.property_list().contains(&P::LOCAL_TIME));
    assert!(view.read_property(P::LOCAL_TIME, None).is_ok());
}
