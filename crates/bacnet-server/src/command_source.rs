//! Explicit source choice for trusted local property writes.
use bacnet_objects::{command_source::CommandOrigin, database::ObjectDatabase};
use bacnet_types::{
    enums::{ErrorClass, ErrorCode},
    error::Error,
    primitives::ObjectIdentifier,
};

/// Initiator asserted by a trusted local caller. The selected local Device owns
/// the command in either case; an object changes only its published source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCommandSource {
    /// The selected concrete local Device initiated the command.
    ServerDevice,
    /// An existing concrete local object initiated the command.
    Object(ObjectIdentifier),
}

pub(crate) fn resolve_local(
    db: &ObjectDatabase,
    source: LocalCommandSource,
) -> Result<CommandOrigin, Error> {
    let denied = || Error::Protocol {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::WRITE_ACCESS_DENIED,
    };
    let owner_device = db.local_device().identifier().ok_or_else(denied)?;
    let initiating_object = match source {
        LocalCommandSource::ServerDevice => None,
        LocalCommandSource::Object(oid) => {
            if db.get(&oid).is_none() {
                return Err(denied());
            }
            Some(oid)
        }
    };
    let origin = CommandOrigin::Local {
        owner_device,
        initiating_object,
    };
    origin.validate()?;
    Ok(origin)
}

/// On absent/invalid local identity, context-free dispatch preserves unrelated
/// property writes while first-party tracked commands fail closed in the object.
pub(crate) fn write_target(
    object: &mut dyn bacnet_objects::traits::BACnetObject,
    property: bacnet_types::enums::PropertyIdentifier,
    index: Option<u32>,
    value: bacnet_types::primitives::PropertyValue,
    priority: Option<u8>,
    origin: Option<&CommandOrigin>,
) -> Result<(), Error> {
    match origin {
        Some(origin) => object.write_property_from(property, index, value, priority, origin),
        None => object.write_property(property, index, value, priority),
    }
}

#[cfg(test)]
pub(crate) fn test_origin() -> bacnet_objects::command_source::CommandOrigin {
    bacnet_objects::command_source::CommandOrigin::Local {
        owner_device: bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::DEVICE,
            1,
        )
        .unwrap(),
        initiating_object: None,
    }
}
