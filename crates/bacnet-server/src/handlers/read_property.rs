use super::*;
use bacnet_device_core::read::{self, NoOverlay, ReadFailure, ReadScope};

/// The handler tests' ReadProperty: bacnet-device-core's staged read with no
/// overlay and no work limit, writing the ReadPropertyACK service bytes. A
/// built-in Device follows its declared service profile, including absent or
/// empty COV lists; running server reads use the executor's view instead.
pub(crate) fn handle_read_property(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyRequest::decode(service_data).map_err(Error::into_request_reject)?;
    let scope = ReadScope::UNLIMITED;
    let lookup_oid = read::resolve_target(db, &scope, &request.object_identifier);
    let plan = read::plan_property(
        db,
        &scope,
        None::<&NoOverlay>,
        lookup_oid,
        request.property_identifier,
        request.property_array_index,
    )
    .map_err(unlimited_read)?;
    read::read_property_ack(db, None::<&NoOverlay>, &request, plan, buf, |_, _, _| {})
}

/// The handler tests' ReadPropertyMultiple: bacnet-device-core's planned
/// read with no overlay and no work or byte limit, writing the
/// ReadPropertyMultiple-ACK service bytes. Running servers read within their
/// configured budget and through the executor's view instead.
pub(crate) fn handle_read_property_multiple(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let unlimited = bacnet_device_core::read::ReadPropertyMultipleBudget {
        max_result_elements: usize::MAX,
        max_service_ack_bytes: usize::MAX,
    };
    super::rpm_budget::handle_rpm_budgeted(db, service_data, buf, unlimited).map_err(unlimited_read)
}

/// A read with no limit fails only with a service error.
fn unlimited_read(failure: ReadFailure) -> Error {
    match failure {
        ReadFailure::Service(error) => error,
        _ => unreachable!("a read with no limit ran past one"),
    }
}

#[path = "tests/rpm_result_index.rs"]
mod rpm_result_index_tests;
