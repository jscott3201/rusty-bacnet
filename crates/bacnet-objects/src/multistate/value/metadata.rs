use super::MultiStateValueObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyPresenceCondition::{Commandable, IntrinsicReporting},
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Preserve legacy order and optional base codes for implemented conditional rows.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::EVENT_DETECTION_ENABLE,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::EVENT_ENABLE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::TIME_DELAY, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::NOTIFY_TYPE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::NOTIFICATION_CLASS,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(
        P::ACKED_TRANSITIONS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_TIME_STAMPS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::NUMBER_OF_STATES, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(
        P::CURRENT_COMMAND_PRIORITY,
        Optional,
        Some(Commandable),
        ReadOnly,
    ),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::RELIABILITY_EVALUATION_INHIBIT, Optional, None, Always),
    // Always denotes the element-write route, not whole-array replacement.
    PropertyMetadata::new(P::STATE_TEXT, Optional, None, Always),
    PropertyMetadata::new(P::ALARM_VALUES, Optional, Some(IntrinsicReporting), Always),
    // Both reads are unconditional today. Append optional, read-only projections
    // without changing their existing values, encoding, or tracking lifecycle.
    PropertyMetadata::new(P::VALUE_SOURCE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LAST_COMMAND_TIME, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &MultiStateValueObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multistate::MultiStateOutputObject;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    #[test]
    fn property_metadata_multistate_command_validation_and_optional_readbacks() {
        let objects: [Box<dyn BACnetObject>; 2] = [
            Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
            Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()),
        ];
        for mut object in objects {
            let readbacks = [P::VALUE_SOURCE, P::LAST_COMMAND_TIME]
                .map(|p| object.read_property(p, None).unwrap());
            for p in [P::PRESENT_VALUE, P::PRIORITY_ARRAY, P::RELINQUISH_DEFAULT] {
                let index = (p == P::PRIORITY_ARRAY).then_some(8);
                for invalid in [0, 4, u64::from(u32::MAX) + 1] {
                    let before = object.read_property(p, index).unwrap();
                    assert!(
                        matches!(object.write_property(p, index, PropertyValue::Unsigned(invalid), Some(8)),
                        Err(Error::Protocol { class, code }) if class == u32::from(ErrorClass::PROPERTY.to_raw())
                            && code == u32::from(ErrorCode::VALUE_OUT_OF_RANGE.to_raw()))
                    );
                    assert_eq!(object.read_property(p, index).unwrap(), before);
                }
                object
                    .write_property(p, index, PropertyValue::Unsigned(3), Some(8))
                    .unwrap();
            }
            assert!(
                matches!(object.write_property(P::PRIORITY_ARRAY, None, PropertyValue::Null, None),
                Err(Error::Protocol { class, code }) if class == u32::from(ErrorClass::PROPERTY.to_raw())
                    && code == u32::from(ErrorCode::WRITE_ACCESS_DENIED.to_raw()))
            );
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Null, Some(8))
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(3)
            );
            assert_eq!(
                object
                    .read_property(P::CURRENT_COMMAND_PRIORITY, None)
                    .unwrap(),
                PropertyValue::Null
            );
            // Discovery changes only: command writes retain the existing readback lifecycle.
            for (p, before) in [P::VALUE_SOURCE, P::LAST_COMMAND_TIME]
                .into_iter()
                .zip(readbacks)
            {
                assert_eq!(object.read_property(p, None).unwrap(), before);
                assert!(!object.is_array_property(p));
                assert!(!object.is_writable_property(p));
            }
        }
    }

    #[test]
    fn property_metadata_multistate_resize_preserves_retained_configuration() {
        let mut object = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
        let metadata = object.property_metadata().into_owned();
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(3), Some(8))
            .unwrap();
        object.set_relinquish_default(3).unwrap();
        object.set_alarm_values(vec![3]);
        object
            .write_property(
                P::STATE_TEXT,
                Some(1),
                PropertyValue::CharacterString("Retained".into()),
                None,
            )
            .unwrap();
        object.set_number_of_states(1).unwrap();
        assert!(object.set_number_of_states(0).is_err());
        assert_eq!(object.property_metadata().as_ref(), metadata);
        for p in [P::PRESENT_VALUE, P::RELINQUISH_DEFAULT] {
            assert_eq!(
                object.read_property(p, None).unwrap(),
                PropertyValue::Unsigned(3)
            );
        }
        assert_eq!(
            object.read_property(P::PRIORITY_ARRAY, Some(8)).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            object.read_property(P::ALARM_VALUES, None).unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(3)])
        );
        assert_eq!(
            object.read_property(P::STATE_TEXT, Some(0)).unwrap(),
            PropertyValue::Unsigned(1)
        );
        object.set_number_of_states(3).unwrap();
        assert_eq!(
            object.read_property(P::STATE_TEXT, None).unwrap(),
            PropertyValue::List(vec![
                PropertyValue::CharacterString("Retained".into()),
                PropertyValue::CharacterString("State 2".into()),
                PropertyValue::CharacterString("State 3".into()),
            ])
        );
    }
}
