//! LifeSafetyOperation service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;

/// Handle a LifeSafetyOperation request.
///
/// Decodes the request and returns Ok(()) for SimpleACK.
pub fn handle_life_safety_operation(service_data: &[u8]) -> Result<(), Error> {
    let _request = bacnet_services::life_safety::LifeSafetyOperationRequest::decode(service_data)?;
    Ok(())
}
