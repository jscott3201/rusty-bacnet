//! Explicit source choice for trusted local property writes.
use bacnet_objects::{command_source::CommandOrigin, database::ObjectDatabase};
use bacnet_types::{
    enums::{ErrorClass, ErrorCode},
    error::Error,
    primitives::ObjectIdentifier,
};

/// Initiator asserted by a trusted local caller. The local Device (the
/// identity's, in a running server) owns the command in either case; an
/// object changes only its published source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCommandSource {
    /// The concrete local Device initiated the command.
    ServerDevice,
    /// An existing concrete local object initiated the command.
    Object(ObjectIdentifier),
}

pub(crate) fn resolve_local(
    db: &ObjectDatabase,
    source: LocalCommandSource,
) -> Result<CommandOrigin, Error> {
    let denied = || Error::protocol(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    // The Device this server speaks for, not the lowest one (#2227), and
    // only while its object is here.
    let owner_device = db
        .local_device()
        .identifier()
        .filter(|device| db.get(device).is_some())
        .ok_or_else(denied)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn device(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
    }

    fn denied(result: Result<CommandOrigin, Error>) -> bool {
        matches!(
            result,
            Err(Error::Remote(bacnet_types::error::RemoteError::Error { class, code }))
                if class == ErrorClass::PROPERTY && code == ErrorCode::WRITE_ACCESS_DENIED
        )
    }

    /// Bound to the identity's Device 856, the database owns a local command
    /// only while that Device object is here: not when it's empty, and not
    /// through a lower Device 5 it holds instead (#2227).
    #[test]
    fn a_bound_database_owns_a_command_only_with_the_identity_device_present() {
        let identity = bacnet_objects::DeviceIdentity::new(856, 0).unwrap();
        let mut db = ObjectDatabase::new();
        crate::local_device::bind_database(&mut db, &identity);
        assert!(denied(resolve_local(&db, LocalCommandSource::ServerDevice)));
        db.add(crate::local_device::lower_device(5)).unwrap();
        assert!(denied(resolve_local(&db, LocalCommandSource::ServerDevice)));
        db.add(crate::local_device::lower_device(856)).unwrap();
        assert_eq!(
            resolve_local(&db, LocalCommandSource::ServerDevice).unwrap(),
            CommandOrigin::Local {
                owner_device: device(856),
                initiating_object: None,
            }
        );
    }
}
