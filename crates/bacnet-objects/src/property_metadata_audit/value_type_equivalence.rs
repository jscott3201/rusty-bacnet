//! Before/after table test for #2177: the array and list classification
//! that `traits::defaults` held before the metadata rows carried value
//! types, frozen here, against [`standard_value_type`], which every
//! built-in row resolves to. Equal over every (object type, property) pair,
//! so every RP/RPM/WP/WPM array-index gate and list check answers as before,
//! for properties an object has and for those it lacks.

use crate::property_metadata::standard_value_type;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

#[test]
fn standard_shapes_match_the_old_classification_on_every_pair() {
    let mut differences = Vec::new();
    for object_type in (0..=1023).map(ObjectType::from_raw) {
        for property in (0..=1023).map(PropertyIdentifier::from_raw).chain(
            PropertyIdentifier::ALL_NAMED
                .iter()
                .map(|&(_, property)| property)
                .filter(|property| property.to_raw() > 1023),
        ) {
            let value_type = standard_value_type(object_type, property);
            let array = value_type.is_some_and(|v| v.is_array());
            let list = value_type.is_some_and(|v| v.is_list());
            if array != old_array(object_type, property) || list != old_list(object_type, property)
            {
                differences.push(format!("{object_type:?} {property:?}: {value_type:?}"));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

fn old_array(object_type: ObjectType, property: PropertyIdentifier) -> bool {
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
        P::ALARM_VALUES => matches!(
            object_type,
            ObjectType::CHARACTERSTRING_VALUE | ObjectType::BITSTRING_VALUE
        ),
        // BitString Value has no Fault_Values, so it no longer has an entry
        // of its own (#2465).
        P::FAULT_VALUES => object_type == ObjectType::CHARACTERSTRING_VALUE,
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES => object_type == ObjectType::CHANNEL,
        P::LOG_DEVICE_OBJECT_PROPERTY => object_type == ObjectType::TREND_LOG_MULTIPLE,
        P::PRESENT_VALUE => object_type == ObjectType::GLOBAL_GROUP,
        _ => false,
    }
}

fn old_list(object_type: ObjectType, property: PropertyIdentifier) -> bool {
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
        PropertyIdentifier::ALARM_VALUES => !matches!(
            object_type,
            ObjectType::CHARACTERSTRING_VALUE | ObjectType::BITSTRING_VALUE
        ),
        PropertyIdentifier::FAULT_VALUES => object_type != ObjectType::CHARACTERSTRING_VALUE,
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
