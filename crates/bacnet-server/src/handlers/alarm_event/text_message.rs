//! ConfirmedTextMessage service handler.
//!
//! One file per service so the Batch 4 handler PRs do not serialize
//! on a single module.

use super::super::*;

/// Handle a ConfirmedTextMessage request.
///
/// Returns the decoded request for the application layer.
pub fn handle_text_message(
    service_data: &[u8],
) -> Result<bacnet_services::text_message::TextMessageRequest, Error> {
    bacnet_services::text_message::TextMessageRequest::decode(service_data)
}
