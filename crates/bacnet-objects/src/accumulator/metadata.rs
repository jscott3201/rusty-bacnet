use super::AccumulatorObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Canonical effective rows for the Accumulator (type 23, ASHRAE 135-2020
// §12.61 Table 12-79; printed pp. 601-609 / PDF pp. 603-611). The Pulse
// Converter keeps its own rows in pulse_converter/metadata.rs.
// Order preserves the earlier row order; PROPERTY_LIST is appended last. Only
// implemented rows are described: table rows the object does not serve
// (Device_Type, Value_Change_Time, Logging_Record, Logging_Object,
// High/Limit rows, Limit_Enable, event/intrinsic/audit/tag/profile rows) stay
// absent until dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED (the object has no write_object_name arm).
// Description carries the table O code with a routed CharacterString write
// arm, so Optional/Always. Out_Of_Service carries the table R code with the
// routed Boolean arm, so RequiredRead/Always.
// Table-R served rows with no network write route stay RequiredRead/ReadOnly;
// table-R rows with a write arm are RequiredRead/Always. Table-O served rows
// are Optional, with Always exactly where dispatch accepts the write
// (Max_Pres_Value is table R with an arm, so RequiredRead/Always;
// Limit_Monitoring_Interval is table O with an arm, so Optional/Always;
// Pulse_Rate is table O and writable only out of service (Clause 12.61.20),
// so Optional/WhenOutOfService; Present_Value is table R with footnote 1,
// writable only out of service (Clause 12.61.4, #2267), so
// RequiredRead/WhenOutOfService;
// Prescale, Reliability, Value_Before_Change, and Value_Set have no arm, so
// Optional/ReadOnly). Prescale is present only once the application sets
// one; without it the row is left out, so the property reads as unknown
// rather than as a NULL its datatype doesn't have.
// Status_Flags is computed with event_state=0
// by the shared common arm even though the object owns an Event_State field;
// that quirk is preserved, not fixed, by this migration. Presence is None
// throughout: the implementation models no commandable, intrinsic-reporting,
// or paired-text gating. The object is not createable at runtime (the
// `creation` table has no constructor for it) and remains deleteable
// (delete denies only Device and NetworkPort, so the builtin_deleteable=true
// default holds); neither needs an override. Array gating keeps the default:
// Property_List admits an index (BACnetARRAY per Table 12-79) while every
// other served row rejects one. COV keeps its `cov()` lend, and the COV
// gating path (read_property plus `CovReporting::supports_cov_property`)
// never consults metadata.
const ACCUMULATOR_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::MAX_PRES_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::SCALE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESCALE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PULSE_RATE, Optional, None, WhenOutOfService),
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

/// The rows, Prescale's left out until the application sets one: the
/// property is optional and BACnetPrescale has no value for none.
pub(super) fn for_accumulator_object(object: &AccumulatorObject) -> Cow<'_, [PropertyMetadata]> {
    if object.has_prescale() {
        Cow::Borrowed(ACCUMULATOR_BASE)
    } else {
        Cow::Owned(
            ACCUMULATOR_BASE
                .iter()
                .filter(|row| row.property_identifier != P::PRESCALE)
                .copied()
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::HashSet;
    use crate::property_metadata::PropertyWriteCapability;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::EngineeringUnits;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::{Error, RemoteError};
    use bacnet_types::primitives::PropertyValue;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY
                    && code == expected),
            "expected {expected:?}, got {error:?}"
        );
    }

    fn assert_exact_sets(object: &dyn BACnetObject, all: &[P], required: &[P]) {
        let metadata = object.property_metadata();
        assert_eq!(metadata.len(), all.len() + 1);
        assert_eq!(
            crate::test_rows::listed(&object.property_metadata()),
            crate::test_rows::listed_from(all)
        );
        assert_eq!(
            crate::test_rows::required(&object.property_metadata()),
            required
        );
        assert_eq!(
            metadata
                .iter()
                .map(|row| row.property_identifier)
                .collect::<HashSet<_>>()
                .len(),
            metadata.len()
        );
        assert!(crate::creation::builtin_deleteable(
            object.object_identifier().object_type()
        ));
        assert!(object.cov().is_some_and(|cov| cov.supports_subscribe_cov()));
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            let expected = if required.contains(&row.property_identifier) {
                RequiredRead
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
        assert!(crate::property_metadata::is_array_property(
            object,
            P::PROPERTY_LIST
        ));
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
        let mut object = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        // Prescale is served only once the application sets one.
        let without_prescale =
            AccumulatorObject::new(2, "ACC-2", EngineeringUnits::NO_UNITS).unwrap();
        object.set_prescale(bacnet_types::constructed::BACnetPrescale {
            multiplier: 1,
            modulo_divide: 100,
        });
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
        let all_without: Vec<_> = all.into_iter().filter(|p| *p != P::PRESCALE).collect();
        assert_exact_sets(&without_prescale, &all_without, &required);
        assert_indexed_property_list(&without_prescale, &all_without);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        // Both in their context-tagged Clause 21 forms (#1487).
        assert_eq!(
            object.read_property(P::SCALE, None).unwrap(),
            PropertyValue::ApplicationData(vec![0x0C, 0x3F, 0x80, 0x00, 0x00])
        );
        assert_eq!(
            object.read_property(P::PRESCALE, None).unwrap(),
            PropertyValue::ApplicationData(vec![0x09, 0x01, 0x19, 0x64])
        );
        // Scale, Prescale, and Property_List-adjacent scalars are not
        // BACnetARRAY rows, so the service gate rejects an index on them.
        assert!(!crate::property_metadata::is_array_property(
            &object,
            P::SCALE
        ));
        assert!(!crate::property_metadata::is_array_property(
            &object,
            P::PRESCALE
        ));
        assert!(!crate::property_metadata::is_array_property(
            &object,
            P::PRESENT_VALUE
        ));
    }

    #[test]
    fn property_metadata_accumulator_write_capabilities_match_dispatch() {
        let writable = [
            P::DESCRIPTION,
            P::OUT_OF_SERVICE,
            P::MAX_PRES_VALUE,
            P::LIMIT_MONITORING_INTERVAL,
        ];
        for out_of_service in [false, true] {
            let mut object =
                AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
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
                } else if p == P::PULSE_RATE || p == P::PRESENT_VALUE {
                    PropertyWriteCapability::WhenOutOfService
                } else {
                    PropertyWriteCapability::ReadOnly
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    crate::property_metadata::is_writable_property(&object, p),
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
            assert!(!crate::property_metadata::is_writable_property(
                &object,
                P::OBJECT_NAME
            ));
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
            // Present_Value takes a write only out of service (Table 12-79
            // footnote 1, #2267); in service it stays the input's count.
            // The row advertises it writable in both states; dispatch gates it.
            assert!(crate::property_metadata::is_writable_property(
                &object,
                P::PRESENT_VALUE
            ));
            let written =
                object.write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(10), None);
            if out_of_service {
                written.unwrap();
            } else {
                assert_error(written.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(if out_of_service { 10 } else { 0 })
            );
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }

    #[test]
    fn property_metadata_accumulator_writes_store_verbatim_with_range_gates() {
        for out_of_service in [false, true] {
            let mut object =
                AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
            object.set_prescale(bacnet_types::constructed::BACnetPrescale {
                multiplier: 1,
                modulo_divide: 100,
            });
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
            // Pulse_Rate takes a write only out of service (Clause 12.61.20).
            let pulse_rate =
                object.write_property(P::PULSE_RATE, None, PropertyValue::Unsigned(25), None);
            if out_of_service {
                pulse_rate.unwrap();
            } else {
                assert_error(pulse_rate.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            let rate = PropertyValue::Unsigned(if out_of_service { 25 } else { 0 });
            assert_eq!(object.read_property(P::PULSE_RATE, None).unwrap(), rate);
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
            // Pulse_Rate is an Unsigned (Table 12-79): out of service the
            // REAL it was once served as is refused without touching state.
            let refusal = if out_of_service {
                ErrorCode::INVALID_DATA_TYPE
            } else {
                ErrorCode::WRITE_ACCESS_DENIED
            };
            for value in [2.5, f32::NAN] {
                assert_error(
                    object
                        .write_property(P::PULSE_RATE, None, PropertyValue::Real(value), None)
                        .unwrap_err(),
                    refusal,
                );
            }
            assert_eq!(object.read_property(P::PULSE_RATE, None).unwrap(), rate);
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
                (P::LIMIT_MONITORING_INTERVAL, PropertyValue::Enumerated(60)),
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
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
                assert!(!crate::property_metadata::is_writable_property(&object, p));
            }
        }
    }

    #[test]
    fn property_metadata_accumulator_unserved_rows_stay_unknown() {
        fn assert_unserved(object: &mut dyn BACnetObject, p: P) {
            assert!(!crate::property_metadata::is_writable_property(&*object, p));
            assert_error(
                object.read_property(p, None).unwrap_err(),
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert_error(
                object
                    .write_property(p, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }

        // Table 12-79 O rows with no read arm (plus a Pulse Converter row).
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        assert_unserved(&mut acc, P::DEVICE_TYPE);
        assert_unserved(&mut acc, P::VALUE_CHANGE_TIME);
        assert_unserved(&mut acc, P::COUNT);
        // Prescale until the application sets one: a NULL written to it is
        // refused as for any property the object doesn't have.
        assert_unserved(&mut acc, P::PRESCALE);
    }
}
