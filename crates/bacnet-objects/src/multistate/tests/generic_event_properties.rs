//! Generic intrinsic-reporting property wiring on Multi-state Input and
//! Multi-state Value (#229).
//!
//! Kept out of `multistate/input.rs` and `multistate/value.rs`, which already
//! carry inline test modules and have little headroom under the 700-LOC cap.

use super::super::*;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::EventState;

/// Commission `Event_Enable` the way a client does: a Clause 20.2.10 bit string
/// through `write_property`, never a direct field assignment.
fn write_event_enable(object: &mut dyn BACnetObject, bits: EventTransitionBits) {
    object
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bits.to_bacnet()],
            },
            None,
        )
        .unwrap();
}

/// `Event_Enable` names one bit per transition, so a fixture that sets all three
/// cannot tell a correct bit from a wrong one — an inverted or off-by-one mask
/// passes it. This drives Multi-state Input into OFFNORMAL with a single bit set
/// at a time: TO_OFFNORMAL distributes, and TO_FAULT — a bit that exists but
/// names a different transition — does not.
///
/// Multi-state Input is the vehicle because it is the only one of the four types
/// wired by #229 whose detector can be given an alarm value through a public API
/// (`set_alarm_values`); the other three have no such path until #228 lands.
#[test]
fn msi_event_enable_bit_selects_which_transition_distributes() {
    for (bits, distributes) in [
        (EventTransitionBits::TO_OFFNORMAL, true),
        (EventTransitionBits::TO_FAULT, false),
        (EventTransitionBits::TO_NORMAL, false),
        (EventTransitionBits::empty(), false),
        (EventTransitionBits::all(), true),
    ] {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        msi.set_alarm_values(vec![2]);
        write_event_enable(&mut msi, bits);
        msi.set_present_value(2);

        let outcome = msi
            .evaluate_intrinsic_reporting()
            .expect("an alarm value in Present_Value must report a transition");
        assert_eq!(
            outcome.change.to,
            EventState::OFFNORMAL,
            "Event_Enable {bits} must not change which transition occurs"
        );
        assert_eq!(
            outcome.distribute, distributes,
            "Event_Enable {bits}: TO_OFFNORMAL distribution"
        );
    }
}

/// Every property that commissions intrinsic reporting must survive a network
/// write and read back, and be advertised in both `property_list` and
/// `is_writable_property` so the PICS matches dispatch.
fn assert_event_properties_round_trip(object: &mut dyn BACnetObject, label: &str) {
    for (property, value) in [
        (
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::all().to_bacnet()],
            },
        ),
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(1),
        ),
        (
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(42),
        ),
        (PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(7)),
    ] {
        object
            .write_property(property, None, value.clone(), None)
            .unwrap_or_else(|e| panic!("{label}: {property:?} write rejected: {e:?}"));
        assert_eq!(
            object.read_property(property, None).unwrap(),
            value,
            "{label}: {property:?} must read back what was written"
        );
        assert!(
            object.property_list().contains(&property),
            "{label}: {property:?} must be advertised in Property_List"
        );
        assert!(
            object.is_writable_property(property),
            "{label}: {property:?} must be advertised writable"
        );
    }

    // Acked_Transitions is readable but never writable: only AcknowledgeAlarm
    // may change it, and a property write would assign where the service ORs,
    // so it could both fabricate and erase acknowledgments.
    assert!(
        object
            .write_property(
                PropertyIdentifier::ACKED_TRANSITIONS,
                None,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![EventTransitionBits::TO_OFFNORMAL.to_bacnet()],
                },
                None,
            )
            .is_err(),
        "{label}: ACKED_TRANSITIONS write must be refused"
    );
    assert!(!object.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));
    assert!(object
        .property_list()
        .contains(&PropertyIdentifier::ACKED_TRANSITIONS));

    // Event_State is maintained by the detector, never assignable from the
    // network — a write arm would let a client fake an alarm state.
    assert!(
        object
            .write_property(
                PropertyIdentifier::EVENT_STATE,
                None,
                PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
                None,
            )
            .is_err(),
        "{label}: EVENT_STATE write must be refused"
    );
    assert!(!object.is_writable_property(PropertyIdentifier::EVENT_STATE));
}

#[test]
fn msi_event_properties_round_trip_and_match_pics() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    assert_event_properties_round_trip(&mut msi, "MSI");
}

#[test]
fn msv_event_properties_round_trip_and_match_pics() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    assert_event_properties_round_trip(&mut msv, "MSV");
}
