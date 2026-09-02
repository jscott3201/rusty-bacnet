use super::super::confirmed_response_result;
use crate::tsm::TsmResponse;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

#[test]
fn high_level_error_projection_remains_class_and_code_only() {
    let result = confirmed_response_result(TsmResponse::Error {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
    });
    assert!(matches!(
        result,
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
    ));
}
