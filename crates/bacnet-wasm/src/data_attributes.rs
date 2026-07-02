//! BACnet/SC Data Option mapping for WASM/browser clients.

use crate::sc_frame::{ScControl, ScFunction, ScMessage, ScOption, Vmac, BROADCAST_VMAC};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

pub(crate) const MAX_SC_DATA_ATTRIBUTES: usize = 64;
pub(crate) const MAX_SC_DATA_ATTRIBUTE_PAYLOAD: usize = u16::MAX as usize;
const SECURE_PATH_OPTION_TYPE: u8 = 1;
const SC_MIN_HEADER_LEN: usize = 4;
const MAX_SC_OPTIONS: usize = 64;

/// Data-link attribute carried by BACnet/SC Data Options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataAttribute {
    /// Attribute/header option type. BACnet/SC uses values 1..31.
    #[serde(alias = "optionType")]
    pub option_type: u8,
    /// Whether the final consumer must understand this attribute.
    #[serde(alias = "mustUnderstand")]
    pub must_understand: bool,
    /// Attribute payload bytes, if any.
    pub data: Vec<u8>,
}

pub(crate) fn from_data_options(msg: &ScMessage) -> Vec<DataAttribute> {
    msg.data_options
        .iter()
        .map(|option| DataAttribute {
            option_type: option.option_type,
            must_understand: option.must_understand,
            data: option.data.clone(),
        })
        .collect()
}

pub(crate) fn to_data_options(data_attributes: &[DataAttribute]) -> Result<Vec<ScOption>, Error> {
    if data_attributes.len() > MAX_SC_DATA_ATTRIBUTES {
        return Err(Error::Encoding(format!(
            "BACnet/SC Data Options exceed {MAX_SC_DATA_ATTRIBUTES} attributes"
        )));
    }

    data_attributes
        .iter()
        .map(|attribute| {
            validate_data_attribute(attribute)?;

            Ok(ScOption {
                option_type: attribute.option_type,
                must_understand: attribute.must_understand,
                data: attribute.data.clone(),
            })
        })
        .collect()
}

pub(crate) fn encoded_data_options_len(data_attributes: &[DataAttribute]) -> Result<usize, Error> {
    if data_attributes.len() > MAX_SC_DATA_ATTRIBUTES {
        return Err(Error::Encoding(format!(
            "BACnet/SC Data Options exceed {MAX_SC_DATA_ATTRIBUTES} attributes"
        )));
    }

    let mut len = 0usize;
    for attribute in data_attributes {
        validate_data_attribute(attribute)?;
        len += 1;
        if !attribute.data.is_empty() {
            len += 2 + attribute.data.len();
        }
    }
    Ok(len)
}

fn validate_data_attribute(attribute: &DataAttribute) -> Result<(), Error> {
    if !(1..=31).contains(&attribute.option_type) {
        return Err(Error::Encoding(format!(
            "BACnet/SC Data Option type must be 1..31, got {}",
            attribute.option_type
        )));
    }
    if attribute.data.len() > MAX_SC_DATA_ATTRIBUTE_PAYLOAD {
        return Err(Error::Encoding(format!(
            "BACnet/SC Data Option type {} payload length {} exceeds 65535",
            attribute.option_type,
            attribute.data.len()
        )));
    }
    if attribute.option_type == SECURE_PATH_OPTION_TYPE
        && (!attribute.must_understand || !attribute.data.is_empty())
    {
        return Err(Error::Encoding(
            "BACnet/SC Secure Path Data Option must set Must Understand and omit data".into(),
        ));
    }
    Ok(())
}

pub(crate) fn rejected_data_option(msg: &ScMessage) -> Option<&ScOption> {
    if msg.function != ScFunction::EncapsulatedNpdu {
        return None;
    }

    msg.data_options.iter().find(|option| {
        is_malformed_secure_path_data_option(option)
            || (option.must_understand && !is_understood_data_option(option))
    })
}

pub(crate) fn option_header_marker(option: &ScOption) -> u8 {
    let mut marker = option.option_type & 0x1F;
    if option.must_understand {
        marker |= 0x40;
    }
    if !option.data.is_empty() {
        marker |= 0x20;
    }
    marker
}

pub(crate) fn unsupported_must_understand_result(msg: &ScMessage) -> Option<Option<ScMessage>> {
    let option = rejected_data_option(msg)?;
    if msg.destination_vmac == Some(BROADCAST_VMAC) {
        return Some(None);
    }

    Some(Some(build_bvlc_result_nak(
        msg.message_id,
        msg.function,
        option_header_marker(option),
        msg.originating_vmac,
        ErrorClass::COMMUNICATION,
        ErrorCode::HEADER_NOT_UNDERSTOOD,
    )))
}

pub(crate) fn malformed_secure_path_result_from_frame(frame: &[u8]) -> Option<Option<ScMessage>> {
    if frame.len() < SC_MIN_HEADER_LEN || !ScControl::has_valid_reserved_bits(frame[1]) {
        return None;
    }

    let function = ScFunction::from_raw(frame[0]);
    if function != ScFunction::EncapsulatedNpdu {
        return None;
    }

    let control = ScControl::from_byte(frame[1]);
    if !control.has_data_options {
        return None;
    }

    let message_id = u16::from_be_bytes([frame[2], frame[3]]);
    let mut offset = SC_MIN_HEADER_LEN;
    let originating_vmac = if control.has_originating_vmac {
        read_vmac(frame, &mut offset)?
    } else {
        None
    };
    let destination_vmac = if control.has_destination_vmac {
        read_vmac(frame, &mut offset)?
    } else {
        None
    };

    if control.has_dest_options {
        skip_sc_options(frame, &mut offset)?;
    }

    let error_header_marker = malformed_secure_path_marker(frame, &mut offset)?;
    if destination_vmac == Some(BROADCAST_VMAC) {
        return Some(None);
    }
    Some(Some(build_bvlc_result_nak(
        message_id,
        function,
        error_header_marker,
        originating_vmac,
        ErrorClass::COMMUNICATION,
        ErrorCode::HEADER_NOT_UNDERSTOOD,
    )))
}

fn build_bvlc_result_nak(
    message_id: u16,
    result_for: ScFunction,
    error_header_marker: u8,
    destination_vmac: Option<Vmac>,
    error_class: ErrorClass,
    error_code: ErrorCode,
) -> ScMessage {
    let error_class = error_class.to_raw().to_be_bytes();
    let error_code = error_code.to_raw().to_be_bytes();
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![
            result_for.to_raw(),
            0x01,
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
        && option.must_understand
        && option.data.is_empty()
}

fn is_malformed_secure_path_data_option(option: &ScOption) -> bool {
    option.option_type == SECURE_PATH_OPTION_TYPE
        && (!option.must_understand || !option.data.is_empty())
}

fn read_vmac(frame: &[u8], offset: &mut usize) -> Option<Option<Vmac>> {
    let end = offset.checked_add(6)?;
    let bytes = frame.get(*offset..end)?;
    let mut vmac = [0u8; 6];
    vmac.copy_from_slice(bytes);
    *offset = end;
    Some(Some(vmac))
}

fn skip_sc_options(frame: &[u8], offset: &mut usize) -> Option<()> {
    for _ in 0..MAX_SC_OPTIONS {
        let marker = *frame.get(*offset)?;
        *offset += 1;
        if marker & 0x1F == 0 {
            return None;
        }
        let has_data = marker & 0x20 != 0;
        let more_follows = marker & 0x80 != 0;
        if has_data {
            let length_end = offset.checked_add(2)?;
            let length =
                u16::from_be_bytes(frame.get(*offset..length_end)?.try_into().ok()?) as usize;
            *offset = length_end;
            *offset = offset.checked_add(length)?;
            frame.get(..*offset)?;
        }
        if !more_follows {
            return Some(());
        }
    }
    None
}

fn malformed_secure_path_marker(frame: &[u8], offset: &mut usize) -> Option<u8> {
    for _ in 0..MAX_SC_OPTIONS {
        let marker = *frame.get(*offset)?;
        *offset += 1;
        let option_type = marker & 0x1F;
        if option_type == 0 {
            return None;
        }
        let must_understand = marker & 0x40 != 0;
        let has_data = marker & 0x20 != 0;
        let more_follows = marker & 0x80 != 0;
        if option_type == SECURE_PATH_OPTION_TYPE && (!must_understand || has_data) {
            return Some(marker);
        }
        if has_data {
            let length_end = offset.checked_add(2)?;
            let length =
                u16::from_be_bytes(frame.get(*offset..length_end)?.try_into().ok()?) as usize;
            *offset = length_end;
            *offset = offset.checked_add(length)?;
            frame.get(..*offset)?;
        }
        if !more_follows {
            return None;
        }
    }
    None
}
