//! The bounded ReadPropertyMultiple helpers the handler tests compare with
//! the legacy unbounded one: bacnet-device-core's planning and reads, with
//! no overlay.
use super::*;
use bacnet_device_core::read::{
    NoOverlay, ReadFailure, ReadPropertyMultipleBudget, ReadScope, RpmPlan,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::PropertyReference;

/// Atomic with respect to the caller's buffer, not object read side effects.
pub(crate) fn handle_rpm_budgeted(
    db: &ObjectDatabase,
    data: &[u8],
    buf: &mut BytesMut,
    budget: ReadPropertyMultipleBudget,
) -> Result<(), ReadFailure> {
    handle_rpm_budgeted_observed(db, data, buf, budget, |_, _, _, _| {})
}

/// Decode, then evaluate without a server COV context.
pub(crate) fn handle_rpm_budgeted_observed(
    db: &ObjectDatabase,
    data: &[u8],
    buf: &mut BytesMut,
    budget: ReadPropertyMultipleBudget,
    completed: impl FnMut(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        Option<(ErrorClass, ErrorCode)>,
    ),
) -> Result<(), ReadFailure> {
    let request = ReadPropertyMultipleRequest::decode(data)
        .map_err(|error| ReadFailure::Service(error.into_request_reject()))?;
    RpmPlan::new(
        db,
        &ReadScope::UNLIMITED,
        None::<&NoOverlay>,
        &request,
        budget.max_result_elements,
    )?
    .read(
        db,
        None::<&NoOverlay>,
        buf,
        budget.max_service_ack_bytes,
        completed,
    )
}

#[path = "tests/rpm_budget.rs"]
mod tests;
