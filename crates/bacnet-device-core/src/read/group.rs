//! A Group object's Present_Value, rebuilt from its members on every read
//! (Clause 12.14.6).
//!
//! A Group stores no Present_Value: the object can't reach its members, so
//! the read services build the list here, under the database guard the
//! request already holds.
//!
//! Work policy (#1172): a Group may list any number of members, and the
//! request that reads its Present_Value pays for them. Each member row, after
//! ALL, REQUIRED and OPTIONAL expand, counts against the same work limit as
//! the request's own rows (`ReadPropertyMultipleBudget::max_result_elements`).
//! Several Groups in one ReadPropertyMultiple share that one limit, and a
//! request that would pass it is aborted with OUT_OF_RESOURCES before any
//! member is read, the answer ReadPropertyMultiple gives any request over its
//! work limit. ReadProperty and ReadRange have no work limit of their own, so
//! they get the limit of a ReadPropertyMultiple that names only this
//! Present_Value: the row itself plus the member rows. On the shared
//! endpoint, which has no ReadPropertyMultiple budget, ReadProperty uses the
//! session's configured read work limit (#1215). The low-level helpers with
//! no executor view have no limit.
//!
//! A member may name the Device's COV subscription lists. Every read service
//! plans its rows, a Group's member rows included, before it reads a value,
//! and samples the live lists once for the request when the plan reads one,
//! so those member rows serve the request's one live snapshot (#1171, #1213).
use super::budget::{ReadFailure, Work};
use super::property;
use super::rpm::{self, PlannedObject, PlannedRow};
use super::{ReadOverlay, ReadScope};
use alloc::vec::Vec;
use bacnet_encoding::constructed::decode_read_access_specification;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::rpm::ReadAccessResult;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

/// A Group's members, planned for the request that reads its Present_Value.
/// A list that doesn't decode keeps its error for the row to report.
pub struct GroupMembers(Result<Vec<PlannedObject>, Error>);

impl GroupMembers {
    /// Every planned member row, with the member object it reads; none when
    /// the list didn't decode.
    pub(crate) fn rows(&self) -> impl Iterator<Item = (ObjectIdentifier, &PlannedRow)> {
        self.0.as_deref().into_iter().flat_map(rpm::planned_rows)
    }
}

/// Plan a single-property read (ReadProperty, ReadRange, the local read) of
/// a found object: the read's own row and, for a Group's whole
/// Present_Value, the member rows, all charged to the scope's work limit.
pub fn plan_property<O: ReadOverlay>(
    db: &ObjectDatabase,
    scope: &ReadScope,
    overlay: Option<&O>,
    object: &dyn BACnetObject,
    reference: &PropertyReference,
) -> Result<Option<GroupMembers>, ReadFailure> {
    let mut work = Work::new(scope.work_limit);
    // This read's own row, as a one-property ReadPropertyMultiple counts it.
    work.charge()?;
    plan_members(db, scope, overlay, object, reference, &mut work)
}

/// Read `property` of a found object the way the single-property services
/// serve it. A Group's whole Present_Value is rebuilt from the `members` the
/// read planned; every other read is the object's own answer (the
/// host's, when `object` is a served view).
pub fn read_served_property<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    members: Option<GroupMembers>,
) -> Result<PropertyValue, Error> {
    match members {
        Some(members) => value(db, overlay, members),
        None => object.read_property(property, array_index),
    }
}

/// The members, when `reference` reads the whole Present_Value of `object`
/// and `object` is a Group; `None` for any other row. Each member is planned
/// as ReadPropertyMultiple plans one specification, and every member row is
/// charged to `work`.
pub(crate) fn plan_members<O: ReadOverlay>(
    db: &ObjectDatabase,
    scope: &ReadScope,
    overlay: Option<&O>,
    object: &dyn BACnetObject,
    reference: &PropertyReference,
    work: &mut Work,
) -> Result<Option<GroupMembers>, ReadFailure> {
    if reference.property_identifier != PropertyIdentifier::PRESENT_VALUE
        || reference.property_array_index.is_some()
        || object.object_identifier().object_type() != ObjectType::GROUP
    {
        return Ok(None);
    }
    let planned = match members(object) {
        Ok(specs) => Ok(rpm::plan_specs(db, scope, overlay, &specs, work, false)?),
        Err(error) => Err(error),
    };
    Ok(Some(GroupMembers(planned)))
}

/// One ReadAccessResult per member, in List_Of_Group_Members order.
///
/// Each member is read as ReadPropertyMultiple reads one specification:
/// ALL, REQUIRED and OPTIONAL expand against the member, a member naming no
/// object in this device gets OBJECT / UNKNOWN_OBJECT for each reference,
/// and a property that fails to read carries its error in place of a value.
/// Member rows were planned without Group expansion, so they use each
/// object's own reader and a Group listed as a member can't make this read
/// recurse.
pub(crate) fn value<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    members: GroupMembers,
) -> Result<PropertyValue, Error> {
    let results = members
        .0?
        .into_iter()
        .map(|member| {
            let stored = property::read_target_object(db, &member.lookup_oid);
            let served = stored.and_then(|object| overlay.and_then(|overlay| overlay.view(object)));
            let object = served
                .as_ref()
                .map(|served| served as &dyn BACnetObject)
                .or(stored);
            let result = ReadAccessResult {
                object_identifier: member.lookup_oid,
                list_of_results: member
                    .properties
                    .into_iter()
                    .map(|row| rpm::read_row(db, overlay, object, row))
                    .collect(),
            };
            let mut encoded = BytesMut::new();
            result.encode(&mut encoded);
            PropertyValue::ApplicationData(encoded.to_vec())
        })
        .collect();
    Ok(PropertyValue::List(results))
}

/// The group's members, decoded from its List_Of_Group_Members elements.
/// A built-in Group always serves decodable ones; a custom object whose
/// list doesn't decode fails the read with PROPERTY / OTHER.
pub(crate) fn members(group: &dyn BACnetObject) -> Result<Vec<ReadAccessSpecification>, Error> {
    let PropertyValue::List(elements) =
        group.read_property(PropertyIdentifier::LIST_OF_GROUP_MEMBERS, None)?
    else {
        return Err(other());
    };
    elements
        .iter()
        .map(|element| match element {
            PropertyValue::ApplicationData(bytes) => {
                match decode_read_access_specification(bytes, 0) {
                    Ok((member, end)) if end == bytes.len() => Ok(member),
                    _ => Err(other()),
                }
            }
            _ => Err(other()),
        })
        .collect()
}

fn other() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::OTHER)
}
