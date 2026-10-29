//! Narrow local property/reference reads for the accompaniment stage.
use super::*;
use crate::device_reference::{check_device_member, decode_reference, decode_references};
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;

fn property_problem(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    detail: impl ToString,
) -> AccompanimentProblem {
    AccompanimentProblem::Property {
        object,
        property,
        detail: detail.to_string(),
    }
}
fn reference_problem(
    reference: &BACnetDeviceObjectReference,
    reason: AccompanimentReferenceFailure,
) -> AccompanimentProblem {
    AccompanimentProblem::Reference {
        reference: reference.clone(),
        reason,
    }
}
fn optional(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<Option<PropertyValue>, AccompanimentProblem> {
    let object = db
        .get(&oid)
        .ok_or_else(|| property_problem(oid, property, "selected object is missing"))?;
    match object.read_property(property, None) {
        Ok(value) => Ok(Some(value)),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::UNKNOWN_PROPERTY
                && !object.property_list().contains(&property) =>
        {
            Ok(None)
        }
        Err(error) => Err(property_problem(oid, property, error)),
    }
}
pub(super) fn requirement(
    db: &ObjectDatabase,
    rights: ObjectIdentifier,
) -> Result<Option<BACnetDeviceObjectReference>, AccompanimentProblem> {
    let property = PropertyIdentifier::ACCOMPANIMENT;
    let Some(value) = optional(db, rights, property)? else {
        return Ok(None);
    };
    let reference: BACnetDeviceObjectReference =
        decode_reference(&value).map_err(|error| property_problem(rights, property, error))?;
    check_device_member(reference.device_identifier)
        .map_err(|error| property_problem(rights, property, error))?;
    if crate::access_control::rights::unspecified(
        reference.object_identifier,
        reference.device_identifier,
    ) {
        Ok(None)
    } else {
        Ok(Some(reference))
    }
}
pub(super) fn period(
    db: &ObjectDatabase,
    point: ObjectIdentifier,
    fallback: Duration,
) -> Result<Duration, AccompanimentProblem> {
    match optional(db, point, PropertyIdentifier::ACCOMPANIMENT_TIME)? {
        None => Ok(fallback),
        Some(PropertyValue::Unsigned(seconds)) => Ok(Duration::from_secs(seconds)),
        Some(_) => Err(property_problem(
            point,
            PropertyIdentifier::ACCOMPANIMENT_TIME,
            "expected Unsigned seconds",
        )),
    }
}
fn local(
    db: &ObjectDatabase,
    reference: &BACnetDeviceObjectReference,
) -> Result<(), AccompanimentProblem> {
    if check_device_member(reference.device_identifier).is_err() {
        return Err(reference_problem(
            reference,
            AccompanimentReferenceFailure::WrongType,
        ));
    }
    if reference.object_identifier.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
        || reference
            .device_identifier
            .is_some_and(|device| device.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE)
    {
        return Err(reference_problem(
            reference,
            AccompanimentReferenceFailure::Wildcard,
        ));
    }
    if !db.local_device().is_local(reference.device_identifier) {
        return Err(reference_problem(
            reference,
            AccompanimentReferenceFailure::Remote,
        ));
    }
    Ok(())
}
fn target<'a>(
    db: &'a ObjectDatabase,
    reference: &BACnetDeviceObjectReference,
) -> Result<&'a dyn BACnetObject, AccompanimentProblem> {
    local(db, reference)?;
    if !matches!(
        reference.object_identifier.object_type(),
        ObjectType::ACCESS_RIGHTS | ObjectType::ACCESS_CREDENTIAL | ObjectType::ACCESS_USER
    ) {
        return Err(reference_problem(
            reference,
            AccompanimentReferenceFailure::WrongType,
        ));
    }
    db.get(&reference.object_identifier)
        .ok_or_else(|| reference_problem(reference, AccompanimentReferenceFailure::Missing))
}

/// A concrete local requirement must exist even while no candidate has arrived.
/// Otherwise an expired wait would manufacture a denial from unavailable data.
pub(super) fn available(
    db: &ObjectDatabase,
    reference: &BACnetDeviceObjectReference,
) -> Result<(), AccompanimentProblem> {
    target(db, reference).map(|_| ())
}

/// Match only the selected criterion; a match does not replace the companion's
/// own rights evaluation. Direct user ownership never traverses Members.
pub(super) fn matches(
    db: &ObjectDatabase,
    reference: &BACnetDeviceObjectReference,
    companion: ObjectIdentifier,
) -> Result<bool, AccompanimentProblem> {
    let object = target(db, reference)?;
    match reference.object_identifier.object_type() {
        ObjectType::ACCESS_CREDENTIAL => Ok(reference.object_identifier == companion),
        ObjectType::ACCESS_RIGHTS => assigned(db, reference, object, companion),
        ObjectType::ACCESS_USER => owned(db, reference, object, companion),
        _ => unreachable!("target admitted only the three requirement kinds"),
    }
}
fn assigned(
    db: &ObjectDatabase,
    reference: &BACnetDeviceObjectReference,
    rights: &dyn BACnetObject,
    companion: ObjectIdentifier,
) -> Result<bool, AccompanimentProblem> {
    let oid = reference.object_identifier;
    let enable = PropertyIdentifier::LOG_ENABLE;
    match rights.read_property(enable, None) {
        Ok(PropertyValue::Boolean(true)) => {}
        Ok(PropertyValue::Boolean(false)) => return Ok(false),
        Ok(_) => return Err(property_problem(oid, enable, "expected Boolean")),
        Err(error) => return Err(property_problem(oid, enable, error)),
    }
    let holder = db.get(&companion).ok_or_else(|| {
        reference_problem(&companion.into(), AccompanimentReferenceFailure::Missing)
    })?;
    let assignments = crate::access_control::rights_evaluation::assigned_access_rights(holder)
        .map_err(|error| {
            property_problem(companion, PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS, error)
        })?;
    // Preserve existing assignment semantics: disabled/unused/remote entries
    // do not assign this local target. Rights diagnostics remain in the result.
    Ok(assignments.iter().any(|assignment| {
        assignment.enable
            && assignment.assigned_access_rights.object_identifier == oid
            && db
                .local_device()
                .is_local(assignment.assigned_access_rights.device_identifier)
    }))
}
fn owned(
    db: &ObjectDatabase,
    reference: &BACnetDeviceObjectReference,
    user: &dyn BACnetObject,
    companion: ObjectIdentifier,
) -> Result<bool, AccompanimentProblem> {
    let oid = reference.object_identifier;
    let property = PropertyIdentifier::CREDENTIALS;
    let value = user
        .read_property(property, None)
        .map_err(|error| property_problem(oid, property, error))?;
    let entries: Vec<BACnetDeviceObjectReference> =
        decode_references(&value).map_err(|error| property_problem(oid, property, error))?;
    let mut matched = false;
    let mut unknown = None;
    for entry in entries {
        if entry.object_identifier.object_type() != ObjectType::ACCESS_CREDENTIAL
            || check_device_member(entry.device_identifier).is_err()
        {
            return Err(reference_problem(
                &entry,
                AccompanimentReferenceFailure::WrongType,
            ));
        }
        match local(db, &entry) {
            Ok(()) => matched |= entry.object_identifier == companion,
            Err(problem) => {
                unknown.get_or_insert(problem);
            }
        }
    }
    if matched {
        Ok(true)
    } else if let Some(problem) = unknown {
        Err(problem)
    } else {
        Ok(false)
    }
}
