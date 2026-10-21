use super::*;

#[test]
fn database_revision_is_set_through_the_device_authority() {
    let mut device: Box<dyn BACnetObject> = Box::new(make_device());

    device
        .device_authority_internal()
        .expect("the Device has an authority")
        .set_database_revision(42);

    assert_eq!(
        device
            .read_property(PropertyIdentifier::DATABASE_REVISION, None)
            .unwrap(),
        PropertyValue::Unsigned(42)
    );
}
