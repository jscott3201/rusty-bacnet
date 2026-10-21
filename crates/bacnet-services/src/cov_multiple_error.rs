//! SubscribeCOVPropertyMultiple-Error (Clause 21), a CHOICE: a general error
//! under `[0]`, or under `[1]` the first failed subscription's `[0]` monitored
//! object identifier, `[1]` property reference and `[2]` error.

use crate::common::error_type::{decode_error_in, decode_error_pdu, encode_error_in, error_pdu};
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::tagged::{
    decode_ctx_constructed, decode_ctx_object_id, expect_end,
};
use bacnet_encoding::constructed::{decode_property_reference, encode_property_reference};
use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::{BACnetObjectPropertyReference, PropertyReference};
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, ErrorDetail};
use bytes::BytesMut;

const WHAT: &str = "SubscribeCOVPropertyMultiple-Error";

/// The Result(-) body of SubscribeCOVPropertyMultiple (Clause 13.16.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeCOVPropertyMultipleError {
    /// Error class, of the general error or of the failed subscription.
    pub error_class: ErrorClass,
    /// Error code, of the general error or of the failed subscription.
    pub error_code: ErrorCode,
    /// The monitored object and the property reference (property and array
    /// index) of the COV reference that failed, for the
    /// first-failed-subscription choice; `None` for the general error choice,
    /// which reports a failure before any subscription was processed.
    pub first_failed_subscription: Option<BACnetObjectPropertyReference>,
}

impl SubscribeCOVPropertyMultipleError {
    /// Encode the choice the body holds.
    pub fn encode(&self, buf: &mut BytesMut) {
        let Some(subscription) = &self.first_failed_subscription else {
            encode_error_in(buf, 0, self.error_class, self.error_code);
            return;
        };
        tags::encode_opening_tag(buf, 1);
        primitives::encode_ctx_object_id(buf, 0, &subscription.object_identifier);
        tags::encode_opening_tag(buf, 1);
        encode_property_reference(
            buf,
            &PropertyReference {
                property_identifier: PropertyIdentifier::from_raw(subscription.property_identifier),
                property_array_index: subscription.property_array_index,
            },
        );
        tags::encode_closing_tag(buf, 1);
        encode_error_in(buf, 2, self.error_class, self.error_code);
        tags::encode_closing_tag(buf, 1);
    }

    /// Decode one complete body, either choice, with no trailing content.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let opening = tags::decode_tag(data, 0)?.0;
        if opening.is_opening_tag(0) {
            let ((error_class, error_code), end) = decode_error_in(data, 0, 0, WHAT)?;
            expect_end(data, end, end, WHAT)?;
            return Ok(Self {
                error_class,
                error_code,
                first_failed_subscription: None,
            });
        }
        let (subscription, end) = decode_ctx_constructed(data, 0, 1, WHAT)?;
        expect_end(data, end, end, WHAT)?;
        let (object_identifier, offset) = decode_ctx_object_id(
            subscription,
            0,
            0,
            "SubscribeCOVPropertyMultiple-Error monitored object identifier",
        )?;
        let reference_at = offset;
        let (reference_body, offset) = decode_ctx_constructed(subscription, offset, 1, WHAT)?;
        let (reference, reference_end) = decode_property_reference(reference_body, 0)?;
        expect_end(
            reference_body,
            reference_end,
            reference_at,
            "SubscribeCOVPropertyMultiple-Error monitored property reference",
        )?;
        let ((error_class, error_code), end) = decode_error_in(subscription, offset, 2, WHAT)?;
        expect_end(subscription, end, end, WHAT)?;
        Ok(Self {
            error_class,
            error_code,
            first_failed_subscription: Some(BACnetObjectPropertyReference {
                object_identifier,
                property_identifier: reference.property_identifier.to_raw(),
                property_array_index: reference.property_array_index,
            }),
        })
    }

    /// Build the SubscribeCOVPropertyMultiple Error PDU carrying this body.
    pub fn to_error_pdu(&self, invoke_id: u8) -> ErrorPdu {
        error_pdu(
            invoke_id,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            (self.error_class, self.error_code),
            |body| self.encode(body),
        )
    }
}

impl TryFrom<&ErrorPdu> for SubscribeCOVPropertyMultipleError {
    type Error = Error;

    /// Decode and verify the body of a SubscribeCOVPropertyMultiple Error
    /// PDU. A plain class/code error, which older devices send, is refused.
    fn try_from(pdu: &ErrorPdu) -> Result<Self, Self::Error> {
        decode_error_pdu(
            pdu,
            &[ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE],
            WHAT,
            Self::decode,
            |error| (error.error_class, error.error_code),
        )
    }
}

impl From<SubscribeCOVPropertyMultipleError> for Error {
    fn from(error: SubscribeCOVPropertyMultipleError) -> Self {
        Error::protocol(
            error.error_class,
            error.error_code,
            error
                .first_failed_subscription
                .map(ErrorDetail::FirstFailedSubscription),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;
    use bytes::Bytes;

    fn subscription(index: Option<u32>) -> SubscribeCOVPropertyMultipleError {
        SubscribeCOVPropertyMultipleError {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::NOT_COV_PROPERTY,
            first_failed_subscription: Some(BACnetObjectPropertyReference {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap(),
                property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
                property_array_index: index,
            }),
        }
    }

    fn general() -> SubscribeCOVPropertyMultipleError {
        SubscribeCOVPropertyMultipleError {
            error_class: ErrorClass::SERVICES,
            error_code: ErrorCode::VALUE_OUT_OF_RANGE,
            first_failed_subscription: None,
        }
    }

    fn encoded(error: &SubscribeCOVPropertyMultipleError) -> Vec<u8> {
        let mut body = BytesMut::new();
        error.encode(&mut body);
        body.to_vec()
    }

    #[test]
    fn both_choices_round_trip_with_exact_shape() {
        for (error, wire) in [
            (general(), &[0x0E, 0x91, 5, 0x91, 37, 0x0F][..]),
            // [1] { [0] object (0x0C, four octets), [1] { [0] property 85 },
            // [2] { class 2, code 44 } }
            (
                subscription(None),
                &[
                    0x1E, 0x0C, 0x00, 0x80, 0x00, 0x07, 0x1E, 0x09, 85, 0x1F, 0x2E, 0x91, 2, 0x91,
                    44, 0x2F, 0x1F,
                ],
            ),
            (
                subscription(Some(8)),
                &[
                    0x1E, 0x0C, 0x00, 0x80, 0x00, 0x07, 0x1E, 0x09, 85, 0x19, 8, 0x1F, 0x2E, 0x91,
                    2, 0x91, 44, 0x2F, 0x1F,
                ],
            ),
        ] {
            assert_eq!(encoded(&error), wire);
            assert_eq!(
                SubscribeCOVPropertyMultipleError::decode(wire).unwrap(),
                error
            );
            let pdu = error.to_error_pdu(4);
            assert_eq!(
                SubscribeCOVPropertyMultipleError::try_from(&pdu).unwrap(),
                error
            );
        }
    }

    #[test]
    fn only_the_subscription_choice_carries_a_detail() {
        assert!(matches!(
            Error::from(general()),
            Error::Protocol { class: ErrorClass::SERVICES, code: ErrorCode::VALUE_OUT_OF_RANGE }
        ));
        let error = subscription(Some(8));
        let reference = error.first_failed_subscription.clone().unwrap();
        assert!(matches!(
            Error::from(error),
            Error::Structured { class: ErrorClass::PROPERTY, code: ErrorCode::NOT_COV_PROPERTY, detail }
                if *detail == ErrorDetail::FirstFailedSubscription(reference)
        ));
    }

    #[test]
    fn decode_refuses_malformed_plain_and_mismatched() {
        let body = encoded(&subscription(Some(8)));
        for malformed in [
            Vec::new(),
            body[..body.len() - 1].to_vec(),
            [&body[..], &[0x00][..]].concat(),
            [&encoded(&general())[..], &body[..]].concat(),
            // The failed subscription without its [2] error.
            [&body[..12], &[0x1F][..]].concat(),
            // A three-octet object identifier.
            [&[0x1E, 0x0B][..], &body[3..]].concat(),
        ] {
            assert!(
                SubscribeCOVPropertyMultipleError::decode(&malformed).is_err(),
                "{malformed:02x?}"
            );
        }

        let mut pdu = subscription(None).to_error_pdu(1);
        pdu.error_code = ErrorCode::UNKNOWN_OBJECT;
        assert!(SubscribeCOVPropertyMultipleError::try_from(&pdu).is_err());
        let mut pdu = general().to_error_pdu(1);
        pdu.error_data = Bytes::from_static(&[0x91, 5, 0x91, 37]);
        assert!(SubscribeCOVPropertyMultipleError::try_from(&pdu).is_err());
        let mut pdu = general().to_error_pdu(1);
        pdu.service_choice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY;
        assert!(SubscribeCOVPropertyMultipleError::try_from(&pdu).is_err());
    }
}
