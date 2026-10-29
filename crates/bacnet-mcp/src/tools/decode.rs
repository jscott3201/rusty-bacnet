//! `bacnet_decode`: a frame, NPDU or APDU the agent passes as hex, decoded
//! by the capture screen's decoder (`bacnet_ops::decode`). Nothing is sent,
//! so it works in passive mode.
//!
//! The octets are the agent's, but whatever wrote them chose any character
//! string inside: those come back as untrusted envelopes, like any device
//! text.

use bacnet_ops::decode::frame::{FrameSummary, Framing, summarize_frame, summarize_npdu};
use bacnet_ops::decode::tree::{Field, decode_apdu_tree, decode_npdu_tree, decode_tree};
use bacnet_transport::port::TransportPort;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Spec, schema};
use crate::context::{Call, Ctx};
use crate::errors::ToolError;
use crate::untrusted::{clean, envelope};

pub(super) const NAME: &str = "bacnet_decode";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Decode a BACnet frame",
    description: "Decode a BACnet frame, NPDU or APDU given as hex, with no network traffic: \
                  link bip (B/IP, starting 81), bip6 (B/IPv6, 82), mstp (MS/TP, preamble 55 \
                  FF), sc (BACnet/SC), npdu (starting with version 01), apdu, or auto (the \
                  default; it can't tell sc). Returns a one-line summary and the decoded \
                  layers as a tree of fields, each with the octets it came from. Service \
                  parameters are named, with property values decoded, for ReadProperty, \
                  ReadPropertyMultiple, WriteProperty, the COV and event notifications, \
                  GetEventInformation, DeviceCommunicationControl and ReinitializeDevice \
                  (passwords hidden); any other service, a segment or a damaged frame is \
                  walked tag by tag. Labels quoting device text (character strings, \
                  decoded values that may hold one) come back as untrusted envelopes. At \
                  most 6144 octets. Works in passive mode. Local; read-only.",
    input,
    output,
    idempotent: true,
    open_world: false,
    passive: true,
};

/// The most octets decoded: room for the largest frame the stack accepts,
/// a BACnet/SC BVLC message of up to `DEFAULT_MAX_BVLC_LENGTH` (5705)
/// octets in `bacnet-transport`'s `sc_limits`, which isn't public.
pub(crate) const MAX_OCTETS: usize = 6_144;

/// The longest hex argument, separators included.
const MAX_HEX: usize = MAX_OCTETS * 3 + 16;

/// The longest label of the decoder's own kept whole.
const LABEL_CAP: usize = 512;

/// The most fields returned; a deeper or longer tree is cut.
const MAX_FIELDS: usize = 600;

fn input() -> Value {
    schema::input(
        json!({
            "hex": {
                "type": "string",
                "minLength": 2,
                "maxLength": MAX_HEX,
                "description": "The octets as hex; spaces, colons, dashes and a 0x prefix are \
                                ignored",
            },
            "link": {
                "enum": ["auto", "bip", "bip6", "mstp", "sc", "npdu", "apdu"],
                "default": "auto",
            },
        }),
        &["hex"],
    )
}

fn field_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "label": {
                "type": ["string", "object"],
                "description": "What the field is and holds; an untrusted envelope when it \
                                quotes a character string from the frame",
            },
            "range": {
                "type": "array",
                "items": { "type": "integer" },
                "description": "The octets it came from, [start, end)",
            },
            "children": { "type": "array", "items": { "type": "object" } },
            "truncated": {
                "type": "boolean",
                "description": "Set when the label was cut; range still covers every octet",
            },
        },
        "required": ["label", "range", "children"],
    })
}

fn output() -> Value {
    let text = json!({ "type": ["string", "null"] });
    let integer = json!({ "type": ["integer", "null"] });
    schema::output(json!({
        "link": { "enum": ["bip", "bip6", "mstp", "sc", "npdu", "apdu"] },
        "octets": { "type": "integer" },
        "text": { "type": "string", "description": "One line: what the frame is" },
        "summary": {
            "type": "object",
            "properties": {
                "link": { "type": "string" },
                "broadcast": { "type": "boolean" },
                "link_source": text,
                "link_destination": text,
                "snet": integer,
                "dnet": integer,
                "routed_source": text,
                "routed_destination": text,
                "network_message": text,
                "pdu": text,
                "invoke_id": integer,
                "service": text,
                "problem": text,
                "device": integer,
            },
        },
        "tree": { "type": "array", "items": field_schema() },
        "fields_cut": { "type": "boolean" },
    }))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Link {
    #[default]
    Auto,
    Bip,
    Bip6,
    Mstp,
    Sc,
    Npdu,
    Apdu,
}

impl Link {
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Bip => "bip",
            Self::Bip6 => "bip6",
            Self::Mstp => "mstp",
            Self::Sc => "sc",
            Self::Npdu => "npdu",
            Self::Apdu => "apdu",
        }
    }

    /// The link `octets` look like: a B/IP or B/IPv6 type octet, an MS/TP
    /// preamble, an NPDU's version 1, else an APDU. BACnet/SC frames start
    /// with a function that looks like anything, so they need `sc`.
    fn guess(octets: &[u8]) -> Self {
        match octets {
            [0x81, ..] => Self::Bip,
            [0x82, ..] => Self::Bip6,
            [0x55, 0xFF, ..] => Self::Mstp,
            [0x01, ..] => Self::Npdu,
            _ => Self::Apdu,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    hex: String,
    #[serde(default)]
    link: Link,
}

/// The octets of a hex argument, refused past [`MAX_OCTETS`].
pub(crate) fn octets(text: &str) -> Result<Vec<u8>, ToolError> {
    if text.len() > MAX_HEX {
        return Err(ToolError::invalid(
            "hex",
            format!("at most {MAX_OCTETS} octets"),
        ));
    }
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let digits: Vec<u8> = body
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b':' | b'-'))
        .collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return Err(ToolError::invalid("hex", "an even number of hex digits"));
    }
    if digits.len() / 2 > MAX_OCTETS {
        return Err(ToolError::invalid(
            "hex",
            format!("at most {MAX_OCTETS} octets"),
        ));
    }
    digits
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| ToolError::invalid("hex", "only hex digits and separators"))
        })
        .collect()
}

/// Fields as JSON, at most `budget` of them in all; a field quoting a
/// character string gets its label enveloped.
fn fields_json(fields: &[Field], budget: &mut usize, cap: usize) -> Vec<Value> {
    let mut out = Vec::new();
    for field in fields {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let (label, truncated) = if field.device_text {
            let label = envelope(&field.label, "decoded frame", cap);
            let truncated = label["truncated"] == json!(true);
            (label, truncated)
        } else {
            // The decoder's own words, cleaned all the same; a long one
            // (an unknown tag's octets as hex) is cut and says so.
            let (text, truncated) = clean(&field.label, LABEL_CAP);
            (json!(text), truncated)
        };
        let children = fields_json(&field.children, budget, cap);
        let mut row = json!({
            "label": label,
            "range": [field.range.start, field.range.end],
            "children": children,
        });
        if truncated {
            row["truncated"] = json!(true);
        }
        out.push(row);
    }
    out
}

/// The summary as JSON.
fn summary_json(summary: &FrameSummary) -> Value {
    json!({
        "link": summary.link,
        "broadcast": summary.broadcast,
        "link_source": summary.link_source,
        "link_destination": summary.link_destination,
        "snet": summary.snet,
        "dnet": summary.dnet,
        "routed_source": summary.routed_source,
        "routed_destination": summary.routed_destination,
        "network_message": summary.network_message,
        "pdu": summary.pdu,
        "invoke_id": summary.invoke_id,
        "service": summary.service,
        "problem": summary.problem,
        "device": summary.device,
    })
}

/// One line saying what the frame is: its link message, network message
/// or PDU and service, invoke ID, device and problem.
fn one_line(link: Link, summary: &FrameSummary) -> String {
    let mut parts = vec![link.name().to_string()];
    if !summary.link.is_empty() {
        parts.push(summary.link.clone());
    }
    if let Some(message) = &summary.network_message {
        parts.push(format!("network message {message}"));
    }
    if let Some(pdu) = &summary.pdu {
        parts.push(pdu.clone());
    }
    if let Some(service) = &summary.service {
        parts.push(service.clone());
    }
    if let Some(id) = summary.invoke_id {
        parts.push(format!("invoke {id}"));
    }
    if let Some(device) = summary.device {
        parts.push(format!("device {device}"));
    }
    if let Some(problem) = &summary.problem {
        parts.push(format!("problem: {problem}"));
    }
    parts.join(", ")
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    _call: &Call,
    args: Args,
) -> Result<Value, ToolError> {
    let octets = octets(&args.hex)?;
    let link = match args.link {
        Link::Auto => Link::guess(&octets),
        link => link,
    };
    let framing = match link {
        Link::Bip => Some(Framing::Bip),
        Link::Bip6 => Some(Framing::Bip6),
        Link::Mstp => Some(Framing::Mstp),
        Link::Sc => Some(Framing::Sc),
        _ => None,
    };
    let (summary, tree) = match (framing, link) {
        (Some(framing), _) => (
            summarize_frame(framing, &octets),
            decode_tree(framing, &octets),
        ),
        (None, Link::Npdu) => (summarize_npdu(&octets), decode_npdu_tree(&octets)),
        // An APDU on its own: summarised in an NPDU without routing.
        _ => {
            let mut npdu = vec![0x01, 0x00];
            npdu.extend_from_slice(&octets);
            (summarize_npdu(&npdu), vec![decode_apdu_tree(&octets)])
        }
    };
    let cap = ctx.policy.caps.text;
    let mut budget = MAX_FIELDS;
    let tree = fields_json(&tree, &mut budget, cap);
    // The summary's words come from the decoder, never from a string in
    // the frame, but they are cleaned too.
    let text = clean(&one_line(link, &summary), 1_024).0;
    Ok(json!({
        "link": link.name(),
        "octets": octets.len(),
        "text": text,
        "summary": summary_json(&summary),
        "tree": tree,
        "fields_cut": budget == 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_read_with_separators_and_capped() {
        assert_eq!(octets("0x81 0a:00-04").unwrap(), [0x81, 0x0A, 0x00, 0x04]);
        for bad in ["", "8", "zz", "81 0"] {
            assert!(octets(bad).is_err(), "{bad}");
        }
        assert!(octets(&"00".repeat(MAX_OCTETS)).is_ok());
        assert!(octets(&"00".repeat(MAX_OCTETS + 1)).is_err());
        assert!(octets(&" ".repeat(MAX_HEX + 1)).is_err());
    }

    /// A long label of the decoder's own (an unknown tag's octets) is cut
    /// and says so; a short one carries no flag.
    #[test]
    fn a_cut_label_says_so() {
        let field = |label: String| Field {
            label,
            range: 0..1,
            children: Vec::new(),
            device_text: false,
        };
        let fields = [
            field(format!("[5] {}", "AB".repeat(400))),
            field("Length: 7".into()),
        ];
        let mut budget = MAX_FIELDS;
        let json = fields_json(&fields, &mut budget, 256);
        assert_eq!(json[0]["truncated"], json!(true));
        assert_eq!(
            json[0]["label"].as_str().unwrap().chars().count(),
            LABEL_CAP
        );
        assert!(json[1].get("truncated").is_none());
    }

    #[test]
    fn the_link_is_guessed_from_the_first_octets() {
        assert_eq!(Link::guess(&[0x81, 0x0A]), Link::Bip);
        assert_eq!(Link::guess(&[0x82]), Link::Bip6);
        assert_eq!(Link::guess(&[0x55, 0xFF, 0x00]), Link::Mstp);
        assert_eq!(Link::guess(&[0x01, 0x04]), Link::Npdu);
        assert_eq!(Link::guess(&[0x10, 0x08]), Link::Apdu);
    }
}
