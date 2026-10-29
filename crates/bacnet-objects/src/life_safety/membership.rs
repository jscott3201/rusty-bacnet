//! Member_Of and Zone_Members (#1182): each a BACnetLIST of
//! `BACnetDeviceObjectReference`, so a member held by another device keeps
//! its Device member. The application fills them; both are read-only over the
//! network.

use crate::prelude::*;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;

use crate::common;
use crate::device_reference::check_device_member;

/// What a Member_Of entry names: a Life Safety Zone (Clauses 12.15.29 and
/// 12.16.27).
pub(super) const ZONES: &[ObjectType] = &[ObjectType::LIFE_SAFETY_ZONE];

/// What a Zone_Members entry names: a Life Safety Point or Zone (Clause
/// 12.16.26).
pub(super) const ZONE_MEMBERS: &[ObjectType] =
    &[ObjectType::LIFE_SAFETY_POINT, ObjectType::LIFE_SAFETY_ZONE];

/// Add `reference` to `list` unless it is there already.
///
/// An object of a type outside `types`, or a Device member that isn't a
/// Device identifier, fails with PROPERTY / VALUE_OUT_OF_RANGE and leaves the
/// list unchanged.
pub(super) fn add(
    list: &mut Vec<BACnetDeviceObjectReference>,
    reference: BACnetDeviceObjectReference,
    types: &[ObjectType],
) -> Result<(), Error> {
    check_device_member(reference.device_identifier)?;
    if !types.contains(&reference.object_identifier.object_type()) {
        return Err(common::value_out_of_range_error());
    }
    if !list.contains(&reference) {
        list.push(reference);
    }
    Ok(())
}
