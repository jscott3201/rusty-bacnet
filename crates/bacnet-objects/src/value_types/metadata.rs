use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyPresenceCondition::Commandable,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the 11 remaining commandable value types
// (ASHRAE 135-2020 §12.37-§12.48, skipping §12.42 Time Value; Tables
// 12-44-12-55, skipping 12-49). Implemented table codes are identical on all
// 11: Present_Value R1, Status_Flags R, Out_Of_Service O, Reliability O,
// Priority_Array O2, Relinquish_Default O2, Property_List R (plus
// Object_Identifier/Object_Name/Object_Type R, Description O).
// Order and shape match the Time Value precedent (mod.rs): Present_Value
// RequiredRead/Always; Priority_Array/Relinquish_Default
// Optional/Commandable/Always; Out_Of_Service Optional/Always; Reliability
// Optional/ReadOnly; Property_List RequiredRead/ReadOnly (appended so the
// projection helper omits it while required_properties keeps it).
// Writability mirrors the shared dispatch arms exactly: Object_Name,
// Description, and Out_Of_Service carry the routed common write arms, so
// Always; Present_Value/Priority_Array/Relinquish_Default carry the
// commandable arms, so Always. The legacy hardcoded writability set equals
// the new Always set, so no writability change.
// Only implemented rows are described. Units (table R on Integer,
// Positive-Integer, and Large Analog Value) stays absent by design: adding
// rows without dispatch would break the readable-rows contract. Bit_Text,
// Alarm_Value, Fault/High/Low_Limit, event/intrinsic, value-source, and audit
// rows stay absent until dispatch exists. Value types are not createable at
// runtime (the network factory builds only the eight analog/binary/
// multi-state input/output/value types), so the is_createable=false default
// holds with no override.
pub(crate) const INTEGER_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const POSITIVE_INTEGER_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const LARGE_ANALOG_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const CHARACTERSTRING_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const OCTETSTRING_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const BITSTRING_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const DATE_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const DATETIME_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const DATEPATTERN_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const TIMEPATTERN_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(crate) const DATETIMEPATTERN_VALUE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];
