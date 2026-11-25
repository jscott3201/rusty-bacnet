//! A watch's states and reasons as Python reads them (#2561): every
//! `State` by its name, and every `Why` as a `WatchReason`, its kind, the
//! fields that kind carries and its line of text, an address it names
//! shown in the link's form.

use bacnet_ops::DeviceAddress;
use bacnet_ops::target::AddressStyle;
use bacnet_ops::watch::{State, Why};

use super::{PyWatchReason, state_name};

/// A reason's kind, error, `to`, `watched`, `found`, `sender` and text.
type Fields = (
    &'static str,
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<u32>,
    Option<u32>,
    String,
);

fn fields(reason: PyWatchReason) -> Fields {
    (
        reason.kind,
        reason.error,
        reason.to,
        reason.watched,
        reason.found,
        reason.sender,
        reason.text,
    )
}

fn text(text: &str) -> Option<String> {
    Some(text.to_owned())
}

#[test]
fn every_state_has_its_name_and_reason() {
    let why = Why::Missing;
    let states = [
        (State::Starting, "starting", false),
        (State::Subscribed, "subscribed", false),
        (State::Polled { why: why.clone() }, "polled", true),
        (State::Unreachable { why: why.clone() }, "unreachable", true),
        (State::Gone { why: why.clone() }, "gone", true),
    ];
    for (state, name, reason) in states {
        let (named, carried) = state_name(&state);
        assert_eq!(named, name);
        assert_eq!(carried, reason.then_some(&why), "{name}");
    }
}

#[test]
fn every_reason_is_decoded_with_what_its_kind_carries() {
    let at = "192.168.1.10:47808";
    let error = "BACnet reject: unrecognized-service".to_owned();
    let mstp = DeviceAddress::local(&[9]).unwrap();
    let bip = DeviceAddress::local(&[192, 168, 1, 20, 0xBA, 0xC0]).unwrap();
    let cases: [(Why, AddressStyle, Fields); 10] = [
        (
            Why::Refused(error.clone()),
            AddressStyle::Bip,
            (
                "refused",
                Some(error.clone()),
                None,
                None,
                None,
                None,
                format!("the device refused the subscription: {error}"),
            ),
        ),
        (
            Why::NoAnswer("timeout".into()),
            AddressStyle::Bip,
            (
                "no-answer",
                text("timeout"),
                None,
                None,
                None,
                None,
                "the device didn't answer the read or the subscription: timeout".into(),
            ),
        ),
        (
            Why::Full("no space".into()),
            AddressStyle::Bip,
            (
                "full",
                text("no space"),
                None,
                None,
                None,
                None,
                "the device's subscription table was full: no space".into(),
            ),
        ),
        (
            Why::Lapsed(Some("timeout".into())),
            AddressStyle::Bip,
            (
                "lapsed",
                text("timeout"),
                None,
                None,
                None,
                None,
                "a renewal failed: timeout".into(),
            ),
        ),
        (
            Why::Lapsed(None),
            AddressStyle::Bip,
            (
                "lapsed",
                None,
                None,
                None,
                None,
                None,
                "the renewals stopped".into(),
            ),
        ),
        (
            Why::Missing,
            AddressStyle::Bip,
            (
                "missing",
                None,
                None,
                None,
                None,
                None,
                "no router to the device's network is known".into(),
            ),
        ),
        (
            Why::Moved { to: bip },
            AddressStyle::Bip,
            (
                "moved",
                None,
                text("192.168.1.20:47808"),
                None,
                None,
                None,
                "the device answers at 192.168.1.20:47808 now: watch the point there".into(),
            ),
        ),
        (
            Why::Moved { to: mstp },
            AddressStyle::Mstp,
            (
                "moved",
                None,
                text("mstp:9"),
                None,
                None,
                None,
                "the device answers at mstp:9 now: watch the point there".into(),
            ),
        ),
        (
            Why::Replaced {
                watched: 1234,
                found: 1235,
            },
            AddressStyle::Bip,
            (
                "replaced",
                None,
                None,
                Some(1234),
                Some(1235),
                None,
                format!("device 1235 answers at {at} now, not device 1234"),
            ),
        ),
        (
            Why::ForeignSender { sender: 77 },
            AddressStyle::Bip,
            (
                "foreign-sender",
                None,
                None,
                None,
                None,
                Some(77),
                format!(
                    "its notifications name device 77, which can't be told from another \
                  device at {at}"
                ),
            ),
        ),
    ];
    for (why, style, expected) in cases {
        assert_eq!(
            fields(PyWatchReason::new(&why, at, style)),
            expected,
            "{why:?}"
        );
    }
}
