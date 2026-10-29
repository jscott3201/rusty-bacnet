//! How a Credential Data Input keeps Present_Value in step with the formats
//! it declares (Clauses 12.36.4, 12.36.9 and 12.36.10, #1249).
//!
//! Clause 12.36.4 ties the factor in Present_Value to Supported_Formats and
//! Supported_Format_Classes: its format type is one the reader declares and
//! its format class the class declared beside that format. UNDEFINED (no
//! factor read) and ERROR (a read the reader can't decode) are the two types
//! that need no declaration, each carrying class 0. The object holds every
//! route that sets Present_Value to that rule, so whatever it serves is a
//! factor a client could also write while out of service:
//!
//! - `set_present_value`, the application's report of a read, refuses any
//!   other factor with VALUE_OUT_OF_RANGE and changes nothing.
//! - A client's out-of-service write is checked the same way
//!   (`credential_data_input_out_of_service`).
//! - `set_supported_formats` drops a factor the new list no longer covers:
//!   Present_Value goes back to the UNDEFINED factor, and Update_Time, which
//!   moves with every Present_Value update (Clause 12.36.11), takes the time
//!   from the Device clock, or the object's next sequence number when there
//!   is no usable clock (`UpdateClock`). Out of
//!   service this applies both to the simulated factor served and to the
//!   reader's factor put aside, so the return to service, which serves the
//!   factor put aside again, can't bring back a format the reader dropped.
//!
//! A factor whose format type is still declared but under another class is
//! dropped too: the class beside a format is part of what the reader
//! declares, and keeping the old class would serve a pair no write could set.
//! The value octets aren't checked against the Annex P layout of the format.

use crate::prelude::*;
use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat};
use bacnet_types::enums::AuthenticationFactorType;

use super::UpdateClock;
use super::credential_data_input_out_of_service::Reading;

/// A supported format with the format class a factor read in it carries:
/// one Supported_Formats element and the Supported_Format_Classes element at
/// the same position.
pub(super) type SupportedFormat = (BACnetAuthenticationFactorFormat, u32);

/// The factor served while no read is current: UNDEFINED, class 0, no value
/// octets.
pub(super) fn undefined_factor() -> BACnetAuthenticationFactor {
    BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::UNDEFINED,
        format_class: 0,
        value: Vec::new(),
    }
}

/// Whether `format_type` is a named BACnetAuthenticationFactorType. The
/// production is closed; the bacnet-types production test pins its length.
pub(super) fn is_factor_type(format_type: AuthenticationFactorType) -> bool {
    format_type.to_raw() <= AuthenticationFactorType::USER_PASSWORD.to_raw()
}

/// Whether `format` is a named format whose vendor members suit it: a CUSTOM
/// format names both, any other carries them only as zero (Clause 12.36.9).
pub(super) fn is_well_formed(format: &BACnetAuthenticationFactorFormat) -> bool {
    if !is_factor_type(format.format_type) {
        return false;
    }
    if format.format_type == AuthenticationFactorType::CUSTOM {
        format.vendor_id.is_some() && format.vendor_format.is_some()
    } else {
        format.vendor_id.unwrap_or(0) == 0 && format.vendor_format.unwrap_or(0) == 0
    }
}

/// Whether the reader can serve `factor`: a named format type that is either
/// declared in `formats` with this class, or UNDEFINED or ERROR with class 0.
pub(super) fn is_declared(
    factor: &BACnetAuthenticationFactor,
    formats: &[SupportedFormat],
) -> bool {
    if !is_factor_type(factor.format_type) {
        return false;
    }
    if factor.format_type == AuthenticationFactorType::UNDEFINED {
        return factor.format_class == 0;
    }
    formats.iter().any(|(format, class)| {
        format.format_type == factor.format_type && *class == factor.format_class
    }) || (factor.format_type == AuthenticationFactorType::ERROR && factor.format_class == 0)
}

/// Put `reading` back to the UNDEFINED factor, stamped by `clock`, when
/// `formats` no longer covers its factor; leave it alone otherwise.
pub(super) fn drop_undeclared(
    reading: &mut Reading,
    formats: &[SupportedFormat],
    clock: &mut UpdateClock,
) {
    if is_declared(&reading.present_value, formats) {
        return;
    }
    reading.present_value = undefined_factor();
    reading.update_time = clock.stamp();
}
