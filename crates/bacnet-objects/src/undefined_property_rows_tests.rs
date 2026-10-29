//! Rows the Clause 12 property tables don't define (#1064).
//!
//! A sweep of every object type's served properties against its table found
//! these rows, all carried over from the 0.1.0 import. Each object below has
//! lost the listed rows: they are absent from Property_List and the metadata,
//! report no write route, and a read or a write of any datatype fails with
//! PROPERTY / UNKNOWN_PROPERTY. Where the object keeps Status_Flags, a refused
//! Out_Of_Service write leaves the OUT_OF_SERVICE flag clear, as the object's
//! Status_Flags description requires for a type with no Out_Of_Service.

use crate::access_control::{
    AccessCredentialObject, AccessPointObject, AccessRightsObject, AccessUserObject,
    AccessZoneObject,
};
use crate::averaging::AveragingObject;
use crate::command::CommandObject;
use crate::event_enrollment::EventEnrollmentObject;
use crate::event_log::EventLogObject;
use crate::file::FileObject;
use crate::group::{GroupObject, StructuredViewObject};
use crate::load_control::LoadControlObject;
use crate::notification_class::NotificationClass;
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventType, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{PropertyValue, StatusFlags};

const STATUS_ROWS: &[P] = &[P::STATUS_FLAGS, P::RELIABILITY, P::OUT_OF_SERVICE];

/// Each object paired with the rows its table doesn't define.
fn undefined_rows() -> Vec<(Box<dyn BACnetObject>, &'static [P])> {
    vec![
        (
            Box::new(EventLogObject::new(1, "EL-1", 8).unwrap()),
            &[P::OUT_OF_SERVICE, P::LOG_INTERVAL],
        ),
        (
            Box::new(CommandObject::new(1, "CMD-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(
                EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap(),
            ),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(NotificationClass::new(1, "NC-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(LoadControlObject::new(1, "LC-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(FileObject::new(1, "FILE-1", "raw").unwrap()),
            STATUS_ROWS,
        ),
        (Box::new(GroupObject::new(1, "G-1").unwrap()), STATUS_ROWS),
        (
            Box::new(StructuredViewObject::new(1, "SV-1").unwrap()),
            STATUS_ROWS,
        ),
        (
            Box::new(AveragingObject::new(1, "AVG-1").unwrap()),
            &[
                P::PRESENT_VALUE,
                P::STATUS_FLAGS,
                P::EVENT_STATE,
                P::RELIABILITY,
                P::OUT_OF_SERVICE,
            ],
        ),
        (
            Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(AccessRightsObject::new(1, "AR-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(AccessUserObject::new(1, "USER-1").unwrap()),
            &[
                P::PRESENT_VALUE,
                P::ASSIGNED_ACCESS_RIGHTS,
                P::OUT_OF_SERVICE,
            ],
        ),
        (
            Box::new(AccessPointObject::new(1, "AP-1").unwrap()),
            &[P::PRESENT_VALUE],
        ),
        (
            Box::new(AccessZoneObject::new(1, "AZ-1").unwrap()),
            &[P::PRESENT_VALUE, P::ACCESS_DOORS],
        ),
    ]
}

fn assert_unknown_property(result: Result<(), Error>, context: &str) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY, "{context}");
            assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY, "{context}");
        }
        other => panic!("{context}: expected PROPERTY / UNKNOWN_PROPERTY, got {other:?}"),
    }
}

#[test]
fn rows_outside_the_property_tables_are_unknown() {
    for (mut object, rows) in undefined_rows() {
        let kind = object.object_identifier().object_type();
        let metadata = object.property_metadata().into_owned();
        let list = object.property_list().into_owned();
        for &property in rows {
            let context = format!("{kind:?} {property:?}");
            assert!(!list.contains(&property), "{context} in Property_List");
            assert!(
                !metadata
                    .iter()
                    .any(|row| row.property_identifier == property),
                "{context} in metadata"
            );
            assert!(!object.is_writable_property(property), "{context} writable");
            assert_unknown_property(object.read_property(property, None).map(|_| ()), &context);
            for value in [
                PropertyValue::Boolean(true),
                PropertyValue::Enumerated(1),
                PropertyValue::Unsigned(1),
                PropertyValue::Real(1.0),
                PropertyValue::Null,
            ] {
                assert_unknown_property(
                    object.write_property(property, None, value, None),
                    &context,
                );
            }
        }
        assert_eq!(object.property_metadata().as_ref(), metadata, "{kind:?}");
        if !rows.contains(&P::STATUS_FLAGS) {
            match object.read_property(P::STATUS_FLAGS, None).unwrap() {
                PropertyValue::BitString { data, .. } => assert_eq!(
                    data[0] & (StatusFlags::OUT_OF_SERVICE.bits() << 4),
                    0,
                    "{kind:?} OUT_OF_SERVICE flag"
                ),
                other => panic!("{kind:?} Status_Flags must be a bit string, got {other:?}"),
            }
        }
    }
}
