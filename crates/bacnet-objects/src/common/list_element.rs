//! Naming the element a list-valued write refused (#1048).

/// Name the element a list-valued write refused, `index` counting from 0 in
/// the list the write carried: the refusal becomes `Error::Structured` with
/// that element's 1-based position as `FirstFailedElementNumber`. The server's
/// AddListElement handler turns the position into the request's element
/// number (#1048). Errors other than a class and code pass through.
use bacnet_types::error::RemoteError;
pub(crate) fn at_list_element(
    error: bacnet_types::error::Error,
    index: usize,
) -> bacnet_types::error::Error {
    use bacnet_types::error::{Error, ErrorDetail};
    match error {
        Error::Remote(RemoteError::Error { class, code }) => {
            let position = u32::try_from(index).map_or(u32::MAX, |index| index.saturating_add(1));
            Error::with_detail(
                class,
                code,
                Some(ErrorDetail::FirstFailedElementNumber(position)),
            )
        }
        other => other,
    }
}

/// Assert that a list-valued write was refused with `class` / `code`, naming
/// the element at `position` (from 1) of the list it carried.
#[cfg(test)]
pub(crate) fn assert_list_element_refused<T: core::fmt::Debug>(
    result: Result<T, bacnet_types::error::Error>,
    class: bacnet_types::enums::ErrorClass,
    code: bacnet_types::enums::ErrorCode,
    position: u32,
    context: &str,
) {
    use bacnet_types::error::{Error, ErrorDetail};
    match result {
        Err(Error::Structured {
            class: actual_class,
            code: actual_code,
            detail,
        }) => {
            assert_eq!(actual_class, class, "{context}: class");
            assert_eq!(actual_code, code, "{context}: {code:?}");
            assert_eq!(
                *detail,
                ErrorDetail::FirstFailedElementNumber(position),
                "{context}: element"
            );
        }
        other => panic!("{context}: expected {class:?}/{code:?} at {position}, got {other:?}"),
    }
}
