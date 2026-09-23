use super::*;

#[test]
fn device_description_write_read() {
    let mut dev = make_device();
    dev.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Main building controller".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        dev.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Main building controller".into())
    );
    dev.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::Null,
        Some(8),
    )
    .unwrap();
    assert_eq!(
        dev.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Main building controller".into())
    );
    assert!(dev
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            Some(0),
            PropertyValue::Null,
            None
        )
        .is_err());
}
