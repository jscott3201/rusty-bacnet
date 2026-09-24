use super::AnalogValueObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyPresenceCondition::{Commandable, IntrinsicReporting},
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Base conformance is independent of implemented writability. Commandable and
// intrinsic rows retain their optional base code. Preserve legacy list order.
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
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::UNITS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(
        P::CURRENT_COMMAND_PRIORITY,
        Optional,
        Some(Commandable),
        ReadOnly,
    ),
    PropertyMetadata::new(P::COV_INCREMENT, Optional, None, Always),
    PropertyMetadata::new(P::HIGH_LIMIT, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::LOW_LIMIT, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::DEADBAND, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::LIMIT_ENABLE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::EVENT_ENABLE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::NOTIFY_TYPE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::NOTIFICATION_CLASS,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::TIME_DELAY, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::RELIABILITY_EVALUATION_INHIBIT, Optional, None, Always),
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
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(object: &AnalogValueObject) -> Cow<'_, [PropertyMetadata]> {
    let mut rows = Cow::Borrowed(BASE);
    if object.audit_policy != crate::audit::ObjectAuditPolicy::default() {
        rows.to_mut().extend(object.audit_policy.metadata());
    }
    // Fault limits are paired; engineering bounds are independently readable.
    if object.fault_out_of_range.limits.is_some() {
        rows.to_mut().extend([
            PropertyMetadata::new(P::FAULT_HIGH_LIMIT, Optional, None, ReadOnly),
            PropertyMetadata::new(P::FAULT_LOW_LIMIT, Optional, None, ReadOnly),
        ]);
    }
    if object.min_pres_value.is_some() {
        rows.to_mut().push(PropertyMetadata::new(
            P::MIN_PRES_VALUE,
            Optional,
            None,
            ReadOnly,
        ));
    }
    if object.max_pres_value.is_some() {
        rows.to_mut().push(PropertyMetadata::new(
            P::MAX_PRES_VALUE,
            Optional,
            None,
            ReadOnly,
        ));
    }
    rows
}
