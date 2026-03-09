//! BACnet frame decoder — BVLC → NPDU → APDU pipeline.
//!
//! Takes raw UDP payload bytes (starting with BVLC header 0x81) and decodes
//! through all protocol layers. No pcap dependency — always compiled.

use bacnet_encoding::apdu::{self, Apdu};
use bacnet_encoding::npdu;
use bacnet_transport::bvll::{self, BvllMessage};

use bacnet_types::enums::BvlcFunction;
use bytes::Bytes;
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

/// Decoded network layer.
#[derive(Debug)]
pub struct DecodedNpdu {
    pub is_network_message: bool,
    pub expecting_reply: bool,
    pub source_network: Option<u16>,
    pub dest_network: Option<u16>,
    pub hop_count: u8,
    pub network_message_type: Option<u8>,
    pub apdu: Option<DecodedApdu>,
}

/// Decoded application layer.
#[derive(Debug)]
pub struct DecodedApdu {
    pub pdu_type: String,
    pub invoke_id: Option<u8>,
    pub segmented: bool,
    pub service_name: String,
    pub service_data: Bytes,
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

    let forwarded_from = match (bvll.originating_ip, bvll.originating_port) {
        (Some(ip), Some(port)) => Some((ip, port)),
        _ => None,
    };

    let npdu = if is_npdu_carrier(bvll.function) {
        Some(decode_npdu_layer(&bvll)?)
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

/// Decode the NPDU layer from a BVLL message.
fn decode_npdu_layer(bvll: &BvllMessage) -> Result<DecodedNpdu, String> {
    let npdu_result =
        npdu::decode_npdu(bvll.payload.clone()).map_err(|e| format!("NPDU decode error: {e}"))?;

    let source_network = npdu_result.source.as_ref().map(|a| a.network);
    let dest_network = npdu_result.destination.as_ref().map(|a| a.network);

    let apdu = if !npdu_result.is_network_message && !npdu_result.payload.is_empty() {
        Some(decode_apdu_layer(&npdu_result)?)
    } else {
        None
    };

    Ok(DecodedNpdu {
        is_network_message: npdu_result.is_network_message,
        expecting_reply: npdu_result.expecting_reply,
        source_network,
        dest_network,
        hop_count: npdu_result.hop_count,
        network_message_type: npdu_result.message_type,
        apdu,
    })
}

/// Decode the APDU layer from an NPDU.
fn decode_apdu_layer(npdu_data: &npdu::Npdu) -> Result<DecodedApdu, String> {
    let apdu_result = apdu::decode_apdu(npdu_data.payload.clone())
        .map_err(|e| format!("APDU decode error: {e}"))?;

    let decoded = match apdu_result {
        Apdu::ConfirmedRequest(ref cr) => DecodedApdu {
            pdu_type: "Confirmed-Request".to_string(),
            invoke_id: Some(cr.invoke_id),
            segmented: cr.segmented,
            service_name: format!("{}", cr.service_choice),
            service_data: cr.service_request.clone(),
        },
        Apdu::UnconfirmedRequest(ref ur) => DecodedApdu {
            pdu_type: "Unconfirmed-Request".to_string(),
            invoke_id: None,
            segmented: false,
            service_name: format!("{}", ur.service_choice),
            service_data: ur.service_request.clone(),
        },
        Apdu::SimpleAck(ref sa) => DecodedApdu {
            pdu_type: "Simple-ACK".to_string(),
            invoke_id: Some(sa.invoke_id),
            segmented: false,
            service_name: format!("{}-ACK", sa.service_choice),
            service_data: Bytes::new(),
        },
        Apdu::ComplexAck(ref ca) => DecodedApdu {
            pdu_type: "Complex-ACK".to_string(),
            invoke_id: Some(ca.invoke_id),
            segmented: ca.segmented,
            service_name: format!("{}-ACK", ca.service_choice),
            service_data: ca.service_ack.clone(),
        },
        Apdu::SegmentAck(ref sa) => DecodedApdu {
            pdu_type: "Segment-ACK".to_string(),
            invoke_id: Some(sa.invoke_id),
            segmented: false,
            service_name: "SegmentACK".to_string(),
            service_data: Bytes::new(),
        },
        Apdu::Error(ref ep) => DecodedApdu {
            pdu_type: "Error".to_string(),
            invoke_id: Some(ep.invoke_id),
            segmented: false,
            service_name: format!("{}-Error", ep.service_choice),
            service_data: ep.error_data.clone(),
        },
        Apdu::Reject(ref rp) => DecodedApdu {
            pdu_type: "Reject".to_string(),
            invoke_id: Some(rp.invoke_id),
            segmented: false,
            service_name: format!("Reject({})", rp.reject_reason),
            service_data: Bytes::new(),
        },
        Apdu::Abort(ref ap) => DecodedApdu {
            pdu_type: "Abort".to_string(),
            invoke_id: Some(ap.invoke_id),
            segmented: false,
            service_name: format!("Abort({})", ap.abort_reason),
            service_data: Bytes::new(),
        },
    };

    Ok(decoded)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::apdu::{self, Apdu, ConfirmedRequest, UnconfirmedRequest};
    use bacnet_encoding::npdu::{self, Npdu};
    use bacnet_transport::bvll;
    use bacnet_types::enums::{
        BvlcFunction, ConfirmedServiceChoice, NetworkPriority, UnconfirmedServiceChoice,
    };
    use bytes::{Bytes, BytesMut};

    /// Build a complete BVLC-wrapped packet from an NPDU + APDU.
    fn build_packet(function: BvlcFunction, npdu_data: &Npdu, apdu_data: Option<&Apdu>) -> Vec<u8> {
        // Encode APDU
        let mut apdu_buf = BytesMut::new();
        if let Some(apdu) = apdu_data {
            apdu::encode_apdu(&mut apdu_buf, apdu);
        }

        // Build NPDU with APDU as payload
        let mut npdu_with_payload = npdu_data.clone();
        npdu_with_payload.payload = Bytes::from(apdu_buf.to_vec());

        let mut npdu_buf = BytesMut::new();
        npdu::encode_npdu(&mut npdu_buf, &npdu_with_payload).unwrap();

        // Wrap in BVLC
        let mut bvlc_buf = BytesMut::new();
        bvll::encode_bvll(&mut bvlc_buf, function, &npdu_buf);
        bvlc_buf.to_vec()
    }

    fn simple_npdu() -> Npdu {
        Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
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
        assert!(npdu_layer.expecting_reply);

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
        bvll::encode_bvll(&mut buf, BvlcFunction::BVLC_RESULT, &[0x00, 0x00]);
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
