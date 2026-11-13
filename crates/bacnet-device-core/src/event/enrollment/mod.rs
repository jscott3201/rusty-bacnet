//! Event Enrollment's algorithms (#2319, behind `event-enrollment`): the
//! event algorithms an Event Enrollment object evaluates its monitored
//! value with, its fault algorithms, and the reads of its configuration and
//! monitored reference, moved from `bacnet-server`'s evaluator unchanged.
//! The pass that drives them is still the server's.

pub mod algorithms;
pub mod fault;
pub mod reference;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};

/// Why an enrollment's own configuration couldn't be read for a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalConfigurationReadError {
    /// A required property is missing: the configuration is malformed.
    Malformed,
    /// The read failed some other way, for now.
    Unavailable,
}

/// Classify a failed read of a required property: UNKNOWN_PROPERTY makes
/// the configuration malformed, anything else leaves it unavailable.
pub fn classify_required_property_read_error(error: &Error) -> LocalConfigurationReadError {
    if matches!(
        error,
        Error::Remote(RemoteError::Error { class, code })
            if *class == ErrorClass::PROPERTY
                && *code == ErrorCode::UNKNOWN_PROPERTY
    ) {
        LocalConfigurationReadError::Malformed
    } else {
        LocalConfigurationReadError::Unavailable
    }
}

#[cfg(test)]
mod tests;
