use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::CovReportedProperty;

/// The Table 13-1 properties a whole-object COV notification carries after
/// its leading value and Status_Flags, behind
/// [`super::BACnetObject::cov_reported_properties`]. A free function for the
/// same `dyn` reason as [`array_property_default`].
///
/// Every row Table 13-1 gives more than Present_Value and Status_Flags is
/// here. Access Point's row leads with Access_Event instead of Present_Value;
/// the server supplies that leading value, so only the rest is listed.
pub(super) fn cov_reported_properties_default(
    object_type: ObjectType,
) -> &'static [CovReportedProperty] {
    use CovReportedProperty::{Trigger, Value};
    use PropertyIdentifier as P;
    match object_type {
        ObjectType::ACCESS_DOOR => &[Trigger(P::DOOR_ALARM_STATE)],
        ObjectType::ACCESS_POINT => &[
            Value(P::ACCESS_EVENT_TAG),
            Trigger(P::ACCESS_EVENT_TIME),
            Value(P::ACCESS_EVENT_CREDENTIAL),
            Value(P::ACCESS_EVENT_AUTHENTICATION_FACTOR),
        ],
        ObjectType::CREDENTIAL_DATA_INPUT => &[Trigger(P::UPDATE_TIME)],
        ObjectType::LOAD_CONTROL => &[
            Trigger(P::REQUESTED_SHED_LEVEL),
            Trigger(P::START_TIME),
            Trigger(P::SHED_DURATION),
            Trigger(P::DUTY_WINDOW),
        ],
        ObjectType::LOOP => &[Value(P::SETPOINT), Value(P::CONTROLLED_VARIABLE_VALUE)],
        ObjectType::PULSE_CONVERTER => &[Value(P::UPDATE_TIME)],
        ObjectType::STAGING => &[Trigger(P::PRESENT_STAGE)],
        _ => &[],
    }
}

/// The default array/list classification behind
/// [`super::BACnetObject::is_array_property`]: every property that a Clause 12
/// object table of the 2020 standard (Tables 12-2 to 12-83) types as a
/// BACnetARRAY, whether or not the stack serves it yet. A client shapes a
/// remote value from it through [`super::standard_array_property`], and the
/// built-in objects don't override it, so the two agree. An indexed read of a
/// listed property an object doesn't hold fails as an unknown property.
///
/// - **Identifier-stable** arrays are arrays on every object type that has
///   them, so the object type isn't consulted. Beside the universal
///   PROPERTY_LIST and TAGS, they are: the Device's OBJECT_LIST,
///   STRUCTURED_OBJECT_LIST and CONFIGURATION_FILES (Table 12-13); the
///   event-reporting EVENT_TIME_STAMPS, EVENT_MESSAGE_TEXTS and
///   EVENT_MESSAGE_TEXTS_CONFIG; the commandable PRIORITY_ARRAY,
///   COMMAND_TIME_ARRAY and VALUE_SOURCE_ARRAY; STATE_TEXT (Tables 12-21 to
///   12-23); PRIORITY (Table 12-24); WEEKLY_SCHEDULE and EXCEPTION_SCHEDULE
///   (Table 12-28); SHED_LEVELS and SHED_LEVEL_DESCRIPTIONS (Table 12-32); the
///   SUBORDINATE_LIST, SUBORDINATE_ANNOTATIONS, SUBORDINATE_TAGS,
///   SUBORDINATE_NODE_TYPES and SUBORDINATE_RELATIONSHIPS of Table 12-34;
///   DOOR_MEMBERS (Table 12-30); ACCESS_DOORS and the AUTHENTICATION_POLICY_LIST
///   and AUTHENTICATION_POLICY_NAMES pair (Table 12-36); the
///   POSITIVE_ACCESS_RULES and NEGATIVE_ACCESS_RULES of Table 12-39;
///   AUTHENTICATION_FACTORS and ASSIGNED_ACCESS_RIGHTS (Table 12-40);
///   SUPPORTED_FORMATS and SUPPORTED_FORMAT_CLASSES (Table 12-43); BIT_TEXT
///   (Table 12-47); GROUP_MEMBERS (Tables 12-57 and 12-76) and
///   GROUP_MEMBER_NAMES (Table 12-57); PORT_FILTER (Table 12-58);
///   EXECUTION_DELAY and CONTROL_GROUPS (Table 12-62); IP_DNS_SERVER,
///   IPV6_DNS_SERVER and LINK_SPEEDS (Table 12-71); STATE_CHANGE_VALUES
///   (Table 12-75); the Lift's FLOOR_TEXT, CAR_DOOR_TEXT, CAR_DOOR_STATUS,
///   CAR_DOOR_COMMAND, LANDING_DOOR_STATUS, MAKING_CAR_CALL,
///   REGISTERED_CAR_CALL and ASSIGNED_LANDING_CALLS (Table 12-77); STAGES,
///   STAGE_NAMES and TARGET_REFERENCES (Table 12-80); and MONITORED_OBJECTS
///   (Table 12-82).
/// - **Type-dependent** identifiers are arrays on some object types only:
///   ACTION and its descriptions ACTION_TEXT on Command (Table 12-12; Loop's
///   ACTION is a single value); ALARM_VALUES and FAULT_VALUES on
///   CharacterString Value and BitString Value (Tables 12-44 and 12-47; lists
///   on the multi-state, life-safety, access and timer types);
///   LIST_OF_OBJECT_PROPERTY_REFERENCES on Channel (Table 12-62; a list on
///   Schedule); LOG_DEVICE_OBJECT_PROPERTY on Trend Log Multiple (Table 12-35;
///   a single reference on Trend Log); and PRESENT_VALUE on Global Group
///   (Table 12-57).
///
/// Everything else, the BACnetLIST properties included, takes no array index:
/// Clause 12.1.5.2 leaves ReadRange as the only positional access to a list.
///
/// This is a free function (not a per-object override) so the default trait
/// method can delegate to it
/// without requiring `Self: Sized` (which would break `dyn BACnetObject`
/// dispatch).
#[inline]
pub(super) fn array_property_default(
    object_type: ObjectType,
    property: PropertyIdentifier,
) -> bool {
    use PropertyIdentifier as P;
    match property {
        P::PROPERTY_LIST
        | P::TAGS
        | P::OBJECT_LIST
        | P::STRUCTURED_OBJECT_LIST
        | P::CONFIGURATION_FILES
        | P::EVENT_TIME_STAMPS
        | P::EVENT_MESSAGE_TEXTS
        | P::EVENT_MESSAGE_TEXTS_CONFIG
        | P::PRIORITY_ARRAY
        | P::COMMAND_TIME_ARRAY
        | P::VALUE_SOURCE_ARRAY
        | P::STATE_TEXT
        | P::PRIORITY
        | P::WEEKLY_SCHEDULE
        | P::EXCEPTION_SCHEDULE
        | P::SHED_LEVELS
        | P::SHED_LEVEL_DESCRIPTIONS
        | P::SUBORDINATE_LIST
        | P::SUBORDINATE_ANNOTATIONS
        | P::SUBORDINATE_TAGS
        | P::SUBORDINATE_NODE_TYPES
        | P::SUBORDINATE_RELATIONSHIPS
        | P::DOOR_MEMBERS
        | P::ACCESS_DOORS
        | P::AUTHENTICATION_POLICY_LIST
        | P::AUTHENTICATION_POLICY_NAMES
        | P::POSITIVE_ACCESS_RULES
        | P::NEGATIVE_ACCESS_RULES
        | P::AUTHENTICATION_FACTORS
        | P::ASSIGNED_ACCESS_RIGHTS
        | P::SUPPORTED_FORMATS
        | P::SUPPORTED_FORMAT_CLASSES
        | P::BIT_TEXT
        | P::GROUP_MEMBERS
        | P::GROUP_MEMBER_NAMES
        | P::PORT_FILTER
        | P::EXECUTION_DELAY
        | P::CONTROL_GROUPS
        | P::IP_DNS_SERVER
        | P::IPV6_DNS_SERVER
        | P::LINK_SPEEDS
        | P::STATE_CHANGE_VALUES
        | P::FLOOR_TEXT
        | P::CAR_DOOR_TEXT
        | P::CAR_DOOR_STATUS
        | P::CAR_DOOR_COMMAND
        | P::LANDING_DOOR_STATUS
        | P::MAKING_CAR_CALL
        | P::REGISTERED_CAR_CALL
        | P::ASSIGNED_LANDING_CALLS
        | P::STAGES
        | P::STAGE_NAMES
        | P::TARGET_REFERENCES
        | P::MONITORED_OBJECTS => true,
        P::ACTION | P::ACTION_TEXT => object_type == ObjectType::COMMAND,
        P::ALARM_VALUES | P::FAULT_VALUES => matches!(
            object_type,
            ObjectType::CHARACTERSTRING_VALUE | ObjectType::BITSTRING_VALUE
        ),
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES => object_type == ObjectType::CHANNEL,
        P::LOG_DEVICE_OBJECT_PROPERTY => object_type == ObjectType::TREND_LOG_MULTIPLE,
        P::PRESENT_VALUE => object_type == ObjectType::GLOBAL_GROUP,
        _ => false,
    }
}

/// The default BACnetLIST classification behind
/// [`super::BACnetObject::is_list_property`]: every property that a Clause 12
/// object table of the 2020 standard types as a BACnetLIST, read by clients
/// through [`super::standard_list_property`]. Two identifier classes:
///
/// - **Identifier-stable BACnetLIST** properties are lists on every object
///   type that defines them: DATE_LIST (Table 12-11); the Device lists of
///   Table 12-13 (address bindings, the COV and COV-multiple subscriptions,
///   the VT classes and sessions, and the three recipient lists);
///   LIST_OF_GROUP_MEMBERS (Table 12-17); RECIPIENT_LIST (Tables 12-24 and
///   12-58); LOG_BUFFER (Tables 12-29, 12-31, 12-35 and 12-83); the
///   life-safety mode, alarm-value and zone-member lists (Tables 12-18 and
///   12-19); the access-control event, zone, user, credential and exemption
///   lists (Tables 12-30 and 12-36 to 12-40); COVU_RECIPIENTS (Table 12-57);
///   SUBSCRIBED_RECIPIENTS (Table 12-58); the B/IP, MS/TP and routing tables
///   of Network Port (Table 12-71); LANDING_CALLS (Table 12-76); and
///   FAULT_SIGNALS (Tables 12-77 and 12-78).
/// - **Type-dependent** identifiers classify by `object_type`: ALARM_VALUES /
///   FAULT_VALUES are lists except on CharacterString Value and BitString
///   Value, where they are arrays; LIST_OF_OBJECT_PROPERTY_REFERENCES is a list
///   except on Channel (an array); PRESENT_VALUE is a list only on Group
///   (Table 12-17); MEMBER_OF is a list on Life Safety Point, Life Safety Zone
///   and Access User but a single BACnetDeviceObjectReference on Audit Log.
///
/// Everything else is a scalar, a constructed single value or a BACnetARRAY.
/// The 2020 tables define no BACnetARRAY of BACnetLIST, so no identifier is
/// both an array and a list here.
#[inline]
pub(super) fn list_property_default(object_type: ObjectType, property: PropertyIdentifier) -> bool {
    match property {
        PropertyIdentifier::DATE_LIST
        | PropertyIdentifier::DEVICE_ADDRESS_BINDING
        | PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS
        | PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
        | PropertyIdentifier::VT_CLASSES_SUPPORTED
        | PropertyIdentifier::ACTIVE_VT_SESSIONS
        | PropertyIdentifier::TIME_SYNCHRONIZATION_RECIPIENTS
        | PropertyIdentifier::UTC_TIME_SYNCHRONIZATION_RECIPIENTS
        | PropertyIdentifier::RESTART_NOTIFICATION_RECIPIENTS
        | PropertyIdentifier::LIST_OF_GROUP_MEMBERS
        | PropertyIdentifier::RECIPIENT_LIST
        | PropertyIdentifier::LOG_BUFFER
        | PropertyIdentifier::ACCEPTED_MODES
        | PropertyIdentifier::LIFE_SAFETY_ALARM_VALUES
        | PropertyIdentifier::ZONE_MEMBERS
        | PropertyIdentifier::MASKED_ALARM_VALUES
        | PropertyIdentifier::FAILED_ATTEMPT_EVENTS
        | PropertyIdentifier::ACCESS_ALARM_EVENTS
        | PropertyIdentifier::ACCESS_TRANSACTION_EVENTS
        | PropertyIdentifier::CREDENTIALS_IN_ZONE
        | PropertyIdentifier::ENTRY_POINTS
        | PropertyIdentifier::EXIT_POINTS
        | PropertyIdentifier::MEMBERS
        | PropertyIdentifier::CREDENTIALS
        | PropertyIdentifier::REASON_FOR_DISABLE
        | PropertyIdentifier::AUTHORIZATION_EXEMPTIONS
        | PropertyIdentifier::COVU_RECIPIENTS
        | PropertyIdentifier::SUBSCRIBED_RECIPIENTS
        | PropertyIdentifier::BBMD_BROADCAST_DISTRIBUTION_TABLE
        | PropertyIdentifier::BBMD_FOREIGN_DEVICE_TABLE
        | PropertyIdentifier::MANUAL_SLAVE_ADDRESS_BINDING
        | PropertyIdentifier::SLAVE_ADDRESS_BINDING
        | PropertyIdentifier::VIRTUAL_MAC_ADDRESS_TABLE
        | PropertyIdentifier::ROUTING_TABLE
        | PropertyIdentifier::LANDING_CALLS
        | PropertyIdentifier::FAULT_SIGNALS => true,
        PropertyIdentifier::ALARM_VALUES | PropertyIdentifier::FAULT_VALUES => !matches!(
            object_type,
            ObjectType::CHARACTERSTRING_VALUE | ObjectType::BITSTRING_VALUE
        ),
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => {
            object_type != ObjectType::CHANNEL
        }
        PropertyIdentifier::PRESENT_VALUE => object_type == ObjectType::GROUP,
        PropertyIdentifier::MEMBER_OF => matches!(
            object_type,
            ObjectType::LIFE_SAFETY_POINT | ObjectType::LIFE_SAFETY_ZONE | ObjectType::ACCESS_USER
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_classification_follows_the_datatype_and_never_overlaps_arrays() {
        for object_type in (0..=63).map(ObjectType::from_raw) {
            for property in (0..=1023).map(PropertyIdentifier::from_raw) {
                assert!(
                    !(array_property_default(object_type, property)
                        && list_property_default(object_type, property)),
                    "{object_type:?} {property:?} is both an array and a list"
                );
            }
        }
        let cases = [
            (ObjectType::CALENDAR, PropertyIdentifier::DATE_LIST, true),
            (
                ObjectType::NOTIFICATION_CLASS,
                PropertyIdentifier::RECIPIENT_LIST,
                true,
            ),
            (
                ObjectType::ELEVATOR_GROUP,
                PropertyIdentifier::LANDING_CALLS,
                true,
            ),
            (
                ObjectType::MULTI_STATE_INPUT,
                PropertyIdentifier::ALARM_VALUES,
                true,
            ),
            (
                ObjectType::CHARACTERSTRING_VALUE,
                PropertyIdentifier::ALARM_VALUES,
                false,
            ),
            (
                ObjectType::BITSTRING_VALUE,
                PropertyIdentifier::ALARM_VALUES,
                false,
            ),
            (
                ObjectType::SCHEDULE,
                PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
                true,
            ),
            (
                ObjectType::CHANNEL,
                PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
                false,
            ),
            (ObjectType::GROUP, PropertyIdentifier::PRESENT_VALUE, true),
            (
                ObjectType::GLOBAL_GROUP,
                PropertyIdentifier::PRESENT_VALUE,
                false,
            ),
            (
                ObjectType::DATETIME_VALUE,
                PropertyIdentifier::PRESENT_VALUE,
                false,
            ),
            (
                ObjectType::LIFE_SAFETY_ZONE,
                PropertyIdentifier::MEMBER_OF,
                true,
            ),
            (ObjectType::AUDIT_LOG, PropertyIdentifier::MEMBER_OF, false),
            (
                ObjectType::ELEVATOR_GROUP,
                PropertyIdentifier::LANDING_CALL_CONTROL,
                false,
            ),
            (
                ObjectType::MULTI_STATE_VALUE,
                PropertyIdentifier::STATE_TEXT,
                false,
            ),
            (ObjectType::DEVICE, PropertyIdentifier::OBJECT_LIST, false),
            (ObjectType::DEVICE, PropertyIdentifier::PROPERTY_LIST, false),
        ];
        for (object_type, property, list) in cases {
            assert_eq!(
                list_property_default(object_type, property),
                list,
                "{object_type:?} {property:?}"
            );
        }
    }
}
