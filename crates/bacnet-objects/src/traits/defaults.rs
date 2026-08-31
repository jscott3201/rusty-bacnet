use bacnet_types::enums::{ObjectType, PropertyIdentifier};

/// The default array/list classification behind
/// [`super::BACnetObject::is_array_property`], keyed by the Clause 12 property
/// tables. Three identifier classes:
///
/// - **Identifier-stable BACnetARRAY** properties admit an index on every
///   object type that defines them: OBJECT_LIST (Table 12-13), PROPERTY_LIST
///   (every table), STATE_TEXT (Tables 12-21/12-22/12-23), PRIORITY
///   (Table 12-24), WEEKLY_SCHEDULE / EXCEPTION_SCHEDULE (Table 12-28),
///   EVENT_TIME_STAMPS / EVENT_MESSAGE_TEXTS (Table 12-2 family),
///   PRIORITY_ARRAY (the commandable family), TAGS (Annex Y),
///   SUBORDINATE_LIST / SUBORDINATE_ANNOTATIONS (Table 12-34),
///   GROUP_MEMBERS / GROUP_MEMBER_NAMES (Table 12-57; Elevator/Lift also type
///   GROUP_MEMBERS BACnetARRAY), ACTION (Table 12-12), and STAGES /
///   STAGE_NAMES / TARGET_REFERENCES (Table 12-80).
/// - **Type-dependent** identifiers classify by `object_type`: ALARM_VALUES /
///   FAULT_VALUES are BACnetARRAY[N] on CharacterString Value (Table 12-44)
///   and BitString Value (Table 12-47) but BACnetLIST on the multi-state,
///   life-safety, and access families; LIST_OF_OBJECT_PROPERTY_REFERENCES is
///   BACnetARRAY[N] on Channel (Table 12-62) but BACnetLIST on Schedule
///   (Table 12-28) and Timer (Table 12-75); PRESENT_VALUE is
///   BACnetARRAY[N] of BACnetPropertyAccessResult on Global Group
///   (Table 12-57) but scalar elsewhere.
/// - **Everything else** — scalars and the identifier-stable BACnetLIST
///   properties DATE_LIST (Table 12-11), LIST_OF_GROUP_MEMBERS
///   (Table 12-17), RECIPIENT_LIST (Table 12-24), LOG_BUFFER
///   (Tables 12-29/12-31), DEVICE_ADDRESS_BINDING and
///   ACTIVE_COV_SUBSCRIPTIONS (Table 12-13) — takes no index: Clause 12.1.5.2
///   makes ReadRange the only positional access to a BACnetLIST. Array-typed
///   identifiers whose object types are not modeled in-tree (e.g.
///   ACTION_TEXT, EVENT_MESSAGE_TEXTS_CONFIG, VALUE_SOURCE_ARRAY) stay
///   rejected until their object-side modeling lands.
///
/// Like [`historical_writable_default`] this is a free function (not a
/// per-object override) so the default trait method can delegate to it
/// without requiring `Self: Sized` (which would break `dyn BACnetObject`
/// dispatch).
#[inline]
pub(super) fn array_property_default(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> bool {
    match property {
        PropertyIdentifier::OBJECT_LIST
        | PropertyIdentifier::PROPERTY_LIST
        | PropertyIdentifier::STATE_TEXT
        | PropertyIdentifier::PRIORITY
        | PropertyIdentifier::WEEKLY_SCHEDULE
        | PropertyIdentifier::EXCEPTION_SCHEDULE
        | PropertyIdentifier::EVENT_TIME_STAMPS
        | PropertyIdentifier::EVENT_MESSAGE_TEXTS
        | PropertyIdentifier::PRIORITY_ARRAY
        | PropertyIdentifier::TAGS
        | PropertyIdentifier::SUBORDINATE_LIST
        | PropertyIdentifier::SUBORDINATE_ANNOTATIONS
        | PropertyIdentifier::GROUP_MEMBERS
        | PropertyIdentifier::GROUP_MEMBER_NAMES
        | PropertyIdentifier::ACTION
        | PropertyIdentifier::STAGES
        | PropertyIdentifier::STAGE_NAMES
        | PropertyIdentifier::TARGET_REFERENCES => true,
        PropertyIdentifier::ALARM_VALUES | PropertyIdentifier::FAULT_VALUES => matches!(
            object_type,
            ObjectType::CHARACTERSTRING_VALUE | ObjectType::BITSTRING_VALUE
        ),
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => {
            object_type == ObjectType::CHANNEL
        }
        PropertyIdentifier::PRESENT_VALUE => object_type == ObjectType::GLOBAL_GROUP,
        _ => false,
    }
}

/// The historical PICS writable-property heuristic, used by the default
/// [`super::BACnetObject::is_writable_property`] so unmigrated object types keep
/// their current PICS output.
///
/// This is a free function (not a per-object override) so the default trait
/// method can delegate to it without requiring `Self: Sized` (which would
/// break `dyn BACnetObject` dispatch). Object implementations should override
/// [`super::BACnetObject::is_writable_property`] to mirror their real
/// `write_property` arms exactly rather than calling this.
#[inline]
pub(super) fn historical_writable_default(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> bool {
    // Universal read-only properties.
    if property == PropertyIdentifier::OBJECT_IDENTIFIER
        || property == PropertyIdentifier::OBJECT_TYPE
        || property == PropertyIdentifier::PROPERTY_LIST
        || property == PropertyIdentifier::STATUS_FLAGS
    {
        return false;
    }

    if property == PropertyIdentifier::OBJECT_NAME {
        return true;
    }

    if property == PropertyIdentifier::PRESENT_VALUE {
        return object_type != ObjectType::ANALOG_INPUT
            && object_type != ObjectType::BINARY_INPUT
            && object_type != ObjectType::MULTI_STATE_INPUT;
    }

    property == PropertyIdentifier::DESCRIPTION
        || property == PropertyIdentifier::OUT_OF_SERVICE
        || property == PropertyIdentifier::COV_INCREMENT
        || property == PropertyIdentifier::HIGH_LIMIT
        || property == PropertyIdentifier::LOW_LIMIT
        || property == PropertyIdentifier::DEADBAND
        || property == PropertyIdentifier::NOTIFICATION_CLASS
}
