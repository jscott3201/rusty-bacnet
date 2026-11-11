//! A command origin for the server's tests. The local source a trusted
//! caller asserts, and its resolution, are
//! `bacnet_device_core::write::local`'s (#2446).

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

mod tests {
    use bacnet_device_core::write::local::{LocalCommandSource, resolve_source as resolve_local};
    use bacnet_objects::{command_source::CommandOrigin, database::ObjectDatabase};
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::{error::Error, primitives::ObjectIdentifier};

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
