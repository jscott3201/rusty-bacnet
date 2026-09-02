//! Incremental WritePropertyMultiple request grammar.

use bacnet_encoding::tags::{self, TagClass};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::RejectReason;
use bacnet_types::primitives::ObjectIdentifier;

use crate::common::{
    BACnetPropertyValue, PropertyValueDecodeError, PropertyValueDecodeStage, MAX_DECODED_ITEMS,
};

/// Stable request-decoding stage reported by the incremental WPM cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePropertyMultipleDecodeStage {
    /// Context `[0]` object identifier.
    ObjectIdentifier,
    /// Opening context `[1]` property list.
    PropertyList,
    /// Required property identifier of a list item.
    PropertyIdentifier,
    /// Optional property array index.
    ArrayIndex,
    /// Constructed property value.
    Value,
    /// Optional write priority.
    Priority,
    /// Closing context `[1]` property list delimiter.
    PropertyListEnd,
    /// Bounded object/property cardinality.
    ItemLimit,
}

/// A classified WPM syntax failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyMultipleCursorError {
    /// Zero-based byte offset where decoding failed.
    pub offset: usize,
    /// Stable request stage that failed.
    pub stage: WritePropertyMultipleDecodeStage,
    /// Narrow Clause 18.9 Reject classification for a pre-write failure.
    pub reject_reason: RejectReason,
    /// Complete failed coordinate, present only after every member decoded.
    pub first_failed_write_attempt: Option<BACnetObjectPropertyReference>,
    /// Human-readable local diagnostic; not encoded on the wire.
    pub message: String,
}

/// One complete write attempt in request wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePropertyAttempt {
    /// Complete object/property coordinate.
    pub reference: BACnetObjectPropertyReference,
    /// Raw, exactly bounded contents of the property's constructed value.
    pub value: Vec<u8>,
    /// Optional command priority.
    pub priority: Option<u8>,
}

/// Events emitted by [`WritePropertyMultipleCursor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritePropertyMultipleEvent {
    /// A context `[0]` object and its opening property list decoded.
    ObjectStart(ObjectIdentifier),
    /// One complete property write attempt.
    WriteAttempt(WritePropertyAttempt),
    /// The current object's property list closed.
    ObjectEnd,
}

#[derive(Debug, Clone, Copy)]
enum State {
    Object,
    Properties(ObjectIdentifier),
    Done,
    Failed,
}

/// Bounded, allocation-light cursor over a WPM request body.
pub struct WritePropertyMultipleCursor<'a> {
    data: &'a [u8],
    offset: usize,
    state: State,
    object_count: usize,
    property_count: usize,
    total_attempts: usize,
}

impl<'a> WritePropertyMultipleCursor<'a> {
    /// Create a cursor over one WPM service request body.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            state: State::Object,
            object_count: 0,
            property_count: 0,
            total_attempts: 0,
        }
    }

    /// Return the next unread byte offset.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Decode the next object/list/write event without reading later attempts.
    pub fn next_event(
        &mut self,
    ) -> Result<Option<WritePropertyMultipleEvent>, WritePropertyMultipleCursorError> {
        loop {
            match self.state {
                State::Done => return Ok(None),
                State::Failed => return Ok(None),
                State::Object => {
                    if self.offset == self.data.len() {
                        self.state = State::Done;
                        return Ok(None);
                    }
                    if self.object_count >= MAX_DECODED_ITEMS {
                        return self.fail(self.limit_error("WPM object count exceeds limit"));
                    }
                    let oid = match self.decode_object_identifier() {
                        Ok(oid) => oid,
                        Err(error) => return self.fail(error),
                    };
                    if let Err(error) = self.decode_property_list_opening() {
                        return self.fail(error);
                    }
                    self.object_count += 1;
                    self.property_count = 0;
                    self.state = State::Properties(oid);
                    return Ok(Some(WritePropertyMultipleEvent::ObjectStart(oid)));
                }
                State::Properties(oid) => {
                    if self.offset >= self.data.len() {
                        return self.fail(self.error(
                            self.offset,
                            WritePropertyMultipleDecodeStage::PropertyListEnd,
                            RejectReason::MISSING_REQUIRED_PARAMETER,
                            None,
                            "WPM property list is missing closing tag 1",
                        ));
                    }
                    let (tag, tag_end) = match tags::decode_tag(self.data, self.offset) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            return self.fail(self.error(
                                self.offset,
                                WritePropertyMultipleDecodeStage::PropertyIdentifier,
                                RejectReason::INVALID_DATA_ENCODING,
                                None,
                                error.to_string(),
                            ));
                        }
                    };
                    if tag.is_closing_tag(1) {
                        self.offset = tag_end;
                        self.state = State::Object;
                        return Ok(Some(WritePropertyMultipleEvent::ObjectEnd));
                    }
                    if tag.is_closing {
                        return self.fail(self.error(
                            self.offset,
                            WritePropertyMultipleDecodeStage::PropertyListEnd,
                            RejectReason::INVALID_TAG,
                            None,
                            "WPM property list has an unmatched closing tag",
                        ));
                    }
                    if self.property_count >= MAX_DECODED_ITEMS
                        || self.total_attempts >= MAX_DECODED_ITEMS
                    {
                        return self.fail(self.limit_error("WPM property count exceeds limit"));
                    }
                    let (property, end) = match BACnetPropertyValue::decode_in_list_detailed(
                        self.data,
                        self.offset,
                        1,
                    ) {
                        Ok(decoded) => decoded,
                        Err(error) => return self.fail(self.property_error(oid, error)),
                    };
                    self.offset = end;
                    self.property_count += 1;
                    self.total_attempts += 1;
                    return Ok(Some(WritePropertyMultipleEvent::WriteAttempt(
                        WritePropertyAttempt {
                            reference: BACnetObjectPropertyReference {
                                object_identifier: oid,
                                property_identifier: property.property_identifier.to_raw(),
                                property_array_index: property.property_array_index,
                            },
                            value: property.value,
                            priority: property.priority,
                        },
                    )));
                }
            }
        }
    }

    fn decode_object_identifier(
        &mut self,
    ) -> Result<ObjectIdentifier, WritePropertyMultipleCursorError> {
        let start = self.offset;
        let (tag, content_start) = tags::decode_tag(self.data, start).map_err(|error| {
            self.error(
                start,
                WritePropertyMultipleDecodeStage::ObjectIdentifier,
                RejectReason::INVALID_DATA_ENCODING,
                None,
                error.to_string(),
            )
        })?;
        if !tag.is_context(0) {
            let reason = if tag.class == TagClass::Application {
                RejectReason::INVALID_PARAMETER_DATA_TYPE
            } else {
                RejectReason::INVALID_TAG
            };
            return Err(self.error(
                start,
                WritePropertyMultipleDecodeStage::ObjectIdentifier,
                reason,
                None,
                "WPM object identifier must use primitive context tag 0",
            ));
        }
        let end = content_start
            .checked_add(tag.length as usize)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| {
                self.error(
                    content_start,
                    WritePropertyMultipleDecodeStage::ObjectIdentifier,
                    RejectReason::INVALID_DATA_ENCODING,
                    None,
                    "WPM object identifier payload is truncated",
                )
            })?;
        if tag.length != 4 {
            return Err(self.error(
                content_start,
                WritePropertyMultipleDecodeStage::ObjectIdentifier,
                RejectReason::INVALID_DATA_ENCODING,
                None,
                "WPM object identifier must contain exactly four octets",
            ));
        }
        let oid = ObjectIdentifier::decode(&self.data[content_start..end]).map_err(|error| {
            self.error(
                content_start,
                WritePropertyMultipleDecodeStage::ObjectIdentifier,
                RejectReason::INVALID_DATA_ENCODING,
                None,
                error.to_string(),
            )
        })?;
        self.offset = end;
        Ok(oid)
    }

    fn decode_property_list_opening(&mut self) -> Result<(), WritePropertyMultipleCursorError> {
        if self.offset >= self.data.len() {
            return Err(self.error(
                self.offset,
                WritePropertyMultipleDecodeStage::PropertyList,
                RejectReason::MISSING_REQUIRED_PARAMETER,
                None,
                "WPM object is missing its property list",
            ));
        }
        let start = self.offset;
        let (tag, end) = tags::decode_tag(self.data, start).map_err(|error| {
            self.error(
                start,
                WritePropertyMultipleDecodeStage::PropertyList,
                RejectReason::INVALID_DATA_ENCODING,
                None,
                error.to_string(),
            )
        })?;
        if !tag.is_opening_tag(1) {
            return Err(self.error(
                start,
                WritePropertyMultipleDecodeStage::PropertyList,
                if tag.class == TagClass::Application {
                    RejectReason::INVALID_PARAMETER_DATA_TYPE
                } else {
                    RejectReason::INVALID_TAG
                },
                None,
                "WPM expected opening tag 1 for the property list",
            ));
        }
        self.offset = end;
        Ok(())
    }

    fn property_error(
        &self,
        oid: ObjectIdentifier,
        error: PropertyValueDecodeError,
    ) -> WritePropertyMultipleCursorError {
        let stage = match error.stage {
            PropertyValueDecodeStage::PropertyIdentifier => {
                WritePropertyMultipleDecodeStage::PropertyIdentifier
            }
            PropertyValueDecodeStage::ArrayIndex => WritePropertyMultipleDecodeStage::ArrayIndex,
            PropertyValueDecodeStage::Value => WritePropertyMultipleDecodeStage::Value,
            PropertyValueDecodeStage::Priority => WritePropertyMultipleDecodeStage::Priority,
        };
        let reference = error
            .reference_complete
            .then(|| BACnetObjectPropertyReference {
                object_identifier: oid,
                property_identifier: error
                    .property_identifier
                    .expect("complete reference has a property")
                    .to_raw(),
                property_array_index: error.property_array_index,
            });
        self.error(
            error.offset,
            stage,
            error.reject_reason,
            reference,
            error.error.to_string(),
        )
    }

    fn limit_error(&self, message: &str) -> WritePropertyMultipleCursorError {
        self.error(
            self.offset,
            WritePropertyMultipleDecodeStage::ItemLimit,
            RejectReason::TOO_MANY_ARGUMENTS,
            None,
            message,
        )
    }

    fn error(
        &self,
        offset: usize,
        stage: WritePropertyMultipleDecodeStage,
        reject_reason: RejectReason,
        first_failed_write_attempt: Option<BACnetObjectPropertyReference>,
        message: impl Into<String>,
    ) -> WritePropertyMultipleCursorError {
        WritePropertyMultipleCursorError {
            offset,
            stage,
            reject_reason,
            first_failed_write_attempt,
            message: message.into(),
        }
    }

    fn fail<T>(
        &mut self,
        error: WritePropertyMultipleCursorError,
    ) -> Result<T, WritePropertyMultipleCursorError> {
        self.state = State::Failed;
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::BACnetPropertyValue;
    use crate::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
    use bacnet_encoding::{primitives, tags};
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bytes::BytesMut;

    fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(object_type, instance).unwrap()
    }

    fn property(property_identifier: PropertyIdentifier) -> BACnetPropertyValue {
        let mut value = BytesMut::new();
        primitives::encode_app_null(&mut value);
        BACnetPropertyValue {
            property_identifier,
            property_array_index: None,
            value: value.to_vec(),
            priority: None,
        }
    }

    fn encode(specs: Vec<WriteAccessSpecification>) -> BytesMut {
        let mut data = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: specs,
        }
        .encode(&mut data);
        data
    }

    fn attempts(data: &[u8]) -> Vec<WritePropertyAttempt> {
        let mut cursor = WritePropertyMultipleCursor::new(data);
        let mut attempts = Vec::new();
        while let Some(event) = cursor.next_event().unwrap() {
            if let WritePropertyMultipleEvent::WriteAttempt(attempt) = event {
                attempts.push(attempt);
            }
        }
        attempts
    }

    #[test]
    fn yields_multi_object_and_property_attempts_in_wire_order() {
        let first = oid(ObjectType::ANALOG_OUTPUT, 1);
        let second = oid(ObjectType::BINARY_OUTPUT, 2);
        let data = encode(vec![
            WriteAccessSpecification {
                object_identifier: first,
                list_of_properties: vec![
                    property(PropertyIdentifier::DESCRIPTION),
                    property(PropertyIdentifier::PRESENT_VALUE),
                ],
            },
            WriteAccessSpecification {
                object_identifier: second,
                list_of_properties: vec![property(PropertyIdentifier::OUT_OF_SERVICE)],
            },
        ]);

        let coordinates: Vec<_> = attempts(&data)
            .into_iter()
            .map(|attempt| {
                (
                    attempt.reference.object_identifier,
                    attempt.reference.property_identifier,
                )
            })
            .collect();
        assert_eq!(
            coordinates,
            vec![
                (first, PropertyIdentifier::DESCRIPTION.to_raw()),
                (first, PropertyIdentifier::PRESENT_VALUE.to_raw()),
                (second, PropertyIdentifier::OUT_OF_SERVICE.to_raw()),
            ]
        );
    }

    #[test]
    fn empty_request_and_empty_property_lists_are_noops() {
        assert!(attempts(&[]).is_empty());
        let object = oid(ObjectType::BINARY_VALUE, 1);
        let data = encode(vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![],
        }]);
        let mut cursor = WritePropertyMultipleCursor::new(&data);
        assert_eq!(
            cursor.next_event().unwrap(),
            Some(WritePropertyMultipleEvent::ObjectStart(object))
        );
        assert_eq!(
            cursor.next_event().unwrap(),
            Some(WritePropertyMultipleEvent::ObjectEnd)
        );
        assert_eq!(cursor.next_event().unwrap(), None);
        assert_eq!(
            WritePropertyMultipleRequest::decode(&data)
                .unwrap()
                .list_of_write_access_specs[0]
                .list_of_properties,
            vec![]
        );
    }

    fn first_error(data: &[u8]) -> WritePropertyMultipleCursorError {
        let mut cursor = WritePropertyMultipleCursor::new(data);
        loop {
            match cursor.next_event() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("expected malformed request"),
                Err(error) => return error,
            }
        }
    }

    #[test]
    fn pre_write_failures_use_narrow_reject_classifications() {
        let object = oid(ObjectType::BINARY_VALUE, 1);
        let mut object_bytes = BytesMut::new();
        primitives::encode_ctx_object_id(&mut object_bytes, 0, &object);

        let mut wrong_context = object_bytes.clone();
        wrong_context[0] = 0x1c;
        assert_eq!(
            first_error(&wrong_context).reject_reason,
            RejectReason::INVALID_TAG
        );

        assert_eq!(
            first_error(&object_bytes).reject_reason,
            RejectReason::MISSING_REQUIRED_PARAMETER
        );

        let mut wrong_type = BytesMut::new();
        primitives::encode_app_object_id(&mut wrong_type, &object);
        assert_eq!(
            first_error(&wrong_type).reject_reason,
            RejectReason::INVALID_PARAMETER_DATA_TYPE
        );

        assert_eq!(
            first_error(&[0x0c, 0, 0, 0]).reject_reason,
            RejectReason::INVALID_DATA_ENCODING
        );

        let invalid_priority = encode(vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![BACnetPropertyValue {
                priority: Some(17),
                ..property(PropertyIdentifier::PRESENT_VALUE)
            }],
        }]);
        let error = first_error(&invalid_priority);
        assert_eq!(error.reject_reason, RejectReason::PARAMETER_OUT_OF_RANGE);
        assert_eq!(error.stage, WritePropertyMultipleDecodeStage::Priority);
        assert!(error.first_failed_write_attempt.is_some());

        let mut unexpected = encode(vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![property(PropertyIdentifier::DESCRIPTION)],
        }]);
        unexpected.truncate(unexpected.len() - 1);
        unexpected.extend_from_slice(&[0x49, 0x00, 0x1f]);
        let error = first_error(&unexpected);
        assert_eq!(error.reject_reason, RejectReason::INVALID_TAG);
        assert!(error.first_failed_write_attempt.is_some());

        let mut malformed_value = object_bytes.clone();
        tags::encode_opening_tag(&mut malformed_value, 1);
        primitives::encode_ctx_unsigned(
            &mut malformed_value,
            0,
            PropertyIdentifier::DESCRIPTION.to_raw() as u64,
        );
        tags::encode_opening_tag(&mut malformed_value, 2);
        malformed_value.extend_from_slice(&[0x75, 10, b'x', 0x2f, 0x1f]);
        assert_eq!(
            first_error(&malformed_value).reject_reason,
            RejectReason::INVALID_DATA_ENCODING
        );
    }

    #[test]
    fn total_attempt_bound_prevents_unbounded_collection() {
        let object = oid(ObjectType::BINARY_VALUE, 1);
        let data = encode(vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: (0..=MAX_DECODED_ITEMS)
                .map(|_| property(PropertyIdentifier::DESCRIPTION))
                .collect(),
        }]);
        let error = first_error(&data);
        assert_eq!(error.reject_reason, RejectReason::TOO_MANY_ARGUMENTS);
        assert_eq!(error.stage, WritePropertyMultipleDecodeStage::ItemLimit);
    }

    #[test]
    fn unmatched_closing_tag_is_invalid_tag() {
        let object = oid(ObjectType::BINARY_VALUE, 1);
        let mut data = BytesMut::new();
        primitives::encode_ctx_object_id(&mut data, 0, &object);
        tags::encode_opening_tag(&mut data, 1);
        tags::encode_closing_tag(&mut data, 2);
        assert_eq!(first_error(&data).reject_reason, RejectReason::INVALID_TAG);
    }
}
