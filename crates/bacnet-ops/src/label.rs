//! How objects and properties are named in text, the same in every front
//! end: enumerations in kebab case, objects as `analog-input 1` (decision
//! D4 of #1887). A command priority names its use with
//! [`Priority::purpose`](crate::Priority::purpose).

use bacnet_types::primitives::ObjectIdentifier;

/// `ANALOG_INPUT` as `analog-input`.
pub fn kebab(name: impl std::fmt::Display) -> String {
    name.to_string().to_lowercase().replace('_', "-")
}

/// An object as the list shows it, such as `analog-input 1`.
pub fn object_label(object: ObjectIdentifier) -> String {
    format!(
        "{} {}",
        kebab(object.object_type()),
        object.instance_number()
    )
}
