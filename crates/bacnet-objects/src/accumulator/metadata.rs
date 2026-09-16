use super::{AccumulatorObject, PulseConverterObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Canonical effective rows for the Accumulator pair (ASHRAE 135-2020; PDF = printed + 2):
// - Accumulator (type 23, §12.61 Table 12-79; printed pp. 601-609 / PDF pp. 603-611)
// - Pulse Converter (type 24, §12.23 Table 12-27; printed pp. 301-307 / PDF pp. 303-309)
// Order preserves each legacy projection; PROPERTY_LIST is appended so the
// projection helper omits it while required_properties keeps it. Only
// implemented rows are described: table rows the objects do not serve
// (Accumulator Device_Type, Value_Change_Time, Logging_Record,
// Logging_Object, High/Limit rows, Limit_Enable, Count-adjacent rows,
// event/intrinsic/audit/tag/profile rows; Pulse Converter Count,
// Update_Time, Count_Change_Time, Count_Before_Change, COV_Period,
// event/intrinsic/audit/tag/profile rows) stay absent until dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED (neither object has a write_object_name arm).
// Description carries the table O code with a routed CharacterString write
// arm, so Optional/Always. Out_Of_Service carries the table R code with the
// routed Boolean arm, so RequiredRead/Always.
// Table-R served rows with no network write route stay RequiredRead/ReadOnly;
// table-R rows with a write arm are RequiredRead/Always. Table-O served rows
// are Optional, with Always exactly where dispatch accepts the write
// (Accumulator Max_Pres_Value is table R with an arm, so
// RequiredRead/Always; Pulse_Rate and Limit_Monitoring_Interval are table O
// with arms, so Optional/Always; Prescale, Reliability, Value_Before_Change,
// and Value_Set have no arm, so Optional/ReadOnly).
// Accumulator Present_Value is the one deliberate dispatch-first deviation:
// Table 12-79 codes it R with footnote 1 ("required to be writable when
// Out_Of_Service is TRUE") and §12.61 states it "shall be writable when
// Out_Of_Service is TRUE", but the write arm unconditionally denies it and
// no Value_Set mechanism advances it, so the metadata mirrors dispatch as
// RequiredRead/ReadOnly rather than advertising a route that does not exist.
// Pulse_Rate is served as Real while Table 12-79 types it Unsigned, and
// Status_Flags is computed with event_state=0 by the shared common arm even
// though the object owns an Event_State field; both quirks are preserved, not
// fixed, by this migration.
// Pulse Converter Present_Value carries the table R code with footnote 1 and
// §12.23 states it "shall be writable when Out_Of_Service is TRUE"; dispatch
// gates it behind Out_Of_Service (in-service writes are denied before value
// validation), so RequiredRead/WhenOutOfService (D5 clause-backed, preserved
// exactly). Adjust_Value carries the table W code with a routed Real arm, so
// RequiredWrite/Always. Scale_Factor is table R with an arm
// (RequiredRead/Always); Input_Reference and COV_Increment are table O with
// arms (Optional/Always; COV_Increment footnote 2 ties it to COV reporting,
// which the object provides via supports_cov and cov_increment).
// Writability otherwise mirrors dispatch exactly: the Pulse Converter
// is_writable override ({PV, SCALE_FACTOR, ADJUST_VALUE, INPUT_REFERENCE,
// DESCRIPTION, OUT_OF_SERVICE, COV_INCREMENT}) translated one row at a time
// and then deleted, so PICS writable flags are unchanged. Presence is None
// throughout: the implementation models no commandable, intrinsic-reporting,
// or paired-text gating on this family.
// The pair is not createable at runtime (the network factory builds only the
// eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. Array gating keeps the default: Property_List
// admits an index (BACnetARRAY per Tables 12-79/12-27) while every other
// served row rejects one. COV keeps its overrides: supports_cov=true on both
// objects plus cov_increment()=Some on Pulse Converter, and the COV gating
// path (read_property plus supports_cov_property to supports_cov) never
// consults metadata.
const ACCUMULATOR_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MAX_PRES_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::SCALE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESCALE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PULSE_RATE, Optional, None, Always),
    PropertyMetadata::new(P::UNITS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LIMIT_MONITORING_INTERVAL, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::VALUE_BEFORE_CHANGE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::VALUE_SET, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const PULSE_CONVERTER_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::UNITS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SCALE_FACTOR, RequiredRead, None, Always),
    PropertyMetadata::new(P::ADJUST_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::COV_INCREMENT, Optional, None, Always),
    PropertyMetadata::new(P::INPUT_REFERENCE, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_accumulator_object(_object: &AccumulatorObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCUMULATOR_BASE)
}

pub(super) fn for_pulse_converter_object(
    _object: &PulseConverterObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(PULSE_CONVERTER_BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property_metadata::PropertyWriteCapability;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
    use std::collections::HashSet;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    fn assert_exact_sets(object: &dyn BACnetObject, all: &[P], required: &[P]) {
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), all.len() + 1);
        assert_eq!(object.property_list().as_ref(), all);
        assert_eq!(object.required_properties().as_ref(), required);
        assert_eq!(
            metadata
                .iter()
                .map(|row| row.property_identifier)
                .collect::<HashSet<_>>()
                .len(),
            metadata.len()
        );
        assert!(!object.is_createable());
        assert!(object.is_deleteable());
        assert!(object.supports_cov());
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            let expected = if required.contains(&row.property_identifier) {
                if row.property_identifier == P::ADJUST_VALUE {
                    RequiredWrite
                } else {
                    RequiredRead
                }
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
            object.read_property(row.property_identifier, None).unwrap();
        }
    }

    fn assert_indexed_property_list(object: &dyn BACnetObject, all: &[P]) {
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(wire.len() as u64)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [wire.len() as u32 + 1, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_accumulator_exact_sets_readable_rows_and_indexed_list() {
        let object = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::MAX_PRES_VALUE,
            P::SCALE,
            P::PRESCALE,
            P::PULSE_RATE,
            P::UNITS,
            P::LIMIT_MONITORING_INTERVAL,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::VALUE_BEFORE_CHANGE,
            P::VALUE_SET,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::MAX_PRES_VALUE,
            P::SCALE,
            P::UNITS,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object.read_property(P::SCALE, None).unwrap(),
            PropertyValue::List(vec![PropertyValue::Real(1.0)])
        );
        assert_eq!(
            object.read_property(P::PRESCALE, None).unwrap(),
            PropertyValue::Null
        );
        // Scale, Prescale, and Property_List-adjacent scalars are not
        // BACnetARRAY rows, so the service gate rejects an index on them.
        assert!(!object.is_array_property(P::SCALE));
        assert!(!object.is_array_property(P::PRESCALE));
        assert!(!object.is_array_property(P::PRESENT_VALUE));
    }

    #[test]
    fn property_metadata_pulse_converter_exact_sets_readable_rows_and_indexed_list() {
        let object = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::UNITS,
            P::SCALE_FACTOR,
            P::ADJUST_VALUE,
            P::COV_INCREMENT,
            P::INPUT_REFERENCE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::UNITS,
            P::SCALE_FACTOR,
            P::ADJUST_VALUE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Real(0.0)
        );
        assert_eq!(
            object.read_property(P::SCALE_FACTOR, None).unwrap(),
            PropertyValue::Real(1.0)
        );
        assert_eq!(
            object.read_property(P::INPUT_REFERENCE, None).unwrap(),
            PropertyValue::Null
        );
        assert_eq!(object.cov_increment(), Some(0.0));
        assert!(!object.is_array_property(P::INPUT_REFERENCE));
        assert!(!object.is_array_property(P::PRESENT_VALUE));
    }

    #[test]
    fn property_metadata_accumulator_write_capabilities_match_dispatch() {
        let writable = [
            P::DESCRIPTION,
            P::OUT_OF_SERVICE,
            P::MAX_PRES_VALUE,
            P::PULSE_RATE,
            P::LIMIT_MONITORING_INTERVAL,
        ];
        for out_of_service in [false, true] {
            let mut object = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let original = object.property_metadata().into_owned();
            for row in &original {
                let p = row.property_identifier;
                let capability = if writable.contains(&p) {
                    PropertyWriteCapability::Always
                } else {
                    PropertyWriteCapability::ReadOnly
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    object.is_writable_property(p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if capability.is_writable() {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Object_Name has no network write route: a rename falls through
            // to WRITE_ACCESS_DENIED even with a well-formed value.
            assert!(!object.is_writable_property(P::OBJECT_NAME));
            assert_error(
                object
                    .write_property(
                        P::OBJECT_NAME,
                        None,
                        PropertyValue::CharacterString("renamed".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Present_Value is always denied — even out of service and even
            // with its own readback — because no arm routes it. The metadata
            // mirrors dispatch (ReadOnly) rather than the Table 12-79
            // footnote-1 writability the implementation does not provide.
            assert!(!object.is_writable_property(P::PRESENT_VALUE));
            assert_error(
                object
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(10), None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(0)
            );
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }

    #[test]
    fn property_metadata_pulse_converter_write_capabilities_match_dispatch() {
        let always = [
            P::DESCRIPTION,
            P::OUT_OF_SERVICE,
            P::SCALE_FACTOR,
            P::ADJUST_VALUE,
            P::INPUT_REFERENCE,
            P::COV_INCREMENT,
        ];
        for out_of_service in [false, true] {
            let mut object = PulseConverterObject::new(1, "PC-1", 62).unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let original = object.property_metadata().into_owned();
            for row in &original {
                let p = row.property_identifier;
                let capability = if always.contains(&p) {
                    PropertyWriteCapability::Always
                } else if p == P::PRESENT_VALUE {
                    PropertyWriteCapability::WhenOutOfService
                } else {
                    PropertyWriteCapability::ReadOnly
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    object.is_writable_property(p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if capability == PropertyWriteCapability::Always
                    || (capability == PropertyWriteCapability::WhenOutOfService && out_of_service)
                {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Object_Name has no network write route: a rename falls through
            // to WRITE_ACCESS_DENIED even with a well-formed value.
            assert!(!object.is_writable_property(P::OBJECT_NAME));
            assert_error(
                object
                    .write_property(
                        P::OBJECT_NAME,
                        None,
                        PropertyValue::CharacterString("renamed".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }

    #[test]
    fn property_metadata_pulse_converter_present_value_oos_gate_pins() {
        // In-service writes are denied before value validation (D5): even a
        // mistyped or non-finite value reports WRITE_ACCESS_DENIED, not a
        // datatype or range error, and the stored value is untouched.
        let mut object = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        assert!(object.is_writable_property(P::PRESENT_VALUE));
        for value in [
            PropertyValue::Real(12.5),
            PropertyValue::Unsigned(12),
            PropertyValue::Real(f32::NAN),
        ] {
            assert_error(
                object
                    .write_property(P::PRESENT_VALUE, None, value, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Real(0.0)
        );
        // Out of service, a finite Real round-trips; mistyped and
        // non-finite values are rejected past the gate.
        object
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(12.5), None)
            .unwrap();
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Real(12.5)
        );
        assert_error(
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(12), None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_error(
            object
                .write_property(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Real(f32::INFINITY),
                    None,
                )
                .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Real(12.5)
        );
    }

    #[test]
    fn property_metadata_accumulator_writes_store_verbatim_with_range_gates() {
        for out_of_service in [false, true] {
            let mut object = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            object
                .write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(1000), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::MAX_PRES_VALUE, None).unwrap(),
                PropertyValue::Unsigned(1000)
            );
            object
                .write_property(P::PULSE_RATE, None, PropertyValue::Real(2.5), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::PULSE_RATE, None).unwrap(),
                PropertyValue::Real(2.5)
            );
            object
                .write_property(
                    P::LIMIT_MONITORING_INTERVAL,
                    None,
                    PropertyValue::Unsigned(60),
                    None,
                )
                .unwrap();
            assert_eq!(
                object
                    .read_property(P::LIMIT_MONITORING_INTERVAL, None)
                    .unwrap(),
                PropertyValue::Unsigned(60)
            );
            // Non-finite Pulse_Rate values are refused without touching state.
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_error(
                    object
                        .write_property(P::PULSE_RATE, None, PropertyValue::Real(value), None)
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
            assert_eq!(
                object.read_property(P::PULSE_RATE, None).unwrap(),
                PropertyValue::Real(2.5)
            );
            // Oversized Limit_Monitoring_Interval values are refused.
            assert_error(
                object
                    .write_property(
                        P::LIMIT_MONITORING_INTERVAL,
                        None,
                        PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            // Mistyped values are rejected without changing state.
            for (p, value) in [
                (P::MAX_PRES_VALUE, PropertyValue::Real(1.0)),
                (P::PULSE_RATE, PropertyValue::Unsigned(1)),
                (P::LIMIT_MONITORING_INTERVAL, PropertyValue::Enumerated(60)),
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Rows with no network write route deny even their readback.
            for p in [
                P::SCALE,
                P::PRESCALE,
                P::VALUE_BEFORE_CHANGE,
                P::VALUE_SET,
                P::STATUS_FLAGS,
                P::EVENT_STATE,
                P::RELIABILITY,
                P::UNITS,
            ] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
        }
    }

    #[test]
    fn property_metadata_pulse_converter_writes_store_verbatim_with_range_gates() {
        for out_of_service in [false, true] {
            let mut object = PulseConverterObject::new(1, "PC-1", 62).unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            object
                .write_property(P::SCALE_FACTOR, None, PropertyValue::Real(2.5), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::SCALE_FACTOR, None).unwrap(),
                PropertyValue::Real(2.5)
            );
            object
                .write_property(P::ADJUST_VALUE, None, PropertyValue::Real(0.5), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::ADJUST_VALUE, None).unwrap(),
                PropertyValue::Real(0.5)
            );
            object
                .write_property(P::COV_INCREMENT, None, PropertyValue::Real(0.5), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::COV_INCREMENT, None).unwrap(),
                PropertyValue::Real(0.5)
            );
            assert_eq!(object.cov_increment(), Some(0.5));
            // Non-finite and negative values are refused without touching state.
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                for p in [P::SCALE_FACTOR, P::ADJUST_VALUE] {
                    assert_error(
                        object
                            .write_property(p, None, PropertyValue::Real(value), None)
                            .unwrap_err(),
                        ErrorCode::VALUE_OUT_OF_RANGE,
                    );
                }
                assert_error(
                    object
                        .write_property(P::COV_INCREMENT, None, PropertyValue::Real(value), None)
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
            assert_error(
                object
                    .write_property(P::COV_INCREMENT, None, PropertyValue::Real(-1.0), None)
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(
                object.read_property(P::SCALE_FACTOR, None).unwrap(),
                PropertyValue::Real(2.5)
            );
            // Input_Reference stores a local reference verbatim and Null clears it.
            let oid = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap();
            let prop_raw = P::PRESENT_VALUE.to_raw();
            let reference = PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(oid),
                PropertyValue::Enumerated(prop_raw),
            ]);
            object
                .write_property(P::INPUT_REFERENCE, None, reference.clone(), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::INPUT_REFERENCE, None).unwrap(),
                reference
            );
            object
                .write_property(P::INPUT_REFERENCE, None, PropertyValue::Null, None)
                .unwrap();
            assert_eq!(
                object.read_property(P::INPUT_REFERENCE, None).unwrap(),
                PropertyValue::Null
            );
            // Mistyped values are rejected without changing state.
            for (p, value) in [
                (P::SCALE_FACTOR, PropertyValue::Unsigned(1)),
                (P::ADJUST_VALUE, PropertyValue::Unsigned(1)),
                (P::COV_INCREMENT, PropertyValue::Null),
                (P::INPUT_REFERENCE, PropertyValue::Unsigned(1)),
                (P::DESCRIPTION, PropertyValue::Null),
                (P::OUT_OF_SERVICE, PropertyValue::Null),
            ] {
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            // Rows with no network write route deny even their readback.
            for p in [P::UNITS, P::STATUS_FLAGS, P::EVENT_STATE, P::RELIABILITY] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
        }
    }

    #[test]
    fn property_metadata_accumulator_pair_unserved_rows_stay_unknown() {
        fn assert_unserved(object: &mut dyn BACnetObject, p: P) {
            assert!(!object.is_writable_property(p));
            assert_error(
                object.read_property(p, None).unwrap_err(),
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert_error(
                object
                    .write_property(p, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }

        // Table 12-79 O rows with no read arm (plus a Pulse Converter row).
        let mut acc = AccumulatorObject::new(1, "ACC-1", 95).unwrap();
        assert_unserved(&mut acc, P::DEVICE_TYPE);
        assert_unserved(&mut acc, P::VALUE_CHANGE_TIME);
        assert_unserved(&mut acc, P::COUNT);
        // Table 12-27 R rows with no read arm (plus an Accumulator row).
        let mut pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
        assert_unserved(&mut pc, P::COUNT);
        assert_unserved(&mut pc, P::UPDATE_TIME);
        assert_unserved(&mut pc, P::COUNT_BEFORE_CHANGE);
        assert_unserved(&mut pc, P::DEVICE_TYPE);
    }
}
