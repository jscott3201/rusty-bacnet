//! WritePropertyMultiple service per ASHRAE 135-2020 Clause 15.10.

use bacnet_encoding::primitives;
use bacnet_encoding::tags;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::common::BACnetPropertyValue;

pub mod cursor;
pub mod error;
#[cfg(test)]
mod priority_tests;
#[cfg(test)]
mod validation_tests;

pub use cursor::{
    WritePropertyAttempt, WritePropertyMultipleCursor, WritePropertyMultipleCursorError,
    WritePropertyMultipleDecodeStage, WritePropertyMultipleEvent, WritePropertyMultipleFailureKind,
};
pub use error::WritePropertyMultipleError;

// ---------------------------------------------------------------------------
// WritePropertyMultipleRequest
// ---------------------------------------------------------------------------

/// A single object + list of property values to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAccessSpecification {
    pub object_identifier: ObjectIdentifier,
    pub list_of_properties: Vec<BACnetPropertyValue>,
}

/// WritePropertyMultiple-Request service parameters.
///
/// Uses SimpleACK (no ACK struct needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyMultipleRequest {
    pub list_of_write_access_specs: Vec<WriteAccessSpecification>,
}

impl WritePropertyMultipleRequest {
    /// Validate the complete outbound request without reading remote metadata.
    ///
    /// Each request and object specification must contain writes. Special RPM
    /// selectors are not write targets; supplied priorities must be in 1..=16.
    /// Encoded property values remain opaque, including legal empty list values.
    pub fn validate(&self) -> Result<(), Error> {
        use bacnet_types::enums::PropertyIdentifier as P;
        if self.list_of_write_access_specs.is_empty() {
            return Err(Error::Encoding(
                "WPM requires at least one object specification".into(),
            ));
        }
        for spec in &self.list_of_write_access_specs {
            if spec.list_of_properties.is_empty() {
                return Err(Error::Encoding(
                    "WPM requires at least one property per object".into(),
                ));
            }
            for property in &spec.list_of_properties {
                if matches!(
                    property.property_identifier,
                    P::ALL | P::REQUIRED | P::OPTIONAL
                ) {
                    return Err(Error::Encoding(
                        "WPM does not accept ALL, REQUIRED or OPTIONAL selectors".into(),
                    ));
                }
                crate::write_property::validate_priority(property.priority)?;
            }
        }
        Ok(())
    }

    /// Encode transactionally after validating every object/property write.
    /// Invalid typed input leaves an existing buffer unchanged. Inbound decoding
    /// remains separate and may preserve external empty-list/no-op requests.
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.validate()?;
        for spec in &self.list_of_write_access_specs {
            primitives::encode_ctx_object_id(buf, 0, &spec.object_identifier);
            tags::encode_opening_tag(buf, 1);
            for prop_val in &spec.list_of_properties {
                prop_val.encode(buf);
            }
            tags::encode_closing_tag(buf, 1);
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut cursor = WritePropertyMultipleCursor::new(data);
        let mut specs = Vec::new();
        let mut current = None;

        while let Some(event) = cursor.next_event().map_err(|error| {
            Error::decoding(
                error.offset,
                format!("WPM {:?}: {}", error.stage, error.message),
            )
        })? {
            match event {
                WritePropertyMultipleEvent::ObjectStart(object_identifier) => {
                    current = Some(WriteAccessSpecification {
                        object_identifier,
                        list_of_properties: Vec::new(),
                    });
                }
                WritePropertyMultipleEvent::WriteAttempt(attempt) => {
                    current
                        .as_mut()
                        .expect("cursor emits attempts inside an object")
                        .list_of_properties
                        .push(BACnetPropertyValue {
                            property_identifier: bacnet_types::enums::PropertyIdentifier::from_raw(
                                attempt.reference.property_identifier,
                            ),
                            property_array_index: attempt.reference.property_array_index,
                            value: attempt.value,
                            priority: attempt.priority,
                        });
                }
                WritePropertyMultipleEvent::ObjectEnd => {
                    specs.push(current.take().expect("cursor ends an open object"));
                }
            }
        }

        Ok(Self {
            list_of_write_access_specs: specs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};

    #[test]
    fn request_round_trip() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = WritePropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn multi_object_round_trip() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![
                WriteAccessSpecification {
                    object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                        priority: None,
                    }],
                },
                WriteAccessSpecification {
                    object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 2).unwrap(),
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: vec![0x91, 0x01],
                        priority: Some(8),
                    }],
                },
            ],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let decoded = WritePropertyMultipleRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn legacy_event_parameters_do_not_consume_the_next_property() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
                list_of_properties: vec![
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
                        property_array_index: None,
                        value: vec![0xfe, 0xff, 1, 0xff, 0xff, 0x2f, 2, 0xff, 0xff],
                        priority: None,
                    },
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::NOTIFICATION_CLASS,
                        property_array_index: None,
                        value: vec![0x21, 8],
                        priority: None,
                    },
                ],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();

        assert_eq!(WritePropertyMultipleRequest::decode(&buf).unwrap(), req);
    }

    #[test]
    fn ambiguous_legacy_event_parameters_are_rejected() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
                    property_array_index: None,
                    value: vec![
                        0xfe, 0xff, 0xaa, 0xff, 0xff, 0x2f, 0x09, 0x53, 0x2e, 0xfe, 0xff, 0xbb,
                        0xff, 0xff,
                    ],
                    priority: None,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();

        assert!(WritePropertyMultipleRequest::decode(&buf).is_err());
    }

    // -----------------------------------------------------------------------
    // Malformed-input decode error tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_wpm_request_truncated_1_byte() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(WritePropertyMultipleRequest::decode(&buf[..1]).is_err());
    }

    #[test]
    fn test_decode_wpm_request_truncated_3_bytes() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        assert!(WritePropertyMultipleRequest::decode(&buf[..3]).is_err());
    }

    #[test]
    fn test_decode_wpm_request_truncated_half() {
        let req = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
                    priority: Some(8),
                }],
            }],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf).unwrap();
        let half = buf.len() / 2;
        assert!(WritePropertyMultipleRequest::decode(&buf[..half]).is_err());
    }

    #[test]
    fn test_decode_wpm_request_invalid_tag() {
        assert!(WritePropertyMultipleRequest::decode(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
