//! BACnet frame decoder — BVLC → NPDU → APDU pipeline.
//!
//! Takes raw UDP payload bytes (starting with BVLC header 0x81) and decodes
//! through all protocol layers. No pcap dependency — always compiled.
//!
//! The network and application layers are `bacnet_ops::decode`'s, which
//! the TUI's capture screen (#971) and the MCP server's `bacnet_decode`
//! share; it also reads the other links' frames and decodes a frame into a
//! tree of fields.

use bacnet_ops::decode::{DecodedNpdu, decode_npdu_layer};
use bacnet_transport::bvll::{self, BvlcFunction};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Decoded layer structs
// ---------------------------------------------------------------------------

/// Fully decoded BACnet packet (BVLC + optional NPDU + optional APDU).
#[derive(Debug)]
pub struct DecodedPacket {
    pub bvlc_function: BvlcFunction,
    pub bvlc_length: usize,
    pub forwarded_from: Option<([u8; 4], u16)>,
    pub npdu: Option<DecodedNpdu>,
}

/// One-line summary suitable for packet list output.
#[derive(Serialize)]
pub struct PacketSummary {
    pub bvlc: String,
    pub service: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Decode a raw UDP payload through BVLC → NPDU → APDU layers.
pub fn decode_packet(data: &[u8]) -> Result<DecodedPacket, String> {
    let bvll = bvll::decode_bvll(data).map_err(|e| format!("BVLC decode error: {e}"))?;

    let forwarded_from = bvll
        .origin()
        .map(|origin| (origin.ip().octets(), origin.port()));

    // A management frame is shown whatever its payload holds; an NPDU
    // carrier has to fit its layout for the NPDU to be found.
    let npdu = if is_npdu_carrier(bvll.function) {
        bvll.message()
            .map_err(|e| format!("BVLC decode error: {e}"))?;
        Some(decode_npdu_layer(bvll.npdu().unwrap_or_default())?)
    } else {
        None
    };

    Ok(DecodedPacket {
        bvlc_function: bvll.function,
        bvlc_length: data.len(),
        forwarded_from,
        npdu,
    })
}

/// Produce a one-line summary of the decoded packet.
pub fn summarize(packet: &DecodedPacket) -> PacketSummary {
    let bvlc = format!("{}", packet.bvlc_function);

    let service = match &packet.npdu {
        Some(npdu_layer) => match &npdu_layer.apdu {
            Some(apdu_layer) => apdu_layer.service_name.clone(),
            None => {
                if npdu_layer.is_network_message {
                    format!(
                        "NetworkMessage(0x{:02X})",
                        npdu_layer.network_message_type.unwrap_or(0)
                    )
                } else {
                    "NPDU".to_string()
                }
            }
        },
        None => bvlc.clone(),
    };

    PacketSummary { bvlc, service }
}

/// Produce detailed multi-line decode output (each line indented with 2 spaces).
pub fn format_detail(packet: &DecodedPacket) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push(format!(
        "  BVLC: {} (0x{:02x}), length={}",
        packet.bvlc_function,
        packet.bvlc_function.to_raw(),
        packet.bvlc_length,
    ));

    if let Some((ip, port)) = packet.forwarded_from {
        lines.push(format!(
            "  Forwarded-from: {}.{}.{}.{}:{}",
            ip[0], ip[1], ip[2], ip[3], port,
        ));
    }

    if let Some(npdu_layer) = &packet.npdu {
        let routing = match (npdu_layer.source_network, npdu_layer.dest_network) {
            (None, None) => "no-routing".to_string(),
            (Some(s), None) => format!("snet={s}"),
            (None, Some(d)) => format!("dnet={d}"),
            (Some(s), Some(d)) => format!("snet={s}, dnet={d}"),
        };
        lines.push(format!("  NPDU: version=1, {routing}"));

        if let Some(apdu_layer) = &npdu_layer.apdu {
            let seg = if apdu_layer.segmented { "yes" } else { "no" };
            let invoke = match apdu_layer.invoke_id {
                Some(id) => format!(", invoke-id={id}"),
                None => String::new(),
            };
            lines.push(format!(
                "  APDU: {}{invoke}, seg={seg}",
                apdu_layer.pdu_type,
            ));
            lines.push(format!("  Service: {}", apdu_layer.service_name));
        } else if npdu_layer.is_network_message {
            lines.push(format!(
                "  Network-Message: type=0x{:02X}",
                npdu_layer.network_message_type.unwrap_or(0),
            ));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Returns true for BVLC functions that carry an NPDU payload.
fn is_npdu_carrier(f: BvlcFunction) -> bool {
    f == BvlcFunction::ORIGINAL_UNICAST_NPDU
        || f == BvlcFunction::ORIGINAL_BROADCAST_NPDU
        || f == BvlcFunction::FORWARDED_NPDU
        || f == BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::apdu::{self, Apdu, ConfirmedRequest, UnconfirmedRequest};
    use bacnet_encoding::npdu::{self, Npdu};
    use bacnet_npdu::NetworkPriority;
    use bacnet_transport::bvll::{self, BvlcResultCode, Bvll};
    use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};
    use bytes::{Bytes, BytesMut};

    /// Build a complete BVLC-wrapped packet from an NPDU + APDU.
    fn build_packet(function: BvlcFunction, npdu_data: &Npdu, apdu_data: Option<&Apdu>) -> Vec<u8> {
        // Encode APDU
        let mut apdu_buf = BytesMut::new();
        if let Some(apdu) = apdu_data {
            apdu::encode_apdu(&mut apdu_buf, apdu).expect("valid APDU encoding");
        }

        // Build NPDU with APDU as payload
        let mut npdu_with_payload = npdu_data.clone();
        npdu_with_payload.payload = Bytes::from(apdu_buf.to_vec());

        let mut npdu_buf = BytesMut::new();
        npdu::encode_npdu(&mut npdu_buf, &npdu_with_payload).unwrap();

        // Wrap in BVLC
        let message = match function {
            BvlcFunction::ORIGINAL_BROADCAST_NPDU => Bvll::OriginalBroadcastNpdu(&npdu_buf),
            BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK => {
                Bvll::DistributeBroadcastToNetwork(&npdu_buf)
            }
            _ => Bvll::OriginalUnicastNpdu(&npdu_buf),
        };
        let mut bvlc_buf = BytesMut::new();
        bvll::encode_bvll(&mut bvlc_buf, &message).expect("valid BVLL encoding");
        bvlc_buf.to_vec()
    }

    fn simple_npdu() -> Npdu {
        Npdu {
            expecting_reply: false,
            priority: NetworkPriority::Normal,
            destination: None,
            source: None,
            hop_count: 255,
            message_type: None,
            vendor_id: None,
            payload: Bytes::new(),
        }
    }

    #[test]
    fn decode_whois_broadcast() {
        let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        });
        let data = build_packet(
            BvlcFunction::ORIGINAL_BROADCAST_NPDU,
            &simple_npdu(),
            Some(&apdu),
        );

        let packet = decode_packet(&data).unwrap();
        assert_eq!(packet.bvlc_function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
        assert!(packet.forwarded_from.is_none());

        let npdu_layer = packet.npdu.as_ref().unwrap();
        assert!(!npdu_layer.is_network_message);
        assert!(npdu_layer.source_network.is_none());
        assert!(npdu_layer.dest_network.is_none());

        let apdu_layer = npdu_layer.apdu.as_ref().unwrap();
        assert_eq!(apdu_layer.pdu_type, "Unconfirmed-Request");
        assert!(apdu_layer.invoke_id.is_none());
        assert_eq!(apdu_layer.service_name, "WHO_IS");
    }

    #[test]
    fn decode_read_property_unicast() {
        let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 7,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::from_static(&[0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]),
        });

        let npdu = Npdu {
            expecting_reply: true,
            ..simple_npdu()
        };
        let data = build_packet(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu, Some(&apdu));

        let packet = decode_packet(&data).unwrap();
        assert_eq!(packet.bvlc_function, BvlcFunction::ORIGINAL_UNICAST_NPDU);

        let npdu_layer = packet.npdu.as_ref().unwrap();

        let apdu_layer = npdu_layer.apdu.as_ref().unwrap();
        assert_eq!(apdu_layer.pdu_type, "Confirmed-Request");
        assert_eq!(apdu_layer.invoke_id, Some(7));
        assert!(!apdu_layer.segmented);
        assert_eq!(apdu_layer.service_name, "READ_PROPERTY");
    }

    #[test]
    fn summarize_whois() {
        let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        });
        let data = build_packet(
            BvlcFunction::ORIGINAL_BROADCAST_NPDU,
            &simple_npdu(),
            Some(&apdu),
        );
        let packet = decode_packet(&data).unwrap();
        let summary = summarize(&packet);

        assert_eq!(summary.bvlc, "ORIGINAL_BROADCAST_NPDU");
        assert_eq!(summary.service, "WHO_IS");
    }

    #[test]
    fn summarize_read_property() {
        let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 7,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::new(),
        });
        let data = build_packet(
            BvlcFunction::ORIGINAL_UNICAST_NPDU,
            &simple_npdu(),
            Some(&apdu),
        );
        let packet = decode_packet(&data).unwrap();
        let summary = summarize(&packet);

        assert_eq!(summary.bvlc, "ORIGINAL_UNICAST_NPDU");
        assert_eq!(summary.service, "READ_PROPERTY");
    }

    #[test]
    fn detail_output_has_expected_lines() {
        let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 7,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::new(),
        });
        let data = build_packet(
            BvlcFunction::ORIGINAL_UNICAST_NPDU,
            &simple_npdu(),
            Some(&apdu),
        );
        let packet = decode_packet(&data).unwrap();
        let lines = format_detail(&packet);

        assert!(
            lines.len() >= 4,
            "expected at least 4 lines, got {}",
            lines.len()
        );
        assert!(lines[0].contains("ORIGINAL_UNICAST_NPDU"));
        assert!(lines[0].contains("0x0a"));
        assert!(lines[0].contains("length="));
        assert!(lines[1].contains("NPDU: version=1, no-routing"));
        assert!(lines[2].contains("Confirmed-Request"));
        assert!(lines[2].contains("invoke-id=7"));
        assert!(lines[2].contains("seg=no"));
        assert!(lines[3].contains("READ_PROPERTY"));
    }

    #[test]
    fn decode_bvlc_result_no_npdu() {
        // BVLC-Result is a management frame with no NPDU.
        // Payload: 2-byte result code (0x0000 = success).
        let mut buf = BytesMut::new();
        bvll::encode_bvll(
            &mut buf,
            &Bvll::BvlcResult(BvlcResultCode::SUCCESSFUL_COMPLETION),
        )
        .expect("valid BVLL encoding");
        let data = buf.to_vec();

        let packet = decode_packet(&data).unwrap();
        assert_eq!(packet.bvlc_function, BvlcFunction::BVLC_RESULT);
        assert!(packet.npdu.is_none());
        assert!(packet.forwarded_from.is_none());
    }

    #[test]
    fn decode_truncated_data_returns_error() {
        // Too short for even a BVLC header.
        let result = decode_packet(&[0x81]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("BVLC"));

        // Valid header but length exceeds data.
        let result = decode_packet(&[0x81, 0x0A, 0x00, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_forwarded_npdu_reports_its_origin() {
        // Forwarded-NPDU from 192.168.1.10:47808 carrying a local Who-Is.
        let data = [
            0x81, 0x04, 0x00, 0x0E, 192, 168, 1, 10, 0xBA, 0xC0, 0x01, 0x00, 0x10, 0x08,
        ];
        let packet = decode_packet(&data).unwrap();
        assert_eq!(packet.bvlc_function, BvlcFunction::FORWARDED_NPDU);
        assert_eq!(packet.forwarded_from, Some(([192, 168, 1, 10], 0xBAC0)));
        assert!(packet.npdu.unwrap().apdu.is_some());

        // Too short for its originating address: no NPDU to find.
        let error = decode_packet(&[0x81, 0x04, 0x00, 0x07, 192, 168, 1]).unwrap_err();
        assert!(error.contains("BVLC decode error"), "{error}");

        // A management frame is shown even when its payload is malformed.
        let packet = decode_packet(&[0x81, 0x05, 0x00, 0x07, 0x00, 0x3C, 0x00]).unwrap();
        assert_eq!(packet.bvlc_function, BvlcFunction::REGISTER_FOREIGN_DEVICE);
        assert!(packet.npdu.is_none());
    }

    #[test]
    fn decode_routed_packet() {
        use bacnet_encoding::npdu::NpduAddress;

        let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        });

        let npdu = Npdu {
            source: Some(NpduAddress {
                network: 5,
                mac_address: bacnet_types::MacAddr::from_slice(&[0x01]),
            }),
            destination: Some(NpduAddress {
                network: 10,
                mac_address: bacnet_types::MacAddr::new(),
            }),
            hop_count: 254,
            ..simple_npdu()
        };

        let data = build_packet(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu, Some(&apdu));
        let packet = decode_packet(&data).unwrap();

        let npdu_layer = packet.npdu.as_ref().unwrap();
        assert_eq!(npdu_layer.source_network, Some(5));
        assert_eq!(npdu_layer.dest_network, Some(10));

        let lines = format_detail(&packet);
        let npdu_line = lines.iter().find(|l| l.contains("NPDU:")).unwrap();
        assert!(npdu_line.contains("snet=5"));
        assert!(npdu_line.contains("dnet=10"));
    }
}
