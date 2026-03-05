//! BACnet service encode/decode for WASM consumers.
//!
//! Provides functions to build complete NPDU+APDU byte sequences for common
//! BACnet services, and to decode response APDUs back into JS-friendly values.

use bacnet_encoding::apdu::{self, Apdu, ConfirmedRequest, UnconfirmedRequest};
use bacnet_encoding::npdu::{self, Npdu};
use bacnet_encoding::primitives as enc;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_is::WhoIsRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
    UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Encode a ReadProperty request into NPDU+APDU bytes ready for SC framing.
#[wasm_bindgen(js_name = encodeReadProperty)]
pub fn encode_read_property(
    invoke_id: u8,
    object_type: u32,
    instance: u32,
    property_id: u32,
    array_index: Option<u32>,
) -> Result<Vec<u8>, JsError> {
    let object_identifier =
        ObjectIdentifier::new(ObjectType::from_raw(object_type), instance).map_err(js_err)?;

    let req = ReadPropertyRequest {
        object_identifier,
        property_identifier: PropertyIdentifier::from_raw(property_id),
        property_array_index: array_index,
    };

    let mut service_buf = BytesMut::new();
    req.encode(&mut service_buf);

    encode_confirmed_pdu(
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
        &service_buf,
    )
}

/// Encode a WriteProperty request into NPDU+APDU bytes.
///
/// `value_bytes` should contain pre-encoded application-tagged value data.
#[wasm_bindgen(js_name = encodeWriteProperty)]
pub fn encode_write_property(
    invoke_id: u8,
    object_type: u32,
    instance: u32,
    property_id: u32,
    value_bytes: &[u8],
    priority: Option<u8>,
) -> Result<Vec<u8>, JsError> {
    let object_identifier =
        ObjectIdentifier::new(ObjectType::from_raw(object_type), instance).map_err(js_err)?;

    let req = WritePropertyRequest {
        object_identifier,
        property_identifier: PropertyIdentifier::from_raw(property_id),
        property_array_index: None,
        property_value: value_bytes.to_vec(),
        priority,
    };

    let mut service_buf = BytesMut::new();
    req.encode(&mut service_buf);

    encode_confirmed_pdu(
        invoke_id,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &service_buf,
    )
}

/// Encode a WhoIs request into NPDU+APDU bytes.
#[wasm_bindgen(js_name = encodeWhoIs)]
pub fn encode_who_is(low: Option<u32>, high: Option<u32>) -> Result<Vec<u8>, JsError> {
    let req = WhoIsRequest {
        low_limit: low,
        high_limit: high,
    };

    let mut service_buf = BytesMut::new();
    req.encode(&mut service_buf);

    encode_unconfirmed_pdu(UnconfirmedServiceChoice::WHO_IS, &service_buf)
}

/// Encode a SubscribeCOV request into NPDU+APDU bytes.
#[wasm_bindgen(js_name = encodeSubscribeCov)]
pub fn encode_subscribe_cov(
    invoke_id: u8,
    process_id: u32,
    object_type: u32,
    instance: u32,
    confirmed: bool,
    lifetime: Option<u32>,
) -> Result<Vec<u8>, JsError> {
    let object_identifier =
        ObjectIdentifier::new(ObjectType::from_raw(object_type), instance).map_err(js_err)?;

    let req = SubscribeCOVRequest {
        subscriber_process_identifier: process_id,
        monitored_object_identifier: object_identifier,
        issue_confirmed_notifications: Some(confirmed),
        lifetime,
    };

    let mut service_buf = BytesMut::new();
    req.encode(&mut service_buf);

    encode_confirmed_pdu(
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
        &service_buf,
    )
}

/// Encode a Real (f32) value as application-tagged bytes for use with WriteProperty.
#[wasm_bindgen(js_name = encodeReal)]
pub fn encode_real(value: f32) -> Vec<u8> {
    let mut buf = BytesMut::new();
    enc::encode_app_real(&mut buf, value);
    buf.to_vec()
}

/// Encode an Unsigned Integer value as application-tagged bytes.
#[wasm_bindgen(js_name = encodeUnsigned)]
pub fn encode_unsigned(value: u32) -> Vec<u8> {
    let mut buf = BytesMut::new();
    enc::encode_app_unsigned(&mut buf, value as u64);
    buf.to_vec()
}

/// Encode a Boolean value as application-tagged bytes.
#[wasm_bindgen(js_name = encodeBoolean)]
pub fn encode_boolean(value: bool) -> Vec<u8> {
    let mut buf = BytesMut::new();
    enc::encode_app_boolean(&mut buf, value);
    buf.to_vec()
}

/// Encode an Enumerated value as application-tagged bytes.
#[wasm_bindgen(js_name = encodeEnumerated)]
pub fn encode_enumerated(value: u32) -> Vec<u8> {
    let mut buf = BytesMut::new();
    enc::encode_app_enumerated(&mut buf, value);
    buf.to_vec()
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decoded APDU result returned to JS.
#[derive(Serialize)]
pub struct DecodedApdu {
    /// "confirmed-ack", "simple-ack", "error", "reject", "abort", "unconfirmed"
    pub pdu_type: String,
    pub invoke_id: u8,
    pub service_choice: u8,
    /// Service-specific payload bytes (for ComplexAck)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Vec<u8>>,
    /// Error class (for Error PDU)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<u16>,
    /// Error code (for Error PDU)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u16>,
    /// Reject/abort reason
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<u8>,
}

/// Decode an APDU from raw bytes, returning a JS object.
#[wasm_bindgen(js_name = decodeApdu)]
pub fn decode_apdu(data: &[u8]) -> Result<JsValue, JsError> {
    let apdu = apdu::decode_apdu(Bytes::copy_from_slice(data)).map_err(js_err)?;
    let decoded = match apdu {
        Apdu::ComplexAck(ack) => DecodedApdu {
            pdu_type: "confirmed-ack".into(),
            invoke_id: ack.invoke_id,
            service_choice: ack.service_choice.to_raw(),
            payload: Some(ack.service_ack.to_vec()),
            error_class: None,
            error_code: None,
            reason: None,
        },
        Apdu::SimpleAck(ack) => DecodedApdu {
            pdu_type: "simple-ack".into(),
            invoke_id: ack.invoke_id,
            service_choice: ack.service_choice.to_raw(),
            payload: None,
            error_class: None,
            error_code: None,
            reason: None,
        },
        Apdu::Error(err) => DecodedApdu {
            pdu_type: "error".into(),
            invoke_id: err.invoke_id,
            service_choice: err.service_choice.to_raw(),
            payload: None,
            error_class: Some(err.error_class.to_raw()),
            error_code: Some(err.error_code.to_raw()),
            reason: None,
        },
        Apdu::Reject(rej) => DecodedApdu {
            pdu_type: "reject".into(),
            invoke_id: rej.invoke_id,
            service_choice: 0,
            payload: None,
            error_class: None,
            error_code: None,
            reason: Some(rej.reject_reason.to_raw()),
        },
        Apdu::Abort(abt) => DecodedApdu {
            pdu_type: "abort".into(),
            invoke_id: abt.invoke_id,
            service_choice: 0,
            payload: None,
            error_class: None,
            error_code: None,
            reason: Some(abt.abort_reason.to_raw()),
        },
        Apdu::UnconfirmedRequest(req) => DecodedApdu {
            pdu_type: "unconfirmed".into(),
            invoke_id: 0,
            service_choice: req.service_choice.to_raw(),
            payload: Some(req.service_request.to_vec()),
            error_class: None,
            error_code: None,
            reason: None,
        },
        Apdu::ConfirmedRequest(req) => DecodedApdu {
            pdu_type: "confirmed-request".into(),
            invoke_id: req.invoke_id,
            service_choice: req.service_choice.to_raw(),
            payload: Some(req.service_request.to_vec()),
            error_class: None,
            error_code: None,
            reason: None,
        },
        Apdu::SegmentAck(_) => DecodedApdu {
            pdu_type: "segment-ack".into(),
            invoke_id: 0,
            service_choice: 0,
            payload: None,
            error_class: None,
            error_code: None,
            reason: None,
        },
    };
    serde_wasm_bindgen::to_value(&decoded).map_err(js_err)
}

/// Decode a ReadProperty-ACK payload returning the property value bytes.
#[wasm_bindgen(js_name = decodeReadPropertyAck)]
pub fn decode_read_property_ack(data: &[u8]) -> Result<JsValue, JsError> {
    let ack = ReadPropertyACK::decode(data).map_err(js_err)?;

    #[derive(Serialize)]
    struct RpAck {
        object_type: u32,
        instance: u32,
        property_id: u32,
        array_index: Option<u32>,
        value_bytes: Vec<u8>,
    }

    let result = RpAck {
        object_type: ack.object_identifier.object_type().to_raw(),
        instance: ack.object_identifier.instance_number(),
        property_id: ack.property_identifier.to_raw(),
        array_index: ack.property_array_index,
        value_bytes: ack.property_value,
    };
    serde_wasm_bindgen::to_value(&result).map_err(js_err)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn encode_confirmed_pdu(
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    service_data: &[u8],
) -> Result<Vec<u8>, JsError> {
    let confirmed = ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request: Bytes::copy_from_slice(service_data),
    };

    let npdu = Npdu {
        expecting_reply: true,
        priority: NetworkPriority::NORMAL,
        ..Npdu::default()
    };

    let mut buf = BytesMut::new();
    npdu::encode_npdu(&mut buf, &npdu).map_err(js_err)?;
    apdu::encode_apdu(&mut buf, &Apdu::ConfirmedRequest(confirmed));
    Ok(buf.to_vec())
}

fn encode_unconfirmed_pdu(
    service_choice: UnconfirmedServiceChoice,
    service_data: &[u8],
) -> Result<Vec<u8>, JsError> {
    let unconfirmed = UnconfirmedRequest {
        service_choice,
        service_request: Bytes::copy_from_slice(service_data),
    };

    let npdu = Npdu {
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        ..Npdu::default()
    };

    let mut buf = BytesMut::new();
    npdu::encode_npdu(&mut buf, &npdu).map_err(js_err)?;
    apdu::encode_apdu(&mut buf, &Apdu::UnconfirmedRequest(unconfirmed));
    Ok(buf.to_vec())
}

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_read_property_produces_bytes() {
        // AI:1, PresentValue(85)
        let result = encode_read_property(1, 0, 1, 85, None).unwrap();
        assert!(!result.is_empty());
        // First byte should be NPDU version 0x01
        assert_eq!(result[0], 0x01);
    }

    #[test]
    fn encode_who_is_produces_bytes() {
        let result = encode_who_is(None, None).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0], 0x01); // NPDU version
    }

    #[test]
    fn encode_subscribe_cov_produces_bytes() {
        let result = encode_subscribe_cov(1, 42, 0, 1, true, Some(300)).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0], 0x01);
    }

    #[test]
    fn encode_real_round_trip() {
        let encoded = encode_real(72.5);
        assert!(!encoded.is_empty());
        // Application tag 4 (Real) = tag number 4
        assert_eq!(encoded[0] >> 4, 4);
    }

    #[test]
    fn encode_boolean_values() {
        let t = encode_boolean(true);
        let f = encode_boolean(false);
        assert!(!t.is_empty());
        assert!(!f.is_empty());
        // Application tag 1 (Boolean)
        assert_eq!(t[0] >> 4, 1);
        assert_eq!(f[0] >> 4, 1);
    }

    #[test]
    fn encode_write_property_produces_bytes() {
        let value = encode_real(72.5);
        let result = encode_write_property(1, 0, 1, 85, &value, Some(8)).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result[0], 0x01);
    }
}
