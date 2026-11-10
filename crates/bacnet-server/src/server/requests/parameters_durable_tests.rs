//! The event algorithm parameters a peer writes to an analog object keeping
//! them in storage save through the server (#2331 PR 2): each WriteProperty
//! stages one save off the database guard, the write takes it, and a server
//! rebuilt on the storage serves what was written. A WritePropertyMultiple
//! of two parameters and a reporting setting saves once, and a write that
//! cannot be saved is refused on the wire with the old values kept.
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

const ANALOG: [ObjectType; 3] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
];

/// Each parameter an analog object keeps, with a value a write gives it.
fn parameters() -> [(P, PropertyValue); 6] {
    [
        (P::TIME_DELAY, PropertyValue::Unsigned(7)),
        (P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(3)),
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
    ]
}

/// Each parameter's WriteProperty takes its staged save, without saving
/// again, and a server rebuilt on the storage serves what it wrote.
#[tokio::test(start_paused = true)]
async fn each_parameter_write_survives_a_rebuild() {
    for kind in ANALOG {
        for (property, written) in parameters() {
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
    for kind in ANALOG {
        for (property, written) in parameters() {
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

/// The three rows the joint request writes, and what they hold after it.
fn joint() -> [(P, PropertyValue); 3] {
    [
        (P::HIGH_LIMIT, PropertyValue::Real(80.0)),
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(12)),
        (P::TIME_DELAY, PropertyValue::Unsigned(7)),
    ]
}

fn joint_request(kind: ObjectType) -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: oid(kind, INSTANCE),
        list_of_properties: joint()
            .into_iter()
            .map(|(property, written)| attempt(property, written))
            .collect(),
    }])
}

/// One WritePropertyMultiple of two parameters and a reporting setting
/// saves once, and a rebuild serves all three.
#[tokio::test(start_paused = true)]
async fn a_request_of_two_parameters_and_a_reporting_setting_saves_once() {
    for kind in ANALOG {
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
        for (property, written) in joint() {
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
    for kind in ANALOG {
        let storage = Arc::new(ConfigStorage::default());
        storage.fail.store(true, Ordering::SeqCst);
        let fixture = served_by(kind, &storage).await;
        let object = oid(kind, INSTANCE);
        for (property, written) in parameters() {
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
        for (property, _) in joint() {
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
        for ((property, _), before) in joint().into_iter().zip(before) {
            assert_eq!(fixture.read(object, property).await, before, "{kind:?}");
        }
        assert_eq!(storage.saves.load(Ordering::SeqCst), 0, "{kind:?}");
    }
}
