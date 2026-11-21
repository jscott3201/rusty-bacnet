//! A password-bearing request shows its password nowhere (#2074, #2618):
//! named fields with the password hidden when its decoder takes it, one
//! hidden leaf for all its parameters when the decoder refuses them.

use crate::decode::frame::summarize_npdu;
use crate::label::kebab;
use bacnet_encoding::primitives::{encode_ctx_enumerated, encode_ctx_unsigned};
use bacnet_encoding::tags::{TagClass, encode_tag};
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_types::enums::{EnableDisable, ReinitializedState};
use bytes::BytesMut;

use super::super::{Field, decode_apdu_tree};
use super::password::PASSWORD;
use super::tests::snapshot;

/// `text` as octets, spaces ignored.
fn octets(text: &str) -> Vec<u8> {
    let digits: String = text.split_whitespace().collect();
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).unwrap())
        .collect()
}

const DCC: u8 = 0x11;
const REINIT: u8 = 0x14;

/// What a refused request's parameters at `start..end` show.
fn refused(service: &str, start: usize, end: usize) -> String {
    format!(
        "{service} parameters @{start}..{end}
  Hidden: may hold a password @{start}..{end}
  Not read as named fields: reason hidden (it may quote the password) @{start}..{end}"
    )
}

/// A password's contents in `charset`, `text` written in it.
fn in_charset(charset: u8, text: &str) -> Vec<u8> {
    let mut out = vec![charset];
    match charset {
        4 => out.extend(text.encode_utf16().flat_map(u16::to_be_bytes)),
        3 => out.extend(text.chars().flat_map(|c| u32::from(c).to_be_bytes())),
        5 => out.extend(text.chars().map(|c| u8::try_from(u32::from(c)).unwrap())),
        _ => out.extend_from_slice(text.as_bytes()),
    }
    out
}

/// The passwords tried: absent, empty, ASCII, 20 two-octet characters, a
/// mix, 15 characters of UCS-2, ISO-8859-1, every other character set the
/// standard names, and one character too many.
fn passwords() -> Vec<Option<Vec<u8>>> {
    let mut out = vec![
        None,
        Some(vec![0]),
        Some(in_charset(0, "Zq7-hunter2")),
        Some(in_charset(0, &"é".repeat(20))),
        Some(in_charset(0, "pässwörd-ünïcödé")),
        Some(in_charset(4, "abcdefghijklmno")),
        Some(in_charset(5, "pässwörd")),
        Some(in_charset(0, &"a".repeat(21))),
    ];
    out.extend((1..=3).map(|charset| Some(in_charset(charset, "ab"))));
    out
}

/// A context-tagged field of `contents`.
fn field(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_tag(&mut buf, tag, TagClass::Context, contents.len() as u32);
    buf.extend_from_slice(contents);
    buf.to_vec()
}

/// `fields` (a label, if it shows, and its octets) as a request of
/// `service` would show them named, from offset 4.
fn named(service: &str, fields: &[(Option<String>, Vec<u8>)]) -> String {
    let end = 4 + fields.iter().map(|(_, octets)| octets.len()).sum::<usize>();
    let mut lines = vec![format!("{service} parameters @4..{end}")];
    let mut at = 4;
    for (label, octets) in fields {
        if let Some(label) = label {
            lines.push(format!("  {label} @{at}..{}", at + octets.len()));
        }
        at += octets.len();
    }
    lines.join("\n")
}

/// Well-formed DeviceCommunicationControl and ReinitializeDevice requests,
/// every optional field there or not, every enumerated value, the
/// passwords above: each shows as its decoder reads it, the fields named
/// from what it decoded, when it takes the request, as the hidden leaf when
/// it doesn't (a character set it doesn't read, 21 characters).
#[test]
fn a_request_shows_as_its_decoder_reads_it() {
    let mut accepted = Vec::new();
    for duration in [None, Some(0), Some(255), Some(256), Some(u16::MAX)] {
        for &(_, enable) in EnableDisable::ALL_NAMED {
            for password in passwords() {
                let mut buf = BytesMut::new();
                let time = duration.map(|minutes| {
                    encode_ctx_unsigned(&mut buf, 0, u64::from(minutes));
                    buf.split().to_vec()
                });
                encode_ctx_enumerated(&mut buf, 1, enable.to_raw());
                let choice = buf.split().to_vec();
                let secret = password.as_deref().map(|contents| field(2, contents));
                let params: Vec<u8> = [time.clone(), Some(choice.clone()), secret.clone()]
                    .into_iter()
                    .flatten()
                    .flatten()
                    .collect();
                let apdu = [&[0x00, 0x05, 0x01, DCC][..], &params].concat();
                let expected = match DeviceCommunicationControlRequest::decode(&params) {
                    Ok(request) => {
                        accepted.push(password.clone());
                        let fields = [
                            time.map(|octets| {
                                let minutes = request.time_duration.unwrap();
                                (Some(format!("Time duration: {minutes} min")), octets)
                            }),
                            Some((
                                Some(format!("Enable/disable: {}", kebab(request.enable_disable))),
                                choice,
                            )),
                            secret.map(|octets| (Some(PASSWORD.to_string()), octets)),
                        ];
                        named(
                            "device-communication-control",
                            &fields.into_iter().flatten().collect::<Vec<_>>(),
                        )
                    }
                    Err(_) => refused("device-communication-control", 4, apdu.len()),
                };
                assert_eq!(snapshot(&apdu).trim_end(), expected, "{apdu:02X?}");
            }
        }
    }
    for &(_, state) in ReinitializedState::ALL_NAMED {
        for password in passwords() {
            let mut buf = BytesMut::new();
            encode_ctx_enumerated(&mut buf, 0, state.to_raw());
            let reinitialize = buf.split().to_vec();
            let secret = password.as_deref().map(|contents| field(1, contents));
            let params: Vec<u8> = [Some(reinitialize.clone()), secret.clone()]
                .into_iter()
                .flatten()
                .flatten()
                .collect();
            let apdu = [&[0x00, 0x05, 0x01, REINIT][..], &params].concat();
            let expected = match ReinitializeDeviceRequest::decode(&params) {
                Ok(request) => {
                    let state = kebab(request.reinitialized_state);
                    let fields = [
                        Some((Some(format!("Reinitialized state: {state}")), reinitialize)),
                        secret.map(|octets| (Some(PASSWORD.to_string()), octets)),
                    ];
                    named(
                        "reinitialize-device",
                        &fields.into_iter().flatten().collect::<Vec<_>>(),
                    )
                }
                Err(_) => refused("reinitialize-device", 4, apdu.len()),
            };
            assert_eq!(snapshot(&apdu).trim_end(), expected, "{apdu:02X?}");
        }
    }
    // The decoder takes the passwords of more octets than characters, so
    // they show named.
    for taken in [
        in_charset(0, &"é".repeat(20)),
        in_charset(0, "pässwörd-ünïcödé"),
        in_charset(4, "abcdefghijklmno"),
        in_charset(5, "pässwörd"),
    ] {
        assert!(accepted.contains(&Some(taken.clone())), "{taken:02X?}");
    }
}

/// The parameters of a refused request, or of a first segment, are one
/// hidden leaf over all of them, after the APDU header's own fields: no
/// field, fault or nesting read from them, the reason in fixed words.
#[test]
fn a_refused_request_shows_its_header_then_one_hidden_leaf() {
    let cases = [
        // A password that claims too few octets, the rest read as tags.
        "00050111 1900 2A0041 6F",
        "00050111 1900 2A0041 6577",
        "00050114 0900 1900 70613535776F7264",
        "00050111 1900 2A0041 FF41",
        "00050111 1900 3E 2A0041 6F",
        // An earlier field that takes in the password's header.
        "00050114 091C 05707764",
        "00050111 0C19012C 00657A71",
        "00050111 093C 192C 05707764",
        "00050111 0B19012C 19414243",
        "00050111 09 2C 19 41 42 43",
        // The password in a constructed value, a stray closing tag, a
        // password cut short, a duration too long, 24 characters.
        "00050111 1900 2E 7508 0068756E74657232 2F",
        "00050111 1900 3F 2D08 0068756E7465",
        "00050111 1900 2D08 0068756E7465",
        "00050111 0D05 0102030405 1900",
        "00050114 0900 1D19 00 5A71372D68756E746572322D5A71372D68756E746572322D",
        // A first segment.
        "0E050100 0411 1900 2A0041 6F",
    ];
    for apdu in cases {
        let apdu = octets(apdu);
        let segmented = apdu[0] & 0x08 != 0;
        let start = if segmented { 6 } else { 4 };
        let tree = decode_apdu_tree(&apdu);
        let (params, header) = tree.children.split_last().unwrap();
        let labels: Vec<&str> = header.iter().map(|field| field.label.as_str()).collect();
        let expected: &[&str] = if segmented {
            &[
                "PDU type",
                "Max segments",
                "Invoke ID",
                "Sequence number",
                "Proposed window size",
                "Service",
            ]
        } else {
            &["PDU type", "Max segments", "Invoke ID", "Service"]
        };
        assert_eq!(labels.len(), expected.len(), "{labels:?}");
        for (label, prefix) in labels.iter().zip(expected) {
            assert!(label.starts_with(prefix), "{labels:?}");
        }
        assert_eq!(params.range, start..apdu.len());
        let leaves: Vec<(&str, std::ops::Range<usize>, usize)> = params
            .children
            .iter()
            .map(|field| {
                (
                    field.label.as_str(),
                    field.range.clone(),
                    field.children.len(),
                )
            })
            .collect();
        let mut expected = vec![("Hidden: may hold a password", params.range.clone(), 0)];
        if !segmented {
            let reason = "Not read as named fields: reason hidden (it may quote the password)";
            expected.push((reason, params.range.clone(), 0));
        }
        assert_eq!(leaves, expected, "{apdu:02X?}");
    }
}

/// A little xorshift, for a fixed, replayable sequence of requests.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn octets(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next() as u8).collect()
    }
}

/// Parameters for a request of `service`: any octets, or a well-formed
/// request's (any duration, enumerated value, character set and octets in
/// the password), half of those then broken: cut short, an octet changed,
/// one put in, or some added.
fn parameters(rng: &mut Rng, service: u8) -> Vec<u8> {
    if rng.below(3) == 0 {
        let len = 1 + rng.below(48);
        return rng.octets(len);
    }
    let mut buf = BytesMut::new();
    if service == DCC {
        if rng.below(2) == 0 {
            encode_ctx_unsigned(&mut buf, 0, rng.next() % 0x1_0000);
        }
        encode_ctx_enumerated(&mut buf, 1, rng.below(4) as u32);
    } else {
        encode_ctx_enumerated(&mut buf, 0, rng.below(10) as u32);
    }
    let mut params = buf.to_vec();
    if rng.below(4) != 0 {
        let len = rng.below(30);
        let contents = match rng.below(3) {
            0 => in_charset(0, &"aé€".repeat(len / 3)),
            1 => [vec![rng.below(6) as u8], rng.octets(len)].concat(),
            _ => rng.octets(len),
        };
        let tag = if service == DCC { 2 } else { 1 };
        params.extend(field(tag, &contents));
    }
    if rng.below(2) == 0 {
        let at = rng.below(params.len());
        match rng.below(4) {
            0 => params.truncate(at.max(1)),
            1 => params[at] = rng.next() as u8,
            2 => params.insert(at, rng.next() as u8),
            _ => {
                let more = 1 + rng.below(4);
                params.extend(rng.octets(more));
            }
        }
    }
    params
}

/// Everything a front end shows of `apdu`: the tree (labels, ranges,
/// device-text marks, every member of a field the MCP's JSON writes) and
/// the summary.
fn shown(apdu: &[u8]) -> String {
    fn render(field: &Field, depth: usize, out: &mut String) {
        out.push_str(&format!(
            "{}{} @{:?}{}\n",
            "  ".repeat(depth),
            field.label,
            field.range,
            if field.device_text { " !" } else { "" }
        ));
        for child in &field.children {
            render(child, depth + 1, out);
        }
    }
    let mut out = String::new();
    render(&decode_apdu_tree(apdu), 0, &mut out);
    let npdu = [&[0x01, 0x00][..], apdu].concat();
    out.push_str(&format!("{:?}", summarize_npdu(&npdu)));
    out
}

/// Two requests of one service with the same APDU header and as many
/// parameter octets, which its decoder both refuses (or, as a first
/// segment, doesn't read), show the same tree and summary, whatever their
/// parameter octets.
#[test]
fn a_refused_request_shows_nothing_of_its_parameters() {
    let mut rng = Rng(0x2618_2618_0bad_cafe);
    let (mut accepted, mut refusals, mut compared) = (0usize, 0usize, 0usize);
    let rounds = 20_000;
    for _ in 0..rounds {
        let service = if rng.below(2) == 0 { DCC } else { REINIT };
        let header: &[u8] = if rng.below(4) == 0 {
            &[0x0E, 0x05, 0x01, 0x00, 0x04, service]
        } else {
            &[0x00, 0x05, 0x01, service]
        };
        let segment = header.len() == 6;
        let refuses = |params: &[u8]| {
            segment
                || if service == DCC {
                    DeviceCommunicationControlRequest::decode(params).is_err()
                } else {
                    ReinitializeDeviceRequest::decode(params).is_err()
                }
        };
        let one = parameters(&mut rng, service);
        let mut other = if rng.below(2) == 0 {
            rng.octets(one.len())
        } else {
            parameters(&mut rng, service)
        };
        other.resize(one.len(), rng.next() as u8);
        if refuses(&one) {
            refusals += 1;
        } else {
            accepted += 1;
        }
        if refuses(&one) && refuses(&other) {
            compared += 1;
            let (one, other) = ([header, &one].concat(), [header, &other].concat());
            assert_eq!(shown(&one), shown(&other), "{one:02X?}\n{other:02X?}");
        }
    }
    println!("{rounds} requests: {accepted} taken, {refusals} refused; {compared} pairs compared");
    assert!(accepted > rounds / 10 && refusals > rounds / 10 && compared > rounds / 10);
}
