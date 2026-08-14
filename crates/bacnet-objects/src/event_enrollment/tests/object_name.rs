use super::super::*;

#[test]
fn event_enrollment_object_name_writability_matches_dispatch() {
    let mut enrollment = EventEnrollmentObject::new(1, "EE-A", 0).unwrap();

    assert!(enrollment.is_writable_property(PropertyIdentifier::OBJECT_NAME));
    enrollment
        .write_property(
            PropertyIdentifier::OBJECT_NAME,
            None,
            PropertyValue::CharacterString("EE-Renamed".to_string()),
            None,
        )
        .unwrap();

    assert_eq!(enrollment.object_name(), "EE-Renamed");
    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap(),
        PropertyValue::CharacterString("EE-Renamed".to_string())
    );
}
