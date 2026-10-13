//! A Lighting Output carrying out a Lighting_Command written over the wire
//! (#1384), taking the Present_Value warn values, and refusing a
//! Lighting_Command_Default_Priority of 6: the replies exactly as the server
//! encodes them, then what the object serves.
//!
//! LIGHTING_OUTPUT 1 is `0D 80 00 01`; Present_Value is property 85,
//! Tracking_Value 164, In_Progress 378 and Lighting_Command 380.

use super::mutation_list_wire_tests::wire;
use super::mutation_tests::Fixture;
use super::*;
use bacnet_objects::lighting::LightingOutputObject;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::write_property::WritePropertyRequest;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;
const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;
/// SimpleACK for WriteProperty (15), invoke ID 5.
const SIMPLE_ACK: [u8; 3] = [0x20, 5, 15];

fn lo1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, 1).unwrap()
}

/// A fixture holding LO-1 on a monotonic clock the test sets.
async fn fixture() -> (Fixture, Arc<StdMutex<Duration>>) {
    let fixture = Fixture::new(None);
    let now = Arc::new(StdMutex::new(Duration::ZERO));
    let source = Arc::clone(&now);
    let mut db = fixture.db.write().await;
    db.set_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
    db.add(Box::new(LightingOutputObject::new(1, "LO-1").unwrap()))
        .unwrap();
    drop(db);
    (fixture, now)
}

async fn write(
    fixture: &Fixture,
    property: PropertyIdentifier,
    value: &[u8],
    priority: Option<u8>,
) -> Vec<u8> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: lo1(),
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
        priority,
    }
    .encode(&mut request)
    .unwrap();
    wire(fixture, WRITE, request.freeze()).await
}

/// The ComplexACK body after the object identifier and the property: the
/// value between its opening and closing tags.
async fn read(fixture: &Fixture, property: PropertyIdentifier) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: lo1(),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    wire(fixture, READ, request.freeze()).await
}

/// A ReadProperty ComplexACK (12), invoke ID 5, for LO-1 and `property`
/// (`property_octets` as context tag 1) carrying `value`.
fn read_ack(property_octets: &[u8], value: &[u8]) -> Vec<u8> {
    [
        &[0x30, 5, 12, 0x0C, 0x0D, 0x80, 0x00, 0x01][..],
        property_octets,
        &[0x3E],
        value,
        &[0x3F],
    ]
    .concat()
}

const PRESENT_VALUE: [u8; 2] = [0x19, 0x55];
const TRACKING_VALUE: [u8; 2] = [0x19, 0xA4];
const IN_PROGRESS: [u8; 3] = [0x1A, 0x01, 0x7A];

#[tokio::test]
async fn a_fade_written_over_the_wire_is_carried_out() {
    let (fixture, now) = fixture().await;
    // Present_Value 20.0 (0x41A00000) at priority 8.
    let level = [0x44, 0x41, 0xA0, 0x00, 0x00];
    assert_eq!(
        write(&fixture, PropertyIdentifier::PRESENT_VALUE, &level, Some(8)).await,
        SIMPLE_ACK
    );
    // FADE_TO 100.0 % (0x42C80000) over 2,000 ms (0x07D0) at priority 8.
    let fade = [
        0x09, 0x01, 0x1C, 0x42, 0xC8, 0x00, 0x00, 0x4A, 0x07, 0xD0, 0x59, 0x08,
    ];
    assert_eq!(
        write(&fixture, PropertyIdentifier::LIGHTING_COMMAND, &fade, None).await,
        SIMPLE_ACK
    );
    *now.lock().unwrap() = Duration::from_millis(1_000);
    // Present_Value is the target, Tracking_Value is halfway at 60.0
    // (0x42700000), and In_Progress is FADE_ACTIVE (1).
    assert_eq!(
        read(&fixture, PropertyIdentifier::PRESENT_VALUE).await,
        read_ack(&PRESENT_VALUE, &[0x44, 0x42, 0xC8, 0x00, 0x00])
    );
    assert_eq!(
        read(&fixture, PropertyIdentifier::TRACKING_VALUE).await,
        read_ack(&TRACKING_VALUE, &[0x44, 0x42, 0x70, 0x00, 0x00])
    );
    assert_eq!(
        read(&fixture, PropertyIdentifier::IN_PROGRESS).await,
        read_ack(&IN_PROGRESS, &[0x91, 0x01])
    );
    *now.lock().unwrap() = Duration::from_millis(2_000);
    assert_eq!(
        read(&fixture, PropertyIdentifier::IN_PROGRESS).await,
        read_ack(&IN_PROGRESS, &[0x91, 0x00])
    );
}

#[tokio::test]
async fn present_value_warn_values_are_taken_and_their_neighbours_refused() {
    let (fixture, _) = fixture().await;
    // 80.0 (0x42A00000) at priority 8, then -3.0 (0xC0400000): WARN_OFF.
    // With Blink_Warn_Enable FALSE it writes 0.0 to the slot at once.
    let level = [0x44, 0x42, 0xA0, 0x00, 0x00];
    let warn_off = [0x44, 0xC0, 0x40, 0x00, 0x00];
    for value in [level, warn_off] {
        assert_eq!(
            write(&fixture, PropertyIdentifier::PRESENT_VALUE, &value, Some(8)).await,
            SIMPLE_ACK
        );
    }
    assert_eq!(
        read(&fixture, PropertyIdentifier::PRESENT_VALUE).await,
        read_ack(&PRESENT_VALUE, &[0x44, 0x00, 0x00, 0x00, 0x00])
    );
    // -1.5 (0xBFC00000) is no special value: PROPERTY (2) /
    // VALUE_OUT_OF_RANGE (37).
    assert_eq!(
        write(
            &fixture,
            PropertyIdentifier::PRESENT_VALUE,
            &[0x44, 0xBF, 0xC0, 0x00, 0x00],
            Some(8)
        )
        .await,
        [0x50, 5, 15, 0x91, 0x02, 0x91, 0x25]
    );
}

#[tokio::test]
async fn lighting_command_default_priority_refuses_six_over_the_wire() {
    let (fixture, _) = fixture().await;
    let lcdp = PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY;
    // Unsigned 7 is taken; 6, Minimum On/Off's slot, is PROPERTY /
    // VALUE_OUT_OF_RANGE (Clause 12.54.27).
    assert_eq!(write(&fixture, lcdp, &[0x21, 0x07], None).await, SIMPLE_ACK);
    assert_eq!(
        write(&fixture, lcdp, &[0x21, 0x06], None).await,
        [0x50, 5, 15, 0x91, 0x02, 0x91, 0x25]
    );
    // Property 381 is context tag 1 `1A 01 7D`; it still reads 7.
    assert_eq!(
        read(&fixture, lcdp).await,
        read_ack(&[0x1A, 0x01, 0x7D], &[0x21, 0x07])
    );
}

#[path = "lighting_link_wire_tests.rs"]
mod link;
