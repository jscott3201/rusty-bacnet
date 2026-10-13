//! The Addendum 135-2020ca properties of the lighting objects over the wire:
//! Lighting Output's trims (#1528) and both outputs' colour links (#1527).
//! The replies are exactly as the server encodes them.
//!
//! LIGHTING_OUTPUT 1 is `0D 80 00 01`. The trims are High_End_Trim 4194335
//! (`1B 40 00 1F` as context tag 1), Low_End_Trim 4194336 (`1B 40 00 20`)
//! and Trim_Fade_Time 4194337 (`1B 40 00 21`); Tracking_Value is 164 and
//! In_Progress 378.

// A child of `lighting_command_wire_tests`, whose imports (the wire
// fixture and the request types) it takes whole.
use super::*;
use bacnet_objects::traits::BACnetObject;

const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;
const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;
/// SimpleACK for WriteProperty (15), invoke ID 5.
const SIMPLE_ACK: [u8; 3] = [0x20, 5, 15];
/// WriteProperty refused with PROPERTY (2) / VALUE_OUT_OF_RANGE (37).
const OUT_OF_RANGE: [u8; 7] = [0x50, 5, 15, 0x91, 0x02, 0x91, 0x25];
/// WriteProperty refused with PROPERTY (2) / UNKNOWN_PROPERTY (32).
const UNKNOWN_WRITE: [u8; 7] = [0x50, 5, 15, 0x91, 0x02, 0x91, 0x20];

const HIGH_END_TRIM: [u8; 4] = [0x1B, 0x40, 0x00, 0x1F];
const TRIM_FADE_TIME: [u8; 4] = [0x1B, 0x40, 0x00, 0x21];
const TRACKING_VALUE: [u8; 2] = [0x19, 0xA4];
const IN_PROGRESS: [u8; 3] = [0x1A, 0x01, 0x7A];

fn lo1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, 1).unwrap()
}

/// A fixture holding `objects`.
async fn fixture(objects: Vec<Box<dyn BACnetObject>>) -> Fixture {
    let fixture = Fixture::new(None);
    let mut db = fixture.db.write().await;
    for object in objects {
        db.add(object).unwrap();
    }
    drop(db);
    fixture
}

async fn write(
    fixture: &Fixture,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: &[u8],
    priority: Option<u8>,
) -> Vec<u8> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
        priority,
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

#[tokio::test]
async fn trims_over_the_wire() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.set_high_end_trim(Some(80.0)).unwrap();
    let fixture = fixture(vec![Box::new(lo)]).await;
    let (high, low, fade_time) = (
        PropertyIdentifier::HIGH_END_TRIM,
        PropertyIdentifier::LOW_END_TRIM,
        PropertyIdentifier::TRIM_FADE_TIME,
    );
    // 80.0 is REAL 0x42A00000; Trim_Fade_Time starts at Unsigned 0.
    assert_eq!(
        read(&fixture, lo1(), high).await,
        read_ack(lo1(), &HIGH_END_TRIM, &[0x44, 0x42, 0xA0, 0x00, 0x00])
    );
    assert_eq!(
        read(&fixture, lo1(), fade_time).await,
        read_ack(lo1(), &TRIM_FADE_TIME, &[0x21, 0x00])
    );
    // Present_Value 90.0 (0x42B40000) at priority 8 tracks at the trim,
    // and In_Progress reads TRIM_ACTIVE (5).
    let ninety = [0x44, 0x42, 0xB4, 0x00, 0x00];
    let present_value = PropertyIdentifier::PRESENT_VALUE;
    assert_eq!(
        write(&fixture, lo1(), present_value, &ninety, Some(8)).await,
        SIMPLE_ACK
    );
    assert_eq!(
        read(&fixture, lo1(), PropertyIdentifier::TRACKING_VALUE).await,
        read_ack(lo1(), &TRACKING_VALUE, &[0x44, 0x42, 0xA0, 0x00, 0x00])
    );
    assert_eq!(
        read(&fixture, lo1(), PropertyIdentifier::IN_PROGRESS).await,
        read_ack(lo1(), &IN_PROGRESS, &[0x91, 0x05])
    );
    // A trim above 100.0 (100.5 is 0x42C90000) is out of range, and a day
    // and a millisecond of Trim_Fade_Time (0x05265C01) too.
    assert_eq!(
        write(&fixture, lo1(), high, &[0x44, 0x42, 0xC9, 0x00, 0x00], None).await,
        OUT_OF_RANGE
    );
    let too_long = [0x24, 0x05, 0x26, 0x5C, 0x01];
    assert_eq!(
        write(&fixture, lo1(), fade_time, &too_long, None).await,
        OUT_OF_RANGE
    );
    // Low_End_Trim was never set, so it isn't there to write.
    assert_eq!(
        write(&fixture, lo1(), low, &[0x44, 0x41, 0xA0, 0x00, 0x00], None).await,
        UNKNOWN_WRITE
    );
    // Raising the trim to 95.0 (0x42BE0000) with no Trim_Fade_Time lets
    // Tracking_Value up to Present_Value at once: IDLE (0).
    assert_eq!(
        write(&fixture, lo1(), high, &[0x44, 0x42, 0xBE, 0x00, 0x00], None).await,
        SIMPLE_ACK
    );
    assert_eq!(
        read(&fixture, lo1(), PropertyIdentifier::TRACKING_VALUE).await,
        read_ack(lo1(), &TRACKING_VALUE, &ninety)
    );
    assert_eq!(
        read(&fixture, lo1(), PropertyIdentifier::IN_PROGRESS).await,
        read_ack(lo1(), &IN_PROGRESS, &[0x91, 0x00])
    );
}

/// Color_Reference 4194329, Color_Override 4194328 and
/// Override_Color_Reference 4194332 as context tag 1.
const COLOR_REFERENCE: [u8; 4] = [0x1B, 0x40, 0x00, 0x19];
const COLOR_OVERRIDE: [u8; 4] = [0x1B, 0x40, 0x00, 0x18];
const OVERRIDE_COLOR_REFERENCE: [u8; 4] = [0x1B, 0x40, 0x00, 0x1C];
/// COLOR 1 and COLOR_TEMPERATURE 2 as application object identifiers: type
/// 63 and 64 in the top ten bits.
const COLOR_1: [u8; 5] = [0xC4, 0x0F, 0xC0, 0x00, 0x01];
const COLOR_TEMPERATURE_2: [u8; 5] = [0xC4, 0x10, 0x00, 0x00, 0x02];

/// Both lighting outputs' colour links over the wire (#1527), and the
/// override, once written, reaching the colour object it names.
/// BINARY_LIGHTING_OUTPUT 1 is `0D C0 00 01`.
#[tokio::test]
async fn colour_links_over_the_wire() {
    use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
    use bacnet_objects::lighting::{
        BinaryLightingOutputObject, ColorLink, ColorOverride, LightingColor, OutputColor,
    };
    let link = ColorLink {
        reference: ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap(),
        color_override: Some(ColorOverride {
            active: false,
            reference: ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 2).unwrap(),
        }),
    };
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.set_color_link(Some(link)).unwrap();
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    blo.set_color_link(Some(link)).unwrap();
    let mut ct = ColorTemperatureObject::new(2, "CT-2").unwrap();
    ct.set_present_value(2_700).unwrap();
    let fixture = fixture(vec![
        Box::new(lo),
        Box::new(blo),
        Box::new(LightingOutputObject::new(2, "LO-2").unwrap()),
        Box::new(ColorObject::new(1, "CLR-1").unwrap()),
        Box::new(ct),
    ])
    .await;
    let blo1 = ObjectIdentifier::new(ObjectType::BINARY_LIGHTING_OUTPUT, 1).unwrap();
    let (reference, color_override, override_reference) = (
        PropertyIdentifier::COLOR_REFERENCE,
        PropertyIdentifier::COLOR_OVERRIDE,
        PropertyIdentifier::OVERRIDE_COLOR_REFERENCE,
    );
    for output in [lo1(), blo1] {
        assert_eq!(
            read(&fixture, output, reference).await,
            read_ack(output, &COLOR_REFERENCE, &COLOR_1)
        );
        // FALSE is application tag 1 with value 0.
        assert_eq!(
            read(&fixture, output, color_override).await,
            read_ack(output, &COLOR_OVERRIDE, &[0x10])
        );
        assert_eq!(
            read(&fixture, output, override_reference).await,
            read_ack(output, &OVERRIDE_COLOR_REFERENCE, &COLOR_TEMPERATURE_2)
        );
        // An ANALOG_VALUE 1 reference (`C4 00 80 00 01`) is out of range,
        // and an Unsigned override is the wrong datatype (9).
        let analog_value = [0xC4, 0x00, 0x80, 0x00, 0x01];
        assert_eq!(
            write(&fixture, output, reference, &analog_value, None).await,
            OUT_OF_RANGE
        );
        assert_eq!(
            write(&fixture, output, color_override, &[0x21, 0x01], None).await,
            [0x50, 5, 15, 0x91, 0x02, 0x91, 0x09]
        );
        // TRUE turns the override on, and the output's colour now comes
        // from COLOR_TEMPERATURE 2.
        assert_eq!(
            write(&fixture, output, color_override, &[0x11], None).await,
            SIMPLE_ACK
        );
        assert_eq!(
            read(&fixture, output, color_override).await,
            read_ack(output, &COLOR_OVERRIDE, &[0x11])
        );
        assert_eq!(
            fixture.db.read().await.lighting_color(&output),
            Some(LightingColor {
                source: ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 2).unwrap(),
                color: OutputColor::Kelvin(2_700),
            })
        );
        // A Color_Reference of COLOR 4194303 (`C4 0F FF FF FF`) names no
        // companion; it's taken and read back.
        let none = [0xC4, 0x0F, 0xFF, 0xFF, 0xFF];
        assert_eq!(
            write(&fixture, output, reference, &none, None).await,
            SIMPLE_ACK
        );
        assert_eq!(
            read(&fixture, output, reference).await,
            read_ack(output, &COLOR_REFERENCE, &none)
        );
        // The instance alone means none, so ANALOG_VALUE 4194303
        // (`C4 00 BF FF FF`) is taken too.
        let none_of_another_type = [0xC4, 0x00, 0xBF, 0xFF, 0xFF];
        assert_eq!(
            write(
                &fixture,
                output,
                override_reference,
                &none_of_another_type,
                None
            )
            .await,
            SIMPLE_ACK
        );
        assert_eq!(fixture.db.read().await.lighting_color(&output), None);
    }
    // An output with no link has none of the rows: ReadProperty (12) answers
    // UNKNOWN_PROPERTY.
    let lo2 = ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, 2).unwrap();
    assert_eq!(
        read(&fixture, lo2, reference).await,
        [0x50, 5, 12, 0x91, 0x02, 0x91, 0x20]
    );
}
