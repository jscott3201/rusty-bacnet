//! ReadPropertyMultiple service per ASHRAE 135-2020 Clause 15.7.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::{
    extract_property_value, PropertyReference, PropertyValueBoundary, MAX_DECODED_ITEMS,
};

// ---------------------------------------------------------------------------
// ReadPropertyMultipleRequest
// ---------------------------------------------------------------------------

/// A single object + list of property references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAccessSpecification {
    pub object_identifier: ObjectIdentifier,
    pub list_of_property_references: Vec<PropertyReference>,
}

/// ReadPropertyMultiple-Request service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyMultipleRequest {
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
            // [0] object-identifier
            primitives::encode_ctx_object_id(buf, 0, &spec.object_identifier);
            // [1] list-of-property-references (opening/closing)
            tags::encode_opening_tag(buf, 1);
            for prop_ref in &spec.list_of_property_references {
                prop_ref.encode(buf);
            }
            tags::encode_closing_tag(buf, 1);
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut specs = Vec::new();

        while offset < data.len() {
            if specs.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(
                    offset,
                    "RPM request exceeds max decoded items",
                ));
            }

            // [0] object-identifier
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_context(0) {
                return Err(Error::decoding(
                    offset,
                    "RPM request expected context tag 0",
                ));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "RPM request truncated at object-id"));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // [1] list-of-property-references (opening tag 1)
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(Error::decoding(
                    offset,
                    "RPM request expected opening tag 1",
                ));
            }
            offset = tag_end;

            let mut prop_refs = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(offset, "RPM request missing closing tag 1"));
                }
                if prop_refs.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::decoding(offset, "RPM property refs exceeds max"));
                }
                // Check for closing tag 1
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }
                // Decode property reference starting from current offset (not tag_end)
                let (pr, new_offset) = PropertyReference::decode(data, offset)?;
                prop_refs.push(pr);
                offset = new_offset;
            }

            specs.push(ReadAccessSpecification {
                object_identifier,
                list_of_property_references: prop_refs,
            });
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
    pub property_identifier: PropertyIdentifier,
    pub property_array_index: Option<u32>,
    /// Success: raw application-tagged value bytes. Mutually exclusive with `error`.
    pub property_value: Option<Vec<u8>>,
    /// Failure: (ErrorClass, ErrorCode). Mutually exclusive with `property_value`.
    pub error: Option<(ErrorClass, ErrorCode)>,
}

/// Results for a single object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAccessResult {
    pub object_identifier: ObjectIdentifier,
    pub list_of_results: Vec<ReadResultElement>,
}

/// ReadPropertyMultiple-ACK service parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPropertyMultipleACK {
    pub list_of_read_access_results: Vec<ReadAccessResult>,
}

impl ReadAccessResult {
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
    pub fn encode(&self, buf: &mut BytesMut) {
        for result in &self.list_of_read_access_results {
            ReadAccessResult::encode_header(buf, &result.object_identifier);
            for elem in &result.list_of_results {
                elem.encode(buf);
            }
            ReadAccessResult::encode_footer(buf);
        }
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let mut results = Vec::new();

        while offset < data.len() {
            if results.len() >= MAX_DECODED_ITEMS {
                return Err(Error::decoding(offset, "RPM ACK exceeds max decoded items"));
            }

            // [0] object-identifier
            let (tag, pos) = tags::decode_tag(data, offset)?;
            if !tag.is_context(0) {
                return Err(Error::decoding(offset, "RPM ACK expected context tag 0"));
            }
            let end = pos + tag.length as usize;
            if end > data.len() {
                return Err(Error::decoding(pos, "RPM ACK truncated at object-id"));
            }
            let object_identifier = ObjectIdentifier::decode(&data[pos..end])?;
            offset = end;

            // [1] list-of-results (opening tag 1)
            let (tag, tag_end) = tags::decode_tag(data, offset)?;
            if !tag.is_opening_tag(1) {
                return Err(Error::decoding(offset, "RPM ACK expected opening tag 1"));
            }
            offset = tag_end;

            let mut elements = Vec::new();
            loop {
                if offset >= data.len() {
                    return Err(Error::decoding(offset, "RPM ACK missing closing tag 1"));
                }
                if elements.len() >= MAX_DECODED_ITEMS {
                    return Err(Error::decoding(offset, "RPM ACK results exceeds max"));
                }
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_closing_tag(1) {
                    offset = tag_end;
                    break;
                }

                // [2] property-identifier
                if !tag.is_context(2) {
                    return Err(Error::decoding(offset, "RPM ACK expected context tag 2"));
                }
                let end = tag_end + tag.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(tag_end, "RPM ACK truncated at property-id"));
                }
                let prop_raw = primitives::decode_unsigned(&data[tag_end..end])?;
                let prop_raw = u32::try_from(prop_raw)
                    .map_err(|_| Error::decoding(tag_end, "RPM ACK property-id exceeds u32"))?;
                let property_identifier = PropertyIdentifier::from_raw(prop_raw);
                offset = end;

                // [3] property-array-index (optional)
                let mut array_index = None;
                let (tag, tag_end) = tags::decode_tag(data, offset)?;
                if tag.is_context(3) {
                    let end = tag_end + tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(tag_end, "RPM ACK truncated at array-index"));
                    }
                    let value = primitives::decode_unsigned(&data[tag_end..end])?;
                    array_index = Some(u32::try_from(value).map_err(|_| {
                        Error::decoding(tag_end, "RPM ACK array-index exceeds u32")
                    })?);
                    offset = end;
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
                        let (error_class, error_code, new_offset) =
                            decode_error_pair(data, tag_end)?;
                        elements.push(ReadResultElement {
                            property_identifier,
                            property_array_index: array_index,
                            property_value: None,
                            error: Some((error_class, error_code)),
                        });
                        offset = new_offset;
                    } else {
                        return Err(Error::decoding(offset, "RPM ACK expected tag 4 or 5"));
                    }
                } else if tag.is_opening_tag(4) {
                    // [4] property-value
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
                    // [5] property-access-error
                    let (error_class, error_code, new_offset) = decode_error_pair(data, tag_end)?;
                    elements.push(ReadResultElement {
                        property_identifier,
                        property_array_index: array_index,
                        property_value: None,
                        error: Some((error_class, error_code)),
                    });
                    offset = new_offset;
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
    // error-class: app-tagged enumerated
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            "RPM error class: expected application-tagged enumerated",
        ));
    }
    let end = pos + tag.length as usize;
    if end > data.len() {
        return Err(Error::decoding(pos, "RPM error truncated at error-class"));
    }
    let error_class_raw = primitives::decode_unsigned(&data[pos..end])?;
    let error_class_raw = u16::try_from(error_class_raw).map_err(|_| {
        Error::decoding(
            pos,
            format!("RPM error class {error_class_raw} exceeds u16"),
        )
    })?;
    let error_class = ErrorClass::from_raw(error_class_raw);
    let mut offset = end;

    // error-code: app-tagged enumerated
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            "RPM error code: expected application-tagged enumerated",
        ));
    }
    let end = pos + tag.length as usize;
    if end > data.len() {
        return Err(Error::decoding(pos, "RPM error truncated at error-code"));
    }
    let error_code_raw = primitives::decode_unsigned(&data[pos..end])?;
    let error_code_raw = u16::try_from(error_code_raw).map_err(|_| {
        Error::decoding(pos, format!("RPM error code {error_code_raw} exceeds u16"))
    })?;
    let error_code = ErrorCode::from_raw(error_code_raw);
    offset = end;

    // closing tag 5
    let (tag, tag_end) = tags::decode_tag(data, offset)?;
    if !tag.is_closing_tag(5) {
        return Err(Error::decoding(offset, "RPM error expected closing tag 5"));
    }

    Ok((error_class, error_code, tag_end))
}

#[cfg(test)]
#[path = "rpm_tests.rs"]
mod tests;
