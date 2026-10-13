//! The Color and Color Temperature objects carrying out a Color_Command
//! written over the wire (#1474), and their property rows as the addendum's
//! tables give them: the replies exactly as the server encodes them, then
//! what the objects serve, on a monotonic clock the test sets.
//!
//! COLOR 1 is `0F C0 00 01` and COLOR_TEMPERATURE 1 `10 00 00 01`.
//! Present_Value is property 85, Tracking_Value 164, In_Progress 378,
//! Default_Fade_Time 374, Status_Flags 111 and Color_Command 4194334.

use super::mutation_list_wire_tests::wire;
use super::mutation_tests::Fixture;
use super::*;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::write_property::WritePropertyRequest;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;
const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;
/// SimpleACK for WriteProperty (15), invoke ID 5.
const SIMPLE_ACK: [u8; 3] = [0x20, 5, 15];
/// WriteProperty refused with PROPERTY (2) / VALUE_OUT_OF_RANGE (37).
const OUT_OF_RANGE: [u8; 7] = [0x50, 5, 15, 0x91, 0x02, 0x91, 0x25];

const PRESENT_VALUE: [u8; 2] = [0x19, 0x55];
const TRACKING_VALUE: [u8; 2] = [0x19, 0xA4];
const IN_PROGRESS: [u8; 3] = [0x1A, 0x01, 0x7A];

fn color() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap()
}

fn temperature() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 1).unwrap()
}

/// A fixture holding COLOR 1 and COLOR_TEMPERATURE 1, its limits 2700 to
/// 3000 K, on a monotonic clock the test sets.
async fn fixture() -> (Fixture, Arc<StdMutex<Duration>>) {
    let fixture = Fixture::new(None);
    let now = Arc::new(StdMutex::new(Duration::ZERO));
    let source = Arc::clone(&now);
    let mut db = fixture.db.write().await;
    db.set_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
    db.add(Box::new(ColorObject::new(1, "CLR-1").unwrap()))
        .unwrap();
    let mut ct = ColorTemperatureObject::new(1, "CT-1").unwrap();
    ct.set_min_max(2_700, 3_000).unwrap();
    db.add(Box::new(ct)).unwrap();
    drop(db);
    (fixture, now)
}

fn set(now: &StdMutex<Duration>, milliseconds: u64) {
    *now.lock().unwrap() = Duration::from_millis(milliseconds);
}

async fn write(
    fixture: &Fixture,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: &[u8],
) -> Vec<u8> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    wire(fixture, WRITE, request.freeze()).await
}

async fn read(fixture: &Fixture, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    wire(fixture, READ, request.freeze()).await
}

/// A ReadProperty ComplexACK (12), invoke ID 5, for `oid` and `property`
/// (`property_octets` as context tag 1) carrying `value`.
fn read_ack(oid: ObjectIdentifier, property_octets: &[u8], value: &[u8]) -> Vec<u8> {
    [
        &[0x30, 5, 12, 0x0C][..],
        &oid.encode(),
        property_octets,
        &[0x3E],
        value,
        &[0x3F],
    ]
    .concat()
}

/// An xy colour's two application REALs.
fn xy(x: [u8; 4], y: [u8; 4]) -> Vec<u8> {
    [&[0x44][..], &x, &[0x44], &y].concat()
}

const QUARTER: [u8; 4] = [0x3E, 0x80, 0x00, 0x00];
const THREE_EIGHTHS: [u8; 4] = [0x3E, 0xC0, 0x00, 0x00];
const HALF: [u8; 4] = [0x3F, 0x00, 0x00, 0x00];
const THREE_QUARTERS: [u8; 4] = [0x3F, 0x40, 0x00, 0x00];

#[tokio::test]
async fn a_colour_fade_written_over_the_wire_runs_and_stops() {
    let (fixture, now) = fixture().await;
    let cc = PropertyIdentifier::COLOR_COMMAND;
    // Present_Value is W in Table 12-X: (0.25, 0.5) goes in at once.
    let start = xy(QUARTER, HALF);
    assert_eq!(
        write(&fixture, color(), PropertyIdentifier::PRESENT_VALUE, &start).await,
        SIMPLE_ACK
    );
    // FADE_TO_COLOR to (0.75, 0.25) over 2,000 ms (0x07D0).
    let fade = [
        &[0x09, 0x01, 0x1E][..],
        &xy(THREE_QUARTERS, QUARTER),
        &[0x1F, 0x3A, 0x07, 0xD0],
    ]
    .concat();
    assert_eq!(write(&fixture, color(), cc, &fade).await, SIMPLE_ACK);
    set(&now, 1_000);
    // Present_Value is the target, Tracking_Value halfway at (0.5, 0.375),
    // and In_Progress FADE_ACTIVE (1).
    assert_eq!(
        read(&fixture, color(), PropertyIdentifier::PRESENT_VALUE).await,
        read_ack(color(), &PRESENT_VALUE, &xy(THREE_QUARTERS, QUARTER))
    );
    assert_eq!(
        read(&fixture, color(), PropertyIdentifier::TRACKING_VALUE).await,
        read_ack(color(), &TRACKING_VALUE, &xy(HALF, THREE_EIGHTHS))
    );
    assert_eq!(
        read(&fixture, color(), PropertyIdentifier::IN_PROGRESS).await,
        read_ack(color(), &IN_PROGRESS, &[0x91, 0x01])
    );
    // STOP (`09 06`) ends it there: Present_Value takes the colour reached.
    assert_eq!(
        write(&fixture, color(), cc, &[0x09, 0x06]).await,
        SIMPLE_ACK
    );
    set(&now, 2_000);
    assert_eq!(
        read(&fixture, color(), PropertyIdentifier::PRESENT_VALUE).await,
        read_ack(color(), &PRESENT_VALUE, &xy(HALF, THREE_EIGHTHS))
    );
    assert_eq!(
        read(&fixture, color(), PropertyIdentifier::IN_PROGRESS).await,
        read_ack(color(), &IN_PROGRESS, &[0x91, 0x00])
    );
    // A coordinate past 1.0 (1.5 is 0x3FC00000) is out of range.
    let past = xy([0x3F, 0xC0, 0x00, 0x00], HALF);
    assert_eq!(
        write(&fixture, color(), PropertyIdentifier::PRESENT_VALUE, &past).await,
        OUT_OF_RANGE
    );
}

#[tokio::test]
async fn temperature_steps_ramps_and_writes_clamp_over_the_wire() {
    let (fixture, now) = fixture().await;
    let ct = temperature();
    let cc = PropertyIdentifier::COLOR_COMMAND;
    let pv = PropertyIdentifier::PRESENT_VALUE;
    // The limits moved 4000 K down to 3000 K (0x0BB8). STEP_UP_CCT by 100 K
    // stays there, and STEP_DOWN_CCT by 500 K (0x01F4) stops at 2700 K
    // (0x0A8C).
    assert_eq!(
        write(&fixture, ct, cc, &[0x09, 0x04, 0x59, 0x64]).await,
        SIMPLE_ACK
    );
    assert_eq!(
        read(&fixture, ct, pv).await,
        read_ack(ct, &PRESENT_VALUE, &[0x22, 0x0B, 0xB8])
    );
    let step_down = [0x09, 0x05, 0x5A, 0x01, 0xF4];
    assert_eq!(write(&fixture, ct, cc, &step_down).await, SIMPLE_ACK);
    assert_eq!(
        read(&fixture, ct, pv).await,
        read_ack(ct, &PRESENT_VALUE, &[0x22, 0x0A, 0x8C])
    );
    // RAMP_TO_CCT to 3000 K at 100 K/s takes 3 s; halfway it is RAMP_ACTIVE
    // (2) at 2850 K (0x0B22).
    let ramp = [0x09, 0x03, 0x2A, 0x0B, 0xB8, 0x49, 0x64];
    assert_eq!(write(&fixture, ct, cc, &ramp).await, SIMPLE_ACK);
    set(&now, 1_500);
    assert_eq!(
        read(&fixture, ct, PropertyIdentifier::TRACKING_VALUE).await,
        read_ack(ct, &TRACKING_VALUE, &[0x22, 0x0B, 0x22])
    );
    assert_eq!(
        read(&fixture, ct, PropertyIdentifier::IN_PROGRESS).await,
        read_ack(ct, &IN_PROGRESS, &[0x91, 0x02])
    );
    // A Present_Value write halts it. 1000 K (0x03E8) is clamped up to 2700
    // K rather than refused; 999 K (0x03E7) is outside the object's range.
    assert_eq!(
        write(&fixture, ct, pv, &[0x22, 0x03, 0xE8]).await,
        SIMPLE_ACK
    );
    assert_eq!(
        read(&fixture, ct, PropertyIdentifier::TRACKING_VALUE).await,
        read_ack(ct, &TRACKING_VALUE, &[0x22, 0x0A, 0x8C])
    );
    assert_eq!(
        read(&fixture, ct, PropertyIdentifier::IN_PROGRESS).await,
        read_ack(ct, &IN_PROGRESS, &[0x91, 0x00])
    );
    assert_eq!(
        write(&fixture, ct, pv, &[0x22, 0x03, 0xE7]).await,
        OUT_OF_RANGE
    );
}

#[tokio::test]
async fn default_fade_time_and_the_dropped_rows_over_the_wire() {
    let (fixture, _) = fixture().await;
    let fade_time = PropertyIdentifier::DEFAULT_FADE_TIME;
    for oid in [color(), temperature()] {
        // It reads 100 ms (0x64) from the start, and 99 ms is refused.
        assert_eq!(
            read(&fixture, oid, fade_time).await,
            read_ack(oid, &[0x1A, 0x01, 0x76], &[0x21, 0x64])
        );
        assert_eq!(
            write(&fixture, oid, fade_time, &[0x21, 0x63]).await,
            OUT_OF_RANGE
        );
        // A day, 86,400,000 ms (0x05265C00), is taken.
        let day = [0x24, 0x05, 0x26, 0x5C, 0x00];
        assert_eq!(write(&fixture, oid, fade_time, &day).await, SIMPLE_ACK);
        // Neither table has Status_Flags: ReadProperty (12) answers
        // PROPERTY (2) / UNKNOWN_PROPERTY (32).
        assert_eq!(
            read(&fixture, oid, PropertyIdentifier::STATUS_FLAGS).await,
            [0x50, 5, 12, 0x91, 0x02, 0x91, 0x20]
        );
    }
}

/// Audit_Level (498, `1A 01 F2`) and Auditable_Operations (501, `1A 01 F5`)
/// once provisioned (#1525). COLOR 2 is `0F C0 00 02` and COLOR_TEMPERATURE 2
/// `10 00 00 02`.
#[tokio::test]
async fn provisioned_audit_rows_over_the_wire() {
    use bacnet_objects::audit::ObjectAuditPolicy;
    use bacnet_types::bitstring::AuditOperationFlags;
    use bacnet_types::enums::{AuditLevel, AuditOperation};
    let (fixture, _) = fixture().await;
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    let policy = ObjectAuditPolicy {
        level: Some(AuditLevel::AUDIT_CONFIG),
        operations: Some(operations),
        ..ObjectAuditPolicy::default()
    };
    let color2 = ObjectIdentifier::new(ObjectType::COLOR, 2).unwrap();
    let temperature2 = ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 2).unwrap();
    {
        let mut db = fixture.db.write().await;
        let mut color = ColorObject::new(2, "CLR-2").unwrap();
        color.set_audit_policy(policy);
        db.add(Box::new(color)).unwrap();
        let mut temperature = ColorTemperatureObject::new(2, "CT-2").unwrap();
        temperature.set_audit_policy(policy);
        db.add(Box::new(temperature)).unwrap();
    }
    const LEVEL: [u8; 3] = [0x1A, 0x01, 0xF2];
    const OPERATIONS: [u8; 3] = [0x1A, 0x01, 0xF5];
    let level = PropertyIdentifier::AUDIT_LEVEL;
    for oid in [color2, temperature2] {
        // AUDIT_CONFIG (2) as an application ENUMERATED.
        assert_eq!(
            read(&fixture, oid, level).await,
            read_ack(oid, &LEVEL, &[0x91, 0x02])
        );
        // WRITE alone: a two-bit BIT STRING, six bits unused, 0x40.
        assert_eq!(
            read(&fixture, oid, PropertyIdentifier::AUDITABLE_OPERATIONS).await,
            read_ack(oid, &OPERATIONS, &[0x82, 0x06, 0x40])
        );
        // AUDIT_ALL (1) is taken and reads back.
        assert_eq!(write(&fixture, oid, level, &[0x91, 0x01]).await, SIMPLE_ACK);
        assert_eq!(
            read(&fixture, oid, level).await,
            read_ack(oid, &LEVEL, &[0x91, 0x01])
        );
        // Neither table has Audit_Priority_Filter (500).
        assert_eq!(
            read(&fixture, oid, PropertyIdentifier::AUDIT_PRIORITY_FILTER).await,
            [0x50, 5, 12, 0x91, 0x02, 0x91, 0x20]
        );
    }
    // Unprovisioned, the rows are absent.
    assert_eq!(
        read(&fixture, color(), level).await,
        [0x50, 5, 12, 0x91, 0x02, 0x91, 0x20]
    );
}
