use super::MultiStateOutputObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::event::options::REPORTING_OPTION_METADATA;
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyPresenceCondition::{CommandableValueSourceTracking, ValueSourceTracking},
    PropertyPresenceCondition::{IntrinsicReportingOptional, IntrinsicReportingRequired},
    PropertyWriteCapability::WhenCommandOwner,
    PropertyWriteCapability::{Always, ReadOnly, Through, WhenOutOfService},
};

// Output is always commandable. Preserve legacy order and keep feedback's
// independent write route distinct from command resolution and state-set bounds.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(
        P::FEEDBACK_VALUE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::EVENT_DETECTION_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::EVENT_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::TIME_DELAY,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
    PropertyMetadata::new(
        P::NOTIFY_TYPE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::NOTIFICATION_CLASS,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::ACKED_TRANSITIONS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_TIME_STAMPS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS,
        Optional,
        Some(IntrinsicReportingOptional),
        ReadOnly,
    ),
    // Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1329).
    REPORTING_OPTION_METADATA[0],
    REPORTING_OPTION_METADATA[1],
    REPORTING_OPTION_METADATA[2],
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    // No write of its own: State_Text written whole or at index 0 sets it (#1443).
    PropertyMetadata::new(
        P::NUMBER_OF_STATES,
        RequiredRead,
        None,
        Through(P::STATE_TEXT),
    ),
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::CURRENT_COMMAND_PRIORITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::RELIABILITY_EVALUATION_INHIBIT, Optional, None, Always),
    // Element writes, and whole or index-0 writes that set Number_Of_States.
    PropertyMetadata::new(P::STATE_TEXT, Optional, None, Always),
    // The enabled command-source mechanism makes these paired properties required.
    PropertyMetadata::new(
        P::VALUE_SOURCE,
        Optional,
        Some(ValueSourceTracking),
        WhenCommandOwner,
    ),
    PropertyMetadata::new(
        P::VALUE_SOURCE_ARRAY,
        Optional,
        Some(CommandableValueSourceTracking),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::LAST_COMMAND_TIME,
        Optional,
        Some(CommandableValueSourceTracking),
        ReadOnly,
    ),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(object: &MultiStateOutputObject) -> Cow<'_, [PropertyMetadata]> {
    let mut rows = Cow::Borrowed(BASE);
    for row in object.profile.metadata() {
        rows.to_mut().push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    #[test]
    fn property_metadata_multistate_feedback_stays_independent_of_commands_and_resize() {
        for out_of_service in [false, true] {
            let mut object = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
            let metadata = object.property_metadata().into_owned();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            object
                .write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Unsigned(2),
                    Some(8),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
                PropertyValue::Unsigned(1)
            );
            for feedback in [0, 7, u64::from(u32::MAX)] {
                object
                    .write_property(
                        P::FEEDBACK_VALUE,
                        None,
                        PropertyValue::Unsigned(feedback),
                        None,
                    )
                    .unwrap();
                assert_eq!(
                    object.read_property(P::PRESENT_VALUE, None).unwrap(),
                    PropertyValue::Unsigned(2)
                );
                assert_eq!(
                    object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
                    PropertyValue::Unsigned(feedback)
                );
            }
            assert!(
                matches!(object.write_property(P::FEEDBACK_VALUE, None, PropertyValue::Unsigned(0x1_0000_0002), None),
                Err(Error::Protocol { class, code }) if class == u32::from(ErrorClass::PROPERTY.to_raw())
                    && code == u32::from(ErrorCode::VALUE_OUT_OF_RANGE.to_raw()))
            );
            object.set_relinquish_default(3).unwrap();
            object
                .write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Null,
                    Some(8),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
            object.set_number_of_states(1).unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(3)
            );
            assert_eq!(
                object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
                PropertyValue::Unsigned(3)
            );
            assert_eq!(
                object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
                PropertyValue::Unsigned(u64::from(u32::MAX))
            );
            assert_eq!(object.property_metadata().as_ref(), metadata);
        }
    }
}
