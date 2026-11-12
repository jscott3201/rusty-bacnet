//! The message text an intrinsic transition commits.

use alloc::format;
use alloc::string::String;
use bacnet_objects::event::EventStateChange;
use bacnet_types::primitives::ObjectIdentifier;

/// Select the server-owned message for one intrinsic transition.
pub fn intrinsic_event_message_text(
    object_identifier: &ObjectIdentifier,
    change: &EventStateChange,
) -> String {
    format!("{object_identifier}: {} -> {}", change.from, change.to)
}
