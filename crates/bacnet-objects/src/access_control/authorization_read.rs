//! Narrow database reads; only genuine property absence admits a fallback.

use super::*;
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;

pub(super) fn object(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    kind: ObjectType,
) -> Result<&dyn BACnetObject, Error> {
    db.get(&oid)
        .filter(|_| oid.object_type() == kind)
        .ok_or_else(|| crate::common::protocol_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
}

pub(super) fn problem(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    failure: AuthorizationReadFailure,
) -> AuthorizationProblem {
    AuthorizationProblem::Property {
        object,
        property,
        failure,
    }
}

pub(super) fn error(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    error: Error,
) -> AuthorizationProblem {
    let failure = match error {
        Error::Remote(RemoteError::Error { class, code }) => {
            AuthorizationReadFailure::Protocol { class, code }
        }
        other => AuthorizationReadFailure::Other(other.to_string()),
    };
    problem(object, property, failure)
}

pub(super) fn malformed(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    detail: &'static str,
) -> AuthorizationProblem {
    problem(
        object,
        property,
        AuthorizationReadFailure::Malformed(detail),
    )
}

pub(super) fn optional(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<Option<PropertyValue>, AuthorizationProblem> {
    let object = db.get(&oid).ok_or_else(|| {
        error(
            oid,
            property,
            crate::common::protocol_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        )
    })?;
    match object.read_property(property, None) {
        Ok(value) => Ok(Some(value)),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::UNKNOWN_PROPERTY
                && !object.property_list().contains(&property) =>
        {
            Ok(None)
        }
        Err(failure) => Err(error(oid, property, failure)),
    }
}

pub(super) fn required(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<PropertyValue, AuthorizationProblem> {
    optional(db, oid, property)?
        .ok_or_else(|| error(oid, property, crate::common::unknown_property_error()))
}

pub(super) fn boolean(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    fallback: AuthorizationFact<bool>,
    missing: &'static str,
) -> Result<bool, AuthorizationProblem> {
    match optional(db, oid, property)? {
        Some(PropertyValue::Boolean(value)) => Ok(value),
        Some(_) => Err(malformed(oid, property, "expected Boolean")),
        None => match fallback {
            AuthorizationFact::Known(value) => Ok(value),
            AuthorizationFact::Unknown => Err(AuthorizationProblem::MissingFact(missing)),
        },
    }
}

pub(super) fn unsigned(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Result<u64, AuthorizationProblem> {
    match required(db, oid, property)? {
        PropertyValue::Unsigned(value) => Ok(value),
        _ => Err(malformed(oid, property, "expected Unsigned")),
    }
}

pub(super) fn exemptions(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
) -> Result<Vec<AuthorizationExemption>, AuthorizationProblem> {
    let property = PropertyIdentifier::AUTHORIZATION_EXEMPTIONS;
    let Some(value) = optional(db, oid, property)? else {
        return Ok(Vec::new());
    };
    let PropertyValue::List(values) = value else {
        return Err(malformed(oid, property, "expected exemption list"));
    };
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let PropertyValue::Enumerated(raw) = value else {
            return Err(malformed(oid, property, "expected Enumerated exemption"));
        };
        if raw > AuthorizationExemption::AUTHORIZATION_DELAY.to_raw() {
            return Err(AuthorizationProblem::UnsupportedValue {
                object: oid,
                property,
                value: raw.into(),
            });
        }
        let exemption = AuthorizationExemption::from_raw(raw);
        if result.contains(&exemption) {
            return Err(malformed(oid, property, "duplicate exemption"));
        }
        result.push(exemption);
    }
    Ok(result)
}

pub(super) fn mode(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
) -> Result<AuthorizationMode, AuthorizationProblem> {
    let property = PropertyIdentifier::AUTHORIZATION_MODE;
    let PropertyValue::Enumerated(raw) = required(db, oid, property)? else {
        return Err(malformed(
            oid,
            property,
            "expected authorization mode enumeration",
        ));
    };
    if raw > AuthorizationMode::NONE.to_raw() {
        return Err(AuthorizationProblem::UnsupportedValue {
            object: oid,
            property,
            value: raw.into(),
        });
    }
    Ok(AuthorizationMode::from_raw(raw))
}
