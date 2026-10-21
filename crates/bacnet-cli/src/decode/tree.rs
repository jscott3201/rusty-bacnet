//! A frame decoded into a tree of fields, each with the octets it came
//! from: the capture screen's decode tree, and the range its hex view
//! highlights for the selected field.
//!
//! Only the frame the user selects is decoded this far. Offsets come from
//! the codecs' own views where they give them (the BVLL and NPDU slices,
//! the NPDU's `payload_offset`) and otherwise from the fixed layout of each
//! header: Clause 9.3 for MS/TP, Annex J.2 and U.2 for the BVLLs, Annex
//! AB.2 for BVLC-SC, Clause 6.2 for the NPCI and Clause 20.1 for the APDU
//! headers. Service parameters are walked tag by tag (Clause 20.2).

use std::ops::Range;

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::{decode_tag, TagClass, MAX_CONTEXT_NESTING_DEPTH};
use bacnet_npdu::{NetworkPriority, Npdu as NpduView};
use bacnet_types::primitives::PropertyValue;
use bytes::Bytes;

use super::frame::{kebab, link, Framing, Link, Npdu};

/// One decoded field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// What it is and holds, such as `Invoke ID: 7`.
    pub label: String,
    /// The octets of the frame it came from; empty when it has none of its
    /// own.
    pub range: Range<usize>,
    /// Its parts.
    pub children: Vec<Field>,
}

impl Field {
    fn leaf(label: impl Into<String>, range: Range<usize>) -> Self {
        Self {
            label: label.into(),
            range,
            children: Vec::new(),
        }
    }

    fn node(label: impl Into<String>, range: Range<usize>, children: Vec<Self>) -> Self {
        Self {
            label: label.into(),
            range,
            children,
        }
    }

    /// Point this field and its parts at `range` instead: for an NPDU
    /// decoded out of COBS-encoded data, whose octets aren't the wire's.
    fn relocate(&mut self, range: &Range<usize>) {
        self.range = range.clone();
        for child in &mut self.children {
            child.relocate(range);
        }
    }
}

fn hex(octets: &[u8]) -> String {
    octets.iter().map(|b| format!("{b:02X}")).collect()
}

/// Decode `frame` into its fields, link layer first.
pub fn decode_tree(framing: Framing, frame: &[u8]) -> Vec<Field> {
    let link = link(framing, frame);
    let npdu_start = match &link.npdu {
        Some(Npdu::At(range)) => range.start,
        Some(Npdu::Encoded { wire, .. }) => wire.start,
        None => frame.len(),
    };
    let mut fields = vec![match framing {
        Framing::Bip => bvll(frame, npdu_start, &link),
        Framing::Bip6 => bvll6(frame, npdu_start, &link),
        Framing::Sc => sc(frame, npdu_start, &link),
        Framing::Mstp => mstp(frame, &link),
    }];
    match link.npdu {
        Some(Npdu::At(range)) => fields.extend(npdu(&frame[range.clone()], range.start)),
        Some(Npdu::Encoded { wire, data }) => {
            let mut decoded = npdu(&data, 0);
            for field in &mut decoded {
                field.relocate(&wire);
            }
            fields.extend(decoded);
        }
        None => {}
    }
    fields
}

/// The link header's node, with the problem the link reported last.
fn header(name: String, range: Range<usize>, mut kids: Vec<Field>, link: &Link) -> Field {
    if let Some(problem) = &link.problem {
        kids.push(Field::leaf(problem.clone(), range.clone()));
    }
    Field::node(name, range, kids)
}

fn bvll(frame: &[u8], npdu_start: usize, link: &Link) -> Field {
    let mut kids = Vec::new();
    if frame.len() >= 4 {
        kids.push(Field::leaf(
            format!("Type: 0x{:02X} (B/IP)", frame[0]),
            0..1,
        ));
        kids.push(Field::leaf(
            format!("Function: {} (0x{:02X})", link.name, frame[1]),
            1..2,
        ));
        let length = u16::from_be_bytes([frame[2], frame[3]]);
        kids.push(Field::leaf(format!("Length: {length}"), 2..4));
        let rest = &frame[4..npdu_start.max(4)];
        match (frame[1], rest) {
            (0x04, [a, b, c, d, hi, lo, ..]) => kids.push(Field::leaf(
                format!(
                    "Original source: {a}.{b}.{c}.{d}:{}",
                    u16::from_be_bytes([*hi, *lo])
                ),
                4..10,
            )),
            (0x00, [hi, lo]) => {
                let code = u16::from_be_bytes([*hi, *lo]);
                let name = bacnet_transport::bvll::BvlcResultCode::from_raw(code);
                kids.push(Field::leaf(
                    format!("Result code: {} (0x{code:04X})", kebab(name)),
                    4..6,
                ));
                // The code says it all.
                return Field::node(format!("BVLL: {}", link.name), 0..npdu_start, kids);
            }
            (0x05, [hi, lo]) => kids.push(Field::leaf(
                format!("Time-to-live: {} s", u16::from_be_bytes([*hi, *lo])),
                4..6,
            )),
            (_, []) => {}
            (_, rest) => kids.push(Field::leaf(
                format!("Payload: {} octets", rest.len()),
                4..4 + rest.len(),
            )),
        }
    }
    header(format!("BVLL: {}", link.name), 0..npdu_start, kids, link)
}

fn bvll6(frame: &[u8], npdu_start: usize, link: &Link) -> Field {
    let mut kids = Vec::new();
    if frame.len() >= 4 {
        kids.push(Field::leaf(
            format!("Type: 0x{:02X} (B/IPv6)", frame[0]),
            0..1,
        ));
        kids.push(Field::leaf(
            format!("Function: {} (0x{:02X})", link.name, frame[1]),
            1..2,
        ));
        let length = u16::from_be_bytes([frame[2], frame[3]]);
        kids.push(Field::leaf(format!("Length: {length}"), 2..4));
    }
    if let (Some(source), true) = (&link.source, frame.len() >= 7) {
        kids.push(Field::leaf(format!("Source VMAC: {source}"), 4..7));
    }
    if let (Some(destination), true) = (&link.destination, frame.len() >= 10) {
        kids.push(Field::leaf(
            format!("Destination VMAC: {destination}"),
            7..10,
        ));
    }
    let used = kids.last().map_or(4, |field| field.range.end);
    if npdu_start > used {
        kids.push(Field::leaf(
            format!("Function data: {} octets", npdu_start - used),
            used..npdu_start,
        ));
    }
    header(format!("BVLL6: {}", link.name), 0..npdu_start, kids, link)
}

fn sc(frame: &[u8], npdu_start: usize, link: &Link) -> Field {
    let mut kids = Vec::new();
    if frame.len() >= 4 {
        kids.push(Field::leaf(
            format!("Function: {} (0x{:02X})", link.name, frame[0]),
            0..1,
        ));
        let control = frame[1];
        let flags: Vec<&str> = [
            (0x08, "originating VMAC"),
            (0x04, "destination VMAC"),
            (0x02, "destination options"),
            (0x01, "data options"),
        ]
        .iter()
        .filter(|(bit, _)| control & bit != 0)
        .map(|(_, name)| *name)
        .collect();
        kids.push(Field::leaf(
            format!("Control: 0x{control:02X} ({})", flags.join(", ")),
            1..2,
        ));
        let id = u16::from_be_bytes([frame[2], frame[3]]);
        kids.push(Field::leaf(format!("Message ID: {id}"), 2..4));
        let mut at = 4;
        for (bit, name, vmac) in [
            (0x08, "Originating VMAC", &link.source),
            (0x04, "Destination VMAC", &link.destination),
        ] {
            if let (true, Some(vmac)) = (control & bit != 0, vmac) {
                kids.push(Field::leaf(format!("{name}: {vmac}"), at..at + 6));
                at += 6;
            }
        }
        let payload = if link.npdu.is_some() {
            npdu_start
        } else {
            frame.len()
        };
        if control & 0x03 != 0 && link.npdu.is_some() && npdu_start > at {
            kids.push(Field::leaf("Header options", at..npdu_start));
        } else if payload > at {
            kids.push(Field::leaf(
                format!("Options and payload: {} octets", payload - at),
                at..payload,
            ));
        }
    }
    header(format!("BVLC-SC: {}", link.name), 0..npdu_start, kids, link)
}

fn mstp(frame: &[u8], link: &Link) -> Field {
    let mut kids = Vec::new();
    if frame.len() >= 8 {
        let length = usize::from(u16::from_be_bytes([frame[5], frame[6]]));
        kids.push(Field::leaf("Preamble: 55 FF", 0..2));
        kids.push(Field::leaf(
            format!("Frame type: {} ({})", link.name, frame[2]),
            2..3,
        ));
        kids.push(Field::leaf(format!("Destination: {}", frame[3]), 3..4));
        kids.push(Field::leaf(format!("Source: {}", frame[4]), 4..5));
        kids.push(Field::leaf(format!("Length: {length}"), 5..7));
        kids.push(Field::leaf(format!("Header CRC: {:02X}", frame[7]), 7..8));
        let tail = if frame[2] >= 32 && frame[2] < 128 {
            5
        } else {
            2
        };
        if length > 0 && frame.len() >= 8 + tail {
            let end = frame.len() - tail;
            kids.push(Field::leaf(format!("Data: {} octets", end - 8), 8..end));
            let crc = if tail == 5 {
                "Encoded CRC-32K"
            } else {
                "Data CRC"
            };
            kids.push(Field::leaf(
                format!("{crc}: {}", hex(&frame[end..])),
                end..frame.len(),
            ));
        }
    }
    header(format!("MS/TP: {}", link.name), 0..frame.len(), kids, link)
}

/// The NPCI and what follows it: a network message, or the APDU.
fn npdu(npdu: &[u8], base: usize) -> Vec<Field> {
    let at = |range: Range<usize>| base + range.start..base + range.end;
    let view = match NpduView::decode(npdu) {
        Ok(view) => view,
        Err(error) => {
            return vec![Field::leaf(
                format!("NPDU: malformed ({error})"),
                at(0..npdu.len()),
            )]
        }
    };
    let control = npdu.get(1).copied().unwrap_or_default();
    let mut flags = Vec::new();
    for (bit, name) in [
        (0x80, "network message"),
        (0x20, "DNET"),
        (0x08, "SNET"),
        (0x04, "expecting reply"),
    ] {
        if control & bit != 0 {
            flags.push(name.to_string());
        }
    }
    let priority = match view.priority {
        NetworkPriority::Normal => "normal",
        NetworkPriority::Urgent => "urgent",
        NetworkPriority::CriticalEquipment => "critical-equipment",
        NetworkPriority::LifeSafety => "life-safety",
    };
    flags.push(format!("priority {priority}"));
    let mut kids = vec![
        Field::leaf(format!("Version: {}", npdu[0]), at(0..1)),
        Field::leaf(
            format!("Control: 0x{control:02X} ({})", flags.join(", ")),
            at(1..2),
        ),
    ];
    let mut pos = 2;
    for (name, address) in [("D", view.destination), ("S", view.source)] {
        if let Some(address) = address {
            let len = address.mac.len();
            kids.push(Field::leaf(
                format!("{name}NET: {}", address.network),
                at(pos..pos + 2),
            ));
            kids.push(Field::leaf(
                format!("{name}LEN: {len}"),
                at(pos + 2..pos + 3),
            ));
            if len > 0 {
                kids.push(Field::leaf(
                    format!("{name}ADR: {}", hex(address.mac)),
                    at(pos + 3..pos + 3 + len),
                ));
            }
            pos += 3 + len;
        }
    }
    if view.destination.is_some() {
        kids.push(Field::leaf(
            format!("Hop count: {}", view.hop_count),
            at(pos..pos + 1),
        ));
        pos += 1;
    }
    if let Some(kind) = view.message_type {
        kids.push(Field::leaf(
            format!("Message type: {}", kebab(kind)),
            at(pos..pos + 1),
        ));
        pos += 1;
        if let Some(vendor) = view.vendor_id {
            kids.push(Field::leaf(
                format!("Vendor ID: {vendor}"),
                at(pos..pos + 2),
            ));
        }
    }
    let payload = view.payload_offset;
    let mut fields = vec![Field::node("NPDU", at(0..payload), kids)];
    let rest = at(payload..npdu.len());
    match view.message_type {
        Some(kind) if !view.payload.is_empty() => fields.push(Field::leaf(
            format!("{}: {} octets", kebab(kind), view.payload.len()),
            rest,
        )),
        Some(_) => {}
        None if view.payload.is_empty() => {}
        None => fields.push(apdu(view.payload, rest.start)),
    }
    fields
}

const PDU_NAMES: [&str; 8] = [
    "Confirmed-Request",
    "Unconfirmed-Request",
    "Simple-ACK",
    "Complex-ACK",
    "Segment-ACK",
    "Error",
    "Reject",
    "Abort",
];

/// The APDU header, field by field, then its parameters as tags.
fn apdu(apdu: &[u8], base: usize) -> Field {
    let at = |range: Range<usize>| base + range.start..base + range.end;
    let whole = at(0..apdu.len());
    let decoded = match decode_apdu(Bytes::copy_from_slice(apdu)) {
        Ok(decoded) => decoded,
        Err(error) => return Field::leaf(format!("APDU: malformed ({error})"), whole),
    };
    let first = apdu[0];
    let kind = PDU_NAMES[usize::from(first >> 4)];
    let mut kids = vec![Field::leaf(
        format!("PDU type: {kind} (flags 0x{:X})", first & 0x0F),
        at(0..1),
    )];
    let mut leaf = |label: String, range: Range<usize>| kids.push(Field::leaf(label, at(range)));
    let segmented = first & 0x08 != 0;
    let (service, params, extra) = match &decoded {
        Apdu::ConfirmedRequest(request) => {
            let segments = request
                .max_segments
                .map_or_else(|| "unspecified".to_string(), |n| n.to_string());
            leaf(
                format!(
                    "Max segments: {segments}, max APDU: {}",
                    request.max_apdu_length
                ),
                1..2,
            );
            leaf(format!("Invoke ID: {}", request.invoke_id), 2..3);
            let mut pos = 3;
            if segmented {
                leaf(format!("Sequence number: {}", apdu[3]), 3..4);
                leaf(format!("Proposed window size: {}", apdu[4]), 4..5);
                pos = 5;
            }
            leaf(
                format!("Service: {}", kebab(request.service_choice)),
                pos..pos + 1,
            );
            (kebab(request.service_choice), pos + 1, None)
        }
        Apdu::UnconfirmedRequest(request) => {
            leaf(format!("Service: {}", kebab(request.service_choice)), 1..2);
            (kebab(request.service_choice), 2, None)
        }
        Apdu::SimpleAck(ack) => {
            leaf(format!("Invoke ID: {}", ack.invoke_id), 1..2);
            leaf(format!("Service: {}", kebab(ack.service_choice)), 2..3);
            (kebab(ack.service_choice), 3, None)
        }
        Apdu::ComplexAck(ack) => {
            leaf(format!("Invoke ID: {}", ack.invoke_id), 1..2);
            let mut pos = 2;
            if segmented {
                leaf(format!("Sequence number: {}", apdu[2]), 2..3);
                leaf(format!("Proposed window size: {}", apdu[3]), 3..4);
                pos = 4;
            }
            leaf(
                format!("Service: {}", kebab(ack.service_choice)),
                pos..pos + 1,
            );
            (kebab(ack.service_choice), pos + 1, None)
        }
        Apdu::SegmentAck(ack) => {
            leaf(format!("Invoke ID: {}", ack.invoke_id), 1..2);
            leaf(format!("Sequence number: {}", ack.sequence_number), 2..3);
            leaf(
                format!("Actual window size: {}", ack.actual_window_size),
                3..4,
            );
            (String::new(), apdu.len(), None)
        }
        Apdu::Error(error) => {
            leaf(format!("Invoke ID: {}", error.invoke_id), 1..2);
            leaf(format!("Service: {}", kebab(error.service_choice)), 2..3);
            let problem = format!(
                "Error: {}:{}",
                kebab(error.error_class),
                kebab(error.error_code)
            );
            (kebab(error.service_choice), 3, Some(problem))
        }
        Apdu::Reject(reject) => {
            leaf(format!("Invoke ID: {}", reject.invoke_id), 1..2);
            leaf(
                format!("Reject reason: {}", kebab(reject.reject_reason)),
                2..3,
            );
            (String::new(), apdu.len(), None)
        }
        Apdu::Abort(abort) => {
            leaf(format!("Invoke ID: {}", abort.invoke_id), 1..2);
            leaf(format!("Abort reason: {}", kebab(abort.abort_reason)), 2..3);
            (String::new(), apdu.len(), None)
        }
    };
    if params < apdu.len() {
        let label = extra.unwrap_or_else(|| format!("{service} parameters"));
        let tags = tags(apdu, base, params, apdu.len(), 0);
        kids.push(Field::node(label, at(params..apdu.len()), tags));
    }
    Field::node(format!("APDU: {kind}"), whole, kids)
}

const APPLICATION_TAGS: [&str; 13] = [
    "Null",
    "Boolean",
    "Unsigned",
    "Signed",
    "Real",
    "Double",
    "Octet string",
    "Character string",
    "Bit string",
    "Enumerated",
    "Date",
    "Time",
    "Object identifier",
];

/// An application value as the screen writes it: enumerations and object
/// types in kebab-case, an object as `analog-input 1`.
fn value_text(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Enumerated(n) => n.to_string(),
        PropertyValue::ObjectIdentifier(object) => {
            format!(
                "{} {}",
                kebab(object.object_type()),
                object.instance_number()
            )
        }
        // TODO(#1888): the TUI's float cap applies here once it lands.
        value => crate::output::format_property_value(value),
    }
}

/// The tagged values in `data[start..end]`, opening and closing tags as
/// nodes. Stops at the first tag it can't read, with a leaf saying so.
fn tags(data: &[u8], base: usize, start: usize, end: usize, depth: usize) -> Vec<Field> {
    walk(data, base, start, end, depth).0
}

/// [`tags`], and where the walk stopped: just past the closing tag that
/// ended this level, or `end`. Every tag header's length comes from
/// `decode_tag`, since tag numbers from 15 up take a second octet.
fn walk(data: &[u8], base: usize, start: usize, end: usize, depth: usize) -> (Vec<Field>, usize) {
    let mut fields = Vec::new();
    let mut pos = start;
    while pos < end {
        let (tag, content) = match decode_tag(&data[..end], pos) {
            Ok(read) if depth < MAX_CONTEXT_NESTING_DEPTH => read,
            Ok(_) => {
                fields.push(Field::leaf("Nested too deeply", base + pos..base + end));
                return (fields, end);
            }
            Err(error) => {
                fields.push(Field::leaf(
                    format!("Malformed tag: {error}"),
                    base + pos..base + end,
                ));
                return (fields, end);
            }
        };
        if tag.is_closing {
            return (fields, content);
        }
        if tag.is_opening {
            let (inner, after) = walk(data, base, content, end, depth + 1);
            fields.push(Field::node(
                format!("[{}] opening", tag.number),
                base + pos..base + after,
                inner,
            ));
            pos = after;
            continue;
        }
        // An application Boolean holds its value in the header and has no
        // content octets; the value decoder knows where each one ends.
        let decoded = (tag.class == TagClass::Application)
            .then(|| decode_application_value(&data[..end], pos));
        let after = match &decoded {
            Some(Ok((_, after))) => *after,
            _ => content.saturating_add(tag.length as usize),
        }
        .min(end)
        .max(content);
        let label = match decoded {
            Some(Ok((value, _))) => format!(
                "{}: {}",
                APPLICATION_TAGS
                    .get(usize::from(tag.number))
                    .unwrap_or(&"Application"),
                value_text(&value)
            ),
            Some(Err(error)) => format!("Application tag {}: {error}", tag.number),
            None => {
                let octets = &data[content..after];
                let number = (octets.len() <= 4)
                    .then(|| octets.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)));
                match number {
                    Some(n) if !octets.is_empty() => {
                        format!("[{}] {} ({n})", tag.number, hex(octets))
                    }
                    _ => format!("[{}] {}", tag.number, hex(octets)),
                }
            }
        };
        fields.push(Field::leaf(label, base + pos..base + after));
        pos = after;
    }
    (fields, end)
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
