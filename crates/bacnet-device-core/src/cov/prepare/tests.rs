//! Value preparation's own checks; the server's `prepare_tests` run the
//! rest against its admission.

use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_types::enums::EngineeringUnits;

fn prepared(
    object: &dyn BACnetObject,
    p: PropertyIdentifier,
    i: Option<u32>,
    inc: Option<f32>,
) -> PreparedCovValue {
    prepare_value(object, p, i, inc, &object.read_property(p, i).unwrap()).unwrap()
}

#[test]
fn cov_sample_takes_event_message_texts_config_whole() {
    // A whole Event_Message_Texts_Config is a known array (#1329), sampled
    // and compared like Event_Message_Texts.
    let p = PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG;
    let mut av = AnalogValueObject::new(1, "value", EngineeringUnits::NO_UNITS).unwrap();
    let before = prepared(&av, p, None, None);
    assert!(!before.numeric);
    let texts = ["High", "", ""].map(|text| PropertyValue::CharacterString(text.into()));
    av.write_property(p, None, PropertyValue::List(texts.to_vec()), None)
        .unwrap();
    let after = prepared(&av, p, None, None);
    assert!(after.reports(Some(&before.sample)));
    assert!(!after.reports(Some(&after.sample)));
}
