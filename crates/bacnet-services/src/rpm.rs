//! ReadPropertyMultiple service per ASHRAE 135-2020 Clause 15.7.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_encoding::constructed::tagged::{
    decode_app_enumerated, decode_ctx_object_id, decode_ctx_unsigned, decode_optional_ctx,
    expect_closing, expect_opening,
};
use bacnet_encoding::constructed::{
    decode_read_access_specification, encode_read_access_specification, extract_property_value,
    PropertyValueBoundary,
};
use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::constructed::ReadAccessSpecification;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::MAX_DECODED_ITEMS;

// ---------------------------------------------------------------------------
// ReadPropertyMultipleRequest
// ---------------------------------------------------------------------------

/// ReadPropertyMultiple-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyMultipleRequest {
    /// Per-object read specifications; must not be empty when encoding.
    pub list_of_read_access_specs: Vec<ReadAccessSpecification>,
}

impl ReadPropertyMultipleRequest {
    /// Encode a nonempty request without modifying output on validation failure.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        if self.list_of_read_access_specs.is_empty()
            || self
                .list_of_read_access_specs
                .iter()
                .any(|spec| spec.list_of_property_references.is_empty())
        {
            return Err(Error::Encoding(
                "RPM requires nonempty object and property lists".into(),
            ));
        }
        for spec in &self.list_of_read_access_specs {
            encode_read_access_specification(buf, spec);
        }
        Ok(())
    }

    /// Decode the request from service-request octets; fails on malformed or truncated input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut specs = Vec::new();

        while offset < data.len() {
            if specs.len() >= MAX_DECODED_ITEMS {
                return Err(Error::overflow(
                    offset,
                    "RPM request exceeds max decoded items",
                ));
            }

            let (spec, next) = decode_read_access_specification(data, offset)?;
            specs.push(spec);
            offset = next;
        }

        Ok(Self {
            list_of_read_access_specs: specs,
        })
    }
}

// ---------------------------------------------------------------------------
// ReadPropertyMultipleACK
// ---------------------------------------------------------------------------

/// A single result element: success (value) or failure (error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResultElement {
    /// Property this result answers.
    pub property_identifier: PropertyIdentifier,
    /// Array index that was requested, echoed from the request; `None` when the whole property was
    /// read.
    pub property_array_index: Option<u32>,
    /// Success: raw application-tagged value bytes. Mutually exclusive with `error`.
    pub property_value: Option<Vec<u8>>,
    /// Failure: (ErrorClass, ErrorCode). Mutually exclusive with `property_value`.
    pub error: Option<(ErrorClass, ErrorCode)>,
}

/// Results for a single object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAccessResult {
    /// Object the results belong to.
    pub object_identifier: ObjectIdentifier,
    /// One element per property read, in the order returned by the responder.
    pub list_of_results: Vec<ReadResultElement>,
}

/// ReadPropertyMultiple-ACK service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyMultipleACK {
    /// Per-object results, in request order.
    pub list_of_read_access_results: Vec<ReadAccessResult>,
}

impl ReadAccessResult {
    /// Append the whole result: the object identifier in context tag `[0]`,
    /// then each element between an opening and a closing tag `[1]`.
    pub fn encode(&self, buf: &mut BytesMut) {
        Self::encode_header(buf, &self.object_identifier);
        for elem in &self.list_of_results {
            elem.encode(buf);
        }
        Self::encode_footer(buf);
    }

    /// Encode an object's identifier and opening list-of-results tag.
    pub fn encode_header(buf: &mut BytesMut, object_identifier: &ObjectIdentifier) {
        primitives::encode_ctx_object_id(buf, 0, object_identifier);
        tags::encode_opening_tag(buf, 1);
    }

    /// Encode the closing list-of-results tag.
    pub fn encode_footer(buf: &mut BytesMut) {
        tags::encode_closing_tag(buf, 1);
    }
}

impl ReadResultElement {
    /// Encode one result, preserving value-over-error precedence.
    pub fn encode(&self, buf: &mut BytesMut) {
        primitives::encode_ctx_unsigned(buf, 2, self.property_identifier.to_raw() as u64);
        if let Some(idx) = self.property_array_index {
            primitives::encode_ctx_unsigned(buf, 3, idx as u64);
        }
        if let Some(ref value) = self.property_value {
            tags::encode_opening_tag(buf, 4);
            buf.extend_from_slice(value);
            tags::encode_closing_tag(buf, 4);
        } else if let Some((class, code)) = self.error {
            tags::encode_opening_tag(buf, 5);
            primitives::encode_app_enumerated(buf, class.to_raw() as u32);
            primitives::encode_app_enumerated(buf, code.to_raw() as u32);
            tags::encode_closing_tag(buf, 5);
        }
    }
}

impl ReadPropertyMultipleACK {
    /// Encode the acknowledgment parameters into `buf`.
    pub fn encode(&self, buf: &mut BytesMut) {
        for result in &self.list_of_read_access_results {
            result.encode(buf);
        }
    }

    /// Decode the acknowledgment from its service-ack octets; fails on malformed or truncated
    /// input.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut results = Vec::new();

        while offset < data.len() {
            if results.len() >= MAX_DECODED_ITEMS {
                return Err(Error::overflow(offset, "RPM ACK exceeds max decoded items"));
            }

            // [0] object-identifier
            let (object_identifier, end) =
                decode_ctx_object_id(data, offset, 0, "RPM ACK object-id")?;

            // [1] list-of-results (opening tag 1)
            offset = expect_opening(data, end, 1, "RPM ACK list-of-results")?;

            let mut elements = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(offset, "RPM ACK missing closing tag 1"));
                }
                if elements.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::overflow(offset, "RPM ACK results exceeds max"));
                }
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }

                // [2] property-identifier
                let (prop_raw, end) =
                    decode_ctx_unsigned::<u32>(data, offset, 2, "RPM ACK property-id")?;
                let property_identifier = PropertyIdentifier::from_raw(prop_raw);

                // [3] property-array-index (optional)
                let (array_index, end) = decode_optional_ctx(
                    data,
                    end,
                    3,
                    "RPM ACK array-index",
                    decode_ctx_unsigned::<u32>,
                )?;
                offset = end;

                // [4] property-value or [5] property-access-error
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_opening_tag(4) {
                    let (value_bytes, new_offset) = extract_property_value(
                        data,
                        tag_end,
                        4,
                        property_identifier,
                        &[
                            PropertyValueBoundary::Context(2),
                            PropertyValueBoundary::Closing(1),
                        ],
                    )?;
                    elements.push(ReadResultElement {
                        property_identifier,
                        property_array_index: array_index,
                        property_value: Some(value_bytes.to_vec()),
                        error: None,
                    });
                    offset = new_offset;
                } else if tag.is_opening_tag(5) {
                    let (error_class, error_code, new_offset) = decode_error_pair(data, tag_end)?;
                    elements.push(ReadResultElement {
                        property_identifier,
                        property_array_index: array_index,
                        property_value: None,
                        error: Some((error_class, error_code)),
                    });
                    offset = new_offset;
                } else if array_index.is_some() {
                    return Err(Error::decoding(offset, "RPM ACK expected tag 4 or 5"));
                } else {
                    return Err(Error::decoding(offset, "RPM ACK expected tag 3, 4, or 5"));
                }
            }

            results.push(ReadAccessResult {
                object_identifier,
                list_of_results: elements,
            });
        }

        Ok(Self {
            list_of_read_access_results: results,
        })
    }
}

/// Decode an error-class + error-code pair from inside opening/closing tag 5,
/// followed by consuming the closing tag.
fn decode_error_pair(data: &[u8], offset: usize) -> Result<(ErrorClass, ErrorCode, usize), Error> {
    let (error_class, offset) = decode_app_enumerated::<u16>(data, offset, "RPM error class")?;
    let (error_code, offset) = decode_app_enumerated::<u16>(data, offset, "RPM error code")?;
    let end = expect_closing(data, offset, 5, "RPM error")?;
    Ok((
        ErrorClass::from_raw(error_class),
        ErrorCode::from_raw(error_code),
        end,
    ))
}

#[cfg(test)]
#[path = "rpm_tests.rs"]
mod tests;
