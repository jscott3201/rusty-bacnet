use bacnet_types::error::{DecodeError, DecodingKind, Error};

use super::{ScFunction, ScMessage, SC_MIN_HEADER};

/// Decoded BACnet/SC BVLC-Result payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScBvlcResult {
    /// ACK: successful completion for a BVLC function.
    Ack {
        /// Function code of the BVLC message this result answers.
        result_for: ScFunction,
    },
    /// NAK: failed BVLC function with error information.
    Nak {
        /// Function code of the BVLC message this result answers.
        result_for: ScFunction,
        /// Marker of the header option at fault, or 0 when no header option was involved.
        error_header_marker: u8,
        /// BACnet Error Class value.
        error_class: u16,
        /// BACnet Error Code value.
        error_code: u16,
        /// Optional UTF-8 error details, not Clause 20 CharacterString encoded.
        error_details: String,
    },
}

/// Decode the AB.2.4 BVLC-Result payload from an already decoded SC message.
pub fn decode_sc_bvlc_result(msg: &ScMessage) -> Result<ScBvlcResult, Error> {
    if msg.function != ScFunction::Result {
        return Err(Error::decoding(0, "SC message is not BVLC-Result"));
    }
    if !msg.data_options.is_empty() {
        return Err(Error::decoding(
            SC_MIN_HEADER,
            "data options are not allowed on a BVLC-Result",
        ));
    }
    if msg.payload.len() < 2 {
        return Err(Error::decoding(0, "BVLC-Result payload too short"));
    }

    let result_for = ScFunction::from_raw(msg.payload[0]);
    match msg.payload[1] {
        0x00 => {
            if msg.payload.len() != 2 {
                return Err(Error::decoding(
                    2,
                    "BVLC-Result ACK has trailing error fields",
                ));
            }
            Ok(ScBvlcResult::Ack { result_for })
        }
        0x01 => {
            if msg.payload.len() < 7 {
                return Err(Error::decoding(2, "BVLC-Result NAK payload too short"));
            }
            let details = core::str::from_utf8(&msg.payload[7..])
                .map_err(|_| Error::decoding(7, "BVLC-Result error details are not UTF-8"))?
                .to_owned();
            Ok(ScBvlcResult::Nak {
                result_for,
                error_header_marker: msg.payload[2],
                error_class: u16::from_be_bytes([msg.payload[3], msg.payload[4]]),
                error_code: u16::from_be_bytes([msg.payload[5], msg.payload[6]]),
                error_details: details,
            })
        }
        code => Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            1,
            "unknown BACnet/SC BVLC-Result code {:#04X}",
        )
        .with_value(code.into())
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::sc_frame::{decode_sc_message, ScOption};

    #[test]
    fn bvlc_result_ack_decode() {
        let msg = ScMessage {
            function: ScFunction::Result,
            message_id: 0x1001,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x0C, 0x00]),
        };

        assert_eq!(
            decode_sc_bvlc_result(&msg).unwrap(),
            ScBvlcResult::Ack {
                result_for: ScFunction::ProprietaryMessage,
            }
        );
    }

    #[test]
    fn bvlc_result_nak_decode_with_details() {
        // AB.2.17 examples encode BVLC-Result NAKs with originating VMAC and optional details.
        let mut data = Vec::new();
        data.extend_from_slice(&[
            0x00, // BVLC-Result
            0x08, // originating VMAC present
            0xB5, 0xEC, // copied response message ID
            0x92, 0x7B, 0xF7, 0x1A, 0x96, 0xA2, // originating VMAC
            0x01, // result for Encapsulated-NPDU
            0x01, // NAK
            0xBF, // error header marker
            0x00, 0x07, // error class COMMUNICATION
            0x01, 0x11, // proprietary error code 273
        ]);
        let details = [
            0x55, 0x6E, 0x6D, 0xC3, 0xB6, 0x67, 0x6C, 0x69, 0x63, 0x68, 0x65, 0x72, 0x20, 0x43,
            0x6F, 0x64, 0x65, 0x21,
        ];
        data.extend_from_slice(&details);

        let msg = decode_sc_message(&data).unwrap();
        assert_eq!(
            decode_sc_bvlc_result(&msg).unwrap(),
            ScBvlcResult::Nak {
                result_for: ScFunction::EncapsulatedNpdu,
                error_header_marker: 0xBF,
                error_class: 7,
                error_code: 273,
                error_details: String::from_utf8(details.to_vec()).unwrap(),
            }
        );
    }

    #[test]
    fn bvlc_result_nak_decode_without_details() {
        let msg = ScMessage {
            function: ScFunction::Result,
            message_id: 0xB5EC,
            originating_vmac: Some([0x92, 0x7B, 0xF7, 0x1A, 0x96, 0xA2]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x01, 0x3F, 0x00, 0x07, 0x01, 0x17]),
        };

        assert_eq!(
            decode_sc_bvlc_result(&msg).unwrap(),
            ScBvlcResult::Nak {
                result_for: ScFunction::EncapsulatedNpdu,
                error_header_marker: 0x3F,
                error_class: 7,
                error_code: 279,
                error_details: String::new(),
            }
        );
    }

    #[test]
    fn bvlc_result_rejects_malformed_payloads() {
        for payload in [
            &[][..],
            &[0x06][..],
            &[0x0C, 0x00, 0x00][..],
            &[0x06, 0x01, 0x00, 0x00, 0x07, 0x01][..],
            &[0x06, 0x02][..],
            &[0x06, 0x01, 0x00, 0x00, 0x07, 0x01, 0x11, 0xFF][..],
        ] {
            let msg = ScMessage {
                function: ScFunction::Result,
                message_id: 1,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::copy_from_slice(payload),
            };
            assert!(decode_sc_bvlc_result(&msg).is_err());
        }

        let data_option_msg = ScMessage {
            function: ScFunction::Result,
            message_id: 1,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: vec![ScOption {
                option_type: 1,
                must_understand: false,
                data: Vec::new(),
            }],
            payload: Bytes::from_static(&[0x0C, 0x00]),
        };
        assert!(decode_sc_bvlc_result(&data_option_msg).is_err());
    }

    /// An unknown result code prints as an upper-case, two-digit octet
    /// (#1803).
    #[test]
    fn unknown_result_code_prints_a_padded_octet() {
        let msg = ScMessage {
            function: ScFunction::Result,
            message_id: 1,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x06, 0x0B]),
        };
        match decode_sc_bvlc_result(&msg) {
            Err(Error::Decoding(refused)) => assert_eq!(
                refused.message().to_string(),
                "unknown BACnet/SC BVLC-Result code 0x0B"
            ),
            other => panic!("expected a decoding error, got {other:?}"),
        }
    }
}
