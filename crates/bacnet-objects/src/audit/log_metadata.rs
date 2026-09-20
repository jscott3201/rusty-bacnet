use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

use super::AuditLogObject;

// Canonical effective rows for Audit Log (type 61, Clause 12.64 Table 12-83).
// Order preserves the legacy property_list projection; PROPERTY_LIST is
// appended so the projection helper omits it while required_properties keeps
// it. Only implemented rows are described: table rows the object does not
// serve (Log_Buffer, intrinsic-reporting,
// Audit_Level, Tags, Profile_*) stay absent until dispatch exists. Enable
// carries the table W code; Description carries the table O code and the only
// other network write route.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LOG_ENABLE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::BUFFER_SIZE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RECORD_COUNT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::TOTAL_RECORD_COUNT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(object: &AuditLogObject) -> Cow<'_, [PropertyMetadata]> {
    if object.forwarding.is_none() {
        return Cow::Borrowed(BASE);
    }
    let mut rows = BASE.to_vec();
    for p in [
        P::MEMBER_OF,
        P::DELETE_ON_FORWARD,
        P::ISSUE_CONFIRMED_NOTIFICATIONS,
        P::RELIABILITY,
    ] {
        rows.push(PropertyMetadata::new(p, Optional, None, ReadOnly));
    }
    Cow::Owned(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

    use crate::clock::{ClockFrame, ClockReader};
    use crate::traits::BACnetObject;

    use super::super::{AuditLogPersistence, AuditLogSnapshot};

    #[derive(Default)]
    struct MemoryPersistence {
        snapshot: Mutex<Option<AuditLogSnapshot>>,
    }

    impl AuditLogPersistence for MemoryPersistence {
        fn load(
            &self,
            _expected_object: ObjectIdentifier,
        ) -> Result<Option<AuditLogSnapshot>, Error> {
            Ok(self.snapshot.lock().unwrap().clone())
        }

        fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
            *self.snapshot.lock().unwrap() = Some(snapshot.clone());
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FixedClock(Option<ClockFrame>);

    impl ClockReader for FixedClock {
        fn read_clock(&self) -> Option<ClockFrame> {
            self.0
        }
    }

    fn frame() -> ClockFrame {
        ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        }
    }

    fn log() -> AuditLogObject {
        AuditLogObject::new(1, "AL-1", 4, Arc::new(MemoryPersistence::default())).unwrap()
    }

    #[test]
    fn forwarding_properties_are_conditional_read_only_and_instance_owned() {
        use bacnet_types::constructed::BACnetDeviceObjectReference;
        use bacnet_types::enums::Reliability;
        let mut object = log();
        let parent = BACnetDeviceObjectReference {
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
            object_identifier: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 2).unwrap(),
        };
        let properties = [
            P::MEMBER_OF,
            P::DELETE_ON_FORWARD,
            P::ISSUE_CONFIRMED_NOTIFICATIONS,
            P::RELIABILITY,
        ];
        for p in properties {
            assert!(!object.property_list().contains(&p));
            assert_error(
                object.read_property(p, None).unwrap_err(),
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
        object.set_member_of(Some(parent.clone()));
        for p in properties {
            assert!(object.property_list().contains(&p));
            assert!(!object.is_writable_property(p));
            assert_error(
                object
                    .write_property(p, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        // Independent context-tagged DeviceObjectReference: [0] Device:9, [1] AuditLog:2.
        assert_eq!(
            object.read_property(P::MEMBER_OF, None).unwrap(),
            PropertyValue::ApplicationData(vec![0x0c, 0x02, 0, 0, 9, 0x1c, 0x0f, 0x40, 0, 2])
        );
        assert_eq!(
            object.read_property(P::DELETE_ON_FORWARD, None).unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            object
                .read_property(P::ISSUE_CONFIRMED_NOTIFICATIONS, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        let old = object.audit_log_forwarding_internal().unwrap();
        assert_eq!(
            object.read_property(P::RELIABILITY, None).unwrap(),
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
        );
        old.status().set_configured(true);
        let epoch = old.status().begin_delivery();
        old.status().complete_delivery(epoch, false);
        assert_eq!(
            object.read_property(P::STATUS_FLAGS, None).unwrap(),
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0x40]
            }
        );
        object.set_member_of(Some(parent));
        let new = object.audit_log_forwarding_internal().unwrap();
        new.status().set_configured(true);
        old.status().complete_delivery(epoch, false);
        assert_eq!(
            object.read_property(P::RELIABILITY, None).unwrap(),
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
        );
        object.set_member_of(None);
        assert!(!object.property_list().contains(&P::MEMBER_OF));
    }

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn forwarding_parent_configuration_through_object_capability_is_opt_in() {
        use bacnet_types::constructed::BACnetDeviceObjectReference;
        let parent = BACnetDeviceObjectReference {
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
            object_identifier: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 2).unwrap(),
        };
        let mut object: Box<dyn BACnetObject> = Box::new(log());
        object
            .set_audit_log_parent_internal(parent.clone())
            .unwrap();
        assert_eq!(
            object.audit_log_forwarding_internal().unwrap().parent(),
            &parent
        );
        assert_eq!(
            object.read_property(P::MEMBER_OF, None).unwrap(),
            PropertyValue::ApplicationData(vec![0x0c, 0x02, 0, 0, 9, 0x1c, 0x0f, 0x40, 0, 2])
        );
        assert!(!object.is_writable_property(P::MEMBER_OF));

        let mut other: Box<dyn BACnetObject> =
            Box::new(crate::binary::BinaryValueObject::new(1, "Other").unwrap());
        assert!(matches!(
            other.set_audit_log_parent_internal(parent),
            Err(Error::Protocol { class, code })
                if class == ErrorClass::OBJECT.to_raw() as u32
                    && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32
        ));
        assert!(other.audit_log_forwarding_internal().is_none());
    }

    #[test]
    fn property_metadata_log_exact_sets_and_indexed_list() {
        let object = log();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::LOG_ENABLE,
            P::BUFFER_SIZE,
            P::RECORD_COUNT,
            P::TOTAL_RECORD_COUNT,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::LOG_ENABLE,
            P::BUFFER_SIZE,
            P::RECORD_COUNT,
            P::TOTAL_RECORD_COUNT,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::PROPERTY_LIST,
        ];
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
        assert!(!object.supports_cov());
        for row in metadata.iter() {
            let expected = if row.property_identifier == P::LOG_ENABLE {
                RequiredWrite
            } else if required.contains(&row.property_identifier) {
                RequiredRead
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
            assert_eq!(
                row.presence_condition, None,
                "{:?}",
                row.property_identifier
            );
            object.read_property(row.property_identifier, None).unwrap();
        }
        assert!(!required.contains(&P::DESCRIPTION));

        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 7);
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
    fn property_metadata_log_write_capabilities_match_dispatch() {
        let mut object = log();
        let metadata = object.property_metadata().into_owned();
        for row in &metadata {
            let p = row.property_identifier;
            let writable = matches!(p, P::LOG_ENABLE | P::DESCRIPTION);
            assert_eq!(
                row.write_capability,
                if writable { Always } else { ReadOnly },
                "{p:?}"
            );
            assert_eq!(object.is_writable_property(p), writable, "{p:?}");
            // Writing back the current value is a no-op success for both
            // writable rows and needs no clock; read-only rows deny access.
            let before = object.read_property(p, None).unwrap();
            let result = object.write_property(p, None, before.clone(), None);
            if writable {
                result.unwrap();
            } else {
                assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert_eq!(object.read_property(p, None).unwrap(), before);
        }
        for (property, value, error) in [
            (
                P::LOG_ENABLE,
                PropertyValue::Unsigned(0),
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                P::DESCRIPTION,
                PropertyValue::Boolean(false),
                ErrorCode::INVALID_DATA_TYPE,
            ),
        ] {
            assert_error(
                object
                    .write_property(property, None, value, None)
                    .unwrap_err(),
                error,
            );
        }
        // The metadata fixes two legacy-default drifts: Object_Name is not
        // writable and Enable is, matching the write_property arms.
        assert!(!object.is_writable_property(P::OBJECT_NAME));
        assert!(object.is_writable_property(P::LOG_ENABLE));
        for p in [
            P::OBJECT_NAME,
            P::RECORD_COUNT,
            P::LOG_BUFFER,
            P::RELIABILITY,
        ] {
            assert!(!object.is_writable_property(p));
            assert_error(
                object
                    .write_property(p, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        assert_error(
            object.read_property(P::LOG_BUFFER, None).unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_error(
            object.read_property(P::RELIABILITY, None).unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_eq!(object.property_metadata().as_ref(), metadata);
    }

    #[test]
    fn property_metadata_log_enable_write_round_trips_with_clock() {
        use bacnet_types::enums::ErrorClass;

        let mut object = log();
        let error = object
            .write_property(P::LOG_ENABLE, None, PropertyValue::Boolean(false), None)
            .unwrap_err();
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::DEVICE.to_raw() as u32
                    && code == ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32),
            "expected DEVICE/OPERATIONAL_PROBLEM, got {error:?}"
        );
        assert!(object.log_enable());

        object.bind_clock_internal(Some(Arc::new(FixedClock(Some(frame())))));
        object
            .write_property(P::LOG_ENABLE, None, PropertyValue::Boolean(false), None)
            .unwrap();
        assert!(!object.log_enable());
        object
            .write_property(P::LOG_ENABLE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        assert!(object.log_enable());
        assert_eq!(
            object.read_property(P::TOTAL_RECORD_COUNT, None).unwrap(),
            PropertyValue::Unsigned(2)
        );
        assert_eq!(
            object.object_identifier().object_type(),
            ObjectType::AUDIT_LOG
        );
    }
}
