use crate::port::DataAttribute;
use crate::sc_frame::{encode_sc_message, ScFunction, ScMessage, ScOption, BROADCAST_VMAC};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tracing::warn;

use super::WebSocketPort;

const MAX_SC_DATA_ATTRIBUTES: usize = 64;
const SECURE_PATH_OPTION_TYPE: u8 = 1;

pub(super) fn from_data_options(msg: &ScMessage) -> Vec<DataAttribute> {
    msg.data_options
        .iter()
        .map(|option| DataAttribute {
            option_type: option.option_type,
            must_understand: option.must_understand,
            data: option.data.clone(),
        })
        .collect()
}

pub(super) fn unsupported_must_understand_data_option(msg: &ScMessage) -> Option<&ScOption> {
    if msg.function != ScFunction::EncapsulatedNpdu {
        return None;
    }

    msg.data_options
        .iter()
        .find(|option| option.must_understand && !is_understood_data_option(option))
}

pub(super) fn option_header_marker(option: &ScOption) -> u8 {
    let mut marker = option.option_type & 0x1F;
    if option.must_understand {
        marker |= 0x40;
    }
    if !option.data.is_empty() {
        marker |= 0x20;
    }
    marker
}

pub(super) async fn reject_unsupported_must_understand_data_option<W: WebSocketPort>(
    msg: &ScMessage,
    ws: &W,
) -> bool {
    let Some(option) = unsupported_must_understand_data_option(msg) else {
        return false;
    };

    let marker = option_header_marker(option);
    warn!(
        option_type = option.option_type,
        marker, "BACnet/SC unsupported Must Understand Data Option"
    );

    if msg.destination_vmac != Some(BROADCAST_VMAC) {
        let nak = build_bvlc_result_nak(
            msg.message_id,
            msg.function,
            marker,
            ErrorClass::COMMUNICATION,
            ErrorCode::HEADER_NOT_UNDERSTOOD,
        );
        let mut nak_buf = BytesMut::new();
        encode_sc_message(&mut nak_buf, &nak);
        if let Err(e) = ws.send(&nak_buf).await {
            warn!("BACnet/SC data option NAK send error: {}", e);
        }
    }

    true
}

fn build_bvlc_result_nak(
    message_id: u16,
    result_for: ScFunction,
    error_header_marker: u8,
    error_class: ErrorClass,
    error_code: ErrorCode,
) -> ScMessage {
    let error_class = error_class.to_raw().to_be_bytes();
    let error_code = error_code.to_raw().to_be_bytes();
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![
            result_for.to_raw(),
            0x01, // NAK
            error_header_marker,
            error_class[0],
            error_class[1],
            error_code[0],
            error_code[1],
        ]),
    }
}

fn is_understood_data_option(option: &ScOption) -> bool {
    option.option_type == SECURE_PATH_OPTION_TYPE
}

pub(super) fn to_data_options(data_attributes: &[DataAttribute]) -> Result<Vec<ScOption>, Error> {
    if data_attributes.len() > MAX_SC_DATA_ATTRIBUTES {
        return Err(Error::Encoding(format!(
            "BACnet/SC Data Options exceed {MAX_SC_DATA_ATTRIBUTES} attributes"
        )));
    }

    data_attributes
        .iter()
        .map(|attribute| {
            if !(1..=31).contains(&attribute.option_type) {
                return Err(Error::Encoding(format!(
                    "BACnet/SC Data Option type must be 1..31, got {}",
                    attribute.option_type
                )));
            }
            if attribute.data.len() > u16::MAX as usize {
                return Err(Error::Encoding(format!(
                    "BACnet/SC Data Option type {} payload length {} exceeds 65535",
                    attribute.option_type,
                    attribute.data.len()
                )));
            }

            Ok(ScOption {
                option_type: attribute.option_type,
                must_understand: attribute.must_understand,
                data: attribute.data.clone(),
            })
        })
        .collect()
}
