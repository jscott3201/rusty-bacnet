//! The NotificationParameters choices that report a value against limits
//! (floating limit and the out-of-range family), one decoder per choice.
//! [`decode_bounded`](super::parameters_decode::decode_bounded) checks the
//! choice's framing and picks the decoder by its tag.

use super::decode_helpers::{decode_context_status_flags, decode_context_value, finish_variant};
use super::*;

/// `[4]` floating limit: the fields up to `variant_body_end`.
pub(super) fn decode_floating_limit(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] reference-value
    let (reference_value, pos) = decode_context_value(
        data,
        inner_start,
        0,
        "FloatingLimit reference-value",
        primitives::decode_real,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "FloatingLimit status-flags")?;
    // [2] setpoint-value
    let (setpoint_value, pos) = decode_context_value(
        data,
        pos,
        2,
        "FloatingLimit setpoint-value",
        primitives::decode_real,
    )?;
    // [3] error-limit
    let (error_limit, pos) = decode_context_value(
        data,
        pos,
        3,
        "FloatingLimit error-limit",
        primitives::decode_real,
    )?;
    finish_variant(
        NotificationParameters::FloatingLimit {
            reference_value,
            status_flags,
            setpoint_value,
            error_limit,
        },
        pos,
        variant_body_end,
        4,
    )
}

/// `[5]` out of range: the fields up to `variant_body_end`.
pub(super) fn decode_out_of_range(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] exceeding-value
    let (exceeding_value, pos) = decode_context_value(
        data,
        inner_start,
        0,
        "OutOfRange exceeding-value",
        primitives::decode_real,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "OutOfRange status-flags")?;
    // [2] deadband
    let (deadband, pos) =
        decode_context_value(data, pos, 2, "OutOfRange deadband", primitives::decode_real)?;
    // [3] exceeded-limit
    let (exceeded_limit, pos) = decode_context_value(
        data,
        pos,
        3,
        "OutOfRange exceeded-limit",
        primitives::decode_real,
    )?;
    finish_variant(
        NotificationParameters::OutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        },
        pos,
        variant_body_end,
        5,
    )
}

/// `[11]` unsigned range: the fields up to `variant_body_end`.
pub(super) fn decode_unsigned_range(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] exceeding-value
    let (exceeding_value, pos) = decode_context_value(
        data,
        inner_start,
        0,
        "UnsignedRange exceeding-value",
        primitives::decode_unsigned,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "UnsignedRange status-flags")?;
    // [2] exceeded-limit
    let (exceeded_limit, pos) = decode_context_value(
        data,
        pos,
        2,
        "UnsignedRange exceeded-limit",
        primitives::decode_unsigned,
    )?;
    finish_variant(
        NotificationParameters::UnsignedRange {
            exceeding_value,
            status_flags,
            exceeded_limit,
        },
        pos,
        variant_body_end,
        11,
    )
}

/// `[14]` double out of range: the fields up to `variant_body_end`.
pub(super) fn decode_double_out_of_range(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] exceeding-value
    let (exceeding_value, pos) = decode_context_value(
        data,
        inner_start,
        0,
        "DoubleOutOfRange exceeding-value",
        primitives::decode_double,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "DoubleOutOfRange status-flags")?;
    // [2] deadband
    let (deadband, pos) = decode_context_value(
        data,
        pos,
        2,
        "DoubleOutOfRange deadband",
        primitives::decode_double,
    )?;
    // [3] exceeded-limit
    let (exceeded_limit, pos) = decode_context_value(
        data,
        pos,
        3,
        "DoubleOutOfRange exceeded-limit",
        primitives::decode_double,
    )?;
    finish_variant(
        NotificationParameters::DoubleOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        },
        pos,
        variant_body_end,
        14,
    )
}

/// `[15]` signed out of range: the fields up to `variant_body_end`.
pub(super) fn decode_signed_out_of_range(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] exceeding-value
    let (exceeding_value, pos) = decode_context_value(
        data,
        inner_start,
        0,
        "SignedOutOfRange exceeding-value",
        primitives::decode_signed,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "SignedOutOfRange status-flags")?;
    // [2] deadband
    let (deadband, pos) = decode_context_value(
        data,
        pos,
        2,
        "SignedOutOfRange deadband",
        primitives::decode_unsigned,
    )?;
    // [3] exceeded-limit
    let (exceeded_limit, pos) = decode_context_value(
        data,
        pos,
        3,
        "SignedOutOfRange exceeded-limit",
        primitives::decode_signed,
    )?;
    finish_variant(
        NotificationParameters::SignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        },
        pos,
        variant_body_end,
        15,
    )
}

/// `[16]` unsigned out of range: the fields up to `variant_body_end`.
pub(super) fn decode_unsigned_out_of_range(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] exceeding-value
    let (exceeding_value, pos) = decode_context_value(
        data,
        inner_start,
        0,
        "UnsignedOutOfRange exceeding-value",
        primitives::decode_unsigned,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "UnsignedOutOfRange status-flags")?;
    // [2] deadband
    let (deadband, pos) = decode_context_value(
        data,
        pos,
        2,
        "UnsignedOutOfRange deadband",
        primitives::decode_unsigned,
    )?;
    // [3] exceeded-limit
    let (exceeded_limit, pos) = decode_context_value(
        data,
        pos,
        3,
        "UnsignedOutOfRange exceeded-limit",
        primitives::decode_unsigned,
    )?;
    finish_variant(
        NotificationParameters::UnsignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        },
        pos,
        variant_body_end,
        16,
    )
}
