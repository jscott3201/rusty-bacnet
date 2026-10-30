//! The worker on its own: the hub's settings for the list, rows as hub
//! points, how a snapshot reads as a row's source, the log's lines, and
//! how a row's samples are written out.

use std::time::UNIX_EPOCH;

use bacnet_ops::format::RawRow;
use bacnet_ops::watch::Via;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::*;
use crate::tui::watched::Sample;

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

/// The list's settings are a valid hub, at the bounds of the options too:
/// a 10 s lifetime, a 1 s poll interval, a long request budget.
#[test]
fn the_list_settings_make_a_valid_hub() {
    let defaults = hub_options(&WatchOptions::default());
    assert!(defaults.validate().is_ok());
    assert_eq!(defaults.missing, MissingDevice::Fail);
    assert_eq!(defaults.reads, Reads::value());
    assert_eq!(defaults.renewal_margin, DEFAULT_REQUEST_BUDGET * 2);
    for (lifetime, poll, budget) in [(10, 1, 24), (28_800, 3_600, 8), (300, 10, 600)] {
        let options = WatchOptions {
            lifetime,
            poll_every: Duration::from_secs(poll),
            workspace: None,
            request_budget: Duration::from_secs(budget),
        };
        let hub = hub_options(&options);
        assert!(hub.validate().is_ok(), "{options:?}: {:?}", hub.validate());
        assert_eq!(hub.lifetime.secs(), lifetime);
    }
}

/// Present_Value follows the object (SubscribeCOV); any other property is
/// a property watch.
#[test]
fn rows_map_to_hub_points() {
    let mut point = Point {
        device: 2001,
        network: Some(2),
        object: ai(1),
        property: PropertyIdentifier::PRESENT_VALUE,
    };
    assert_eq!(hub_point(&point).watched, Watched::Object);
    point.property = PropertyIdentifier::STATUS_FLAGS;
    assert_eq!(
        hub_point(&point).watched,
        Watched::Property {
            property: PropertyIdentifier::STATUS_FLAGS,
            index: None,
        }
    );
    assert_eq!(
        (hub_point(&point).device, hub_point(&point).object),
        (2001, ai(1))
    );
}

fn snapshot(source: Option<hub::Source>, why: Option<Why>, note: Option<&str>) -> Snapshot {
    Snapshot {
        source,
        why,
        note: note.map(str::to_string),
        ..Snapshot::default()
    }
}

/// The hub's reasons read as the list has always shown them, and a poll's
/// error, which only changes the note, doesn't change the source.
#[test]
fn a_snapshot_reads_as_the_row_source() {
    let poll = Some(hub::Source::Poll);
    let cases = [
        (snapshot(None, None, None), None),
        (
            snapshot(Some(hub::Source::Cov), None, None),
            Some(Source::Cov { lifetime: 60 }),
        ),
        (
            snapshot(poll, Some(Why::Refused("rejected".into())), Some("timeout")),
            Some(Source::Polled("COV refused: rejected".into())),
        ),
        (
            snapshot(poll, Some(Why::NoAnswer("timeout".into())), None),
            Some(Source::Polled("no answer: timeout".into())),
        ),
        (
            snapshot(poll, Some(Why::Lapsed(Some("timeout".into()))), None),
            Some(Source::Polled("renewal failed: timeout".into())),
        ),
        (
            snapshot(poll, Some(Why::Lapsed(None)), None),
            Some(Source::Polled("renewal stopped".into())),
        ),
        (
            snapshot(
                None,
                Some(Why::Missing),
                Some("device 7 is not in the device table"),
            ),
            Some(Source::Failed("device 7 is not in the device table".into())),
        ),
        // Between the steps of a change: no source yet.
        (snapshot(poll, None, Some("timeout")), None),
        (
            snapshot(Some(hub::Source::Cov), Some(Why::Refused("x".into())), None),
            None,
        ),
    ];
    for (snapshot, expected) in cases {
        assert_eq!(source_of(&snapshot, 60), expected, "{snapshot:?}");
    }
}

/// The log keeps the device a plain instance; a routed one's network has
/// its own column, last, empty on this network.
#[test]
fn a_log_line_puts_a_routed_device_network_in_its_own_column() {
    let mut written = Written {
        shown: Sample {
            value: Ok("72.5 °F".into()),
            number: Some(72.5),
            flags: None,
            at: std::time::Instant::now(),
        },
        logged: Ok("72.5".into()),
        source: "polled",
        time: 7,
    };
    let mut point = Point {
        device: 2001,
        network: Some(2),
        object: ai(1),
        property: PropertyIdentifier::PRESENT_VALUE,
    };
    let line = csv_line(&point, &written);
    assert_eq!(line, "7,2001,analog-input 1,present-value,72.5,,polled,2");
    assert_eq!(
        csv::HEADER.split(',').count(),
        line.split(',').count(),
        "a field per header column"
    );
    point.network = None;
    written.source = "cov";
    let line = csv_line(&point, &written);
    assert_eq!(line, "7,2001,analog-input 1,present-value,72.5,,cov,");
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut buf, value).unwrap();
    buf.to_vec()
}

fn present_value(value: f32) -> RawRow {
    (
        PropertyIdentifier::PRESENT_VALUE,
        Ok(encoded(&PropertyValue::Real(value))),
    )
}

/// A sample of `values` that came `after` the test's start.
fn sample(values: Result<Vec<RawRow>, String>, via: Via, after: Duration) -> hub::Sample {
    hub::Sample {
        values,
        via,
        at: tokio::time::Instant::now() + after,
        time: UNIX_EPOCH + Duration::from_secs(1) + after,
    }
}

/// AI 1's first read: 70 °F, named.
fn first_read() -> hub::Sample {
    let units = EngineeringUnits::DEGREES_FAHRENHEIT.to_raw();
    let rows = vec![
        present_value(70.0),
        (
            PropertyIdentifier::UNITS,
            Ok(encoded(&PropertyValue::Enumerated(units))),
        ),
        (
            PropertyIdentifier::OBJECT_NAME,
            Ok(encoded(&PropertyValue::CharacterString("Zone".into()))),
        ),
    ];
    sample(Ok(rows), Via::First, Duration::ZERO)
}

fn notified(value: f32, after_ms: u64) -> hub::Sample {
    sample(
        Ok(vec![present_value(value)]),
        Via::Notification,
        Duration::from_millis(after_ms),
    )
}

fn ai1_value() -> Point {
    Point {
        device: 2001,
        network: None,
        object: ai(1),
        property: PropertyIdentifier::PRESENT_VALUE,
    }
}

/// What each written sample shows, and how it came.
fn shown(taken: &Taken) -> Vec<(String, &'static str)> {
    taken
        .written
        .iter()
        .map(|w| (w.shown.value.clone().unwrap(), w.source))
        .collect()
}

/// #2167: the device notifies as it takes the subscription, before the hub
/// hands the first read on. The notification waits for the read's units
/// and follows it, so the row ends on the newer value, with its units, and
/// the log gets both in order.
#[tokio::test(start_paused = true)]
async fn a_notification_before_the_first_read_shows_with_units_once_it_lands() {
    let mut writer = Writer::new(ai1_value());
    let mut taken = Taken::default();
    writer.take(notified(70.2, 10), false, &mut taken);
    assert!(taken.written.is_empty(), "{:?}", taken.written);
    writer.take(first_read(), false, &mut taken);
    assert_eq!(
        taken.details,
        [Details {
            name: Some("Zone".into()),
            units: Some("degrees-fahrenheit".into()),
        }]
    );
    assert_eq!(
        shown(&taken),
        [("70 °F".into(), "cov"), ("70.2 °F".into(), "cov")]
    );
    // The tick sorts by arrival: the notification stays last.
    assert!(taken.written[0].shown.at < taken.written[1].shown.at);
    assert_eq!(
        csv_line(&ai1_value(), &taken.written[1]),
        "1010,2001,analog-input 1,present-value,70.2,,cov,"
    );
    // Later ones go out at once.
    let mut later = Taken::default();
    writer.take(notified(71.0, 20), false, &mut later);
    assert_eq!(shown(&later), [("71 °F".into(), "cov")]);
}

/// A first read that failed has no value to show and no units to give:
/// what waited for it goes out as it is.
#[tokio::test(start_paused = true)]
async fn a_failed_first_read_lets_what_waited_out_without_units() {
    let mut writer = Writer::new(ai1_value());
    let mut taken = Taken::default();
    writer.take(notified(70.2, 10), false, &mut taken);
    let failed = sample(Err("timeout".into()), Via::First, Duration::ZERO);
    writer.take(failed, false, &mut taken);
    assert!(taken.details.is_empty());
    assert_eq!(shown(&taken), [("70.2".into(), "cov")]);
}

/// A later start's read that fails keeps the units the first one found.
#[tokio::test(start_paused = true)]
async fn a_failed_later_read_keeps_the_units() {
    let mut writer = Writer::new(ai1_value());
    let mut taken = Taken::default();
    writer.take(first_read(), false, &mut taken);
    let failed = sample(Err("timeout".into()), Via::First, Duration::from_secs(1));
    writer.take(failed, true, &mut taken);
    writer.take(notified(72.0, 2_000), false, &mut taken);
    assert_eq!(
        shown(&taken),
        [("70 °F".into(), "cov"), ("72 °F".into(), "cov")]
    );
}

/// A start over without its first read seen (lost to a reader that fell
/// behind) lets what waits out, and nothing waits after; a poll ends the
/// wait too, after what waited.
#[tokio::test(start_paused = true)]
async fn a_start_over_without_its_read_lets_what_waited_out() {
    let mut writer = Writer::new(ai1_value());
    let mut taken = Taken::default();
    writer.take(notified(70.2, 10), false, &mut taken);
    writer.release(false, &mut taken);
    writer.take(notified(70.4, 20), false, &mut taken);
    assert_eq!(
        shown(&taken),
        [("70.2".into(), "cov"), ("70.4".into(), "cov")]
    );

    let mut writer = Writer::new(ai1_value());
    let mut taken = Taken::default();
    writer.take(notified(70.2, 10), false, &mut taken);
    let poll = sample(
        Ok(vec![present_value(70.6)]),
        Via::Poll,
        Duration::from_secs(10),
    );
    writer.take(poll, true, &mut taken);
    assert_eq!(
        shown(&taken),
        [("70.2".into(), "cov"), ("70.6".into(), "polled")]
    );
    // A first read that comes after all still sets the units.
    writer.take(first_read(), true, &mut taken);
    writer.take(notified(71.0, 20_000), false, &mut taken);
    assert_eq!(shown(&taken)[3], ("71 °F".into(), "cov"));
}

/// What waits is bounded like the stream: the oldest go, counted.
#[tokio::test(start_paused = true)]
async fn what_waits_for_the_first_read_is_bounded() {
    let mut writer = Writer::new(ai1_value());
    let mut taken = Taken::default();
    for step in 0..PENDING_PER_ROW as u64 + 2 {
        writer.take(notified(60.0 + step as f32, 10 + step), false, &mut taken);
    }
    assert_eq!(taken.dropped, 2);
    writer.take(first_read(), false, &mut taken);
    assert_eq!(taken.written.len(), PENDING_PER_ROW + 1);
    assert_eq!(taken.written[1].shown.value.as_deref(), Ok("62 °F"));
}
