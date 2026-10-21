//! Shared BACnet service data types per ASHRAE 135-2020 Clause 21.

pub use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::error::Error;

/// Safety limit for decoded sequences to prevent unbounded allocations.
pub const MAX_DECODED_ITEMS: usize = 10_000;

pub(crate) mod error_type;

/// A refusal of raw octets a request carries, met while checking them before
/// they are encoded, as the encoder's static refusal named by `context`
/// (#1802; see [`Error::into_encode_refusal`]).
pub(crate) fn refused_value(error: Error, context: &'static str) -> Error {
    match error.into_encode_refusal() {
        Error::Encode(refused) => refused.within(context).into(),
        other => other,
    }
}
