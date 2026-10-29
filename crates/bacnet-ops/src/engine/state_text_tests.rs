//! The one-shot CLI's state text reads, by [`Destination`], against the lab
//! (#2122): a refused RPM is tried once a read, and a reference that names
//! a device is read only when the discovery table ties the MAC to it.

use std::cell::Cell;

use super::*;
use crate::destination::Destination;
use crate::format::{LogColumns, LoggedProperty};

/// Present_Value of Binary Value `instance`, on `device` if named.
fn binary(instance: u32, device: Option<u32>) -> Option<LoggedProperty> {
    Some(LoggedProperty {
        object: oid(ObjectType::BINARY_VALUE, instance),
        property: PropertyIdentifier::PRESENT_VALUE,
        index: None,
        device,
    })
}

fn active(columns: &LogColumns, column: usize) -> Option<&str> {
    columns.state_texts(column).active.as_deref()
}

/// A device that refuses ReadPropertyMultiple gets one RPM a read: the
/// first object falls back to ReadProperty, and the second goes straight
/// there. Both are named.
#[tokio::test(start_paused = true)]
async fn a_refused_rpm_is_tried_once_a_read() {
    let (engine, server) = lab_on(RefusesRpm::new).await;
    let mut columns: LogColumns = [binary(1, None), binary(2, None)].into_iter().collect();
    Destination::Device(LAB)
        .log_state_texts(&engine.client, &mut columns)
        .await;
    assert_eq!(engine.client.transport().rejected(), 1);
    assert_eq!(active(&columns, 0), Some("Running"));
    assert_eq!(active(&columns, 1), Some("Open"));
    assert_eq!(columns.state_texts(1).inactive.as_deref(), Some("Shut"));
    stop(engine, server).await;
}

/// By MAC, a reference that names the lab device is read once the
/// discovery table holds the lab at that MAC, and one that names another
/// device isn't. A second device at the same MAC makes the match
/// ambiguous, so only unqualified references are read.
#[tokio::test(start_paused = true)]
async fn a_device_qualified_reference_needs_one_device_at_the_mac() {
    let (engine, server) = lab().await;
    let to = Destination::Mac(vec![2]);
    let wanted = [
        binary(1, Some(LAB)),
        binary(2, Some(LAB + 1)),
        binary(2, None),
    ];
    let mut columns: LogColumns = wanted.into_iter().collect();
    to.log_state_texts(&engine.client, &mut columns).await;
    assert_eq!(active(&columns, 0), Some("Running"), "the lab, by MAC");
    assert_eq!(active(&columns, 1), None, "another device");
    assert_eq!(active(&columns, 2), Some("Open"));

    engine.client.add_device(LAB + 5, &[2]).await.unwrap();
    let mut columns: LogColumns = wanted.into_iter().collect();
    to.log_state_texts(&engine.client, &mut columns).await;
    assert_eq!(active(&columns, 0), None, "two devices at the MAC");
    assert_eq!(active(&columns, 2), Some("Open"));
    stop(engine, server).await;
}

/// One object's texts for a one-shot read (#2132): a binary value's pair,
/// even from a device that refuses RPM, which two objects sharing the
/// caller's memory of the refusal learn once; nothing for a missing
/// object or a type without texts.
#[tokio::test(start_paused = true)]
async fn one_objects_state_texts_fall_back_to_none() {
    let (engine, server) = lab_on(RefusesRpm::new).await;
    let to = Destination::Device(LAB);
    let no_rpm = Cell::new(false);
    let texts = to
        .state_texts(&engine.client, oid(ObjectType::BINARY_VALUE, 2), &no_rpm)
        .await
        .unwrap();
    assert_eq!(texts.active.as_deref(), Some("Open"));
    assert_eq!(texts.inactive.as_deref(), Some("Shut"));
    assert_eq!(engine.client.transport().rejected(), 1);
    let texts = to
        .state_texts(&engine.client, oid(ObjectType::BINARY_VALUE, 1), &no_rpm)
        .await
        .unwrap();
    assert_eq!(texts.active.as_deref(), Some("Running"));
    assert_eq!(engine.client.transport().rejected(), 1, "one RPM in all");
    for object in [
        oid(ObjectType::BINARY_VALUE, 9),
        oid(ObjectType::MULTI_STATE_VALUE, 9),
        oid(ObjectType::ANALOG_INPUT, 1),
    ] {
        let texts = to.state_texts(&engine.client, object, &no_rpm).await;
        assert_eq!(texts.ok(), Some(StateTexts::default()), "{object:?}");
    }
    stop(engine, server).await;
}
