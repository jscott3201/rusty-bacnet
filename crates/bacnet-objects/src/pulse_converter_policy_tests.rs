use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

fn assert_write_access_denied(result: Result<(), Error>) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED);
        }
        other => panic!("expected PROPERTY/WRITE_ACCESS_DENIED, got {other:?}"),
    }
}

fn present_value(pc: &PulseConverterObject) -> PropertyValue {
    pc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
}

#[test]
fn present_value_is_denied_in_service_before_value_validation() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert!(pc.is_writable_property(PropertyIdentifier::PRESENT_VALUE));

    for value in [
        PropertyValue::Real(12.5),
        PropertyValue::Unsigned(12),
        PropertyValue::Real(f32::NAN),
    ] {
        assert_write_access_denied(pc.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value,
            None,
        ));
        assert_eq!(present_value(&pc), PropertyValue::Real(0.0));
    }
}

#[test]
fn present_value_round_trips_while_out_of_service() {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    pc.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(12.5),
        None,
    )
    .unwrap();

    assert_eq!(present_value(&pc), PropertyValue::Real(12.5));
}
