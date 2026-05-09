use super::*;

/// Handle a WriteGroup request.
///
/// Decodes the request and returns the parsed data for the server to apply.
pub fn handle_write_group(
    service_data: &[u8],
) -> Result<bacnet_services::write_group::WriteGroupRequest, Error> {
    bacnet_services::write_group::WriteGroupRequest::decode(service_data)
}
