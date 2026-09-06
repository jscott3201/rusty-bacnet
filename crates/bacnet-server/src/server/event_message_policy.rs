use bacnet_objects::event::EventStateChange;
use bacnet_types::primitives::ObjectIdentifier;

/// Select the server-owned message for one built-in intrinsic transition.
pub(super) fn intrinsic_event_message_text(
    object_identifier: &ObjectIdentifier,
    change: &EventStateChange,
) -> String {
    format!("{object_identifier}: {} -> {}", change.from, change.to)
}
