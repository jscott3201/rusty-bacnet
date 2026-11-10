//! A multi-state object's kept Alarm_Values and its state count, which
//! isn't kept (#2331). A peer adds a state through State_Text and lists it:
//! one request doing both saves off the database guard, and a server
//! rebuilt on the storage with the configured count still starts, serving
//! the list as it was written.
//!
//! PROPERTY 2; VALUE_OUT_OF_RANGE 37.

use super::durable_write_wire_tests::{
    ERROR_PDU, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::reporting_durable_tests::{INSTANCE, served_by, write_property};
use super::write_config_durable_tests::ConfigStorage;
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums::Reliability;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

/// The two types that keep Alarm_Values. The fixture builds each with
/// three states.
const LISTED: [ObjectType; 2] = [ObjectType::MULTI_STATE_INPUT, ObjectType::MULTI_STATE_VALUE];

/// A fourth state, by State_Text's size at index 0.
fn four_states() -> (Option<u32>, PropertyValue) {
    (Some(0), PropertyValue::Unsigned(4))
}

/// A list naming the fourth state and one the object is built with.
fn list() -> PropertyValue {
    PropertyValue::List([4, 2].map(PropertyValue::Unsigned).to_vec())
}

fn reliability(reliability: Reliability) -> PropertyValue {
    PropertyValue::Enumerated(reliability.to_raw())
}

async fn write(
    fixture: &Fixture,
    kind: ObjectType,
    property: P,
    written: PropertyValue,
) -> Vec<u8> {
    let (index, written) = match property {
        P::STATE_TEXT => four_states(),
        _ => (None, written),
    };
    let request = write_property(kind, property, index, value(written));
    wire(fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await
}

/// The peer's two writes, each accepted, leave a list naming a state the
/// object as the application builds it doesn't have. The rebuilt server
/// starts and serves that list. The Input reports no fault; the Value
/// reports CONFIGURATION_ERROR until a list within its states is written.
/// A write of the stored list is refused as any list past the states is,
/// and a later kept write keeps the stored list as it is.
#[tokio::test(start_paused = true)]
async fn a_list_naming_a_state_a_peer_added_is_served_after_a_rebuild() {
    for kind in LISTED {
        let object = oid(kind, INSTANCE);
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let grown = write(&fixture, kind, P::STATE_TEXT, PropertyValue::Null).await;
        assert_eq!(grown, SIMPLE_ACK_WRITE, "{kind:?}");
        assert_eq!(
            write(&fixture, kind, P::ALARM_VALUES, list()).await,
            SIMPLE_ACK_WRITE,
            "{kind:?}"
        );
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{kind:?}");
        drop(fixture);

        let rebuilt = served_by(kind, &storage).await;
        let three = PropertyValue::Unsigned(3);
        assert_eq!(rebuilt.read(object, P::NUMBER_OF_STATES).await, three);
        assert_eq!(rebuilt.read(object, P::ALARM_VALUES).await, list());
        let value_object = kind == ObjectType::MULTI_STATE_VALUE;
        let expected = if value_object {
            Reliability::CONFIGURATION_ERROR
        } else {
            Reliability::NO_FAULT_DETECTED
        };
        assert_eq!(
            rebuilt.read(object, P::RELIABILITY).await,
            reliability(expected),
            "{kind:?}"
        );
        // The wire rule stands: the same list written back is refused.
        let refused = write(&rebuilt, kind, P::ALARM_VALUES, list()).await;
        assert_eq!(refused[0], ERROR_PDU, "{kind:?}");
        assert_eq!(refused[3..7], [0x91, 2, 0x91, 37], "{kind:?}");
        assert_eq!(rebuilt.read(object, P::ALARM_VALUES).await, list());
        // Another kept write saves the stored list with it, unchanged.
        let delay = PropertyValue::Unsigned(7);
        assert_eq!(
            write(&rebuilt, kind, P::TIME_DELAY, delay.clone()).await,
            SIMPLE_ACK_WRITE
        );
        drop(rebuilt);
        let again = served_by(kind, &storage).await;
        assert_eq!(again.read(object, P::ALARM_VALUES).await, list());
        assert_eq!(again.read(object, P::TIME_DELAY).await, delay);
        // A list within the states repairs the Value.
        let within = PropertyValue::List([3, 2].map(PropertyValue::Unsigned).to_vec());
        assert_eq!(
            write(&again, kind, P::ALARM_VALUES, within.clone()).await,
            SIMPLE_ACK_WRITE
        );
        assert_eq!(again.read(object, P::ALARM_VALUES).await, within);
        assert_eq!(
            again.read(object, P::RELIABILITY).await,
            reliability(Reliability::NO_FAULT_DETECTED),
            "{kind:?}"
        );
    }
}

fn attempt(property: P, index: Option<u32>, written: PropertyValue) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: index,
        value: value(written),
        priority: None,
    }
}

/// One WritePropertyMultiple that adds a state and then lists it saves
/// once, with the database guard dropped: the stage checks the list against
/// the count the request's own State_Text write leaves. Checked against the
/// count from before the request, the list would be refused at the stage
/// and saved where it is written, under the guard.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_that_adds_a_state_and_lists_it_saves_off_the_guard() {
    for kind in LISTED {
        let object = oid(kind, INSTANCE);
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let (index, states) = four_states();
        let request = wpm(vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![
                attempt(P::STATE_TEXT, index, states),
                attempt(P::ALARM_VALUES, None, list()),
            ],
        }]);
        let (response, available) = while_saving(
            &fixture,
            &storage,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            request,
            WAIT,
        )
        .await;
        assert_eq!(response, SIMPLE_ACK_WPM, "{kind:?}");
        assert!(
            available,
            "the database was held while the {kind:?} saved its list"
        );
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{kind:?}");
        assert_eq!(fixture.read(object, P::ALARM_VALUES).await, list());
        let four = PropertyValue::Unsigned(4);
        assert_eq!(fixture.read(object, P::NUMBER_OF_STATES).await, four);
    }
}
