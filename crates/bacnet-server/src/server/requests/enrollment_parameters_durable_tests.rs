//! The event algorithm parameters a peer writes to an Event Enrollment
//! keeping them in storage (Event_Parameters, Fault_Parameters and
//! Time_Delay_Normal) save through the server (#2331): each WriteProperty
//! stages one save off the database guard, and a server rebuilt on the
//! storage serves what was written and evaluates with it. An event only the
//! restored parameters raise is raised, counted in passes of ten seconds
//! each made by hand. A WritePropertyMultiple of two parameters and a
//! reporting setting saves once, a write that cannot be saved is refused on
//! the wire and the old parameters go on evaluating, and a value past what
//! storage keeps is refused before anything is saved.
//!
//! DEVICE 0, RESOURCES 3; NO_SPACE_TO_WRITE_PROPERTY 20,
//! OPERATIONAL_PROBLEM 25.

use super::durable_write_wire_tests::{
    ERROR_PDU, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::write_config_durable_tests::ConfigStorage;
use super::*;
use crate::event_enrollment::evaluate_event_enrollments;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::write_config::WriteConfigPersistence;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    FaultParameters,
};
use bacnet_types::enums::{EngineeringUnits, EventState, EventType, Reliability};
use bytes::BytesMut;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

/// The enrollment's instance.
const ENROLLMENT: u32 = 5;
/// The seconds one evaluation pass stands for.
const PASS: u64 = 10;

fn enrollment() -> ObjectIdentifier {
    oid(ObjectType::EVENT_ENROLLMENT, ENROLLMENT)
}

/// Analog Input 9, which the fixture gains with a Present_Value of 60.
fn analog() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 9)
}

/// Binary Input 9, which the fixture gains ACTIVE.
fn binary() -> ObjectIdentifier {
    oid(ObjectType::BINARY_INPUT, 9)
}

/// What the application configures at each start for an enrollment on
/// Analog Input 9: OUT_OF_RANGE above 100, with no delay.
fn wide_range() -> BACnetEventParameter {
    BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 0.0,
        high_limit: 100.0,
        deadband: 0.0,
    }
}

/// What a peer writes over it: above 50, once it has held for 20 seconds.
fn narrow_range() -> BACnetEventParameter {
    BACnetEventParameter::OutOfRange {
        time_delay: 20,
        low_limit: 0.0,
        high_limit: 50.0,
        deadband: 0.0,
    }
}

/// CHANGE_OF_STATE on `states`, with no delay.
fn listed(states: Vec<BACnetPropertyStates>) -> BACnetEventParameter {
    BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: states,
    }
}

fn range_fault() -> FaultParameters {
    FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 55.0,
    }
}

fn event_octets(parameters: &BACnetEventParameter) -> Vec<u8> {
    let mut octets = BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut octets, parameters).unwrap();
    octets.to_vec()
}

fn fault_octets(parameters: &FaultParameters) -> Vec<u8> {
    let mut octets = BytesMut::new();
    bacnet_encoding::constructed::encode_fault_parameters(&mut octets, parameters).unwrap();
    octets.to_vec()
}

/// The fixture with Analog Input 9 at 60 and Binary Input 9 ACTIVE, and
/// the enrollment as the application builds it at each start: monitoring
/// `source`'s Present_Value with `configured`, and keeping its written
/// settings in `storage`, if any.
async fn served(
    source: ObjectIdentifier,
    configured: BACnetEventParameter,
    storage: Option<&Arc<ConfigStorage>>,
) -> Arc<Fixture> {
    let fixture = Fixture::new(None);
    {
        let mut db = fixture.db.write().await;
        let mut input = AnalogInputObject::new(9, "AI", EngineeringUnits::NO_UNITS).unwrap();
        input.set_present_value(60.0);
        db.add(Box::new(input)).unwrap();
        let mut input = BinaryInputObject::new(9, "BI").unwrap();
        input.set_present_value(1);
        db.add(Box::new(input)).unwrap();
        let mut object =
            EventEnrollmentObject::new(ENROLLMENT, "EE", EventType::OUT_OF_RANGE).unwrap();
        let reference =
            BACnetDeviceObjectPropertyReference::new_local(source, P::PRESENT_VALUE.to_raw());
        object
            .set_object_property_reference(Some(reference))
            .unwrap();
        object.set_event_parameters(configured);
        let object = match storage {
            Some(storage) => object
                .with_reporting_persistence(Arc::clone(storage) as Arc<dyn WriteConfigPersistence>)
                .unwrap(),
            None => object,
        };
        db.add(Box::new(object)).unwrap();
    }
    Arc::new(fixture)
}

/// Give Analog Input 9 the Present_Value `value`, out of service as an
/// input takes one.
async fn set_analog(fixture: &Fixture, value: f32) {
    let mut db = fixture.db.write().await;
    let input = db.get_mut(&analog()).unwrap();
    for (property, value) in [
        (P::OUT_OF_SERVICE, PropertyValue::Boolean(true)),
        (P::PRESENT_VALUE, PropertyValue::Real(value)),
    ] {
        input.write_property(property, None, value, None).unwrap();
    }
}

/// One evaluation pass, ten seconds on: the Event_State the enrollment
/// went to, if it made a transition.
async fn pass(fixture: &Fixture) -> Option<EventState> {
    let mut db = fixture.db.write().await;
    let transitions = evaluate_event_enrollments(&mut db, PASS);
    assert!(transitions.len() <= 1, "{transitions:?}");
    transitions.first().map(|transition| transition.change.to)
}

fn write_request(property: P, octets: Vec<u8>) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: enrollment(),
        property_identifier: property,
        property_array_index: None,
        property_value: octets,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

/// A WriteProperty of `property` to the enrollment, and its answer.
async fn write(fixture: &Fixture, property: P, octets: Vec<u8>) -> Vec<u8> {
    let request = write_request(property, octets);
    wire(fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await
}

/// The Error PDU a WriteProperty with invoke ID 5 is refused with.
fn refusal(class: u8, code: u8) -> [u8; 7] {
    [ERROR_PDU, 5, 15, 0x91, class, 0x91, code]
}

/// The three rows with a value a write gives each, as its octets on the
/// wire and as the enrollment serves it afterwards.
fn rows() -> [(P, Vec<u8>, PropertyValue); 3] {
    let event = event_octets(&narrow_range());
    let fault = fault_octets(&range_fault());
    [
        (
            P::EVENT_PARAMETERS,
            event.clone(),
            PropertyValue::ApplicationData(event),
        ),
        (
            P::FAULT_PARAMETERS,
            fault.clone(),
            PropertyValue::ApplicationData(fault),
        ),
        (
            P::TIME_DELAY_NORMAL,
            value(PropertyValue::Unsigned(30)),
            PropertyValue::Unsigned(30),
        ),
    ]
}

/// The three rows are the ones the object crate keeps for an Event
/// Enrollment; Time_Delay, which the type carries inside Event_Parameters,
/// is no row of it, and an Alert Enrollment keeps none.
#[test]
fn the_covered_rows_are_the_ones_an_enrollment_keeps() {
    use bacnet_objects::snapshot::write_config::keeps_parameter;
    let kind = ObjectType::EVENT_ENROLLMENT;
    for (property, _, _) in rows() {
        assert!(keeps_parameter(kind, property), "{property:?}");
        assert!(!keeps_parameter(ObjectType::ALERT_ENROLLMENT, property));
    }
    for property in [
        P::TIME_DELAY,
        P::EVENT_TYPE,
        P::FAULT_TYPE,
        P::OBJECT_PROPERTY_REFERENCE,
    ] {
        assert!(!keeps_parameter(kind, property), "{property:?}");
    }
}

/// Each row's WriteProperty takes its staged save, without saving again,
/// and a server rebuilt on the storage serves what it wrote over what the
/// application configures.
#[tokio::test(start_paused = true)]
async fn each_enrollment_parameter_write_survives_a_rebuild() {
    for (property, octets, written) in rows() {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served(analog(), wide_range(), Some(&storage)).await;
        assert_ne!(fixture.read(enrollment(), property).await, written);
        let response = write(&fixture, property, octets).await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{property:?}");
        assert_eq!(fixture.read(enrollment(), property).await, written);
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{property:?}");
        drop(fixture);
        let rebuilt = served(analog(), wide_range(), Some(&storage)).await;
        assert_eq!(
            rebuilt.read(enrollment(), property).await,
            written,
            "{property:?} after a rebuild"
        );
    }
}

/// The save runs with the database guard dropped: the database answers a
/// reader and a writer while each row's save is held. A row the server
/// didn't stage would save in place, under the guard.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enrollment_parameter_save_runs_while_the_database_stays_available() {
    for (property, octets, _) in rows() {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served(analog(), wide_range(), Some(&storage)).await;
        let (response, available) = while_saving(
            &fixture,
            &storage,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            write_request(property, octets),
            WAIT,
        )
        .await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{property:?}");
        assert!(
            available,
            "the database was held while the enrollment saved {property:?}"
        );
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{property:?}");
    }
}

/// Written OUT_OF_RANGE parameters, restored, are what the pass evaluates:
/// 60 is inside the configured range and outside the written one, whose
/// Time_Delay of 20 seconds runs for two passes before HIGH_LIMIT. Back at
/// 10, the restored Time_Delay_Normal of 30 seconds holds the return for
/// three passes, one more than the parameters' own delay would. A server
/// on the configured parameters alone raises nothing.
#[tokio::test(start_paused = true)]
async fn a_rebuilt_enrollment_evaluates_with_the_restored_range_and_delays() {
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served(analog(), wide_range(), Some(&storage)).await;
    let [(_, event, _), _, (_, delay, _)] = rows();
    assert_eq!(
        write(&fixture, P::EVENT_PARAMETERS, event).await,
        SIMPLE_ACK_WRITE
    );
    assert_eq!(
        write(&fixture, P::TIME_DELAY_NORMAL, delay).await,
        SIMPLE_ACK_WRITE
    );
    drop(fixture);

    let rebuilt = served(analog(), wide_range(), Some(&storage)).await;
    assert_eq!(pass(&rebuilt).await, None, "the delay is seeded");
    assert_eq!(pass(&rebuilt).await, None, "ten seconds of twenty");
    assert_eq!(pass(&rebuilt).await, Some(EventState::HIGH_LIMIT));
    set_analog(&rebuilt, 10.0).await;
    for seconds in [0, 10, 20] {
        assert_eq!(pass(&rebuilt).await, None, "{seconds} s of thirty");
    }
    assert_eq!(pass(&rebuilt).await, Some(EventState::NORMAL));

    let configured = served(analog(), wide_range(), None).await;
    for _ in 0..4 {
        assert_eq!(pass(&configured).await, None);
    }
}

/// Written CHANGE_OF_STATE parameters, whose list a restore has to bring
/// back whole, are what the pass evaluates: the configured list is empty
/// and raises nothing, and the written one names ACTIVE second.
#[tokio::test(start_paused = true)]
async fn a_rebuilt_enrollment_evaluates_with_the_restored_list_of_states() {
    let configured = || listed(Vec::new());
    let written = listed(vec![
        BACnetPropertyStates::State(7),
        BACnetPropertyStates::BinaryValue(1),
    ]);
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served(binary(), configured(), Some(&storage)).await;
    assert_eq!(pass(&fixture).await, None, "the configured list");
    let response = write(&fixture, P::EVENT_PARAMETERS, event_octets(&written)).await;
    assert_eq!(response, SIMPLE_ACK_WRITE);
    drop(fixture);

    let rebuilt = served(binary(), configured(), Some(&storage)).await;
    let event_type = rebuilt.read(enrollment(), P::EVENT_TYPE).await;
    assert_eq!(
        event_type,
        PropertyValue::Enumerated(EventType::CHANGE_OF_STATE.to_raw())
    );
    assert_eq!(pass(&rebuilt).await, Some(EventState::OFFNORMAL));

    let plain = served(binary(), configured(), None).await;
    assert_eq!(pass(&plain).await, None);
}

/// Written Fault_Parameters, restored, are what the pass evaluates: 60 is
/// past the written bound of 55, so the rebuilt enrollment goes to FAULT
/// with OVER_RANGE, where the configured one, with no fault algorithm,
/// stays NORMAL.
#[tokio::test(start_paused = true)]
async fn a_rebuilt_enrollment_evaluates_with_the_restored_fault_parameters() {
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served(analog(), wide_range(), Some(&storage)).await;
    let octets = fault_octets(&range_fault());
    assert_eq!(
        write(&fixture, P::FAULT_PARAMETERS, octets).await,
        SIMPLE_ACK_WRITE
    );
    drop(fixture);

    let reliability = |value: Reliability| PropertyValue::Enumerated(value.to_raw());
    let state = |value: EventState| PropertyValue::Enumerated(value.to_raw());
    let rebuilt = served(analog(), wide_range(), Some(&storage)).await;
    pass(&rebuilt).await;
    assert_eq!(
        rebuilt.read(enrollment(), P::RELIABILITY).await,
        reliability(Reliability::OVER_RANGE)
    );
    assert_eq!(
        rebuilt.read(enrollment(), P::EVENT_STATE).await,
        state(EventState::FAULT)
    );

    let configured = served(analog(), wide_range(), None).await;
    pass(&configured).await;
    assert_eq!(
        configured.read(enrollment(), P::RELIABILITY).await,
        reliability(Reliability::NO_FAULT_DETECTED)
    );
    assert_eq!(
        configured.read(enrollment(), P::EVENT_STATE).await,
        state(EventState::NORMAL)
    );
}

/// The rebuilt enrollment's Object_Property_Reference isn't kept and may
/// name a property the saved parameters don't suit. No write of them is
/// checked against it, so the enrollment opens and shows what any
/// enrollment configured that way shows: an event algorithm whose input
/// has another datatype raises nothing and reports no fault, and
/// FAULT_OUT_OF_RANGE over a value that isn't a number is
/// CONFIGURATION_ERROR and FAULT.
#[tokio::test(start_paused = true)]
async fn saved_parameters_the_rebuilt_reference_does_not_suit_show_as_misconfigured() {
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served(binary(), listed(Vec::new()), Some(&storage)).await;
    let states = listed(vec![BACnetPropertyStates::BinaryValue(1)]);
    assert_eq!(
        write(&fixture, P::EVENT_PARAMETERS, event_octets(&states)).await,
        SIMPLE_ACK_WRITE
    );
    drop(fixture);

    // The states over an analog value: nothing, and no fault.
    let rebuilt = served(analog(), wide_range(), Some(&storage)).await;
    assert_eq!(
        rebuilt.read(enrollment(), P::EVENT_PARAMETERS).await,
        PropertyValue::ApplicationData(event_octets(&states))
    );
    for _ in 0..3 {
        assert_eq!(pass(&rebuilt).await, None);
    }
    assert_eq!(
        rebuilt.read(enrollment(), P::RELIABILITY).await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(
        write(&rebuilt, P::FAULT_PARAMETERS, fault_octets(&range_fault())).await,
        SIMPLE_ACK_WRITE
    );
    drop(rebuilt);

    // The numeric bounds over a binary value.
    let rebuilt = served(binary(), listed(Vec::new()), Some(&storage)).await;
    pass(&rebuilt).await;
    assert_eq!(
        rebuilt.read(enrollment(), P::RELIABILITY).await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    assert_eq!(
        rebuilt.read(enrollment(), P::EVENT_STATE).await,
        PropertyValue::Enumerated(EventState::FAULT.to_raw())
    );
}

fn attempt(property: P, octets: Vec<u8>) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: octets,
        priority: None,
    }
}

/// A WritePropertyMultiple of Event_Parameters, Time_Delay_Normal and
/// Notification_Class.
fn joint_request(event: Vec<u8>) -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: enrollment(),
        list_of_properties: vec![
            attempt(P::EVENT_PARAMETERS, event),
            attempt(P::TIME_DELAY_NORMAL, value(PropertyValue::Unsigned(30))),
            attempt(P::NOTIFICATION_CLASS, value(PropertyValue::Unsigned(12))),
        ],
    }])
}

/// What the joint request's three rows serve after it.
fn joint_rows() -> [(P, PropertyValue); 3] {
    let event = PropertyValue::ApplicationData(event_octets(&narrow_range()));
    [
        (P::EVENT_PARAMETERS, event),
        (P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(30)),
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(12)),
    ]
}

/// One WritePropertyMultiple of two parameters and a reporting setting
/// saves once, and a rebuild serves all three.
#[tokio::test(start_paused = true)]
async fn a_request_of_two_parameters_and_a_reporting_setting_saves_once() {
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served(analog(), wide_range(), Some(&storage)).await;
    let request = joint_request(event_octets(&narrow_range()));
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    assert_eq!(wire(&fixture, service, request).await, SIMPLE_ACK_WPM);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    drop(fixture);
    let rebuilt = served(analog(), wide_range(), Some(&storage)).await;
    for (property, written) in joint_rows() {
        assert_eq!(
            rebuilt.read(enrollment(), property).await,
            written,
            "{property:?}"
        );
    }
}

/// When the save fails, each row's WriteProperty and the joint request are
/// answered with DEVICE / OPERATIONAL_PROBLEM, the old values are still
/// served, and the pass goes on evaluating with them: 60 stays inside the
/// configured range and no fault algorithm runs.
#[tokio::test(start_paused = true)]
async fn a_parameter_that_cannot_be_saved_is_refused_and_the_old_one_evaluates() {
    let storage = Arc::new(ConfigStorage::default());
    storage.fail.store(true, Ordering::SeqCst);
    let fixture = served(analog(), wide_range(), Some(&storage)).await;
    let mut before = Vec::new();
    for (property, _) in joint_rows() {
        before.push((property, fixture.read(enrollment(), property).await));
    }
    for (property, octets, _) in rows() {
        let old = fixture.read(enrollment(), property).await;
        let response = write(&fixture, property, octets).await;
        assert_eq!(response, refusal(0, 25), "{property:?}");
        assert_eq!(fixture.read(enrollment(), property).await, old);
    }
    let request = joint_request(event_octets(&narrow_range()));
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    let response = wire(&fixture, service, request).await;
    // WritePropertyMultiple's error: the same class and code, then the
    // first attempt, which is the one refused.
    assert_eq!(response[..8], [ERROR_PDU, 5, 16, 0x0E, 0x91, 0, 0x91, 25]);
    for (property, old) in before {
        assert_eq!(fixture.read(enrollment(), property).await, old);
    }
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    assert_eq!(storage.load_saved(), None);

    for _ in 0..4 {
        assert_eq!(pass(&fixture).await, None);
    }
    assert_eq!(
        fixture.read(enrollment(), P::RELIABILITY).await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(
        fixture.read(enrollment(), P::EVENT_STATE).await,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

/// Event_Parameters in the legacy octet layout whose encoding is `octets`
/// long: a tag, a three-octet length and the data.
fn legacy(octets: usize) -> Vec<u8> {
    value(PropertyValue::OctetString(vec![0x5A; octets - 4]))
}

/// Parameters whose encoding is one octet past the 64 KiB storage keeps
/// are refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY before anything
/// is saved, alone and at the head of a WritePropertyMultiple, whose later
/// rows are then not saved ahead of it; at the cap they are taken. A
/// server whose enrollment has no storage takes the longer value.
#[tokio::test(start_paused = true)]
async fn parameters_past_the_cap_are_refused_before_any_save() {
    const CAP: usize = 64 * 1024;
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served(analog(), wide_range(), Some(&storage)).await;
    let before = fixture.read(enrollment(), P::EVENT_PARAMETERS).await;
    let response = write(&fixture, P::EVENT_PARAMETERS, legacy(CAP + 1)).await;
    assert_eq!(response, refusal(3, 20));
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    let response = wire(&fixture, service, joint_request(legacy(CAP + 1))).await;
    assert_eq!(response[..8], [ERROR_PDU, 5, 16, 0x0E, 0x91, 3, 0x91, 20]);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    assert_eq!(storage.load_saved(), None);
    assert_eq!(
        fixture.read(enrollment(), P::EVENT_PARAMETERS).await,
        before
    );
    assert_eq!(
        fixture.read(enrollment(), P::NOTIFICATION_CLASS).await,
        PropertyValue::Unsigned(0)
    );

    let response = write(&fixture, P::EVENT_PARAMETERS, legacy(CAP)).await;
    assert_eq!(response, SIMPLE_ACK_WRITE);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);

    let plain = served(analog(), wide_range(), None).await;
    let response = write(&plain, P::EVENT_PARAMETERS, legacy(CAP + 1)).await;
    assert_eq!(response, SIMPLE_ACK_WRITE);
}
