//! The network and application layers of a frame, decoded as far as a
//! summary needs: routing, the message type, the PDU type, invoke ID,
//! service and what went wrong.

use bacnet_encoding::apdu::{self, Apdu};
use bacnet_encoding::npdu;
use bacnet_services::who_is::IAmRequest;
use bacnet_types::enums::UnconfirmedServiceChoice;
use bytes::Bytes;

/// Decoded network layer.
#[derive(Debug)]
pub struct DecodedNpdu {
    /// A network-layer message rather than an APDU.
    pub is_network_message: bool,
    /// SNET, when a router added one.
    pub source_network: Option<u16>,
    /// DNET, for a routed or globally broadcast NPDU.
    pub dest_network: Option<u16>,
    /// The network-layer message type, for a network message.
    pub network_message_type: Option<u8>,
    /// A DNET with an empty DADR: a broadcast on that network, or a global one.
    pub broadcast: bool,
    /// The APDU, unless this is a network message or carries none.
    pub apdu: Option<DecodedApdu>,
}

/// Decoded application layer.
#[derive(Debug)]
pub struct DecodedApdu {
    /// The PDU type, such as `Confirmed-Request`.
    pub pdu_type: String,
    /// The invoke ID, for a PDU that carries one.
    pub invoke_id: Option<u8>,
    /// The segmented-message flag.
    pub segmented: bool,
    /// The service as the one-shot capture prints it, such as
    /// `READ_PROPERTY-ACK`.
    pub service_name: String,
    /// The service the PDU belongs to, without `-ACK` or `-Error`; `None`
    /// for a Segment-ACK, a Reject or an Abort, which don't carry one.
    pub service: Option<String>,
    /// What went wrong: an Error's class and code, a Reject's or an
    /// Abort's reason.
    pub problem: Option<String>,
    /// The device instance an I-Am announces.
    pub device: Option<u32>,
}

/// Decode the NPDU layer a BVLL message carries.
pub fn decode_npdu_layer(npdu: Bytes) -> Result<DecodedNpdu, String> {
    let npdu_result = npdu::decode_npdu(npdu).map_err(|e| format!("NPDU decode error: {e}"))?;

    let source_network = npdu_result.source.as_ref().map(|a| a.network);
    let dest_network = npdu_result.destination.as_ref().map(|a| a.network);

    let apdu = if !npdu_result.is_network_message() && !npdu_result.payload.is_empty() {
        Some(decode_apdu_layer(&npdu_result)?)
    } else {
        None
    };

    Ok(DecodedNpdu {
        is_network_message: npdu_result.is_network_message(),
        source_network,
        dest_network,
        network_message_type: npdu_result.message_type.map(|kind| kind.to_raw()),
        broadcast: npdu_result
            .destination
            .as_ref()
            .is_some_and(|d| d.mac_address.is_empty()),
        apdu,
    })
}

/// Decode the APDU layer from an NPDU.
fn decode_apdu_layer(npdu_data: &npdu::Npdu) -> Result<DecodedApdu, String> {
    let apdu_result = apdu::decode_apdu(npdu_data.payload.clone())
        .map_err(|e| format!("APDU decode error: {e}"))?;

    let service = |choice: &dyn std::fmt::Display| Some(choice.to_string());
    let decoded = match apdu_result {
        Apdu::ConfirmedRequest(ref cr) => DecodedApdu {
            pdu_type: "Confirmed-Request".to_string(),
            invoke_id: Some(cr.invoke_id),
            segmented: cr.segmented,
            service_name: format!("{}", cr.service_choice),
            service: service(&cr.service_choice),
            problem: None,
            device: None,
        },
        Apdu::UnconfirmedRequest(ref ur) => DecodedApdu {
            pdu_type: "Unconfirmed-Request".to_string(),
            invoke_id: None,
            segmented: false,
            service_name: format!("{}", ur.service_choice),
            service: service(&ur.service_choice),
            problem: None,
            device: (ur.service_choice == UnconfirmedServiceChoice::I_AM)
                .then(|| IAmRequest::decode(&ur.service_request).ok())
                .flatten()
                .map(|i_am| i_am.object_identifier.instance_number()),
        },
        Apdu::SimpleAck(ref sa) => DecodedApdu {
            pdu_type: "Simple-ACK".to_string(),
            invoke_id: Some(sa.invoke_id),
            segmented: false,
            service_name: format!("{}-ACK", sa.service_choice),
            service: service(&sa.service_choice),
            problem: None,
            device: None,
        },
        Apdu::ComplexAck(ref ca) => DecodedApdu {
            pdu_type: "Complex-ACK".to_string(),
            invoke_id: Some(ca.invoke_id),
            segmented: ca.segmented,
            service_name: format!("{}-ACK", ca.service_choice),
            service: service(&ca.service_choice),
            problem: None,
            device: None,
        },
        Apdu::SegmentAck(ref sa) => DecodedApdu {
            pdu_type: "Segment-ACK".to_string(),
            invoke_id: Some(sa.invoke_id),
            segmented: false,
            service_name: "SegmentACK".to_string(),
            service: None,
            problem: None,
            device: None,
        },
        Apdu::Error(ref ep) => DecodedApdu {
            pdu_type: "Error".to_string(),
            invoke_id: Some(ep.invoke_id),
            segmented: false,
            service_name: format!("{}-Error", ep.service_choice),
            service: service(&ep.service_choice),
            problem: Some(format!("{}:{}", ep.error_class, ep.error_code)),
            device: None,
        },
        Apdu::Reject(ref rp) => DecodedApdu {
            pdu_type: "Reject".to_string(),
            invoke_id: Some(rp.invoke_id),
            segmented: false,
            service_name: format!("Reject({})", rp.reject_reason),
            service: None,
            problem: Some(format!("reject {}", rp.reject_reason)),
            device: None,
        },
        Apdu::Abort(ref ap) => DecodedApdu {
            pdu_type: "Abort".to_string(),
            invoke_id: Some(ap.invoke_id),
            segmented: false,
            service_name: format!("Abort({})", ap.abort_reason),
            service: None,
            problem: Some(format!("abort {}", ap.abort_reason)),
            device: None,
        },
    };

    Ok(decoded)
}
