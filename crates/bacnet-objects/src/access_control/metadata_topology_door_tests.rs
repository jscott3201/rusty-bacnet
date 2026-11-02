//! Access Door writes against its metadata: the command priority, the
//! Relinquish_Default gate, and the rows writable only out of service.

use super::*;
use crate::property_metadata::is_writable_property;

#[test]
fn property_metadata_access_door_writes_command_priority_and_gate_relinquish() {
    for out_of_service in [false, true] {
        let mut object = AccessDoorObject::new(1, "DOOR-1").unwrap();
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(out_of_service),
                None,
            )
            .unwrap();
        // A priority write commands Present_Value; relinquishing the slot
        // falls back to Relinquish_Default.
        object
            .write_property(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(1),
                Some(8),
            )
            .unwrap();
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(1)
        );
        assert_eq!(
            object.read_property(P::PRIORITY_ARRAY, Some(8)).unwrap(),
            PropertyValue::Enumerated(1)
        );
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Null, Some(8))
            .unwrap();
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        // Relinquish_Default admits LOCK and UNLOCK (Clause 12.26.11)
        // and resolves Present_Value anew from the empty array.
        for raw in [0u32, 1] {
            object
                .write_property(
                    P::RELINQUISH_DEFAULT,
                    None,
                    PropertyValue::Enumerated(raw),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
                PropertyValue::Enumerated(raw)
            );
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(raw)
            );
        }
        object
            .write_property(
                P::RELINQUISH_DEFAULT,
                None,
                PropertyValue::Enumerated(1),
                None,
            )
            .unwrap();
        for raw in [2, 3, 4] {
            assert_error(
                object
                    .write_property(
                        P::RELINQUISH_DEFAULT,
                        None,
                        PropertyValue::Enumerated(raw),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        assert_eq!(
            object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
            PropertyValue::Enumerated(1)
        );
        // Mistyped values are rejected without changing state.
        for (p, value) in [
            (P::PRESENT_VALUE, PropertyValue::Real(1.0)),
            (P::RELINQUISH_DEFAULT, PropertyValue::Real(1.0)),
            (P::DOOR_PULSE_TIME, PropertyValue::Real(1.0)),
            (P::DESCRIPTION, PropertyValue::Unsigned(1)),
            (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
        ] {
            assert_error(
                object.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(1)
        );
        // The footnote-1 status rows (#1131) and Reliability (#1149) take
        // their own readback only while out of service.
        for p in [
            P::DOOR_STATUS,
            P::LOCK_STATUS,
            P::DOOR_ALARM_STATE,
            P::RELIABILITY,
        ] {
            let value = object.read_property(p, None).unwrap();
            let result = object.write_property(p, None, value, None);
            if out_of_service {
                result.unwrap();
            } else {
                assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert!(is_writable_property(&object, p));
        }
        // The other Table-O status rows and the readable-only rows deny
        // even their own readback on write.
        for p in [
            P::SECURED_STATUS,
            P::DOOR_MEMBERS,
            P::PRIORITY_ARRAY,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::CURRENT_COMMAND_PRIORITY,
            P::ACKED_TRANSITIONS,
            P::EVENT_TIME_STAMPS,
            P::EVENT_MESSAGE_TEXTS,
        ] {
            let value = object.read_property(p, None).unwrap();
            assert_error(
                object.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!is_writable_property(&object, p));
        }
    }
}
