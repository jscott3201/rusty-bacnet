use super::BinaryOutputObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::event::options::REPORTING_OPTION_METADATA;
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyPresenceCondition::{CommandableValueSourceTracking, ValueSourceTracking},
    PropertyPresenceCondition::{
        IntrinsicReportingOptional, IntrinsicReportingRequired, PairedText,
    },
    PropertyWriteCapability::WhenCommandOwner,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Binary Output is always commandable. Base conformance remains independent
// of implemented writability; order preserves the legacy list projection.
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
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::CURRENT_COMMAND_PRIORITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::POLARITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::RELIABILITY_EVALUATION_INHIBIT, Optional, None, Always),
    PropertyMetadata::new(P::ACTIVE_TEXT, Optional, Some(PairedText), Always),
    PropertyMetadata::new(P::INACTIVE_TEXT, Optional, Some(PairedText), Always),
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

pub(super) fn for_object(object: &BinaryOutputObject) -> Cow<'_, [PropertyMetadata]> {
    // This binary implementation has no instance-conditional bounds.
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
    fn property_metadata_feedback_stays_independent_of_command_resolution() {
        for out_of_service in [false, true] {
            let mut object = BinaryOutputObject::new(1, "BO-1").unwrap();
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
                    PropertyValue::Enumerated(1),
                    Some(8),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
            object
                .write_property(P::FEEDBACK_VALUE, None, PropertyValue::Enumerated(1), None)
                .unwrap();
            object
                .write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Null,
                    Some(8),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
            assert_eq!(
                object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
                PropertyValue::Enumerated(1)
            );
            assert!(
                matches!(object.write_property(P::FEEDBACK_VALUE, None, PropertyValue::Enumerated(2), None),
                Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY.to_raw() as u32
                        && code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32)
            );
            assert_eq!(
                object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
                PropertyValue::Enumerated(1)
            );
        }
    }
}
