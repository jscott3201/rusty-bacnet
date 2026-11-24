//! Startup identity resolution and the frozen announcement target.
use bacnet_objects::{DeviceIdentity, database::ObjectDatabase};
use bacnet_types::{error::Error, primitives::ObjectIdentifier};

use crate::server::StartError;

/// Only an absent database requests generation, its Device kept at the
/// configured storage path, if any (#2730). An explicitly empty database
/// stays empty; a supplied Device is checked without rebuilding its state,
/// and a storage path beside it is refused. A refusal hands the database
/// back, the stock one included once it is made (#2752).
pub(crate) fn prepare_database(
    config: &mut crate::server::ServerConfig,
    mut database: Option<ObjectDatabase>,
    local_receive_capacity: u16,
) -> Result<ObjectDatabase, StartError> {
    let refused = StartError::taking(&mut database);
    config.device = config
        .device
        .resolve(local_receive_capacity)
        .map_err(refused)?;
    let database = match database {
        Some(database) => {
            let unused = crate::server::device_storage::refuse_unused_path(
                config,
                "the database the builder was given",
            );
            if let Err(error) = unused {
                return Err(StartError::new(error, Some(database)));
            }
            database
        }
        None => {
            let identity = config
                .device
                .clone()
                .with_services(bacnet_objects::device::EXECUTED_SERVICES);
            crate::server::device_storage::stock_database(config, &identity)
                .map_err(|error| StartError::new(error, None))?
        }
    };
    match validate_startup(&database, &config.device) {
        Ok(()) => Ok(database),
        Err(error) => Err(StartError::new(error, Some(database))),
    }
}

/// Check the global selected Device once before I/O; do not change selection.
pub(crate) fn validate_startup(
    db: &ObjectDatabase,
    identity: &DeviceIdentity,
) -> Result<(), Error> {
    identity.validate()?;
    if let Some(oid) = db.selected_device() {
        identity.validate_device(db.get(&oid).expect("selected under the database guard"))?;
    }
    Ok(())
}

/// The Device this server names as the initiator of every notification it
/// sends (COV, COV-multiple, event, audit) and recognises as itself among an
/// event's recipients: the one its frozen identity names, as I-Am and I-Have
/// do ([`announcement_device`](bacnet_device_core::discovery::announcement_device)),
/// never whichever Device has the lowest instance (#2217).
///
/// Unlike I-Am and I-Have, a notification still goes out while that Device
/// object is missing from the database. It names the identity's instance
/// regardless: that is the device Who-Is was answered for, and the identity
/// can't change while the server runs.
pub(crate) fn initiating_device(identity: &DeviceIdentity) -> ObjectIdentifier {
    identity.device_oid()
}

/// Bind the database to [`initiating_device`] for the server's life, so its
/// [`local_device`](ObjectDatabase::local_device) is this device for every
/// locality check, Value_Source owner, BUFFER_READY reference, event-log
/// de-duplication and live Device list, in this crate and below it, even once
/// the application adds a lower-instance Device (#2227).
pub(crate) fn bind_database(db: &mut ObjectDatabase, identity: &DeviceIdentity) {
    db.bind_local_device_internal(Some(initiating_device(identity)));
}

/// Device `instance`, named after it, for a test to add below the identity's
/// Device once the server runs (startup refuses a database whose lowest
/// Device isn't the identity's).
#[cfg(test)]
pub(crate) fn lower_device(instance: u32) -> Box<bacnet_objects::device::DeviceObject> {
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    Box::new(
        DeviceObject::new(DeviceConfig {
            identity: DeviceIdentity::new(instance, 0)
                .unwrap()
                .with_name(format!("Lower-Device-{instance}")),
            ..DeviceConfig::default()
        })
        .unwrap(),
    )
}
