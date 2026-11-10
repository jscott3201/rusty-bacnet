//! The event algorithm parameters a peer writes to an analog, binary or
//! multi-state object, an Access Door or an Access Zone keeping them in
//! storage save through the server
//! (#2331): each WriteProperty stages one save off the database guard, the
//! write takes it, and a server rebuilt on the storage serves what was
//! written. A WritePropertyMultiple of two parameters and a reporting
//! setting saves once, and a write that cannot be saved is refused on the
//! wire with the old values kept.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::durable_write_wire_tests::{
    ERROR_PDU, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{oid, value, wpm};
use super::reporting_durable_tests::{INSTANCE, served_by, write_property};
use super::write_config_durable_tests::ConfigStorage;
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

/// The eleven object types that keep a parameter: the analog family, the
/// binary one and the multi-state one, an Input, an Output and a Value
/// each, Access Door and Access Zone.
const KEEPERS: [ObjectType; 11] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_ZONE,
];

/// A list of an Access Door's or Access Zone's states, as a write carries
/// it.
fn listed(states: &[u32]) -> PropertyValue {
    PropertyValue::List(
        states
            .iter()
            .copied()
            .map(PropertyValue::Enumerated)
            .collect(),
    )
}

/// The parameters of its own an object of `kind` keeps beside the two time
/// delays, with a value a write gives each: an analog object's four, a
/// Binary Input's or Value's Alarm_Value, a Multi-state Input's or Value's
/// Alarm_Values (the fixture's objects have three states), an Access Door's
/// three lists of door alarm states (TAMPER and a proprietary state,
/// LOCK_DOWN, FREE_ACCESS), an Access Zone's Alarm_Values (AT_UPPER_LIMIT
/// and a proprietary state), and none on a Binary or Multi-state Output.
fn own(kind: ObjectType) -> Vec<(P, PropertyValue)> {
    match kind {
        ObjectType::ACCESS_DOOR => vec![
            (P::ALARM_VALUES, listed(&[4, 300])),
            (P::FAULT_VALUES, listed(&[6])),
            (P::MASKED_ALARM_VALUES, listed(&[7])),
        ],
        ObjectType::ACCESS_ZONE => vec![(P::ALARM_VALUES, listed(&[3, 100]))],
        ObjectType::ANALOG_INPUT | ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE => vec![
            (P::HIGH_LIMIT, PropertyValue::Real(80.0)),
            (P::LOW_LIMIT, PropertyValue::Real(-10.0)),
            (P::DEADBAND, PropertyValue::Real(2.5)),
            (
                P::LIMIT_ENABLE,
                PropertyValue::BitString {
                    unused_bits: 6,
                    data: vec![0xC0],
                },
            ),
        ],
        ObjectType::BINARY_INPUT | ObjectType::BINARY_VALUE => {
            vec![(P::ALARM_VALUE, PropertyValue::Enumerated(0))]
        }
        ObjectType::MULTI_STATE_INPUT | ObjectType::MULTI_STATE_VALUE => {
            let states = [3, 2].map(PropertyValue::Unsigned);
            vec![(P::ALARM_VALUES, PropertyValue::List(states.to_vec()))]
        }
        _ => Vec::new(),
    }
}

/// Each parameter an object of `kind` keeps, with a value a write gives
/// it.
fn parameters(kind: ObjectType) -> Vec<(P, PropertyValue)> {
    let mut parameters = vec![
        (P::TIME_DELAY, PropertyValue::Unsigned(7)),
        (P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(3)),
    ];
    parameters.extend(own(kind));
    parameters
}

/// Each type keeps the parameters the object crate says it does, and the
/// tests below cover every one: the two delays on all eleven, four more on
/// the analog types, one more on the binary and multi-state Inputs and
/// Values and on an Access Zone, and three more on an Access Door.
#[test]
fn the_covered_parameters_are_the_ones_each_type_keeps() {
    use bacnet_objects::snapshot::write_config::keeps_parameter;
    let every = [
        P::TIME_DELAY,
        P::TIME_DELAY_NORMAL,
        P::HIGH_LIMIT,
        P::LOW_LIMIT,
        P::DEADBAND,
        P::LIMIT_ENABLE,
        P::ALARM_VALUE,
        P::ALARM_VALUES,
        P::FAULT_VALUES,
        P::MASKED_ALARM_VALUES,
    ];
    let mut covered = 0;
    for kind in KEEPERS {
        let kept = parameters(kind);
        for property in every {
            let expected = kept.iter().any(|(covered, _)| *covered == property);
            assert_eq!(keeps_parameter(kind, property), expected, "{kind:?}");
        }
        covered += kept.len();
    }
    assert_eq!(covered, 11 * 2 + 3 * 4 + 4 + 3 + 1);
}

/// Each parameter's WriteProperty takes its staged save, without saving
/// again, and a server rebuilt on the storage serves what it wrote.
#[tokio::test(start_paused = true)]
async fn each_parameter_write_survives_a_rebuild() {
    for kind in KEEPERS {
        for (property, written) in parameters(kind) {
            let storage = Arc::new(ConfigStorage::default());
            let fixture = served_by(kind, &storage).await;
            let before = fixture.read(oid(kind, INSTANCE), property).await;
            assert_ne!(before, written, "{kind:?} {property:?} changes nothing");
            let request = write_property(kind, property, None, value(written.clone()));
            let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
            assert_eq!(response, SIMPLE_ACK_WRITE, "{kind:?} {property:?}");
            assert_eq!(fixture.read(oid(kind, INSTANCE), property).await, written);
            assert_eq!(
                storage.saves.load(Ordering::SeqCst),
                1,
                "{kind:?} {property:?}"
            );
            drop(fixture);
            let rebuilt = served_by(kind, &storage).await;
            assert_eq!(
                rebuilt.read(oid(kind, INSTANCE), property).await,
                written,
                "{kind:?} {property:?} after a rebuild"
            );
        }
    }
}

/// The save runs with the database guard dropped: the database answers a
/// reader and a writer while each parameter's save is held. A parameter
/// the server didn't stage would save in place, under the guard.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_parameter_save_runs_while_the_database_stays_available() {
    for kind in KEEPERS {
        for (property, written) in parameters(kind) {
            let storage = Arc::new(ConfigStorage::default());
            let fixture = served_by(kind, &storage).await;
            let request = write_property(kind, property, None, value(written));
            let (response, available) = while_saving(
                &fixture,
                &storage,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                request,
                WAIT,
            )
            .await;
            assert_eq!(response, SIMPLE_ACK_WRITE, "{kind:?} {property:?}");
            assert!(
                available,
                "the database was held while the {kind:?} saved {property:?}"
            );
            assert_eq!(
                storage.saves.load(Ordering::SeqCst),
                1,
                "{kind:?} {property:?}"
            );
        }
    }
}

fn attempt(property: P, written: PropertyValue) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: value(written),
        priority: None,
    }
}

/// The three rows the joint request to an object of `kind` writes, and
/// what they hold after it: a parameter of the type's own (on an Output,
/// which has none, Time_Delay_Normal), a reporting setting and Time_Delay.
fn joint(kind: ObjectType) -> [(P, PropertyValue); 3] {
    let first = own(kind).into_iter().next();
    [
        first.unwrap_or((P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(3))),
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(12)),
        (P::TIME_DELAY, PropertyValue::Unsigned(7)),
    ]
}

fn joint_request(kind: ObjectType) -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: oid(kind, INSTANCE),
        list_of_properties: joint(kind)
            .into_iter()
            .map(|(property, written)| attempt(property, written))
            .collect(),
    }])
}

/// One WritePropertyMultiple of two parameters and a reporting setting
/// saves once, and a rebuild serves all three.
#[tokio::test(start_paused = true)]
async fn a_request_of_two_parameters_and_a_reporting_setting_saves_once() {
    for kind in KEEPERS {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let response = wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            joint_request(kind),
        )
        .await;
        assert_eq!(response, SIMPLE_ACK_WPM, "{kind:?}");
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{kind:?}");
        drop(fixture);
        let rebuilt = served_by(kind, &storage).await;
        for (property, written) in joint(kind) {
            assert_eq!(
                rebuilt.read(oid(kind, INSTANCE), property).await,
                written,
                "{kind:?} {property:?}"
            );
        }
    }
}

/// When the save fails, a parameter's WriteProperty and the joint request
/// are answered with DEVICE / OPERATIONAL_PROBLEM, and the old values are
/// still served.
#[tokio::test(start_paused = true)]
async fn a_parameter_that_cannot_be_saved_is_refused_on_the_wire() {
    for kind in KEEPERS {
        let storage = Arc::new(ConfigStorage::default());
        storage.fail.store(true, Ordering::SeqCst);
        let fixture = served_by(kind, &storage).await;
        let object = oid(kind, INSTANCE);
        for (property, written) in parameters(kind) {
            let before = fixture.read(object, property).await;
            let request = write_property(kind, property, None, value(written));
            let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
            // An Error PDU: invoke ID 5, WriteProperty, DEVICE (0) and
            // OPERATIONAL_PROBLEM (25).
            assert_eq!(
                response,
                [ERROR_PDU, 5, 15, 0x91, 0, 0x91, 25],
                "{kind:?} {property:?}"
            );
            assert_eq!(fixture.read(object, property).await, before);
        }
        let mut before = Vec::new();
        for (property, _) in joint(kind) {
            before.push(fixture.read(object, property).await);
        }
        let response = wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            joint_request(kind),
        )
        .await;
        // WritePropertyMultiple's error: the same class and code, then the
        // first attempt, which is the one refused.
        assert_eq!(
            response[..8],
            [ERROR_PDU, 5, 16, 0x0E, 0x91, 0, 0x91, 25],
            "{kind:?}"
        );
        for ((property, _), before) in joint(kind).into_iter().zip(before) {
            assert_eq!(fixture.read(object, property).await, before, "{kind:?}");
        }
        assert_eq!(storage.saves.load(Ordering::SeqCst), 0, "{kind:?}");
    }
}
