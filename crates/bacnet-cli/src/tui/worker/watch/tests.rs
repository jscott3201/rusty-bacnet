//! The worker on its own: the hub's settings for the list, rows as hub
//! points, how a snapshot reads as a row's source, and the log's lines.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;

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
    let sample = Sample {
        value: Ok("72.5 °F".into()),
        number: Some(72.5),
        flags: None,
        at: std::time::Instant::now(),
    };
    let mut point = Point {
        device: 2001,
        network: Some(2),
        object: ai(1),
        property: PropertyIdentifier::PRESENT_VALUE,
    };
    let line = csv_line(&point, &sample, &Ok("72.5".into()), 7, "polled");
    assert_eq!(line, "7,2001,analog-input 1,present-value,72.5,,polled,2");
    assert_eq!(
        csv::HEADER.split(',').count(),
        line.split(',').count(),
        "a field per header column"
    );
    point.network = None;
    let line = csv_line(&point, &sample, &Ok("72.5".into()), 7, "cov");
    assert_eq!(line, "7,2001,analog-input 1,present-value,72.5,,cov,");
}
