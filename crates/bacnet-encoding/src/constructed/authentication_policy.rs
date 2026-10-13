//! Clause 21 codec for `BACnetAuthenticationPolicy`, the element of an Access
//! Point's Authentication_Policy_List array (Clause 12.31.12).
//!
//! A policy is a bare SEQUENCE and a whole array concatenates them with no
//! wrapper, so the decoder reads one policy at `offset` and returns the
//! offset just past it, the way the Access Credential element codecs do.
//!
//! The decoder checks structure only. An empty entry list, entries naming
//! another object type and indexes out of sequence all decode; the Access
//! Point judges whether a policy is usable. An index or timeout wider than
//! 32 bits is malformed.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{BACnetAuthenticationPolicy, BACnetAuthenticationPolicyEntry};
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::tagged::{
    decode_ctx_boolean, decode_ctx_unsigned, expect_closing, expect_opening, next_is_closing,
};
use super::{decode_device_object_reference, encode_device_object_reference};
use crate::{primitives, tags};

const POLICY: &str = "BACnetAuthenticationPolicy";

/// Encode one `BACnetAuthenticationPolicy` SEQUENCE: the entries inside
/// opening and closing tag 0, each its Credential Data Input reference inside
/// opening and closing tag 0 followed by its index as context `[1]`; then
/// the order flag as context `[1]` and the timeout as context `[2]`.
pub fn encode_authentication_policy(buf: &mut BytesMut, value: &BACnetAuthenticationPolicy) {
    tags::encode_opening_tag(buf, 0);
    for entry in &value.policy {
        tags::encode_opening_tag(buf, 0);
        encode_device_object_reference(buf, &entry.credential_data_input);
        tags::encode_closing_tag(buf, 0);
        primitives::encode_ctx_unsigned(buf, 1, u64::from(entry.index));
    }
    tags::encode_closing_tag(buf, 0);
    primitives::encode_ctx_boolean(buf, 1, value.order_enforced);
    primitives::encode_ctx_unsigned(buf, 2, u64::from(value.timeout));
}

/// Decode one `BACnetAuthenticationPolicy` SEQUENCE at `offset`.
///
/// A missing entry frame, order flag or timeout, an entry whose reference
/// frame holds anything but one device-object reference or that lacks its
/// index, an index or timeout above 32 bits, a BOOLEAN whose contents aren't
/// 0 or 1, or a truncated member fails with [`Error::Decoding`] or
/// [`Error::BufferTooShort`].
pub fn decode_authentication_policy(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetAuthenticationPolicy, usize), Error> {
    let mut pos = expect_opening(data, offset, 0, POLICY)?;
    let mut policy = Vec::new();
    while !next_is_closing(data, pos, 0)? {
        let content = expect_opening(data, pos, 0, POLICY)?;
        let (credential_data_input, end) = decode_device_object_reference(data, content)?;
        let end = expect_closing(data, end, 0, POLICY)?;
        let (index, end) = decode_ctx_unsigned::<u32>(data, end, 1, POLICY)?;
        policy.push(BACnetAuthenticationPolicyEntry {
            credential_data_input,
            index,
        });
        pos = end;
    }
    let pos = expect_closing(data, pos, 0, POLICY)?;
    let (order_enforced, pos) = decode_ctx_boolean(data, pos, 1, POLICY)?;
    let (timeout, end) = decode_ctx_unsigned::<u32>(data, pos, 2, POLICY)?;
    Ok((
        BACnetAuthenticationPolicy {
            policy,
            order_enforced,
            timeout,
        },
        end,
    ))
}
