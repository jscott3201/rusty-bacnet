//! A password-bearing request shows nothing of its password (#2074,
//! #2618): not its octets, nor any number read from them, such as a tag
//! number, a length, a fault's text, a nesting or a range.

use bacnet_encoding::tags::{TagClass, encode_closing_tag, encode_opening_tag, encode_tag};
use bacnet_ops::decode::frame::summarize_npdu;
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_types::enums::{EnableDisable, ReinitializedState};
use bytes::BytesMut;
use proptest::collection::vec;
use proptest::prelude::*;

use super::super::tag_walk::Walk;
use super::super::{Field, decode_apdu_tree};
use super::password::{DCC, Layout, REINIT};
use super::tests::snapshot;

/// `text` as octets, spaces ignored.
fn octets(text: &str) -> Vec<u8> {
    let digits: String = text.split_whitespace().collect();
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).unwrap())
        .collect()
}

#[track_caller]
fn assert_shows(cases: &[(&str, &str)], leaks: &[&str]) {
    for (apdu, expected) in cases {
        let text = snapshot(&octets(apdu));
        assert_eq!(text.trim_end(), *expected, "{apdu}");
        for leaked in leaks {
            assert!(!text.contains(leaked), "{leaked} in {apdu}:\n{text}");
        }
    }
}

/// A password that claims too few octets leaves the rest of it to read as
/// tags: a closing tag [6] ('o'), contents claiming 119 octets ("ew"), an
/// extended closing tag [65] ("\xFFA"), all after the password, and one
/// inside a constructed value. None of their numbers or texts show.
#[test]
fn octets_after_a_short_password_show_nothing() {
    assert_shows(
        &[
            (
                "00050111 1900 2A0041 6F",
                "\
device-communication-control parameters @4..10
  [1] 00 (0) @4..6
  Hidden: may hold a password @6..10
  Not read as named fields: reason hidden (it may quote the password) @4..10",
            ),
            (
                "00050111 1900 2A0041 6577",
                "\
device-communication-control parameters @4..11
  [1] 00 (0) @4..6
  Hidden: may hold a password @6..11
  Not read as named fields: reason hidden (it may quote the password) @4..11",
            ),
            (
                "00050114 0900 1900 70613535776F7264",
                "\
reinitialize-device parameters @4..16
  [0] 00 (0) @4..6
  Hidden: may hold a password @6..16
  Not read as named fields: reason hidden (it may quote the password) @4..16",
            ),
            (
                "00050111 1900 2A0041 FF41",
                "\
device-communication-control parameters @4..11
  [1] 00 (0) @4..6
  Hidden: may hold a password @6..11
  Not read as named fields: reason hidden (it may quote the password) @4..11",
            ),
            (
                "00050111 1900 3E 2A0041 6F",
                "\
device-communication-control parameters @4..11
  [1] 00 (0) @4..6
  Hidden: may hold a password @6..11
  Not read as named fields: reason hidden (it may quote the password) @4..11",
            ),
        ],
        &[
            "[6]",
            "[65]",
            "130",
            "132",
            "closing tag",
            "contents",
            "stopped",
        ],
    );
}

/// An earlier field whose length takes in the password's tag header leaves
/// the password's own octets to read as tags: what follows the fields that
/// match the layout is hidden, a fault included. A field longer than its
/// type allows (a time duration of three or four octets) may hold the
/// password's octets, so it is hidden too.
#[test]
fn an_earlier_field_that_takes_in_the_password_hides_what_follows() {
    assert_shows(
        &[
            // The state claims the password's header; ISO-8859-1 "pwd".
            (
                "00050114 091C 05707764",
                "\
reinitialize-device parameters @4..10
  [0] 1C (28) @4..6
  Hidden: may hold a password @6..10
  Not read as named fields: reason hidden (it may quote the password) @4..10",
            ),
            // A duration of four octets; UTF-8 "ezq".
            (
                "00050111 0C19012C 00657A71",
                "\
device-communication-control parameters @4..12
  Hidden: may hold a password @4..12
  Not read as named fields: reason hidden (it may quote the password) @4..12",
            ),
            // Enable-disable claims the password's header.
            (
                "00050111 093C 192C 05707764",
                "\
device-communication-control parameters @4..12
  [0] 3C (60) @4..6
  [1] 2C (44) @6..8
  Hidden: may hold a password @8..12
  Not read as named fields: reason hidden (it may quote the password) @4..12",
            ),
            // A duration of three octets, then "AB" shaped like [1].
            (
                "00050111 0B19012C 19414243",
                "\
device-communication-control parameters @4..12
  Hidden: may hold a password @4..12
  Not read as named fields: reason hidden (it may quote the password) @4..12",
            ),
        ],
        &[
            "112", "122", "[1] 41", "(65)", "claim", "stopped", "657A", "7077",
        ],
    );
}

/// Whether a request's fields show named depends on its layout and its
/// password's length, not on the password: one in a character set the
/// decoder doesn't read, or not valid UTF-8, shows named like any other.
#[test]
fn only_the_layout_decides_whether_fields_show_named() {
    for password in ["07736563726574", "00FFFEFDFCFBFA", "00736563726574"] {
        let text = snapshot(&octets(&format!("00050111 093C 1901 2D07 {password}")));
        assert_eq!(
            text.trim_end(),
            "\
device-communication-control parameters @4..17
  Time duration: 60 min @4..6
  Enable/disable: disable @6..8
  Password: (hidden) @8..17",
            "{password}"
        );
    }
}

/// `field`'s largest contents over `values`, as the typed encoder writes
/// them: the octets after its header.
fn largest(encoded: impl IntoIterator<Item = BytesMut>, tag: u8) -> u32 {
    encoded
        .into_iter()
        .filter_map(|params| {
            Walk::new(&params)
                .elements
                .iter()
                .find(|element| element.number() == tag)
                .map(|element| element.tag.length)
        })
        .max()
        .unwrap()
}

fn dcc(duration: Option<u16>, enable: EnableDisable, password: Option<&str>) -> BytesMut {
    let mut params = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: duration,
        enable_disable: enable,
        password: password.map(Into::into),
    }
    .encode(&mut params)
    .unwrap();
    params
}

fn reinit(state: ReinitializedState, password: Option<&str>) -> BytesMut {
    let mut params = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: password.map(Into::into),
    }
    .encode(&mut params)
    .unwrap();
    params
}

/// The layout's fields are the typed codecs' own: each field's maximum is
/// the most octets the encoder writes over every value it can carry (a
/// duration's full range, every value of an enumeration), and the decoder
/// takes a field of that many octets.
#[test]
fn the_layout_is_the_typed_codecs() {
    let enables = || EnableDisable::ALL_NAMED.iter().map(|&(_, value)| value);
    let durations = [0, 1, 255, 256, u16::MAX];
    let all_durations = || {
        durations
            .iter()
            .map(|&minutes| dcc(Some(minutes), EnableDisable::ENABLE, None))
    };
    assert_eq!(DCC.before[0].most, largest(all_durations(), 0));
    assert_eq!(
        DCC.before[1].most,
        largest(enables().map(|value| dcc(None, value, None)), 1)
    );
    let states = ReinitializedState::ALL_NAMED
        .iter()
        .map(|&(_, value)| reinit(value, None));
    assert_eq!(REINIT.before[0].most, largest(states, 0));
    // Only the time duration is optional, and the decoder takes each field
    // at its maximum.
    assert_eq!(
        (
            DCC.before
                .iter()
                .map(|field| field.required)
                .collect::<Vec<_>>(),
            REINIT.before[0].required
        ),
        (vec![false, true], true)
    );
    assert!(DeviceCommunicationControlRequest::decode(&octets("0AFFFF 19FF")).is_ok());
    assert!(ReinitializeDeviceRequest::decode(&octets("09FF")).is_ok());
}

/// A well-formed request, its optional fields there or not, matches the
/// layout whole and shows named; its first segment, walked tag by tag,
/// and a refused one (an octet after it) show every field before the
/// password, then the password and the rest as one hidden leaf.
#[test]
fn a_well_formed_request_shows_in_full() {
    let enable = EnableDisable::DISABLE_INITIATION;
    let mut cases = Vec::new();
    for duration in [None, Some(u16::MAX)] {
        for password in [None, Some("Zq7-hunter2")] {
            cases.push((
                DCC,
                0x11,
                dcc(duration, enable, password),
                password.is_some(),
            ));
        }
    }
    for password in [None, Some("Zq7-hunter2")] {
        let state = ReinitializedState::ACTIVATE_CHANGES;
        cases.push((REINIT, 0x14, reinit(state, password), password.is_some()));
    }
    for (secret, service, params, has_password) in cases {
        let walk = Walk::new(&params);
        let fields = walk.elements.len() - usize::from(has_password);
        let expected = Layout {
            before: fields,
            password: has_password.then_some(fields),
        };
        assert_eq!(secret.layout(&walk.elements), expected);
        assert!(secret.whole(&walk));
        let whole = [&[0x00, 0x05, 0x01, service][..], &params].concat();
        let text = snapshot(&whole);
        assert!(!text.contains("  [") && !text.contains("Hidden"), "{text}");
        let named = text
            .lines()
            .filter(|line| line.contains("Password: (hidden)"));
        assert_eq!(named.count(), usize::from(has_password), "{text}");
        let segment = [&[0x0E, 0x05, 0x01, 0x00, 0x04, service][..], &params].concat();
        let refused = [&[0x00, 0x05, 0x01, service][..], &params, &[0x00]].concat();
        for (apdu, hidden) in [(segment, has_password), (refused, true)] {
            let text = snapshot(&apdu);
            let lines: Vec<&str> = text.lines().skip(1).collect();
            let shown = lines.iter().filter(|line| line.starts_with("  [")).count();
            assert_eq!(shown, fields, "{text}");
            let rest = lines
                .iter()
                .filter(|line| line.contains("Hidden: may hold"));
            assert_eq!(rest.count(), usize::from(hidden), "{text}");
        }
    }
}

/// One field generated before the password: a context primitive of `tag`
/// claiming `claim` octets and followed by `given` octets of its own (fewer
/// than it claims take in what follows), its header in `form`; or an
/// opening tag in its place.
#[derive(Debug, Clone)]
enum Earlier {
    Field {
        tag: u8,
        claim: u8,
        given: Vec<u8>,
        form: Form,
    },
    Opener(u8),
}

/// How an earlier field's header is written.
#[derive(Debug, Clone, Copy)]
enum Form {
    /// The shortest form.
    Shortest,
    /// The length in an extended length octet, even when it is under 5.
    LongLength,
    /// The tag number in an extended tag number octet, which it is under 15
    /// for.
    LongNumber,
}

impl Earlier {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Field {
                tag,
                claim,
                given,
                form,
            } => {
                let (first, number) = match form {
                    Form::LongNumber => (0xF8, Some(*tag)),
                    _ => ((tag << 4) | 0x08, None),
                };
                match (form, *claim) {
                    (Form::LongLength, claim) | (_, claim @ 5..) => {
                        out.push(first | 5);
                        out.extend(number);
                        out.push(claim);
                    }
                    (_, claim) => {
                        out.push(first | claim);
                        out.extend(number);
                    }
                }
                out.extend_from_slice(given);
            }
            Self::Opener(tag) => out.push((tag << 4) | 0x0E),
        }
    }
}

fn earlier() -> impl Strategy<Value = Earlier> {
    let form = prop_oneof![
        6 => Just(Form::Shortest),
        1 => Just(Form::LongLength),
        1 => Just(Form::LongNumber),
    ];
    prop_oneof![
        // A field of the layout's shape, as long as it claims: in order or
        // not, once or more.
        4 => (0u8..2, 1u8..=2, vec(any::<u8>(), 2)).prop_map(|(tag, claim, mut given)| {
            given.truncate(usize::from(claim));
            Earlier::Field { tag, claim, given, form: Form::Shortest }
        }),
        // Any of tags 0 to 3 claiming 0 to 7 octets, followed by as many,
        // fewer (taking in what follows) or more, in any form.
        4 => (0u8..4, 0u8..=7, form, vec(any::<u8>(), 0..=9)).prop_map(
            |(tag, claim, form, mut given)| {
                if given.len() > 5 {
                    given.truncate(usize::from(claim));
                }
                Earlier::Field { tag, claim, given, form }
            }
        ),
        1 => (0u8..4).prop_map(Earlier::Opener),
    ]
}

/// How the password sits: its contents as long as it claims, shorter,
/// longer (the rest reading as tags), or inside a constructed [3].
#[derive(Debug, Clone, Copy, PartialEq)]
enum Sits {
    Exact,
    Short,
    Long,
    Wrapped,
}

/// A request of `service` (DCC or ReinitializeDevice), possibly segment 0,
/// with `fields` before a password claiming `claim` octets and holding
/// `contents`, sitting as `sits` says.
fn request(
    reinit: bool,
    segmented: bool,
    fields: &[Earlier],
    claim: u32,
    contents: &[u8],
    sits: Sits,
) -> Vec<u8> {
    let service = if reinit { 0x14 } else { 0x11 };
    let mut apdu = if segmented {
        vec![0x0E, 0x05, 0x01, 0x00, 0x04, service]
    } else {
        vec![0x00, 0x05, 0x01, service]
    };
    for field in fields {
        field.encode(&mut apdu);
    }
    let mut password = BytesMut::new();
    if sits == Sits::Wrapped {
        encode_opening_tag(&mut password, 3);
    }
    encode_tag(
        &mut password,
        if reinit { 1 } else { 2 },
        TagClass::Context,
        claim,
    );
    password.extend_from_slice(contents);
    if sits == Sits::Wrapped {
        encode_closing_tag(&mut password, 3);
    }
    apdu.extend_from_slice(&password);
    apdu
}

/// The password's contents: its character set, then `len - 1` characters
/// from `chars` (repeated as needed), each from 0x20 up: an octet below
/// that, a control character, can read as one of the fields before it.
fn contents(charset: u8, chars: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    if len > 0 {
        out.push(charset);
    }
    out.extend(
        chars
            .iter()
            .cycle()
            .take(len.saturating_sub(1))
            .map(|c| c | 0x20),
    );
    out
}

/// Everything a front end shows: the tree (labels, ranges, device-text
/// marks: every member of a field the MCP's JSON writes) and the summary;
/// a range ending at `end` reads `END`.
fn shown(apdu: &[u8]) -> String {
    fn render(field: &Field, depth: usize, end: usize, out: &mut String) {
        let at = |offset: usize| {
            if offset == end {
                "END".to_string()
            } else {
                offset.to_string()
            }
        };
        out.push_str(&format!(
            "{}{} @{}..{}{}\n",
            "  ".repeat(depth),
            field.label,
            at(field.range.start),
            at(field.range.end),
            if field.device_text { " !" } else { "" }
        ));
        for child in &field.children {
            render(child, depth + 1, end, out);
        }
    }
    let mut out = String::new();
    render(&decode_apdu_tree(apdu), 0, apdu.len(), &mut out);
    let npdu = [&[0x01, 0x00][..], apdu].concat();
    out.push_str(&format!("{:?}", summarize_npdu(&npdu)));
    out
}

/// The fields of the service's layout, from its types alone: the tag, the
/// most octets (an Unsigned16's two, an enumeration's one) and whether it
/// must be there.
fn expected(reinit: bool) -> &'static [(u8, usize, bool)] {
    if reinit {
        &[(0, 1, true)]
    } else {
        &[(0, 2, false), (1, 1, true)]
    }
}

/// The tree of a password-bearing request's walk shows no fault, and the
/// fields it shows as tags are the layout's, in order, each at most once,
/// none required skipped, none longer than its type allows.
fn shows_only_the_layout(apdu: &[u8], reinit: bool) -> Result<(), String> {
    let tree = decode_apdu_tree(apdu);
    let Some(params) = tree.children.last() else {
        return Ok(());
    };
    let mut text = String::new();
    let mut stack = vec![params];
    while let Some(field) = stack.pop() {
        text.push_str(&field.label);
        stack.extend(&field.children);
    }
    if text.contains("Tag walk stopped") {
        return Err(format!("a fault shows: {text}"));
    }
    let mut next = 0;
    let layout = expected(reinit);
    for field in &params.children {
        let Some((number, rest)) = field
            .label
            .strip_prefix('[')
            .and_then(|label| label.split_once("] "))
        else {
            continue;
        };
        let number: u8 = number.parse().map_err(|_| field.label.clone())?;
        let hex = rest.split(' ').next().unwrap_or_default();
        let Some(skipped) = layout[next..].iter().position(|f| f.0 == number) else {
            return Err(format!("{} is out of the layout", field.label));
        };
        let (_, most, _) = layout[next + skipped];
        if layout[next..next + skipped].iter().any(|f| f.2) || hex.len() / 2 > most {
            return Err(format!("{} skips a field or is too long", field.label));
        }
        next += skipped + 1;
    }
    Ok(())
}

/// Whether `shown` holds named fields. Requests compared for what they
/// show are compared at the same answer: it can follow from the password's
/// characters in one case. An earlier field one octet too long takes in the
/// first octet of the password's two-octet header; the walk then reads its
/// length octet as a field and its character set as that field's contents,
/// and the characters after as a password's tag, which may end where the
/// octets do. The tree can't tell which octets the sender meant as the
/// password there.
fn named(shown: &str) -> bool {
    !shown.contains("Not read as named fields")
}

fn sits() -> impl Strategy<Value = Sits> {
    prop_oneof![
        Just(Sits::Exact),
        Just(Sits::Short),
        Just(Sits::Long),
        Just(Sits::Wrapped)
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// Two requests that differ only in their password's characters show
    /// the same tree and summary, whatever the fields before the password:
    /// of wrong lengths (taking in its header or more), missing, repeated,
    /// out of order, in an extended form, or an opening tag in their place.
    /// What the walk shows is the layout's fields only, and no fault.
    #[test]
    fn nothing_shown_is_read_from_the_password(
        reinit in any::<bool>(),
        segmented in any::<bool>(),
        fields in vec(earlier(), 0..4),
        sits in sits(),
        claim in 0u32..30,
        charset in 0u8..6,
        first in vec(any::<u8>(), 1..40),
        second in vec(any::<u8>(), 1..40),
    ) {
        let len = match sits {
            Sits::Short if claim == 0 => return Ok(()),
            Sits::Short => first.len() % claim as usize,
            Sits::Long => claim as usize + 1 + first.len() % 8,
            Sits::Exact | Sits::Wrapped => claim as usize,
        };
        let one = request(reinit, segmented, &fields, claim, &contents(charset, &first, len), sits);
        let other = request(reinit, segmented, &fields, claim, &contents(charset, &second, len), sits);
        let (one_shows, other_shows) = (shown(&one), shown(&other));
        prop_assume!(named(&one_shows) == named(&other_shows));
        prop_assert_eq!(one_shows, other_shows);
        prop_assert_eq!(shows_only_the_layout(&one, reinit), Ok(()));
    }

    /// Two requests whose passwords claim the same length, both cut short
    /// or both overlong, each with a character at least, show the same
    /// tree and summary but for where the octets end: how much of the
    /// password there is shows nothing more.
    #[test]
    fn how_long_the_password_is_shows_nothing(
        reinit in any::<bool>(),
        segmented in any::<bool>(),
        fields in vec(earlier(), 0..4),
        long in any::<bool>(),
        claim in 3u32..30,
        charset in 0u8..6,
        chars in vec(any::<u8>(), 40),
        lens in (1usize..40, 1usize..40),
    ) {
        let (a, b) = if long {
            (claim as usize + lens.0 % 9 + 1, claim as usize + lens.1 % 9 + 1)
        } else {
            (2 + lens.0 % (claim as usize - 2), 2 + lens.1 % (claim as usize - 2))
        };
        let sits = if long { Sits::Long } else { Sits::Short };
        let one = request(reinit, segmented, &fields, claim, &contents(charset, &chars, a), sits);
        let other = request(reinit, segmented, &fields, claim, &contents(charset, &chars, b), sits);
        let (one_shows, other_shows) = (shown(&one), shown(&other));
        prop_assume!(named(&one_shows) == named(&other_shows));
        prop_assert_eq!(one_shows, other_shows);
    }
}
