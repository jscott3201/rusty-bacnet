//! The standard value type of each property, by object type (#2177).
//!
//! Two tables. [`by_type`] holds the properties whose datatype or shape
//! depends on the object type: Present_Value, Relinquish_Default, the
//! limits and the type-dependent arrays and lists. [`by_identifier`] holds
//! every other property, whose datatype the Clause 12 tables give the same
//! on every object type that has it. A type-dependent identifier's entry
//! there is the shape its other types share, so a type without its own
//! entry still classifies it as array, list or single value.

use super::{Datatype as D, ValueType};
use crate::enums::{ObjectType as T, PropertyIdentifier as P};

const fn single(datatype: D) -> ValueType {
    ValueType::single(datatype)
}

const fn array(datatype: D) -> ValueType {
    ValueType::array(datatype)
}

const fn fixed(datatype: D, size: u16) -> ValueType {
    ValueType::fixed_array(datatype, size)
}

const fn list(datatype: D) -> ValueType {
    ValueType::list(datatype)
}

/// The value type the standard gives `property` on an object of
/// `object_type`, or `None` for a property it has no datatype for here
/// (a proprietary one, or one no object type of this stack's tables uses).
///
/// An object's metadata row resolves to this unless it states its own
/// (`bacnet_objects::property_metadata::PropertyMetadata::value_type`).
///
/// ```
/// use bacnet_types::datatype::{standard_value_type, Datatype};
/// use bacnet_types::enums::{ObjectType, PropertyIdentifier};
///
/// let priority_array =
///     standard_value_type(ObjectType::ANALOG_OUTPUT, PropertyIdentifier::PRIORITY_ARRAY)
///         .unwrap();
/// assert!(priority_array.is_array());
/// assert_eq!(priority_array.datatype, Datatype::PriorityValue);
/// let action = standard_value_type(ObjectType::LOOP, PropertyIdentifier::ACTION).unwrap();
/// assert!(!action.is_array());
/// ```
pub const fn standard_value_type(object_type: T, property: P) -> Option<ValueType> {
    match by_type(object_type, property) {
        Some(value_type) => Some(value_type),
        None => by_identifier(property),
    }
}

/// The properties whose value type depends on the object type.
const fn by_type(object_type: T, property: P) -> Option<ValueType> {
    let value_type = match (object_type, property) {
        (T::ANALOG_INPUT | T::LOOP | T::PULSE_CONVERTER | T::STAGING, P::PRESENT_VALUE)
        | (
            T::ANALOG_OUTPUT | T::ANALOG_VALUE | T::LIGHTING_OUTPUT,
            P::PRESENT_VALUE | P::RELINQUISH_DEFAULT,
        )
        | (T::LIGHTING_OUTPUT, P::FEEDBACK_VALUE | P::TRACKING_VALUE) => single(D::Real),
        (
            T::BINARY_INPUT | T::LIFE_SAFETY_POINT | T::LIFE_SAFETY_ZONE | T::LOAD_CONTROL,
            P::PRESENT_VALUE,
        )
        | (
            T::BINARY_OUTPUT | T::BINARY_VALUE | T::ACCESS_DOOR | T::BINARY_LIGHTING_OUTPUT,
            P::PRESENT_VALUE | P::RELINQUISH_DEFAULT,
        ) => single(D::Enumerated),
        (
            T::COMMAND | T::MULTI_STATE_INPUT | T::ACCUMULATOR | T::TIMER | T::COLOR_TEMPERATURE,
            P::PRESENT_VALUE,
        )
        | (
            T::MULTI_STATE_OUTPUT | T::MULTI_STATE_VALUE | T::POSITIVE_INTEGER_VALUE,
            P::PRESENT_VALUE | P::RELINQUISH_DEFAULT,
        )
        | (T::MULTI_STATE_OUTPUT, P::FEEDBACK_VALUE)
        | (T::COLOR_TEMPERATURE, P::TRACKING_VALUE)
        | (
            T::COLOR_TEMPERATURE,
            P::DEFAULT_RAMP_RATE | P::DEFAULT_STEP_INCREMENT | P::MIN_PRES_VALUE,
        )
        | (T::ACCUMULATOR | T::TIMER | T::COLOR_TEMPERATURE, P::MAX_PRES_VALUE)
        | (T::TIMER, P::MIN_PRES_VALUE)
        | (
            T::ACCUMULATOR,
            P::HIGH_LIMIT | P::LOW_LIMIT | P::FAULT_HIGH_LIMIT | P::FAULT_LOW_LIMIT,
        )
        | (
            T::POSITIVE_INTEGER_VALUE,
            P::COV_INCREMENT
            | P::HIGH_LIMIT
            | P::LOW_LIMIT
            | P::DEADBAND
            | P::FAULT_HIGH_LIMIT
            | P::FAULT_LOW_LIMIT
            | P::MIN_PRES_VALUE
            | P::MAX_PRES_VALUE,
        )
        | (T::INTEGER_VALUE, P::COV_INCREMENT | P::DEADBAND) => single(D::Unsigned),
        (
            T::INTEGER_VALUE,
            P::PRESENT_VALUE
            | P::RELINQUISH_DEFAULT
            | P::HIGH_LIMIT
            | P::LOW_LIMIT
            | P::FAULT_HIGH_LIMIT
            | P::FAULT_LOW_LIMIT
            | P::MIN_PRES_VALUE
            | P::MAX_PRES_VALUE,
        )
        | (T::ACCESS_ZONE, P::ADJUST_VALUE) => single(D::Integer),
        (
            T::LARGE_ANALOG_VALUE,
            P::PRESENT_VALUE
            | P::RELINQUISH_DEFAULT
            | P::COV_INCREMENT
            | P::HIGH_LIMIT
            | P::LOW_LIMIT
            | P::DEADBAND
            | P::FAULT_HIGH_LIMIT
            | P::FAULT_LOW_LIMIT
            | P::MIN_PRES_VALUE
            | P::MAX_PRES_VALUE,
        ) => single(D::Double),
        (T::CALENDAR, P::PRESENT_VALUE) => single(D::Boolean),
        (T::BITSTRING_VALUE, P::PRESENT_VALUE | P::RELINQUISH_DEFAULT) => single(D::BitString),
        (T::CHARACTERSTRING_VALUE, P::PRESENT_VALUE | P::RELINQUISH_DEFAULT) => {
            single(D::CharacterString)
        }
        (T::OCTETSTRING_VALUE, P::PRESENT_VALUE | P::RELINQUISH_DEFAULT) => single(D::OctetString),
        (T::DATEPATTERN_VALUE | T::DATE_VALUE, P::PRESENT_VALUE | P::RELINQUISH_DEFAULT) => {
            single(D::Date)
        }
        (T::TIMEPATTERN_VALUE | T::TIME_VALUE, P::PRESENT_VALUE | P::RELINQUISH_DEFAULT) => {
            single(D::Time)
        }
        (
            T::DATETIMEPATTERN_VALUE | T::DATETIME_VALUE,
            P::PRESENT_VALUE | P::RELINQUISH_DEFAULT,
        ) => single(D::DateTime),
        (T::SCHEDULE, P::PRESENT_VALUE | P::SCHEDULE_DEFAULT) => single(D::Any),
        (T::GROUP, P::PRESENT_VALUE) => list(D::ReadAccessResult),
        (T::GLOBAL_GROUP, P::PRESENT_VALUE) => array(D::PropertyAccessResult),
        (T::GLOBAL_GROUP, P::GROUP_MEMBERS) => array(D::DeviceObjectPropertyReference),
        (T::CREDENTIAL_DATA_INPUT, P::PRESENT_VALUE) => single(D::AuthenticationFactor),
        (T::CREDENTIAL_DATA_INPUT, P::UPDATE_TIME) => single(D::TimeStamp),
        (T::ALERT_ENROLLMENT, P::PRESENT_VALUE) => single(D::ObjectIdentifier),
        (T::CHANNEL, P::PRESENT_VALUE) => single(D::ChannelValue),
        (T::COLOR, P::PRESENT_VALUE | P::TRACKING_VALUE) => single(D::XyColor),
        // The reporter's own filter is the plain bit string; every other object
        // type takes the optional form by identifier below.
        (T::AUDIT_REPORTER, P::AUDIT_PRIORITY_FILTER) => single(D::BitString),
        (T::COMMAND, P::ACTION) => array(D::ActionList),
        (T::COMMAND, P::ACTION_TEXT) => array(D::CharacterString),
        (T::MULTI_STATE_INPUT | T::MULTI_STATE_VALUE, P::ALARM_VALUES | P::FAULT_VALUES) => {
            list(D::Unsigned)
        }
        (T::CHARACTERSTRING_VALUE, P::ALARM_VALUES | P::FAULT_VALUES) => {
            array(D::OptionalCharacterString)
        }
        (T::BITSTRING_VALUE, P::ALARM_VALUES) => array(D::BitString),
        (T::LIFE_SAFETY_POINT | T::LIFE_SAFETY_ZONE | T::ACCESS_USER, P::MEMBER_OF) => {
            list(D::DeviceObjectReference)
        }
        (T::EVENT_LOG, P::LOG_BUFFER) => list(D::EventLogRecord),
        (T::TREND_LOG_MULTIPLE, P::LOG_BUFFER) => list(D::LogMultipleRecord),
        (T::AUDIT_LOG, P::LOG_BUFFER) => list(D::AuditLogRecord),
        (T::CHANNEL, P::LIST_OF_OBJECT_PROPERTY_REFERENCES)
        | (T::TREND_LOG_MULTIPLE, P::LOG_DEVICE_OBJECT_PROPERTY) => {
            array(D::DeviceObjectPropertyReference)
        }
        (T::ANALOG_INPUT | T::ANALOG_OUTPUT, P::INTERFACE_VALUE) => single(D::OptionalReal),
        (T::MULTI_STATE_INPUT | T::MULTI_STATE_OUTPUT, P::INTERFACE_VALUE) => {
            single(D::OptionalUnsigned)
        }
        (T::LIFE_SAFETY_ZONE, P::MAINTENANCE_REQUIRED) => single(D::Boolean),
        _ => return None,
    };
    Some(value_type)
}

/// The properties whose value type is the same on every object type that
/// has them.
const fn by_identifier(property: P) -> Option<ValueType> {
    let value_type = match property {
        P::OBJECT_NAME
        | P::DESCRIPTION
        | P::LOCATION
        | P::DEVICE_TYPE
        | P::PROFILE_NAME
        | P::PROFILE_LOCATION
        | P::ACTIVE_TEXT
        | P::INACTIVE_TEXT
        | P::APPLICATION_SOFTWARE_VERSION
        | P::FIRMWARE_REVISION
        | P::MODEL_NAME
        | P::VENDOR_NAME
        | P::FILE_TYPE
        | P::NODE_SUBTYPE => single(D::CharacterString),
        P::OUT_OF_SERVICE
        | P::EVENT_DETECTION_ENABLE
        | P::EVENT_ALGORITHM_INHIBIT
        | P::RELIABILITY_EVALUATION_INHIBIT
        | P::ALIGN_INTERVALS
        | P::ALLOW_GROUP_DELAY_INHIBIT
        | P::ALL_WRITES_SUCCESSFUL
        | P::ARCHIVE
        | P::AUDIT_SOURCE_REPORTER
        | P::BLINK_WARN_ENABLE
        | P::CAR_DOOR_ZONE
        | P::CHANGES_PENDING
        | P::COLOR_OVERRIDE
        | P::DAYLIGHT_SAVINGS_STATUS
        | P::DELETE_ON_FORWARD
        | P::EGRESS_ACTIVE
        | P::IN_PROCESS
        | P::ISSUE_CONFIRMED_NOTIFICATIONS
        | P::LOCAL_FORWARDING_ONLY
        | P::ENABLE
        | P::OCCUPANCY_COUNT_ENABLE
        | P::PASSENGER_ALARM
        | P::POWER_MODE
        | P::READ_ONLY
        | P::SEND_NOW
        | P::STOP_WHEN_FULL
        | P::TIMER_RUNNING
        | P::TRIGGER
        | P::BBMD_ACCEPT_FD_REGISTRATIONS
        | P::BACNET_IP_NAT_TRAVERSAL => single(D::Boolean),
        P::APDU_LENGTH
        | P::APDU_SEGMENT_TIMEOUT
        | P::APDU_TIMEOUT
        | P::NUMBER_OF_APDU_RETRIES
        | P::MAX_APDU_LENGTH_ACCEPTED
        | P::MAX_SEGMENTS_ACCEPTED
        | P::PROTOCOL_VERSION
        | P::PROTOCOL_REVISION
        | P::DATABASE_REVISION
        | P::VENDOR_IDENTIFIER
        | P::NOTIFICATION_CLASS
        | P::TIME_DELAY
        | P::TIME_DELAY_NORMAL
        | P::NUMBER_OF_STATES
        | P::BUFFER_SIZE
        | P::RECORD_COUNT
        | P::TOTAL_RECORD_COUNT
        | P::LAST_NOTIFY_RECORD
        | P::NOTIFICATION_THRESHOLD
        | P::RECORDS_SINCE_NOTIFICATION
        | P::LOG_INTERVAL
        | P::INTERVAL_OFFSET
        | P::COV_RESUBSCRIPTION_INTERVAL
        | P::COV_PERIOD
        | P::FILE_SIZE
        | P::PRIORITY_FOR_WRITING
        | P::CHANNEL_NUMBER
        | P::GLOBAL_IDENTIFIER
        | P::GROUP_ID
        | P::INSTALLATION_ID
        | P::ACCESS_EVENT_TAG
        | P::ACTIVE_AUTHENTICATION_POLICY
        | P::NUMBER_OF_AUTHENTICATION_POLICIES
        | P::ATTEMPTED_SAMPLES
        | P::VALID_SAMPLES
        | P::WINDOW_INTERVAL
        | P::WINDOW_SAMPLES
        | P::UPDATE_INTERVAL
        | P::COUNT
        | P::COUNT_BEFORE_CHANGE
        | P::VALUE_BEFORE_CHANGE
        | P::VALUE_SET
        | P::PULSE_RATE
        | P::LIMIT_MONITORING_INTERVAL
        | P::PRESENT_STAGE
        | P::DEFAULT_COLOR_TEMPERATURE
        | P::DEFAULT_FADE_TIME
        | P::TRIM_FADE_TIME
        | P::DEFAULT_TIMEOUT
        | P::INITIAL_TIMEOUT
        | P::DOOR_EXTENDED_PULSE_TIME
        | P::DOOR_OPEN_TOO_LONG_TIME
        | P::DOOR_PULSE_TIME
        | P::EGRESS_TIME
        | P::CAR_POSITION
        | P::NEXT_STOPPING_FLOOR
        | P::LAST_PRIORITY
        | P::LIGHTING_COMMAND_DEFAULT_PRIORITY
        | P::OCCUPANCY_COUNT
        | P::OCCUPANCY_LOWER_LIMIT
        | P::OCCUPANCY_UPPER_LIMIT
        | P::SHED_DURATION
        | P::DUTY_WINDOW
        | P::MAXIMUM_SEND_DELAY
        | P::NETWORK_NUMBER
        | P::BACNET_IP_UDP_PORT
        | P::BACNET_IPV6_UDP_PORT
        | P::FD_SUBSCRIPTION_LIFETIME
        | P::IPV6_PREFIX_LENGTH
        | P::BACKUP_FAILURE_TIMEOUT => single(D::Unsigned),
        P::UTC_OFFSET => single(D::Integer),
        P::AVERAGE_VALUE
        | P::MAXIMUM_VALUE
        | P::MINIMUM_VALUE
        | P::CONTROLLED_VARIABLE_VALUE
        | P::SETPOINT
        | P::PROPORTIONAL_CONSTANT
        | P::INTEGRAL_CONSTANT
        | P::DERIVATIVE_CONSTANT
        | P::ENERGY_METER
        | P::CAR_LOAD
        | P::LINK_SPEED
        | P::SCALE_FACTOR
        | P::DIRECT_READING
        | P::HIGH_END_TRIM
        | P::LOW_END_TRIM
        | P::COV_INCREMENT
        | P::HIGH_LIMIT
        | P::LOW_LIMIT
        | P::DEADBAND
        | P::FAULT_HIGH_LIMIT
        | P::FAULT_LOW_LIMIT
        | P::MIN_PRES_VALUE
        | P::MAX_PRES_VALUE
        | P::DEFAULT_RAMP_RATE
        | P::DEFAULT_STEP_INCREMENT
        | P::ADJUST_VALUE => single(D::Real),
        P::OBJECT_TYPE
        | P::EVENT_STATE
        | P::RELIABILITY
        | P::NOTIFY_TYPE
        | P::UNITS
        | P::POLARITY
        | P::ALARM_VALUE
        | P::FEEDBACK_VALUE
        | P::TRACKING_VALUE
        | P::ACTION
        | P::SYSTEM_STATUS
        | P::SEGMENTATION_SUPPORTED
        | P::LAST_RESTART_REASON
        | P::BACKUP_AND_RESTORE_STATE
        | P::EVENT_TYPE
        | P::FAULT_TYPE
        | P::FILE_ACCESS_METHOD
        | P::PROGRAM_STATE
        | P::PROGRAM_CHANGE
        | P::REASON_FOR_HALT
        | P::OUTPUT_UNITS
        | P::CONTROLLED_VARIABLE_UNITS
        | P::PROPORTIONAL_CONSTANT_UNITS
        | P::INTEGRAL_CONSTANT_UNITS
        | P::DERIVATIVE_CONSTANT_UNITS
        | P::LOGGING_TYPE
        | P::MODE
        | P::SILENCED
        | P::OPERATION_EXPECTED
        | P::MAINTENANCE_REQUIRED
        | P::ACCESS_EVENT
        | P::AUTHENTICATION_STATUS
        | P::AUTHORIZATION_MODE
        | P::CREDENTIAL_DISABLE
        | P::CREDENTIAL_STATUS
        | P::USER_TYPE
        | P::OCCUPANCY_STATE
        | P::DOOR_STATUS
        | P::LOCK_STATUS
        | P::SECURED_STATUS
        | P::DOOR_ALARM_STATE
        | P::WRITE_STATUS
        | P::NODE_TYPE
        | P::TIMER_STATE
        | P::LAST_STATE_CHANGE
        | P::IN_PROGRESS
        | P::TRANSITION
        | P::NETWORK_TYPE
        | P::PROTOCOL_LEVEL
        | P::NETWORK_NUMBER_QUALITY
        | P::BACNET_IP_MODE
        | P::BACNET_IPV6_MODE
        | P::AUDIT_LEVEL
        | P::GROUP_MODE
        | P::CAR_ASSIGNED_DIRECTION
        | P::CAR_MODE
        | P::CAR_LOAD_UNITS
        | P::CAR_MOVING_DIRECTION
        | P::CAR_DRIVE_STATUS
        | P::ESCALATOR_MODE
        | P::OPERATION_DIRECTION => single(D::Enumerated),
        P::STATUS_FLAGS
        | P::EVENT_ENABLE
        | P::ACKED_TRANSITIONS
        | P::ACK_REQUIRED
        | P::LIMIT_ENABLE
        | P::PROTOCOL_OBJECT_TYPES_SUPPORTED
        | P::PROTOCOL_SERVICES_SUPPORTED
        | P::MEMBER_STATUS_FLAGS
        | P::AUDITABLE_OPERATIONS => single(D::BitString),
        P::DEVICE_UUID
        | P::MAC_ADDRESS
        | P::IP_ADDRESS
        | P::IP_SUBNET_MASK
        | P::IP_DEFAULT_GATEWAY
        | P::BACNET_IP_MULTICAST_ADDRESS
        | P::IPV6_ADDRESS
        | P::IPV6_DEFAULT_GATEWAY
        | P::BACNET_IPV6_MULTICAST_ADDRESS => single(D::OctetString),
        P::LOCAL_DATE => single(D::Date),
        P::LOCAL_TIME => single(D::Time),
        P::OBJECT_IDENTIFIER
        | P::ELEVATOR_GROUP
        | P::MACHINE_ROOM_ID
        | P::COLOR_REFERENCE
        | P::OVERRIDE_COLOR_REFERENCE => single(D::ObjectIdentifier),
        P::ACTIVATION_TIME
        | P::EXPIRATION_TIME
        | P::MODIFICATION_DATE
        | P::START_TIME
        | P::STOP_TIME
        | P::COUNT_CHANGE_TIME
        | P::UPDATE_TIME => single(D::DateTime),
        P::LAST_COMMAND_TIME
        | P::ACCESS_EVENT_TIME
        | P::LAST_RESTORE_TIME
        | P::TIME_OF_DEVICE_RESTART => single(D::TimeStamp),
        P::ACCESS_EVENT_CREDENTIAL | P::ENERGY_METER_REF | P::ACCOMPANIMENT | P::MEMBER_OF => {
            single(D::DeviceObjectReference)
        }
        P::EVENT_ALGORITHM_INHIBIT_REF
        | P::CONTROLLED_VARIABLE_REFERENCE
        | P::MANIPULATED_VARIABLE_REFERENCE
        | P::INPUT_REFERENCE => single(D::ObjectPropertyReference),
        P::OBJECT_PROPERTY_REFERENCE | P::LOG_DEVICE_OBJECT_PROPERTY => {
            single(D::DeviceObjectPropertyReference)
        }
        P::SETPOINT_REFERENCE => single(D::SetpointReference),
        P::EFFECTIVE_PERIOD => single(D::DateRange),
        P::EVENT_PARAMETERS => single(D::EventParameter),
        P::FAULT_PARAMETERS => single(D::FaultParameter),
        P::VALUE_SOURCE => single(D::ValueSource),
        P::LIGHTING_COMMAND => single(D::LightingCommand),
        P::COLOR_COMMAND => single(D::ColorCommand),
        P::DEFAULT_COLOR => single(D::XyColor),
        P::REQUESTED_SHED_LEVEL | P::EXPECTED_SHED_LEVEL | P::ACTUAL_SHED_LEVEL => {
            single(D::ShedLevel)
        }
        P::SCALE => single(D::Scale),
        P::PRESCALE => single(D::Prescale),
        P::ACCESS_EVENT_AUTHENTICATION_FACTOR => single(D::AuthenticationFactor),
        P::LANDING_CALL_CONTROL => single(D::LandingCallStatus),
        P::FD_BBMD_ADDRESS | P::BACNET_IP_GLOBAL_ADDRESS => single(D::HostNPort),
        P::AUDIT_NOTIFICATION_RECIPIENT => single(D::Recipient),
        P::CURRENT_COMMAND_PRIORITY => single(D::OptionalUnsigned),
        P::CLIENT_COV_INCREMENT => single(D::ClientCov),
        P::PROCESS_IDENTIFIER_FILTER => single(D::ProcessIdSelection),
        P::LOW_DIFF_LIMIT => single(D::OptionalReal),
        P::INTERFACE_VALUE => single(D::OptionalBinaryPv),
        P::AUDIT_PRIORITY_FILTER => single(D::OptionalPriorityFilter),
        // BACnetARRAYs.
        P::PROPERTY_LIST
        | P::SUBORDINATE_NODE_TYPES
        | P::SUBORDINATE_RELATIONSHIPS
        | P::CAR_DOOR_STATUS
        | P::CAR_DOOR_COMMAND => array(D::Enumerated),
        P::STATE_TEXT
        | P::SHED_LEVEL_DESCRIPTIONS
        | P::SUBORDINATE_ANNOTATIONS
        | P::AUTHENTICATION_POLICY_NAMES
        | P::BIT_TEXT
        | P::GROUP_MEMBER_NAMES
        | P::FLOOR_TEXT
        | P::CAR_DOOR_TEXT
        | P::STAGE_NAMES => array(D::CharacterString),
        P::OBJECT_LIST | P::STRUCTURED_OBJECT_LIST | P::CONFIGURATION_FILES | P::GROUP_MEMBERS => {
            array(D::ObjectIdentifier)
        }
        P::SHED_LEVELS
        | P::SUPPORTED_FORMAT_CLASSES
        | P::EXECUTION_DELAY
        | P::CONTROL_GROUPS
        | P::MAKING_CAR_CALL => array(D::Unsigned),
        P::SUBORDINATE_LIST | P::DOOR_MEMBERS | P::ACCESS_DOORS | P::TARGET_REFERENCES => {
            array(D::DeviceObjectReference)
        }
        P::IP_DNS_SERVER | P::IPV6_DNS_SERVER => array(D::OctetString),
        P::LINK_SPEEDS => array(D::Real),
        P::TAGS => array(D::NameValue),
        P::SUBORDINATE_TAGS => array(D::NameValueCollection),
        P::EXCEPTION_SCHEDULE => array(D::SpecialEvent),
        P::AUTHENTICATION_POLICY_LIST => array(D::AuthenticationPolicy),
        P::POSITIVE_ACCESS_RULES | P::NEGATIVE_ACCESS_RULES => array(D::AccessRule),
        P::AUTHENTICATION_FACTORS => array(D::CredentialAuthenticationFactor),
        P::ASSIGNED_ACCESS_RIGHTS => array(D::AssignedAccessRights),
        P::SUPPORTED_FORMATS => array(D::AuthenticationFactorFormat),
        P::PORT_FILTER => array(D::PortPermission),
        P::LANDING_DOOR_STATUS => array(D::LandingDoorStatus),
        P::REGISTERED_CAR_CALL => array(D::LiftCarCallList),
        P::ASSIGNED_LANDING_CALLS => array(D::AssignedLandingCalls),
        P::STAGES => array(D::StageLimitValue),
        P::MONITORED_OBJECTS => array(D::ObjectSelector),
        P::EVENT_TIME_STAMPS => fixed(D::TimeStamp, 3),
        P::EVENT_MESSAGE_TEXTS | P::EVENT_MESSAGE_TEXTS_CONFIG => fixed(D::CharacterString, 3),
        P::PRIORITY => fixed(D::Unsigned, 3),
        P::WEEKLY_SCHEDULE => fixed(D::DailySchedule, 7),
        P::STATE_CHANGE_VALUES => fixed(D::TimerStateChangeValue, 7),
        P::PRIORITY_ARRAY => fixed(D::PriorityValue, 16),
        P::COMMAND_TIME_ARRAY => fixed(D::TimeStamp, 16),
        P::VALUE_SOURCE_ARRAY => fixed(D::ValueSource, 16),
        // BACnetLISTs.
        P::ALARM_VALUES
        | P::FAULT_VALUES
        | P::VT_CLASSES_SUPPORTED
        | P::ACCEPTED_MODES
        | P::LIFE_SAFETY_ALARM_VALUES
        | P::MASKED_ALARM_VALUES
        | P::FAILED_ATTEMPT_EVENTS
        | P::ACCESS_ALARM_EVENTS
        | P::ACCESS_TRANSACTION_EVENTS
        | P::REASON_FOR_DISABLE
        | P::AUTHORIZATION_EXEMPTIONS
        | P::FAULT_SIGNALS => list(D::Enumerated),
        P::ZONE_MEMBERS
        | P::CREDENTIALS_IN_ZONE
        | P::ENTRY_POINTS
        | P::EXIT_POINTS
        | P::MEMBERS
        | P::CREDENTIALS => list(D::DeviceObjectReference),
        P::DEVICE_ADDRESS_BINDING | P::MANUAL_SLAVE_ADDRESS_BINDING | P::SLAVE_ADDRESS_BINDING => {
            list(D::AddressBinding)
        }
        P::TIME_SYNCHRONIZATION_RECIPIENTS
        | P::UTC_TIME_SYNCHRONIZATION_RECIPIENTS
        | P::RESTART_NOTIFICATION_RECIPIENTS
        | P::COVU_RECIPIENTS => list(D::Recipient),
        P::ACTIVE_COV_SUBSCRIPTIONS => list(D::CovSubscription),
        P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS => list(D::CovMultipleSubscription),
        P::ACTIVE_VT_SESSIONS => list(D::VtSession),
        P::LIST_OF_GROUP_MEMBERS => list(D::ReadAccessSpecification),
        P::RECIPIENT_LIST => list(D::Destination),
        P::SUBSCRIBED_RECIPIENTS => list(D::EventNotificationSubscription),
        P::DATE_LIST => list(D::CalendarEntry),
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES => list(D::DeviceObjectPropertyReference),
        P::LOG_BUFFER => list(D::LogRecord),
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE => list(D::BdtEntry),
        P::BBMD_FOREIGN_DEVICE_TABLE => list(D::FdtEntry),
        P::VIRTUAL_MAC_ADDRESS_TABLE => list(D::VmacEntry),
        P::ROUTING_TABLE => list(D::RouterEntry),
        P::LANDING_CALLS => list(D::LandingCallStatus),
        _ => return None,
    };
    Some(value_type)
}

#[cfg(test)]
mod audit_filter_tests;
#[cfg(test)]
mod xd_allowlist;
#[cfg(test)]
mod xd_crosscheck;
