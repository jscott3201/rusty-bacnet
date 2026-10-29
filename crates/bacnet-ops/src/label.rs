//! How objects, properties and command priorities are named in text, the
//! same in every front end: enumerations in kebab case, objects as
//! `analog-input 1` (decision D4 of #1887).

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

/// Number of command priorities (Clause 19.2).
pub const PRIORITIES: usize = 16;

/// The standard use of each command priority, paraphrasing Table 19-1:
/// six are assigned, and the rest are left to the site.
pub fn priority_label(priority: usize) -> &'static str {
    match priority {
        1 => "life safety, manual",
        2 => "life safety, automatic",
        5 => "critical equipment",
        6 => "minimum on/off time",
        8 => "operator, manual",
        _ => "site defined",
    }
}
