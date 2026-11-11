//! The table entries #2465 fixed: Audit Reporter's own filter is plain, and
//! BitString Value has no Fault_Values of its own.

use super::standard_value_type;
use crate::datatype::Datatype as D;
use crate::enums::{ObjectType as T, PropertyIdentifier as P};

#[test]
fn the_reporters_priority_filter_is_plain_and_other_types_keep_the_optional_form() {
    let reporter = standard_value_type(T::AUDIT_REPORTER, P::AUDIT_PRIORITY_FILTER).unwrap();
    assert_eq!(reporter.datatype, D::BitString);
    assert!(!reporter.admits_null(None));
    for object_type in [T::BITSTRING_VALUE, T::ANALOG_VALUE, T::DEVICE] {
        let other = standard_value_type(object_type, P::AUDIT_PRIORITY_FILTER).unwrap();
        assert_eq!(other.datatype, D::OptionalPriorityFilter);
        assert!(other.admits_null(None));
    }
}

#[test]
fn bitstring_value_keeps_alarm_values_but_has_no_fault_values_entry_of_its_own() {
    let alarm = standard_value_type(T::BITSTRING_VALUE, P::ALARM_VALUES).unwrap();
    assert_eq!(alarm.datatype, D::BitString);
    assert!(alarm.is_array());
    assert_eq!(
        standard_value_type(T::BITSTRING_VALUE, P::FAULT_VALUES),
        standard_value_type(T::DEVICE, P::FAULT_VALUES),
    );
}
