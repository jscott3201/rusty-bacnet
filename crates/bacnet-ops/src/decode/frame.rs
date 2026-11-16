//! One frame of any link the stack taps, read as far as its list row needs:
//! the link message, the network layer and the APDU's header.
//!
//! The link layers each wrap the NPDU differently, so [`link`] finds the
//! NPDU and what the link says around it; the network and application
//! layers are the same everywhere and go through [`super::decode_npdu_layer`].
//! [`summarize_npdu`] reads an NPDU given without its link.

use std::net::{SocketAddr, SocketAddrV6};
use std::ops::Range;

use crate::octets::hex_sep;
use crate::target::routed_station;
use bacnet_transport::bip6::{Bvll6, decode_bvll6, format_vmac_text};
use bacnet_transport::bvll::{self, BvlcFunction, BvlcResultCode, Bvll};
use bacnet_transport::mstp::codec::{FrameType, decode_frame, decode_frame_in_place};
use bacnet_transport::sc_frame::{
    ScBvlcResult, ScFunction, decode_sc_bvlc_result, decode_sc_message, is_broadcast_vmac,
};
use bacnet_types::MacAddr;
use bacnet_types::enums::ErrorCode;
use bytes::Bytes;

use super::decode_npdu_layer;

/// A routed address as the capture shows it: a station in the target
/// syntax, `2:0x05`, and a broadcast on a network (an empty MAC), which
/// names no station, as `2:*`.
fn routed(network: u16, mac: &[u8]) -> String {
    routed_station(network, mac).unwrap_or_else(|| format!("{network}:*"))
}

/// The link a frame was captured on, which decides how its octets are read.
///
/// Non-exhaustive: more links can join (BACnet Ethernet, Clause 7, is the
/// next one the stack taps), so a `match` outside this crate needs a
/// wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Framing {
    /// A B/IP BVLL message, the UDP payload (Annex J).
    Bip,
    /// A B/IPv6 BVLL message, the UDP payload (Annex U).
    Bip6,
    /// A BVLC-SC message, the WebSocket payload (Annex AB). Only an SC
    /// session (`sc-tls`) captures one.
    Sc,
    /// An MS/TP frame, preamble to CRC (Clause 9.3).
    Mstp,
}

impl Framing {
    /// The short name the capture list shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bip => "BIP",
            Self::Bip6 => "BIP6",
            Self::Sc => "SC",
            Self::Mstp => "MSTP",
        }
    }
}

/// What the capture list shows of a frame, read from its headers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct FrameSummary {
    /// The link message: a BVLC function or an MS/TP frame type, in kebab
    /// case (`original-unicast-npdu`, `token`), or `malformed`. Not the data
    /// link itself, which the caller names with [`Framing`].
    pub link_message: String,
    /// A link or network broadcast.
    pub broadcast: bool,
    /// The link's own source and destination, where the frame names them
    /// (MS/TP stations, B/IPv6 and BACnet/SC VMACs).
    pub link_source: Option<String>,
    /// See [`link_source`](Self::link_source).
    pub link_destination: Option<String>,
    /// The originating node's IP address and UDP port, as a BBMD wrote them
    /// into a message it forwarded: a B/IP Forwarded-NPDU (Annex J.2.5), or
    /// a B/IPv6 Forwarded-NPDU or Forwarded-Address-Resolution (Annex U.2.9,
    /// U.2.5). The forwarding peer itself is not in the octets.
    pub original_source: Option<String>,
    /// SNET, when a router added one.
    pub snet: Option<u16>,
    /// DNET, for a routed or globally broadcast NPDU.
    pub dnet: Option<u16>,
    /// The network-layer message type, for a network message.
    pub network_message: Option<String>,
    /// The APDU type, such as `Confirmed-Request`.
    pub pdu: Option<String>,
    /// The APDU's invoke ID.
    pub invoke_id: Option<u8>,
    /// The service, without `-ACK` or `-Error`.
    pub service: Option<String>,
    /// What went wrong: an Error, Reject or Abort, a NAK, a frame that
    /// doesn't decode.
    pub problem: Option<String>,
    /// A link, NPDU or APDU header decode failure. Unlike [`Self::problem`],
    /// this is absent for a successfully decoded protocol refusal.
    /// Service payloads are not exhaustively validated by this summary.
    pub parse_error: Option<String>,
    /// The device instance an I-Am announces.
    pub device: Option<u32>,
    /// SNET and SADR as a routed target, `2:0x7F000001C335`.
    pub routed_source: Option<String>,
    /// DNET and DADR the same way; `2:*` for a broadcast on network 2.
    pub routed_destination: Option<String>,
}

/// An enumeration's name as the TUI writes it: `READ_PROPERTY` and
/// `EncapsulatedNpdu` become `read-property` and `encapsulated-npdu`.
pub fn kebab(name: impl std::fmt::Display) -> String {
    let mut out = String::new();
    let mut lower_before = false;
    for ch in name.to_string().chars() {
        if ch.is_ascii_uppercase() && lower_before {
            out.push('-');
        }
        lower_before = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        out.push(match ch {
            '_' => '-',
            ch => ch.to_ascii_lowercase(),
        });
    }
    out
}

/// What the link layer says about a frame, and where its NPDU is.
pub struct Link {
    /// The link message in kebab case: a BVLC function, an MS/TP frame
    /// type, or `malformed`.
    pub name: String,
    /// A link broadcast.
    pub broadcast: bool,
    /// The link's source, where the frame names one.
    pub source: Option<String>,
    /// The link's destination, where the frame names one.
    pub destination: Option<String>,
    /// The originating node's address a forwarded message carries; see
    /// [`FrameSummary::original_source`].
    pub original_source: Option<String>,
    /// [`Self::original_source`] as an address.
    pub origin: Option<SocketAddr>,
    /// The source MAC the frame itself names, as the stack writes a MAC on
    /// the link: a B/IPv6 source VMAC (the original source's, in a message
    /// a BBMD relays), a BACnet/SC originating VMAC, an MS/TP source
    /// station, and on B/IP a Forwarded-NPDU's origin as six octets. `None`
    /// where the frame names none: then the source is whoever sent it.
    pub source_mac: Option<MacAddr>,
    /// A NAK, or why the header doesn't decode.
    pub problem: Option<String>,
    /// Whether the link header refused to decode, so `npdu` is unknown
    /// rather than absent.
    pub malformed: bool,
    /// The NPDU, as a range of the frame's octets, or decoded on its own
    /// for an extended MS/TP frame, whose data is COBS-encoded on the wire.
    pub npdu: Option<Npdu>,
}

/// Where a frame's NPDU is.
pub enum Npdu {
    /// Inside the frame, as sent.
    At(Range<usize>),
    /// Encoded in `wire`; `data` is the decoded NPDU.
    Encoded {
        /// The encoded octets in the frame.
        wire: Range<usize>,
        /// The NPDU.
        data: Vec<u8>,
    },
}

/// Where `part`, a slice of `frame`, starts in it.
fn offset_in(frame: &[u8], part: &[u8]) -> usize {
    (part.as_ptr() as usize).saturating_sub(frame.as_ptr() as usize)
}

/// Read the link layer of `frame`.
pub fn link(framing: Framing, frame: &[u8]) -> Link {
    let mut link = Link {
        name: String::new(),
        broadcast: false,
        source: None,
        destination: None,
        original_source: None,
        origin: None,
        source_mac: None,
        problem: None,
        malformed: false,
        npdu: None,
    };
    let refused = match framing {
        Framing::Bip => bip(frame, &mut link),
        Framing::Bip6 => bip6(frame, &mut link),
        Framing::Sc => sc(frame, &mut link),
        Framing::Mstp => mstp(frame, &mut link),
    };
    if let Err(error) = refused {
        if link.name.is_empty() {
            link.name = "malformed".into();
        }
        link.problem = Some(format!("malformed: {error}"));
        link.malformed = true;
    }
    link
}

fn bip(frame: &[u8], link: &mut Link) -> Result<(), String> {
    let message = bvll::decode_bvll(frame).map_err(|e| e.to_string())?;
    // A function Annex J.2 doesn't assign (0x0C and up) fails to read below
    // and stays unnamed, `malformed`, as on the other two BVLC links.
    if message.function.is_defined() {
        link.name = kebab(message.function);
    }
    link.broadcast = matches!(
        message.function,
        BvlcFunction::ORIGINAL_BROADCAST_NPDU
            | BvlcFunction::FORWARDED_NPDU
            | BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK
    );
    match message.message().map_err(|e| e.to_string())? {
        Bvll::BvlcResult(code) if code != BvlcResultCode::SUCCESSFUL_COMPLETION => {
            link.problem = Some(format!("NAK {}", kebab(code)));
        }
        parsed => {
            if let Some(origin) = parsed.origin() {
                link.original_source = Some(origin.to_string());
                link.origin = Some(origin.into());
                let mac = bvll::encode_bip_mac(origin.ip().octets(), origin.port());
                link.source_mac = Some(MacAddr::from_slice(&mac));
            }
            if let Some(npdu) = parsed.npdu() {
                let start = 4 + offset_in(&message.payload, npdu);
                link.npdu = Some(Npdu::At(start..start + npdu.len()));
            }
        }
    }
    Ok(())
}

fn bip6(frame: &[u8], link: &mut Link) -> Result<(), String> {
    let message = decode_bvll6(frame).map_err(|e| e.to_string())?;
    link.name = kebab(message.function());
    link.source = Some(format_vmac_text(message.source_vmac()));
    link.source_mac = Some(MacAddr::from_slice(&message.source_vmac().octets()));
    link.destination = message.destination_vmac().map(format_vmac_text);
    link.original_source = message.origin().map(|address| address.to_string());
    link.origin = message
        .origin()
        .map(|address| SocketAddrV6::new(address.ip, address.port, 0, 0).into());
    link.broadcast = matches!(
        message,
        Bvll6::OriginalBroadcastNpdu { .. }
            | Bvll6::ForwardedNpdu { .. }
            | Bvll6::DistributeBroadcastToNetwork { .. }
    );
    if let Bvll6::BvlcResult { code, .. } = message
        && code.to_raw() != 0
    {
        link.problem = Some(format!("NAK {}", kebab(code)));
    }
    if let Some(npdu) = message.npdu() {
        let start = offset_in(frame, npdu);
        link.npdu = Some(Npdu::At(start..start + npdu.len()));
    }
    Ok(())
}

fn sc(frame: &[u8], link: &mut Link) -> Result<(), String> {
    let message = decode_sc_message(frame).map_err(|e| e.to_string())?;
    // Table AB-1 assigns 0x00 to 0x0C. The codec passes any other function
    // through, so it is refused here, as the B/IP and B/IPv6 codecs refuse
    // theirs: octets of another link read as BACnet/SC land here too.
    if let ScFunction::Unknown(function) = message.function {
        return Err(format!("unknown BVLC-SC function {function:#04X}"));
    }
    link.name = kebab(format!("{:?}", message.function));
    link.source = message.originating_vmac.map(|v| hex_sep(&v, ":"));
    link.source_mac = message.originating_vmac.map(|v| MacAddr::from_slice(&v));
    link.destination = message.destination_vmac.map(|v| hex_sep(&v, ":"));
    link.broadcast = message
        .destination_vmac
        .is_some_and(|vmac| is_broadcast_vmac(&vmac));
    match message.function {
        ScFunction::EncapsulatedNpdu => {
            let start = frame.len() - message.payload.len();
            link.npdu = Some(Npdu::At(start..frame.len()));
        }
        ScFunction::Result => {
            if let ScBvlcResult::Nak { error_code, .. } =
                decode_sc_bvlc_result(&message).map_err(|error| error.to_string())?
            {
                // Named in kebab case like the B/IP NAKs; an unnamed code
                // stays a number.
                let code = ErrorCode::from_raw(error_code);
                link.problem = Some(format!("NAK {}", kebab(code)));
            }
        }
        _ => {}
    }
    Ok(())
}

fn mstp(frame: &[u8], link: &mut Link) -> Result<(), String> {
    if frame.len() >= 5 {
        let frame_type = FrameType::from_raw(frame[2]);
        link.name = frame_type
            .name()
            .map_or_else(|| format!("type {}", frame_type.to_raw()), kebab);
        link.destination = Some(format!("mstp:{}", frame[3]));
        link.source = Some(format!("mstp:{}", frame[4]));
        link.source_mac = Some(MacAddr::from_slice(&frame[4..5]));
        link.broadcast = frame[3] == 255;
    }
    let frame_type = FrameType::from_raw(*frame.get(2).ok_or("shorter than a header")?);
    let carries_npdu = matches!(
        frame_type,
        FrameType::BACNET_DATA_EXPECTING_REPLY
            | FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    );
    if frame_type.is_cobs() {
        let mut copy = frame.to_vec();
        let (decoded, _) = decode_frame_in_place(&mut copy).map_err(|e| e.to_string())?;
        if carries_npdu {
            link.npdu = Some(Npdu::Encoded {
                wire: 8..frame.len().saturating_sub(5).max(8),
                data: decoded.data.to_vec(),
            });
        }
        return Ok(());
    }
    let (decoded, _) = decode_frame(frame).map_err(|e| e.to_string())?;
    if carries_npdu && !decoded.data.is_empty() {
        let start = offset_in(frame, decoded.data);
        link.npdu = Some(Npdu::At(start..start + decoded.data.len()));
    }
    Ok(())
}

/// Summarise a frame for the capture list. Decodes the headers only; the
/// service's own parameters are not decoded here.
pub fn summarize_frame(framing: Framing, frame: &[u8]) -> FrameSummary {
    let link = link(framing, frame);
    let parse_error = link.malformed.then(|| link.problem.clone()).flatten();
    let mut summary = FrameSummary {
        link_message: link.name,
        broadcast: link.broadcast,
        link_source: link.source,
        link_destination: link.destination,
        original_source: link.original_source,
        problem: link.problem,
        parse_error,
        ..FrameSummary::default()
    };
    let npdu = match link.npdu {
        Some(Npdu::At(range)) => Bytes::copy_from_slice(&frame[range]),
        Some(Npdu::Encoded { data, .. }) => Bytes::from(data),
        None => return summary,
    };
    network_layer(&mut summary, npdu);
    summary
}

/// Summarise an NPDU on its own, with no link around it: the network
/// layer and the APDU's header.
pub fn summarize_npdu(npdu: &[u8]) -> FrameSummary {
    let mut summary = FrameSummary::default();
    network_layer(&mut summary, Bytes::copy_from_slice(npdu));
    summary
}

/// What the network layer and the APDU's header add to a summary.
fn network_layer(summary: &mut FrameSummary, npdu: Bytes) {
    // The routing comes from the borrowed view, read once: it holds even
    // when the APDU behind it fails to decode below.
    if let Ok(view) = bacnet_npdu::Npdu::decode(&npdu) {
        summary.snet = view.source.map(|address| address.network);
        summary.dnet = view.destination.map(|address| address.network);
        summary.broadcast |= view
            .destination
            .is_some_and(|address| address.mac.is_empty());
        summary.routed_source = view.source.map(|a| routed(a.network, a.mac));
        summary.routed_destination = view.destination.map(|a| routed(a.network, a.mac));
    }
    match decode_npdu_layer(npdu) {
        Ok(npdu) => {
            summary.network_message = npdu
                .network_message_type
                .map(|kind| kebab(bacnet_npdu::NetworkMessageType::from_raw(kind)));
            if let Some(apdu) = npdu.apdu {
                summary.pdu = Some(apdu.pdu_type);
                summary.invoke_id = apdu.invoke_id;
                summary.service = apdu.service.map(kebab);
                summary.problem = apdu.problem.map(kebab);
                summary.device = apdu
                    .i_am
                    .and_then(Result::ok)
                    .map(|i_am| i_am.object_identifier.instance_number());
            }
        }
        Err(error) => {
            let error = format!("malformed: {error}");
            summary.parse_error = Some(error.clone());
            summary.problem = Some(error);
        }
    }
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod provenance_tests;
