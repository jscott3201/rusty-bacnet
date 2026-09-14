use super::DeviceObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// The full implemented set, ordered by raw identifier. Conditional rows keep
// their optional base code; effective sets below select only readable rows.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::APDU_TIMEOUT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::APPLICATION_SOFTWARE_VERSION,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(P::DAYLIGHT_SAVINGS_STATUS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::DEVICE_ADDRESS_BINDING, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::FIRMWARE_REVISION, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LOCAL_DATE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LOCAL_TIME, Optional, None, ReadOnly),
    PropertyMetadata::new(P::MAX_APDU_LENGTH_ACCEPTED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MODEL_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NUMBER_OF_APDU_RETRIES, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_LIST, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::PROTOCOL_OBJECT_TYPES_SUPPORTED,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(P::PROTOCOL_SERVICES_SUPPORTED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROTOCOL_VERSION, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SEGMENTATION_SUPPORTED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SYSTEM_STATUS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::UTC_OFFSET, Optional, None, ReadOnly),
    PropertyMetadata::new(P::VENDOR_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::VENDOR_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROTOCOL_REVISION, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACTIVE_COV_SUBSCRIPTIONS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DATABASE_REVISION, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MAX_SEGMENTS_ACCEPTED, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LAST_RESTART_REASON, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DEVICE_UUID, Optional, None, ReadOnly),
];

// Precompute the four effective sets from one source so every call can borrow
// rows without caching clock availability or adding per-object metadata state.
const fn effective<const N: usize>(clock: bool, segments: bool) -> [PropertyMetadata; N] {
    let mut rows = [BASE[0]; N];
    let mut source = 0;
    let mut target = 0;
    while source < BASE.len() {
        let property = BASE[source].property_identifier;
        let clock_row = matches!(
            property,
            P::LOCAL_DATE | P::LOCAL_TIME | P::UTC_OFFSET | P::DAYLIGHT_SAVINGS_STATUS
        );
        let segment_row = matches!(property, P::MAX_SEGMENTS_ACCEPTED);
        if (!clock_row || clock) && (!segment_row || segments) {
            rows[target] = BASE[source];
            target += 1;
        }
        source += 1;
    }
    assert!(target == N);
    rows
}

const CLOCKLESS: &[PropertyMetadata] = &effective::<25>(false, false);
const CLOCKLESS_SEGMENTED: &[PropertyMetadata] = &effective::<26>(false, true);
const CLOCKED: &[PropertyMetadata] = &effective::<29>(true, false);

pub(super) fn for_object(object: &DeviceObject) -> Cow<'_, [PropertyMetadata]> {
    let clock = object.clock_frame().is_some();
    let segments = object.properties.contains_key(&P::MAX_SEGMENTS_ACCEPTED);
    Cow::Borrowed(match (clock, segments) {
        (false, false) => CLOCKLESS,
        (false, true) => CLOCKLESS_SEGMENTED,
        (true, false) => CLOCKED,
        (true, true) => BASE,
    })
}
