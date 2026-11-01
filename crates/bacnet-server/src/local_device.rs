//! Startup identity resolution and the frozen announcement target.
use bacnet_objects::{DeviceIdentity, database::ObjectDatabase};
use bacnet_types::{error::Error, primitives::ObjectIdentifier};

/// Only an absent database requests generation. An explicitly empty database
/// stays empty; a supplied Device is checked without rebuilding its state.
pub(crate) fn prepare_database(
    config: &mut crate::server::ServerConfig,
    database: Option<ObjectDatabase>,
    local_receive_capacity: u16,
) -> Result<ObjectDatabase, Error> {
    config.device = config.device.resolve(local_receive_capacity)?;
    let database = match database {
        Some(database) => database,
        None => config
            .device
            .clone()
            .with_services(bacnet_objects::device::EXECUTED_SERVICES)
            .build_database()?,
    };
    validate_startup(&database, &config.device)?;
    Ok(database)
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
