//! Generic intrinsic-reporting property wiring on Binary Input and Binary
//! Value (#229).
//!
//! Distribution itself is only exercised on Multi-state Input (see
//! `multistate/tests/generic_event_properties.rs`): the binary types'
//! `ChangeOfStateDetector` has no public path to an alarm value until #228
//! lands, so there is no way to drive one out of NORMAL from here. What these
//! tests pin is the reachability half of the defect — before #229, Time_Delay
//! and Notify_Type had no arms at all and Event_Enable was readable but not
//! writable, so the transition bits were stuck at (F, F, F).

use super::*;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::EventState;

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

    // A single set bit must land on the transition it names, not on whichever
    // bit a mask error happens to select. TO_NORMAL is wire bit 2 (0x20).
    object
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_NORMAL.to_bacnet()],
            },
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x20],
        },
        "{label}: TO_NORMAL alone must read back as wire bit 2"
    );

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
fn bi_event_properties_round_trip_and_match_pics() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_event_properties_round_trip(&mut bi, "BI");
}

#[test]
fn bv_event_properties_round_trip_and_match_pics() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    assert_event_properties_round_trip(&mut bv, "BV");
}

/// The Event_Time_Stamps placeholder must keep answering while the event set
/// around it becomes writable: #235 owns replacing it with real storage, and a
/// read that starts erroring in the meantime is a regression, not progress.
#[test]
fn event_time_stamps_placeholder_still_reads_on_binary_types() {
    let placeholder = PropertyValue::List(vec![
        PropertyValue::Unsigned(0),
        PropertyValue::Unsigned(0),
        PropertyValue::Unsigned(0),
    ]);
    assert_eq!(
        BinaryInputObject::new(1, "BI-1")
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
            .unwrap(),
        placeholder
    );
    assert_eq!(
        BinaryValueObject::new(1, "BV-1")
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
            .unwrap(),
        placeholder
    );
}
