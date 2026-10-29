use super::NotificationForwarderObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Table 12-58 order. Reliability is always NO_FAULT_DETECTED, so it needs no
// write route (Clause 12.51.7). Port_Filter is optional on a device that does
// not route and is present only when the application configures it.
const ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RECIPIENT_LIST, RequiredRead, None, Always),
    PropertyMetadata::new(P::SUBSCRIBED_RECIPIENTS, RequiredWrite, None, Always),
    PropertyMetadata::new(P::PROCESS_IDENTIFIER_FILTER, RequiredRead, None, Always),
    PropertyMetadata::new(P::PORT_FILTER, Optional, None, Always),
    PropertyMetadata::new(P::LOCAL_FORWARDING_ONLY, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(object: &NotificationForwarderObject) -> Cow<'_, [PropertyMetadata]> {
    if object.port_filter.is_some() {
        return Cow::Borrowed(ROWS);
    }
    Cow::Owned(
        ROWS.iter()
            .copied()
            .filter(|row| row.property_identifier != P::PORT_FILTER)
            .collect(),
    )
}
