//! How the TUI writes a property it couldn't read: in kebab case, like the
//! property names beside it (decision D4 of #1887).
//!
//! This is the TUI's rendering only. [`RowError`]'s own `Display`, which the
//! one-shot commands print, stays `CLASS:CODE`, and MCP returns the
//! structured error.

use bacnet_ops::format::RowError;
use bacnet_ops::label::kebab;
use bacnet_types::enums::ErrorClass;
use bacnet_types::error::RemoteError;

/// A row's error for the browser and the watch list: the error code alone
/// (`unknown-property`) when the class is the property's or the object's
/// own, which the code already makes plain; `class / code` otherwise
/// (`resources / no-space-to-write-property`). A reject or abort gives its
/// named reason (`rejected: unrecognized-service`).
pub(crate) fn row_error_text(error: &RowError) -> String {
    match error {
        RowError::Remote(RemoteError::Error { class, code }) => {
            if [ErrorClass::PROPERTY, ErrorClass::OBJECT].contains(class) {
                kebab(code)
            } else {
                format!("{} / {}", kebab(class), kebab(code))
            }
        }
        RowError::Remote(RemoteError::Reject { reason }) => format!("rejected: {}", kebab(reason)),
        RowError::Remote(RemoteError::Abort { reason }) => format!("aborted: {}", kebab(reason)),
        RowError::NoValue => "no value".into(),
        RowError::Other(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::{AbortReason, ErrorCode, RejectReason};

    use super::*;

    fn error(class: ErrorClass, code: ErrorCode) -> RowError {
        RowError::Remote(RemoteError::Error { class, code })
    }

    #[test]
    fn row_errors_read_in_kebab_case() {
        let unknown = error(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY);
        assert_eq!(row_error_text(&unknown), "unknown-property");
        // The one-shot commands keep the CLASS:CODE form.
        assert_eq!(unknown.to_string(), "PROPERTY:UNKNOWN_PROPERTY");
        assert_eq!(
            row_error_text(&error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
            "unknown-object"
        );
        assert_eq!(
            row_error_text(&error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)),
            "device / operational-problem"
        );
        let reject = RowError::Remote(RemoteError::Reject {
            reason: RejectReason::UNRECOGNIZED_SERVICE,
        });
        assert_eq!(row_error_text(&reject), "rejected: unrecognized-service");
        let abort = RowError::Remote(RemoteError::Abort {
            reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
        });
        assert_eq!(
            row_error_text(&abort),
            "aborted: segmentation-not-supported"
        );
        assert_eq!(row_error_text(&RowError::NoValue), "no value");
        assert_eq!(
            row_error_text(&RowError::Other("timeout".into())),
            "timeout"
        );
    }
}
