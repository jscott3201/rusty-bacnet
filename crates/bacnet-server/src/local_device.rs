//! Startup identity resolution and the frozen announcement target.
use bacnet_objects::{DeviceIdentity, database::ObjectDatabase};
use bacnet_types::error::Error;

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
