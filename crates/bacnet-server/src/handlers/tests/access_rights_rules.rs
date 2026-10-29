//! Access Rights Positive_Access_Rules and Negative_Access_Rules over the
//! wire (#1316): ReadProperty and ReadPropertyMultiple agree on each
//! BACnetAccessRule array whole, its size at index 0, each element and
//! INVALID_ARRAY_INDEX past the end, and ReadPropertyMultiple ALL carries
//! both whole. `access_rights_rule_writes.rs` covers writes of the arrays
//! and of Enable.

use super::access_control_arrays::{array_cases, assert_reads, db_with};
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::access_control::AccessRightsObject;
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    PropertyReference, ReadAccessSpecification,
};

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// SPECIFIED Schedule 1 Present_Value, SPECIFIED Access Point 2, enabled.
pub(super) const BUSINESS_HOURS: &[u8] = &[
    0x09, 0x00, 0x1E, 0x0C, 0x04, 0x40, 0x00, 0x01, 0x19, 0x55, 0x1F, //
    0x29, 0x00, 0x3E, 0x1C, 0x08, 0x40, 0x00, 0x02, 0x3F, 0x49, 0x01,
];
/// ALWAYS, ALL, disabled.
pub(super) const ANYWHERE_OFF: &[u8] = &[0x09, 0x01, 0x29, 0x01, 0x49, 0x00];
/// ALWAYS, SPECIFIED Access Zone 3 in Device 99, enabled.
pub(super) const REMOTE_ZONE: &[u8] = &[
    0x09, 0x01, 0x29, 0x00, 0x3E, 0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x09, 0x00, 0x00, 0x03, 0x3F,
    0x49, 0x01,
];

pub(super) fn configured() -> AccessRightsObject {
    let mut rights = AccessRightsObject::new(7, "AR-7").unwrap();
    rights
        .set_positive_access_rules([
            BACnetAccessRule::new(
                Some(BACnetDeviceObjectPropertyReference::new_local(
                    oid(ObjectType::SCHEDULE, 1),
                    P::PRESENT_VALUE.to_raw(),
                )),
                Some(oid(ObjectType::ACCESS_POINT, 2).into()),
                true,
            ),
            BACnetAccessRule::new(None, None, false),
        ])
        .unwrap();
    rights
        .set_negative_access_rules([BACnetAccessRule::new(
            None,
            Some(BACnetDeviceObjectReference {
                device_identifier: Some(oid(ObjectType::DEVICE, 99)),
                object_identifier: oid(ObjectType::ACCESS_ZONE, 3),
            }),
            true,
        )])
        .unwrap();
    rights
}

#[test]
fn access_rights_rule_arrays_read_per_index() {
    let (db, rights) = db_with(Box::new(AccessRightsObject::new(7, "AR-7").unwrap()));
    let mut cases = array_cases(P::POSITIVE_ACCESS_RULES, &[]);
    cases.extend(array_cases(P::NEGATIVE_ACCESS_RULES, &[]));
    assert_reads(&db, rights, &cases);

    let (db, rights) = db_with(Box::new(configured()));
    let mut cases = array_cases(P::POSITIVE_ACCESS_RULES, &[BUSINESS_HOURS, ANYWHERE_OFF]);
    cases.extend(array_cases(P::NEGATIVE_ACCESS_RULES, &[REMOTE_ZONE]));
    assert_reads(&db, rights, &cases);
}

#[test]
fn rpm_all_carries_both_configured_rule_arrays() {
    let (db, rights) = db_with(Box::new(configured()));
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: rights,
            list_of_property_references: vec![PropertyReference {
                property_identifier: P::ALL,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut response = BytesMut::new();
    handle_read_property_multiple(&db, &request, &mut response).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&response).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    for (property, octets) in [
        (
            P::POSITIVE_ACCESS_RULES,
            [BUSINESS_HOURS, ANYWHERE_OFF].concat(),
        ),
        (P::NEGATIVE_ACCESS_RULES, REMOTE_ZONE.to_vec()),
    ] {
        let found: Vec<_> = results
            .iter()
            .filter(|result| result.property_identifier == property)
            .collect();
        assert_eq!(found.len(), 1, "{property:?}");
        assert_eq!(found[0].property_array_index, None, "{property:?}");
        assert_eq!(found[0].error, None, "{property:?}");
        assert_eq!(
            found[0].property_value.as_deref(),
            Some(octets.as_slice()),
            "{property:?}"
        );
    }
}
