"""Type stubs for rusty_bacnet — Python bindings for the BACnet protocol stack (ASHRAE 135-2020).

Generated from the actual PyO3 source code. Enum classes expose all standard constants
as class attributes; vendor-proprietary values are available via ``from_raw()``.
"""

from __future__ import annotations

from typing import Any, Literal, NotRequired, Optional, TypedDict, Union


# ---------------------------------------------------------------------------
# Enum types
# ---------------------------------------------------------------------------

class ObjectType:
    """BACnet object type enumeration (Clause 12).

    Standard types 0-64; vendor-proprietary 128-1023.
    Use ``ObjectType.from_raw(n)`` for values not listed here.
    """

    # Standard types (0-64)
    ANALOG_INPUT: ObjectType
    ANALOG_OUTPUT: ObjectType
    ANALOG_VALUE: ObjectType
    BINARY_INPUT: ObjectType
    BINARY_OUTPUT: ObjectType
    BINARY_VALUE: ObjectType
    CALENDAR: ObjectType
    COMMAND: ObjectType
    DEVICE: ObjectType
    EVENT_ENROLLMENT: ObjectType
    FILE: ObjectType
    GROUP: ObjectType
    LOOP: ObjectType
    MULTI_STATE_INPUT: ObjectType
    MULTI_STATE_OUTPUT: ObjectType
    NOTIFICATION_CLASS: ObjectType
    PROGRAM: ObjectType
    SCHEDULE: ObjectType
    AVERAGING: ObjectType
    MULTI_STATE_VALUE: ObjectType
    TREND_LOG: ObjectType
    LIFE_SAFETY_POINT: ObjectType
    LIFE_SAFETY_ZONE: ObjectType
    ACCUMULATOR: ObjectType
    PULSE_CONVERTER: ObjectType
    EVENT_LOG: ObjectType
    GLOBAL_GROUP: ObjectType
    TREND_LOG_MULTIPLE: ObjectType
    LOAD_CONTROL: ObjectType
    STRUCTURED_VIEW: ObjectType
    ACCESS_DOOR: ObjectType
    TIMER: ObjectType
    ACCESS_CREDENTIAL: ObjectType
    ACCESS_POINT: ObjectType
    ACCESS_RIGHTS: ObjectType
    ACCESS_USER: ObjectType
    ACCESS_ZONE: ObjectType
    CREDENTIAL_DATA_INPUT: ObjectType
    NETWORK_SECURITY: ObjectType
    BITSTRING_VALUE: ObjectType
    CHARACTERSTRING_VALUE: ObjectType
    DATEPATTERN_VALUE: ObjectType
    DATE_VALUE: ObjectType
    DATETIMEPATTERN_VALUE: ObjectType
    DATETIME_VALUE: ObjectType
    INTEGER_VALUE: ObjectType
    LARGE_ANALOG_VALUE: ObjectType
    OCTETSTRING_VALUE: ObjectType
    POSITIVE_INTEGER_VALUE: ObjectType
    TIMEPATTERN_VALUE: ObjectType
    TIME_VALUE: ObjectType
    NOTIFICATION_FORWARDER: ObjectType
    ALERT_ENROLLMENT: ObjectType
    CHANNEL: ObjectType
    LIGHTING_OUTPUT: ObjectType
    BINARY_LIGHTING_OUTPUT: ObjectType
    NETWORK_PORT: ObjectType
    ELEVATOR_GROUP: ObjectType
    ESCALATOR: ObjectType
    LIFT: ObjectType
    STAGING: ObjectType
    AUDIT_REPORTER: ObjectType
    AUDIT_LOG: ObjectType
    COLOR: ObjectType
    COLOR_TEMPERATURE: ObjectType

    @staticmethod
    def from_raw(value: int) -> ObjectType: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class PropertyIdentifier:
    """BACnet property identifier enumeration (Clause 12).

    All named properties registered at runtime are listed below as class attributes
    matching the BACnet constant names.
    Use ``PropertyIdentifier.from_raw(n)`` for vendor-proprietary values.
    """

    # Common properties
    ACKED_TRANSITIONS: PropertyIdentifier
    ACK_REQUIRED: PropertyIdentifier
    ACTION: PropertyIdentifier
    ACTION_TEXT: PropertyIdentifier
    ACTIVE_TEXT: PropertyIdentifier
    ACTIVE_VT_SESSIONS: PropertyIdentifier
    ALARM_VALUE: PropertyIdentifier
    ALARM_VALUES: PropertyIdentifier
    ALL: PropertyIdentifier
    ALL_WRITES_SUCCESSFUL: PropertyIdentifier
    APDU_SEGMENT_TIMEOUT: PropertyIdentifier
    APDU_TIMEOUT: PropertyIdentifier
    APPLICATION_SOFTWARE_VERSION: PropertyIdentifier
    CHANGE_OF_STATE_COUNT: PropertyIdentifier
    CHANGE_OF_STATE_TIME: PropertyIdentifier
    NOTIFICATION_CLASS: PropertyIdentifier
    CONTROLLED_VARIABLE_REFERENCE: PropertyIdentifier
    COV_INCREMENT: PropertyIdentifier
    DATE_LIST: PropertyIdentifier
    DEADBAND: PropertyIdentifier
    DESCRIPTION: PropertyIdentifier
    DEVICE_ADDRESS_BINDING: PropertyIdentifier
    DEVICE_TYPE: PropertyIdentifier
    EFFECTIVE_PERIOD: PropertyIdentifier
    EVENT_ENABLE: PropertyIdentifier
    EVENT_STATE: PropertyIdentifier
    EVENT_TYPE: PropertyIdentifier
    EXCEPTION_SCHEDULE: PropertyIdentifier
    FEEDBACK_VALUE: PropertyIdentifier
    FILE_ACCESS_METHOD: PropertyIdentifier
    FILE_SIZE: PropertyIdentifier
    FILE_TYPE: PropertyIdentifier
    FIRMWARE_REVISION: PropertyIdentifier
    HIGH_LIMIT: PropertyIdentifier
    INACTIVE_TEXT: PropertyIdentifier
    IN_PROCESS: PropertyIdentifier
    LIMIT_ENABLE: PropertyIdentifier
    LIST_OF_GROUP_MEMBERS: PropertyIdentifier
    LIST_OF_OBJECT_PROPERTY_REFERENCES: PropertyIdentifier
    LOCAL_DATE: PropertyIdentifier
    LOCAL_TIME: PropertyIdentifier
    LOCATION: PropertyIdentifier
    LOW_LIMIT: PropertyIdentifier
    MAX_APDU_LENGTH_ACCEPTED: PropertyIdentifier
    MAX_INFO_FRAMES: PropertyIdentifier
    MAX_MASTER: PropertyIdentifier
    MAX_PRES_VALUE: PropertyIdentifier
    MIN_PRES_VALUE: PropertyIdentifier
    MODEL_NAME: PropertyIdentifier
    NOTIFY_TYPE: PropertyIdentifier
    NUMBER_OF_APDU_RETRIES: PropertyIdentifier
    NUMBER_OF_STATES: PropertyIdentifier
    OBJECT_IDENTIFIER: PropertyIdentifier
    OBJECT_LIST: PropertyIdentifier
    OBJECT_NAME: PropertyIdentifier
    OBJECT_PROPERTY_REFERENCE: PropertyIdentifier
    OBJECT_TYPE: PropertyIdentifier
    OUT_OF_SERVICE: PropertyIdentifier
    OUTPUT_UNITS: PropertyIdentifier
    EVENT_PARAMETERS: PropertyIdentifier
    POLARITY: PropertyIdentifier
    PRESENT_VALUE: PropertyIdentifier
    PRIORITY: PropertyIdentifier
    PRIORITY_ARRAY: PropertyIdentifier
    PRIORITY_FOR_WRITING: PropertyIdentifier
    PROCESS_IDENTIFIER: PropertyIdentifier
    PROGRAM_CHANGE: PropertyIdentifier
    PROGRAM_STATE: PropertyIdentifier
    PROTOCOL_OBJECT_TYPES_SUPPORTED: PropertyIdentifier
    PROTOCOL_SERVICES_SUPPORTED: PropertyIdentifier
    PROTOCOL_VERSION: PropertyIdentifier
    RECIPIENT_LIST: PropertyIdentifier
    RELIABILITY: PropertyIdentifier
    RELINQUISH_DEFAULT: PropertyIdentifier
    RESOLUTION: PropertyIdentifier
    SEGMENTATION_SUPPORTED: PropertyIdentifier
    SETPOINT: PropertyIdentifier
    STATE_TEXT: PropertyIdentifier
    STATUS_FLAGS: PropertyIdentifier
    SYSTEM_STATUS: PropertyIdentifier
    TIME_DELAY: PropertyIdentifier
    UNITS: PropertyIdentifier
    UPDATE_INTERVAL: PropertyIdentifier
    UTC_OFFSET: PropertyIdentifier
    VENDOR_IDENTIFIER: PropertyIdentifier
    VENDOR_NAME: PropertyIdentifier
    WEEKLY_SCHEDULE: PropertyIdentifier
    BUFFER_SIZE: PropertyIdentifier
    COV_RESUBSCRIPTION_INTERVAL: PropertyIdentifier
    EVENT_TIME_STAMPS: PropertyIdentifier
    EVENT_MESSAGE_TEXTS: PropertyIdentifier
    EVENT_MESSAGE_TEXTS_CONFIG: PropertyIdentifier
    LOG_BUFFER: PropertyIdentifier
    LOG_DEVICE_OBJECT_PROPERTY: PropertyIdentifier
    LOG_ENABLE: PropertyIdentifier
    LOG_INTERVAL: PropertyIdentifier
    PROTOCOL_REVISION: PropertyIdentifier
    RECORD_COUNT: PropertyIdentifier
    START_TIME: PropertyIdentifier
    STOP_TIME: PropertyIdentifier
    STOP_WHEN_FULL: PropertyIdentifier
    TOTAL_RECORD_COUNT: PropertyIdentifier
    ACTIVE_COV_SUBSCRIPTIONS: PropertyIdentifier
    DATABASE_REVISION: PropertyIdentifier
    MAINTENANCE_REQUIRED: PropertyIdentifier
    MEMBER_OF: PropertyIdentifier
    MODE: PropertyIdentifier
    SILENCED: PropertyIdentifier
    TRACKING_VALUE: PropertyIdentifier
    ZONE_MEMBERS: PropertyIdentifier
    LIFE_SAFETY_ALARM_VALUES: PropertyIdentifier
    MAX_SEGMENTS_ACCEPTED: PropertyIdentifier
    PROFILE_NAME: PropertyIdentifier
    SCHEDULE_DEFAULT: PropertyIdentifier
    LOGGING_OBJECT: PropertyIdentifier
    LOGGING_TYPE: PropertyIdentifier
    ALIGN_INTERVALS: PropertyIdentifier
    INTERVAL_OFFSET: PropertyIdentifier
    LAST_RESTART_REASON: PropertyIdentifier
    TIME_OF_DEVICE_RESTART: PropertyIdentifier
    TIME_SYNCHRONIZATION_INTERVAL: PropertyIdentifier
    UTC_TIME_SYNCHRONIZATION_RECIPIENTS: PropertyIdentifier
    NODE_SUBTYPE: PropertyIdentifier
    NODE_TYPE: PropertyIdentifier
    STRUCTURED_OBJECT_LIST: PropertyIdentifier
    SUBORDINATE_LIST: PropertyIdentifier
    ACTUAL_SHED_LEVEL: PropertyIdentifier
    REQUESTED_SHED_LEVEL: PropertyIdentifier
    PROPERTY_LIST: PropertyIdentifier
    EVENT_DETECTION_ENABLE: PropertyIdentifier
    EVENT_ALGORITHM_INHIBIT: PropertyIdentifier
    EVENT_ALGORITHM_INHIBIT_REF: PropertyIdentifier
    TIME_DELAY_NORMAL: PropertyIdentifier
    RELIABILITY_EVALUATION_INHIBIT: PropertyIdentifier
    FAULT_TYPE: PropertyIdentifier
    CHANNEL_NUMBER: PropertyIdentifier
    CONTROL_GROUPS: PropertyIdentifier
    EXECUTION_DELAY: PropertyIdentifier
    NETWORK_NUMBER: PropertyIdentifier
    NETWORK_TYPE: PropertyIdentifier
    MAC_ADDRESS: PropertyIdentifier
    COMMAND_TIME_ARRAY: PropertyIdentifier
    CURRENT_COMMAND_PRIORITY: PropertyIdentifier
    LAST_COMMAND_TIME: PropertyIdentifier
    VALUE_SOURCE: PropertyIdentifier
    VALUE_SOURCE_ARRAY: PropertyIdentifier
    BACNET_IPV6_MODE: PropertyIdentifier
    TAGS: PropertyIdentifier
    PRESENT_STAGE: PropertyIdentifier
    STAGES: PropertyIdentifier
    STAGE_NAMES: PropertyIdentifier
    AUDIT_LEVEL: PropertyIdentifier
    DEVICE_UUID: PropertyIdentifier

    # Remaining registered properties
    ARCHIVE: PropertyIdentifier
    BIAS: PropertyIdentifier
    CONTROLLED_VARIABLE_UNITS: PropertyIdentifier
    CONTROLLED_VARIABLE_VALUE: PropertyIdentifier
    DAYLIGHT_SAVINGS_STATUS: PropertyIdentifier
    DERIVATIVE_CONSTANT: PropertyIdentifier
    DERIVATIVE_CONSTANT_UNITS: PropertyIdentifier
    DESCRIPTION_OF_HALT: PropertyIdentifier
    ELAPSED_ACTIVE_TIME: PropertyIdentifier
    ERROR_LIMIT: PropertyIdentifier
    FAULT_VALUES: PropertyIdentifier
    INSTANCE_OF: PropertyIdentifier
    INTEGRAL_CONSTANT: PropertyIdentifier
    INTEGRAL_CONSTANT_UNITS: PropertyIdentifier
    ISSUE_CONFIRMED_NOTIFICATIONS: PropertyIdentifier
    MANIPULATED_VARIABLE_REFERENCE: PropertyIdentifier
    MAXIMUM_OUTPUT: PropertyIdentifier
    MINIMUM_OFF_TIME: PropertyIdentifier
    MINIMUM_ON_TIME: PropertyIdentifier
    MINIMUM_OUTPUT: PropertyIdentifier
    MODIFICATION_DATE: PropertyIdentifier
    OPTIONAL: PropertyIdentifier
    PROGRAM_LOCATION: PropertyIdentifier
    PROPORTIONAL_CONSTANT: PropertyIdentifier
    PROPORTIONAL_CONSTANT_UNITS: PropertyIdentifier
    READ_ONLY: PropertyIdentifier
    REASON_FOR_HALT: PropertyIdentifier
    REQUIRED: PropertyIdentifier
    SETPOINT_REFERENCE: PropertyIdentifier
    TIME_OF_ACTIVE_TIME_RESET: PropertyIdentifier
    TIME_OF_STATE_COUNT_RESET: PropertyIdentifier
    TIME_SYNCHRONIZATION_RECIPIENTS: PropertyIdentifier
    VT_CLASSES_SUPPORTED: PropertyIdentifier
    ATTEMPTED_SAMPLES: PropertyIdentifier
    AVERAGE_VALUE: PropertyIdentifier
    CLIENT_COV_INCREMENT: PropertyIdentifier
    MAXIMUM_VALUE: PropertyIdentifier
    MINIMUM_VALUE: PropertyIdentifier
    NOTIFICATION_THRESHOLD: PropertyIdentifier
    RECORDS_SINCE_NOTIFICATION: PropertyIdentifier
    VALID_SAMPLES: PropertyIdentifier
    WINDOW_INTERVAL: PropertyIdentifier
    WINDOW_SAMPLES: PropertyIdentifier
    MAXIMUM_VALUE_TIMESTAMP: PropertyIdentifier
    MINIMUM_VALUE_TIMESTAMP: PropertyIdentifier
    VARIANCE_VALUE: PropertyIdentifier
    BACKUP_FAILURE_TIMEOUT: PropertyIdentifier
    CONFIGURATION_FILES: PropertyIdentifier
    DIRECT_READING: PropertyIdentifier
    LAST_RESTORE_TIME: PropertyIdentifier
    OPERATION_EXPECTED: PropertyIdentifier
    SETTING: PropertyIdentifier
    AUTO_SLAVE_DISCOVERY: PropertyIdentifier
    MANUAL_SLAVE_ADDRESS_BINDING: PropertyIdentifier
    SLAVE_ADDRESS_BINDING: PropertyIdentifier
    SLAVE_PROXY_ENABLE: PropertyIdentifier
    LAST_NOTIFY_RECORD: PropertyIdentifier
    ACCEPTED_MODES: PropertyIdentifier
    ADJUST_VALUE: PropertyIdentifier
    COUNT: PropertyIdentifier
    COUNT_BEFORE_CHANGE: PropertyIdentifier
    COUNT_CHANGE_TIME: PropertyIdentifier
    COV_PERIOD: PropertyIdentifier
    INPUT_REFERENCE: PropertyIdentifier
    LIMIT_MONITORING_INTERVAL: PropertyIdentifier
    LOGGING_RECORD: PropertyIdentifier
    PRESCALE: PropertyIdentifier
    PULSE_RATE: PropertyIdentifier
    SCALE: PropertyIdentifier
    SCALE_FACTOR: PropertyIdentifier
    UPDATE_TIME: PropertyIdentifier
    VALUE_BEFORE_CHANGE: PropertyIdentifier
    VALUE_SET: PropertyIdentifier
    VALUE_CHANGE_TIME: PropertyIdentifier
    RESTART_NOTIFICATION_RECIPIENTS: PropertyIdentifier
    TRIGGER: PropertyIdentifier
    SUBORDINATE_ANNOTATIONS: PropertyIdentifier
    DUTY_WINDOW: PropertyIdentifier
    EXPECTED_SHED_LEVEL: PropertyIdentifier
    FULL_DUTY_BASELINE: PropertyIdentifier
    SHED_DURATION: PropertyIdentifier
    SHED_LEVEL_DESCRIPTIONS: PropertyIdentifier
    SHED_LEVELS: PropertyIdentifier
    STATE_DESCRIPTION: PropertyIdentifier
    DOOR_ALARM_STATE: PropertyIdentifier
    DOOR_EXTENDED_PULSE_TIME: PropertyIdentifier
    DOOR_MEMBERS: PropertyIdentifier
    DOOR_OPEN_TOO_LONG_TIME: PropertyIdentifier
    DOOR_PULSE_TIME: PropertyIdentifier
    DOOR_STATUS: PropertyIdentifier
    DOOR_UNLOCK_DELAY_TIME: PropertyIdentifier
    LOCK_STATUS: PropertyIdentifier
    MASKED_ALARM_VALUES: PropertyIdentifier
    SECURED_STATUS: PropertyIdentifier
    ABSENTEE_LIMIT: PropertyIdentifier
    ACCESS_ALARM_EVENTS: PropertyIdentifier
    ACCESS_DOORS: PropertyIdentifier
    ACCESS_EVENT: PropertyIdentifier
    ACCESS_EVENT_AUTHENTICATION_FACTOR: PropertyIdentifier
    ACCESS_EVENT_CREDENTIAL: PropertyIdentifier
    ACCESS_EVENT_TIME: PropertyIdentifier
    ACCESS_TRANSACTION_EVENTS: PropertyIdentifier
    ACCOMPANIMENT: PropertyIdentifier
    ACCOMPANIMENT_TIME: PropertyIdentifier
    ACTIVATION_TIME: PropertyIdentifier
    ACTIVE_AUTHENTICATION_POLICY: PropertyIdentifier
    ASSIGNED_ACCESS_RIGHTS: PropertyIdentifier
    AUTHENTICATION_FACTORS: PropertyIdentifier
    AUTHENTICATION_POLICY_LIST: PropertyIdentifier
    AUTHENTICATION_POLICY_NAMES: PropertyIdentifier
    AUTHENTICATION_STATUS: PropertyIdentifier
    AUTHORIZATION_MODE: PropertyIdentifier
    BELONGS_TO: PropertyIdentifier
    CREDENTIAL_DISABLE: PropertyIdentifier
    CREDENTIAL_STATUS: PropertyIdentifier
    CREDENTIALS: PropertyIdentifier
    CREDENTIALS_IN_ZONE: PropertyIdentifier
    DAYS_REMAINING: PropertyIdentifier
    ENTRY_POINTS: PropertyIdentifier
    EXIT_POINTS: PropertyIdentifier
    EXPIRATION_TIME: PropertyIdentifier
    EXTENDED_TIME_ENABLE: PropertyIdentifier
    FAILED_ATTEMPT_EVENTS: PropertyIdentifier
    FAILED_ATTEMPTS: PropertyIdentifier
    FAILED_ATTEMPTS_TIME: PropertyIdentifier
    LAST_ACCESS_EVENT: PropertyIdentifier
    LAST_ACCESS_POINT: PropertyIdentifier
    LAST_CREDENTIAL_ADDED: PropertyIdentifier
    LAST_CREDENTIAL_ADDED_TIME: PropertyIdentifier
    LAST_CREDENTIAL_REMOVED: PropertyIdentifier
    LAST_CREDENTIAL_REMOVED_TIME: PropertyIdentifier
    LAST_USE_TIME: PropertyIdentifier
    LOCKOUT: PropertyIdentifier
    LOCKOUT_RELINQUISH_TIME: PropertyIdentifier
    MAX_FAILED_ATTEMPTS: PropertyIdentifier
    MEMBERS: PropertyIdentifier
    MUSTER_POINT: PropertyIdentifier
    NEGATIVE_ACCESS_RULES: PropertyIdentifier
    NUMBER_OF_AUTHENTICATION_POLICIES: PropertyIdentifier
    OCCUPANCY_COUNT: PropertyIdentifier
    OCCUPANCY_COUNT_ADJUST: PropertyIdentifier
    OCCUPANCY_COUNT_ENABLE: PropertyIdentifier
    OCCUPANCY_LOWER_LIMIT: PropertyIdentifier
    OCCUPANCY_LOWER_LIMIT_ENFORCED: PropertyIdentifier
    OCCUPANCY_STATE: PropertyIdentifier
    OCCUPANCY_UPPER_LIMIT: PropertyIdentifier
    OCCUPANCY_UPPER_LIMIT_ENFORCED: PropertyIdentifier
    PASSBACK_MODE: PropertyIdentifier
    PASSBACK_TIMEOUT: PropertyIdentifier
    POSITIVE_ACCESS_RULES: PropertyIdentifier
    REASON_FOR_DISABLE: PropertyIdentifier
    SUPPORTED_FORMATS: PropertyIdentifier
    SUPPORTED_FORMAT_CLASSES: PropertyIdentifier
    THREAT_AUTHORITY: PropertyIdentifier
    THREAT_LEVEL: PropertyIdentifier
    TRACE_FLAG: PropertyIdentifier
    TRANSACTION_NOTIFICATION_CLASS: PropertyIdentifier
    USER_EXTERNAL_IDENTIFIER: PropertyIdentifier
    USER_INFORMATION_REFERENCE: PropertyIdentifier
    USER_NAME: PropertyIdentifier
    USER_TYPE: PropertyIdentifier
    USES_REMAINING: PropertyIdentifier
    ZONE_FROM: PropertyIdentifier
    ZONE_TO: PropertyIdentifier
    ACCESS_EVENT_TAG: PropertyIdentifier
    GLOBAL_IDENTIFIER: PropertyIdentifier
    VERIFICATION_TIME: PropertyIdentifier
    BASE_DEVICE_SECURITY_POLICY: PropertyIdentifier
    DISTRIBUTION_KEY_REVISION: PropertyIdentifier
    DO_NOT_HIDE: PropertyIdentifier
    KEY_SETS: PropertyIdentifier
    LAST_KEY_SERVER: PropertyIdentifier
    NETWORK_ACCESS_SECURITY_POLICIES: PropertyIdentifier
    PACKET_REORDER_TIME: PropertyIdentifier
    SECURITY_PDU_TIMEOUT: PropertyIdentifier
    SECURITY_TIME_WINDOW: PropertyIdentifier
    SUPPORTED_SECURITY_ALGORITHMS: PropertyIdentifier
    UPDATE_KEY_SET_TIMEOUT: PropertyIdentifier
    BACKUP_AND_RESTORE_STATE: PropertyIdentifier
    BACKUP_PREPARATION_TIME: PropertyIdentifier
    RESTORE_COMPLETION_TIME: PropertyIdentifier
    RESTORE_PREPARATION_TIME: PropertyIdentifier
    BIT_MASK: PropertyIdentifier
    BIT_TEXT: PropertyIdentifier
    IS_UTC: PropertyIdentifier
    GROUP_MEMBERS: PropertyIdentifier
    GROUP_MEMBER_NAMES: PropertyIdentifier
    MEMBER_STATUS_FLAGS: PropertyIdentifier
    REQUESTED_UPDATE_INTERVAL: PropertyIdentifier
    COVU_PERIOD: PropertyIdentifier
    COVU_RECIPIENTS: PropertyIdentifier
    FAULT_PARAMETERS: PropertyIdentifier
    LOCAL_FORWARDING_ONLY: PropertyIdentifier
    PROCESS_IDENTIFIER_FILTER: PropertyIdentifier
    SUBSCRIBED_RECIPIENTS: PropertyIdentifier
    PORT_FILTER: PropertyIdentifier
    AUTHORIZATION_EXEMPTIONS: PropertyIdentifier
    ALLOW_GROUP_DELAY_INHIBIT: PropertyIdentifier
    LAST_PRIORITY: PropertyIdentifier
    WRITE_STATUS: PropertyIdentifier
    SERIAL_NUMBER: PropertyIdentifier
    BLINK_WARN_ENABLE: PropertyIdentifier
    DEFAULT_FADE_TIME: PropertyIdentifier
    DEFAULT_RAMP_RATE: PropertyIdentifier
    DEFAULT_STEP_INCREMENT: PropertyIdentifier
    EGRESS_TIME: PropertyIdentifier
    IN_PROGRESS: PropertyIdentifier
    INSTANTANEOUS_POWER: PropertyIdentifier
    LIGHTING_COMMAND: PropertyIdentifier
    LIGHTING_COMMAND_DEFAULT_PRIORITY: PropertyIdentifier
    MAX_ACTUAL_VALUE: PropertyIdentifier
    MIN_ACTUAL_VALUE: PropertyIdentifier
    POWER: PropertyIdentifier
    TRANSITION: PropertyIdentifier
    EGRESS_ACTIVE: PropertyIdentifier
    INTERFACE_VALUE: PropertyIdentifier
    FAULT_HIGH_LIMIT: PropertyIdentifier
    FAULT_LOW_LIMIT: PropertyIdentifier
    LOW_DIFF_LIMIT: PropertyIdentifier
    STRIKE_COUNT: PropertyIdentifier
    TIME_OF_STRIKE_COUNT_RESET: PropertyIdentifier
    DEFAULT_TIMEOUT: PropertyIdentifier
    INITIAL_TIMEOUT: PropertyIdentifier
    LAST_STATE_CHANGE: PropertyIdentifier
    STATE_CHANGE_VALUES: PropertyIdentifier
    TIMER_RUNNING: PropertyIdentifier
    TIMER_STATE: PropertyIdentifier
    APDU_LENGTH: PropertyIdentifier
    IP_ADDRESS: PropertyIdentifier
    IP_DEFAULT_GATEWAY: PropertyIdentifier
    IP_DHCP_ENABLE: PropertyIdentifier
    IP_DHCP_LEASE_TIME: PropertyIdentifier
    IP_DHCP_LEASE_TIME_REMAINING: PropertyIdentifier
    IP_DHCP_SERVER: PropertyIdentifier
    IP_DNS_SERVER: PropertyIdentifier
    BACNET_IP_GLOBAL_ADDRESS: PropertyIdentifier
    BACNET_IP_MODE: PropertyIdentifier
    BACNET_IP_MULTICAST_ADDRESS: PropertyIdentifier
    BACNET_IP_NAT_TRAVERSAL: PropertyIdentifier
    IP_SUBNET_MASK: PropertyIdentifier
    BACNET_IP_UDP_PORT: PropertyIdentifier
    BBMD_ACCEPT_FD_REGISTRATIONS: PropertyIdentifier
    BBMD_BROADCAST_DISTRIBUTION_TABLE: PropertyIdentifier
    BBMD_FOREIGN_DEVICE_TABLE: PropertyIdentifier
    CHANGES_PENDING: PropertyIdentifier
    COMMAND_NP: PropertyIdentifier
    FD_BBMD_ADDRESS: PropertyIdentifier
    FD_SUBSCRIPTION_LIFETIME: PropertyIdentifier
    LINK_SPEED: PropertyIdentifier
    LINK_SPEEDS: PropertyIdentifier
    LINK_SPEED_AUTONEGOTIATE: PropertyIdentifier
    NETWORK_INTERFACE_NAME: PropertyIdentifier
    NETWORK_NUMBER_QUALITY: PropertyIdentifier
    ROUTING_TABLE: PropertyIdentifier
    VIRTUAL_MAC_ADDRESS_TABLE: PropertyIdentifier
    IPV6_ADDRESS: PropertyIdentifier
    IPV6_PREFIX_LENGTH: PropertyIdentifier
    BACNET_IPV6_UDP_PORT: PropertyIdentifier
    IPV6_DEFAULT_GATEWAY: PropertyIdentifier
    BACNET_IPV6_MULTICAST_ADDRESS: PropertyIdentifier
    IPV6_DNS_SERVER: PropertyIdentifier
    IPV6_AUTO_ADDRESSING_ENABLE: PropertyIdentifier
    IPV6_DHCP_LEASE_TIME: PropertyIdentifier
    IPV6_DHCP_LEASE_TIME_REMAINING: PropertyIdentifier
    IPV6_DHCP_SERVER: PropertyIdentifier
    IPV6_ZONE_INDEX: PropertyIdentifier
    ASSIGNED_LANDING_CALLS: PropertyIdentifier
    CAR_ASSIGNED_DIRECTION: PropertyIdentifier
    CAR_DOOR_COMMAND: PropertyIdentifier
    CAR_DOOR_STATUS: PropertyIdentifier
    CAR_DOOR_TEXT: PropertyIdentifier
    CAR_DOOR_ZONE: PropertyIdentifier
    CAR_DRIVE_STATUS: PropertyIdentifier
    CAR_LOAD: PropertyIdentifier
    CAR_LOAD_UNITS: PropertyIdentifier
    CAR_MODE: PropertyIdentifier
    CAR_MOVING_DIRECTION: PropertyIdentifier
    CAR_POSITION: PropertyIdentifier
    ELEVATOR_GROUP: PropertyIdentifier
    ENERGY_METER: PropertyIdentifier
    ENERGY_METER_REF: PropertyIdentifier
    ESCALATOR_MODE: PropertyIdentifier
    FAULT_SIGNALS: PropertyIdentifier
    FLOOR_TEXT: PropertyIdentifier
    GROUP_ID: PropertyIdentifier
    GROUP_MODE: PropertyIdentifier
    HIGHER_DECK: PropertyIdentifier
    INSTALLATION_ID: PropertyIdentifier
    LANDING_CALLS: PropertyIdentifier
    LANDING_CALL_CONTROL: PropertyIdentifier
    LANDING_DOOR_STATUS: PropertyIdentifier
    LOWER_DECK: PropertyIdentifier
    MACHINE_ROOM_ID: PropertyIdentifier
    MAKING_CAR_CALL: PropertyIdentifier
    NEXT_STOPPING_FLOOR: PropertyIdentifier
    OPERATION_DIRECTION: PropertyIdentifier
    PASSENGER_ALARM: PropertyIdentifier
    POWER_MODE: PropertyIdentifier
    REGISTERED_CAR_CALL: PropertyIdentifier
    ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS: PropertyIdentifier
    PROTOCOL_LEVEL: PropertyIdentifier
    REFERENCE_PORT: PropertyIdentifier
    DEPLOYED_PROFILE_LOCATION: PropertyIdentifier
    PROFILE_LOCATION: PropertyIdentifier
    SUBORDINATE_NODE_TYPES: PropertyIdentifier
    SUBORDINATE_TAGS: PropertyIdentifier
    SUBORDINATE_RELATIONSHIPS: PropertyIdentifier
    DEFAULT_SUBORDINATE_RELATIONSHIP: PropertyIdentifier
    REPRESENTS: PropertyIdentifier
    DEFAULT_PRESENT_VALUE: PropertyIdentifier
    TARGET_REFERENCES: PropertyIdentifier
    AUDIT_SOURCE_REPORTER: PropertyIdentifier
    AUDIT_NOTIFICATION_RECIPIENT: PropertyIdentifier
    AUDIT_PRIORITY_FILTER: PropertyIdentifier
    AUDITABLE_OPERATIONS: PropertyIdentifier
    DELETE_ON_FORWARD: PropertyIdentifier
    MAXIMUM_SEND_DELAY: PropertyIdentifier
    MONITORED_OBJECTS: PropertyIdentifier
    SEND_NOW: PropertyIdentifier
    FLOOR_NUMBER: PropertyIdentifier
    COLOR_COMMAND: PropertyIdentifier
    DEFAULT_COLOR_TEMPERATURE: PropertyIdentifier
    DEFAULT_COLOR: PropertyIdentifier

    @staticmethod
    def from_raw(value: int) -> PropertyIdentifier: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class ErrorClass:
    """BACnet error class enumeration (Clause 18.1.1)."""

    DEVICE: ErrorClass
    OBJECT: ErrorClass
    PROPERTY: ErrorClass
    RESOURCES: ErrorClass
    SECURITY: ErrorClass
    SERVICES: ErrorClass
    VT: ErrorClass
    COMMUNICATION: ErrorClass

    @staticmethod
    def from_raw(value: int) -> ErrorClass: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class ErrorCode:
    """BACnet error code enumeration (Clause 18).

    All 151 registered standard codes are available as class attributes
    (value 33 removed).
    Use ``ErrorCode.from_raw(n)`` for vendor-proprietary codes.
    """

    OTHER: ErrorCode
    AUTHENTICATION_FAILED: ErrorCode
    CONFIGURATION_IN_PROGRESS: ErrorCode
    DEVICE_BUSY: ErrorCode
    DYNAMIC_CREATION_NOT_SUPPORTED: ErrorCode
    FILE_ACCESS_DENIED: ErrorCode
    INCOMPATIBLE_SECURITY_LEVELS: ErrorCode
    INCONSISTENT_PARAMETERS: ErrorCode
    INCONSISTENT_SELECTION_CRITERION: ErrorCode
    INVALID_DATA_TYPE: ErrorCode
    INVALID_FILE_ACCESS_METHOD: ErrorCode
    INVALID_FILE_START_POSITION: ErrorCode
    INVALID_OPERATOR_NAME: ErrorCode
    INVALID_PARAMETER_DATA_TYPE: ErrorCode
    INVALID_TIME_STAMP: ErrorCode
    KEY_GENERATION_ERROR: ErrorCode
    MISSING_REQUIRED_PARAMETER: ErrorCode
    NO_OBJECTS_OF_SPECIFIED_TYPE: ErrorCode
    NO_SPACE_FOR_OBJECT: ErrorCode
    NO_SPACE_TO_ADD_LIST_ELEMENT: ErrorCode
    NO_SPACE_TO_WRITE_PROPERTY: ErrorCode
    NO_VT_SESSIONS_AVAILABLE: ErrorCode
    PROPERTY_IS_NOT_A_LIST: ErrorCode
    OBJECT_DELETION_NOT_PERMITTED: ErrorCode
    OBJECT_IDENTIFIER_ALREADY_EXISTS: ErrorCode
    OPERATIONAL_PROBLEM: ErrorCode
    PASSWORD_FAILURE: ErrorCode
    READ_ACCESS_DENIED: ErrorCode
    SECURITY_NOT_SUPPORTED: ErrorCode
    SERVICE_REQUEST_DENIED: ErrorCode
    TIMEOUT: ErrorCode
    UNKNOWN_OBJECT: ErrorCode
    UNKNOWN_PROPERTY: ErrorCode
    UNKNOWN_VT_CLASS: ErrorCode
    UNKNOWN_VT_SESSION: ErrorCode
    UNSUPPORTED_OBJECT_TYPE: ErrorCode
    VALUE_OUT_OF_RANGE: ErrorCode
    VT_SESSION_ALREADY_CLOSED: ErrorCode
    VT_SESSION_TERMINATION_FAILURE: ErrorCode
    WRITE_ACCESS_DENIED: ErrorCode
    CHARACTER_SET_NOT_SUPPORTED: ErrorCode
    INVALID_ARRAY_INDEX: ErrorCode
    COV_SUBSCRIPTION_FAILED: ErrorCode
    NOT_COV_PROPERTY: ErrorCode
    OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED: ErrorCode
    INVALID_CONFIGURATION_DATA: ErrorCode
    DATATYPE_NOT_SUPPORTED: ErrorCode
    DUPLICATE_NAME: ErrorCode
    DUPLICATE_OBJECT_ID: ErrorCode
    PROPERTY_IS_NOT_AN_ARRAY: ErrorCode
    ABORT_BUFFER_OVERFLOW: ErrorCode
    ABORT_INVALID_APDU_IN_THIS_STATE: ErrorCode
    ABORT_PREEMPTED_BY_HIGHER_PRIORITY_TASK: ErrorCode
    ABORT_SEGMENTATION_NOT_SUPPORTED: ErrorCode
    ABORT_PROPRIETARY: ErrorCode
    ABORT_OTHER: ErrorCode
    INVALID_TAG: ErrorCode
    NETWORK_DOWN: ErrorCode
    REJECT_BUFFER_OVERFLOW: ErrorCode
    REJECT_INCONSISTENT_PARAMETERS: ErrorCode
    REJECT_INVALID_PARAMETER_DATA_TYPE: ErrorCode
    REJECT_INVALID_TAG: ErrorCode
    REJECT_MISSING_REQUIRED_PARAMETER: ErrorCode
    REJECT_PARAMETER_OUT_OF_RANGE: ErrorCode
    REJECT_TOO_MANY_ARGUMENTS: ErrorCode
    REJECT_UNDEFINED_ENUMERATION: ErrorCode
    REJECT_UNRECOGNIZED_SERVICE: ErrorCode
    REJECT_PROPRIETARY: ErrorCode
    REJECT_OTHER: ErrorCode
    UNKNOWN_DEVICE: ErrorCode
    UNKNOWN_ROUTE: ErrorCode
    VALUE_NOT_INITIALIZED: ErrorCode
    INVALID_EVENT_STATE: ErrorCode
    NO_ALARM_CONFIGURED: ErrorCode
    LOG_BUFFER_FULL: ErrorCode
    LOGGED_VALUE_PURGED: ErrorCode
    NO_PROPERTY_SPECIFIED: ErrorCode
    NOT_CONFIGURED_FOR_TRIGGERED_LOGGING: ErrorCode
    UNKNOWN_SUBSCRIPTION: ErrorCode
    PARAMETER_OUT_OF_RANGE: ErrorCode
    LIST_ELEMENT_NOT_FOUND: ErrorCode
    BUSY: ErrorCode
    COMMUNICATION_DISABLED: ErrorCode
    SUCCESS: ErrorCode
    ACCESS_DENIED: ErrorCode
    BAD_DESTINATION_ADDRESS: ErrorCode
    BAD_DESTINATION_DEVICE_ID: ErrorCode
    BAD_SIGNATURE: ErrorCode
    BAD_SOURCE_ADDRESS: ErrorCode
    BAD_TIMESTAMP: ErrorCode
    CANNOT_USE_KEY: ErrorCode
    CANNOT_VERIFY_MESSAGE_ID: ErrorCode
    CORRECT_KEY_REVISION: ErrorCode
    DESTINATION_DEVICE_ID_REQUIRED: ErrorCode
    DUPLICATE_MESSAGE: ErrorCode
    ENCRYPTION_NOT_CONFIGURED: ErrorCode
    ENCRYPTION_REQUIRED: ErrorCode
    INCORRECT_KEY: ErrorCode
    INVALID_KEY_DATA: ErrorCode
    KEY_UPDATE_IN_PROGRESS: ErrorCode
    MALFORMED_MESSAGE: ErrorCode
    NOT_KEY_SERVER: ErrorCode
    SECURITY_NOT_CONFIGURED: ErrorCode
    SOURCE_SECURITY_REQUIRED: ErrorCode
    TOO_MANY_KEYS: ErrorCode
    UNKNOWN_AUTHENTICATION_TYPE: ErrorCode
    UNKNOWN_KEY: ErrorCode
    UNKNOWN_KEY_REVISION: ErrorCode
    UNKNOWN_SOURCE_MESSAGE: ErrorCode
    NOT_ROUTER_TO_DNET: ErrorCode
    ROUTER_BUSY: ErrorCode
    UNKNOWN_NETWORK_MESSAGE: ErrorCode
    MESSAGE_TOO_LONG: ErrorCode
    SECURITY_ERROR: ErrorCode
    ADDRESSING_ERROR: ErrorCode
    WRITE_BDT_FAILED: ErrorCode
    READ_BDT_FAILED: ErrorCode
    REGISTER_FOREIGN_DEVICE_FAILED: ErrorCode
    READ_FDT_FAILED: ErrorCode
    DELETE_FDT_ENTRY_FAILED: ErrorCode
    DISTRIBUTE_BROADCAST_FAILED: ErrorCode
    UNKNOWN_FILE_SIZE: ErrorCode
    ABORT_APDU_TOO_LONG: ErrorCode
    ABORT_APPLICATION_EXCEEDED_REPLY_TIME: ErrorCode
    ABORT_OUT_OF_RESOURCES: ErrorCode
    ABORT_TSM_TIMEOUT: ErrorCode
    ABORT_WINDOW_SIZE_OUT_OF_RANGE: ErrorCode
    FILE_FULL: ErrorCode
    INCONSISTENT_CONFIGURATION: ErrorCode
    INCONSISTENT_OBJECT_TYPE: ErrorCode
    INTERNAL_ERROR: ErrorCode
    NOT_CONFIGURED: ErrorCode
    OUT_OF_MEMORY: ErrorCode
    VALUE_TOO_LONG: ErrorCode
    ABORT_INSUFFICIENT_SECURITY: ErrorCode
    ABORT_SECURITY_ERROR: ErrorCode
    DUPLICATE_ENTRY: ErrorCode
    INVALID_VALUE_IN_THIS_STATE: ErrorCode
    INVALID_OPERATION_IN_THIS_STATE: ErrorCode
    LIST_ITEM_NOT_NUMBERED: ErrorCode
    LIST_ITEM_NOT_TIMESTAMPED: ErrorCode
    INVALID_DATA_ENCODING: ErrorCode
    BVLC_FUNCTION_UNKNOWN: ErrorCode
    BVLC_PROPRIETARY_FUNCTION_UNKNOWN: ErrorCode
    HEADER_ENCODING_ERROR: ErrorCode
    HEADER_NOT_UNDERSTOOD: ErrorCode
    MESSAGE_INCOMPLETE: ErrorCode
    NOT_A_BACNET_SC_HUB: ErrorCode
    PAYLOAD_EXPECTED: ErrorCode
    UNEXPECTED_DATA: ErrorCode
    NODE_DUPLICATE_VMAC: ErrorCode

    @staticmethod
    def from_raw(value: int) -> ErrorCode: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class AuditOperation:
    """BACnet audit operation.

    Typed Audit request mappings accept standard values 0..15 and proprietary
    values 32..63. ``from_raw()`` remains a lossless enum-wrapper constructor;
    reserved or wider values are rejected when used in a request mapping.
    """

    READ: AuditOperation
    WRITE: AuditOperation
    CREATE: AuditOperation
    DELETE: AuditOperation
    LIFE_SAFETY: AuditOperation
    ACKNOWLEDGE_ALARM: AuditOperation
    DEVICE_DISABLE_COMM: AuditOperation
    DEVICE_ENABLE_COMM: AuditOperation
    DEVICE_RESET: AuditOperation
    DEVICE_BACKUP: AuditOperation
    DEVICE_RESTORE: AuditOperation
    SUBSCRIPTION: AuditOperation
    NOTIFICATION: AuditOperation
    AUDITING_FAILURE: AuditOperation
    NETWORK_CHANGES: AuditOperation
    GENERAL: AuditOperation

    @staticmethod
    def from_raw(value: int) -> AuditOperation: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class EnableDisable:
    """BACnet DeviceCommunicationControl enable/disable options (Clause 16.4)."""

    ENABLE: EnableDisable
    DISABLE: EnableDisable
    DISABLE_INITIATION: EnableDisable

    @staticmethod
    def from_raw(value: int) -> EnableDisable: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class ReinitializedState:
    """BACnet ReinitializeDevice state options (Clause 16.5)."""

    COLDSTART: ReinitializedState
    WARMSTART: ReinitializedState
    START_BACKUP: ReinitializedState
    END_BACKUP: ReinitializedState
    START_RESTORE: ReinitializedState
    END_RESTORE: ReinitializedState
    ABORT_RESTORE: ReinitializedState
    ACTIVATE_CHANGES: ReinitializedState

    @staticmethod
    def from_raw(value: int) -> ReinitializedState: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class Segmentation:
    """BACnet segmentation support options (Clause 20.1.2.4)."""

    BOTH: Segmentation
    TRANSMIT: Segmentation
    RECEIVE: Segmentation
    NONE: Segmentation

    @staticmethod
    def from_raw(value: int) -> Segmentation: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class EventState:
    """BACnet event state enumeration (Clause 12)."""

    NORMAL: EventState
    FAULT: EventState
    OFFNORMAL: EventState
    HIGH_LIMIT: EventState
    LOW_LIMIT: EventState
    LIFE_SAFETY_ALARM: EventState

    @staticmethod
    def from_raw(value: int) -> EventState: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class EnrollmentSummaryEventStateFilter:
    """GetEnrollmentSummary event-state filter (Clause 13.11.1.1)."""

    OFFNORMAL: EnrollmentSummaryEventStateFilter
    FAULT: EnrollmentSummaryEventStateFilter
    NORMAL: EnrollmentSummaryEventStateFilter
    ALL: EnrollmentSummaryEventStateFilter
    ACTIVE: EnrollmentSummaryEventStateFilter

    @staticmethod
    def from_raw(value: int) -> EnrollmentSummaryEventStateFilter: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class EventType:
    """BACnet event type enumeration (Clause 12.12.6)."""

    CHANGE_OF_BITSTRING: EventType
    CHANGE_OF_STATE: EventType
    CHANGE_OF_VALUE: EventType
    COMMAND_FAILURE: EventType
    FLOATING_LIMIT: EventType
    OUT_OF_RANGE: EventType
    CHANGE_OF_LIFE_SAFETY: EventType
    EXTENDED: EventType
    BUFFER_READY: EventType
    UNSIGNED_RANGE: EventType
    ACCESS_EVENT: EventType
    DOUBLE_OUT_OF_RANGE: EventType
    SIGNED_OUT_OF_RANGE: EventType
    UNSIGNED_OUT_OF_RANGE: EventType
    CHANGE_OF_CHARACTERSTRING: EventType
    CHANGE_OF_STATUS_FLAGS: EventType
    CHANGE_OF_RELIABILITY: EventType
    NONE: EventType
    CHANGE_OF_DISCRETE_VALUE: EventType
    CHANGE_OF_TIMER: EventType

    @staticmethod
    def from_raw(value: int) -> EventType: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class MessagePriority:
    """BACnet text message priority (Clause 16.5)."""

    NORMAL: MessagePriority
    URGENT: MessagePriority

    @staticmethod
    def from_raw(value: int) -> MessagePriority: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class LifeSafetyOperation:
    """BACnet life safety operation codes (Clause 12.15.13, Table 12-54)."""

    NONE: LifeSafetyOperation
    SILENCE: LifeSafetyOperation
    SILENCE_AUDIBLE: LifeSafetyOperation
    SILENCE_VISUAL: LifeSafetyOperation
    RESET: LifeSafetyOperation
    RESET_ALARM: LifeSafetyOperation
    RESET_FAULT: LifeSafetyOperation
    UNSILENCE: LifeSafetyOperation
    UNSILENCE_AUDIBLE: LifeSafetyOperation
    UNSILENCE_VISUAL: LifeSafetyOperation

    @staticmethod
    def from_raw(value: int) -> LifeSafetyOperation: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


# ---------------------------------------------------------------------------
# Core types
# ---------------------------------------------------------------------------

class ObjectIdentifier:
    """BACnet Object Identifier (type + instance number)."""

    def __init__(self, object_type: ObjectType, instance: int) -> None: ...

    @property
    def object_type(self) -> ObjectType: ...

    @property
    def instance(self) -> int: ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class BACnetTimeStamp:
    """Lossless BACnetTimeStamp CHOICE.

    Time components accept their normal BACnet ranges or 255 for unspecified.
    Date accepts full years 1900..2154 or 255 for unspecified, months 1..14,
    days 1..34, and days-of-week 1..7; each non-year date field also accepts
    255 for unspecified. Supplied values are never normalized.
    """

    @staticmethod
    def sequence_number(value: int) -> BACnetTimeStamp:
        """Construct the Sequence Number CHOICE with a value in 0..65535."""
        ...
    @staticmethod
    def time(
        hour: int, minute: int, second: int, hundredths: int
    ) -> BACnetTimeStamp:
        """Construct the Time CHOICE."""
        ...
    @staticmethod
    def date_time(
        date: tuple[int, int, int, int],
        time: tuple[int, int, int, int],
    ) -> BACnetTimeStamp:
        """Construct DateTime from ``(full_year, month, day, day_of_week)`` and Time tuples."""
        ...

    @property
    def kind(self) -> str:
        """Selected CHOICE: ``sequence_number``, ``time``, or ``date_time``."""
        ...

    @property
    def value(
        self,
    ) -> int | tuple[int, int, int, int] | tuple[
        tuple[int, int, int, int], tuple[int, int, int, int]
    ]:
        """Exact selected value, using a full year for the Date tuple."""
        ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...


class AuditRecipientDevice(TypedDict):
    """Audit recipient selected by Device object identifier."""

    kind: Literal["device"]
    object_identifier: ObjectIdentifier


class AuditRecipientAddress(TypedDict):
    """Audit recipient selected by BACnet network and MAC address."""

    kind: Literal["address"]
    network_number: int
    mac_address: bytes


AuditRecipientInput = AuditRecipientDevice | AuditRecipientAddress


class AuditPropertyReference(TypedDict):
    property_identifier: PropertyIdentifier
    property_array_index: NotRequired[int | None]


class AuditNotificationInput(TypedDict):
    source_device: AuditRecipientInput
    operation: AuditOperation
    target_device: AuditRecipientInput
    source_timestamp: NotRequired[BACnetTimeStamp | None]
    target_timestamp: NotRequired[BACnetTimeStamp | None]
    source_object: NotRequired[ObjectIdentifier | None]
    source_comment: NotRequired[str | None]
    target_comment: NotRequired[str | None]
    invoke_id: NotRequired[int | None]
    source_user_id: NotRequired[int | None]
    source_user_role: NotRequired[int | None]
    target_object: NotRequired[ObjectIdentifier | None]
    target_property: NotRequired[AuditPropertyReference | None]
    target_priority: NotRequired[int | None]
    target_value: NotRequired[bytes | None]
    current_value: NotRequired[bytes | None]
    result: NotRequired[tuple[ErrorClass, ErrorCode] | None]


class AuditNotificationRequestInput(TypedDict):
    notifications: list[AuditNotificationInput]


class AuditLogQueryByTargetInput(TypedDict):
    kind: Literal["by_target"]
    target_device_identifier: ObjectIdentifier
    # Corrected BACnetSuccessFilter (RB-02/RB-20): 0 = all, 1 = successes-only,
    # 2 = failures-only. The pre-RB-02 Boolean is rejected with TypeError.
    successful_actions_only: Literal[0, 1, 2]
    target_device_address: NotRequired[AuditRecipientAddress | None]
    target_object_identifier: NotRequired[ObjectIdentifier | None]
    target_property_identifier: NotRequired[PropertyIdentifier | None]
    target_array_index: NotRequired[int | None]
    target_priority: NotRequired[int | None]
    operations: NotRequired[int | None]


class AuditLogQueryBySourceInput(TypedDict):
    kind: Literal["by_source"]
    source_device_identifier: ObjectIdentifier
    # Corrected BACnetSuccessFilter (RB-02/RB-20): 0 = all, 1 = successes-only,
    # 2 = failures-only. The pre-RB-02 Boolean is rejected with TypeError.
    successful_actions_only: Literal[0, 1, 2]
    source_device_address: NotRequired[AuditRecipientAddress | None]
    source_object_identifier: NotRequired[ObjectIdentifier | None]
    operations: NotRequired[int | None]


AuditLogQueryParametersInput = AuditLogQueryByTargetInput | AuditLogQueryBySourceInput


class AuditLogQueryRequestInput(TypedDict):
    audit_log: ObjectIdentifier
    query_parameters: AuditLogQueryParametersInput
    # Unsigned16 requested count (0..=65535).
    requested_count: int
    # Optional corrected Unsigned64 cursor (0..=2**64-1; RB-20).
    start_at_sequence_number: NotRequired[int | None]


BACnetDateTime = tuple[
    tuple[int, int, int, int],
    tuple[int, int, int, int],
]


class AuditPropertyReferenceResult(TypedDict):
    property_identifier: PropertyIdentifier
    property_array_index: int | None


class AuditNotification(TypedDict):
    """Canonical return mapping for a decoded Audit notification."""

    source_timestamp: BACnetTimeStamp | None
    target_timestamp: BACnetTimeStamp | None
    source_device: AuditRecipientInput
    source_object: ObjectIdentifier | None
    operation: AuditOperation
    source_comment: str | None
    target_comment: str | None
    invoke_id: int | None
    source_user_id: int | None
    source_user_role: int | None
    target_device: AuditRecipientInput
    target_object: ObjectIdentifier | None
    target_property: AuditPropertyReferenceResult | None
    target_priority: int | None
    target_value: bytes | None
    current_value: bytes | None
    result: tuple[ErrorClass, ErrorCode] | None


class AuditLogStatusDatum(TypedDict):
    kind: Literal["log_status"]
    log_status: int


class AuditNotificationDatum(TypedDict):
    kind: Literal["audit_notification"]
    audit_notification: AuditNotification


class AuditTimeChangeDatum(TypedDict):
    kind: Literal["time_change"]
    time_change: float


AuditLogDatum = AuditLogStatusDatum | AuditNotificationDatum | AuditTimeChangeDatum


class AuditLogRecord(TypedDict):
    timestamp: BACnetDateTime
    datum: AuditLogDatum


class AuditLogRecordResult(TypedDict):
    sequence_number: int
    record: AuditLogRecord


class AuditLogQueryAck(TypedDict):
    audit_log: ObjectIdentifier
    records: list[AuditLogRecordResult]
    no_more_items: bool


class PropertyValue:
    """A decoded BACnet property value with tag and Python-native value.

    Create values using the static constructors::

        PropertyValue.null()
        PropertyValue.boolean(True)
        PropertyValue.unsigned(42)
        PropertyValue.real(3.14)
        PropertyValue.character_string("hello")
        PropertyValue.object_identifier(oid)
    """

    @staticmethod
    def null() -> PropertyValue: ...
    @staticmethod
    def boolean(value: bool) -> PropertyValue: ...
    @staticmethod
    def unsigned(value: int) -> PropertyValue: ...
    @staticmethod
    def signed(value: int) -> PropertyValue: ...
    @staticmethod
    def real(value: float) -> PropertyValue: ...
    @staticmethod
    def double(value: float) -> PropertyValue: ...
    @staticmethod
    def character_string(value: str) -> PropertyValue: ...
    @staticmethod
    def octet_string(value: bytes) -> PropertyValue: ...
    @staticmethod
    def enumerated(value: int) -> PropertyValue: ...
    @staticmethod
    def object_identifier(oid: ObjectIdentifier) -> PropertyValue: ...
    @staticmethod
    def date(year: int, month: int, day: int, day_of_week: int) -> PropertyValue:
        """Create a Date value. ``year`` is the full year (e.g. 2026); use 255 for unspecified fields."""
        ...
    @staticmethod
    def time(hour: int, minute: int, second: int, hundredths: int) -> PropertyValue:
        """Create a Time value. Use 255 for unspecified fields."""
        ...
    @staticmethod
    def bit_string(unused_bits: int, data: bytes) -> PropertyValue:
        """Create a BitString value."""
        ...
    @staticmethod
    def list(items: list[PropertyValue]) -> PropertyValue:
        """Create a List (array) value from a list of PropertyValue items."""
        ...
    @staticmethod
    def application_data(bytes: bytes) -> PropertyValue:
        """Create an ApplicationData value from pre-encoded application-layer bytes (e.g. a framed BACnetEventParameter)."""
        ...

    @property
    def tag(self) -> str:
        """Type tag: 'null', 'boolean', 'unsigned', 'signed', 'real', 'double',
        'octet_string', 'character_string', 'bit_string', 'enumerated',
        'date', 'time', 'object_identifier'."""
        ...

    @property
    def value(self) -> Any:
        """The Python-native value (int, float, str, bytes, bool, dict, or None)."""
        ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class DiscoveredDevice:
    """A device discovered via Who-Is / I-Am."""

    @property
    def object_identifier(self) -> ObjectIdentifier: ...

    @property
    def mac_address(self) -> bytes: ...

    @property
    def max_apdu_length(self) -> int: ...

    @property
    def segmentation_supported(self) -> Segmentation: ...

    @property
    def vendor_id(self) -> int: ...

    @property
    def seconds_since_seen(self) -> float: ...

    @property
    def source_network(self) -> Optional[int]: ...

    @property
    def source_address(self) -> Optional[bytes]: ...

    def __repr__(self) -> str: ...


class CovNotification:
    """A Change-of-Value notification received from a remote device."""

    @property
    def subscriber_process_identifier(self) -> int: ...

    @property
    def initiating_device_identifier(self) -> ObjectIdentifier: ...

    @property
    def monitored_object_identifier(self) -> ObjectIdentifier: ...

    @property
    def time_remaining(self) -> int: ...

    @property
    def delivery(self) -> str: ...

    @property
    def source_mac(self) -> bytes: ...

    @property
    def source_network(self) -> Optional[int]: ...

    @property
    def source_address(self) -> Optional[bytes]: ...

    @property
    def values(self) -> Any:
        """List of property value change entries."""
        ...

    def __repr__(self) -> str: ...


class CovNotificationIterator:
    """Async iterator yielding ``CovNotification`` objects."""

    def __aiter__(self) -> CovNotificationIterator: ...
    async def __anext__(self) -> CovNotification: ...


# ---------------------------------------------------------------------------
# Exceptions
# ---------------------------------------------------------------------------

class BacnetError(Exception):
    """Base exception for all BACnet errors."""
    ...

class BacnetProtocolError(BacnetError):
    """Raised on BACnet protocol-level errors (Error PDU).

    Attributes:
        error_class: The BACnet error class (integer).
        error_code: The BACnet error code (integer).
    """
    error_class: int
    error_code: int

class BacnetTimeoutError(BacnetError):
    """Raised when a BACnet operation times out."""
    ...

class BacnetRejectError(BacnetError):
    """Raised when a BACnet request is rejected.

    Attributes:
        reason: The reject reason code (integer).
    """
    reason: int

class BacnetAbortError(BacnetError):
    """Raised when a BACnet transaction is aborted.

    Attributes:
        reason: The abort reason code (integer).
    """
    reason: int


# ---------------------------------------------------------------------------
# Client
# ---------------------------------------------------------------------------

class BACnetClient:
    """Async BACnet client for reading/writing properties on remote devices.

    Supports BACnet/IP (``"bip"``), BACnet/IPv6 (``"ipv6"``),
    BACnet/SC (``"sc"``), and BACnet MS/TP (``"mstp"``) transports.

    For SC, sc_ca_cert, sc_client_cert, and sc_client_key must be nonempty
    paths; omission, None, or empty strings raise ValueError at construction.
    Their None defaults preserve non-SC use and positional layout only.
    Async entry loads the explicit site CA and operational cert/key; invalid
    TLS configuration raises RuntimeError before dialing. No system-root or
    unauthenticated-client fallback is available.

    SC also requires keyword-only sc_device_uuid: exactly 16 bytes, not all zero,
    copied at construction. Missing, None, wrong length, or all-zero values
    raise ValueError after credential-presence validation, before file/network I/O.
    Provision before deployment and durably reuse the same UUID for the device's
    lifetime. No generation, persistence, change detection, or version/variant
    enforcement is provided. Non-SC transports ignore this option.

    Usage::

        async with BACnetClient() as client:
            await client.who_is()
            devices = await client.discovered_devices()
            value = await client.read_property("192.168.1.100:47808", oid, pid)
            print(value.tag, value.value)

        # MS/TP peer address is a station MAC string ("7" or "mstp:7")
        async with BACnetClient(transport="mstp", serial_port="/dev/ttyUSB0", mstp_mac=3) as client:
            value = await client.read_property("7", oid, pid)
    """

    def __init__(
        self,
        interface: str = "0.0.0.0",
        port: int = 0xBAC0,
        broadcast_address: str = "255.255.255.255",
        apdu_timeout_ms: int = 6000,
        transport: str = "bip",
        sc_hub: Optional[str] = None,
        sc_vmac: Optional[bytes] = None,
        sc_ca_cert: Optional[str] = None,
        sc_client_cert: Optional[str] = None,
        sc_client_key: Optional[str] = None,
        sc_heartbeat_interval_ms: Optional[int] = None,
        sc_heartbeat_timeout_ms: Optional[int] = None,
        ipv6_interface: Optional[str] = None,
        *,
        serial_port: Optional[str] = None,
        mstp_baud: int = 38400,
        mstp_mac: int = 1,
        mstp_max_master: int = 127,
        mstp_max_info_frames: int = 1,
        sc_device_uuid: Optional[bytes | bytearray] = None,
    ) -> None: ...

    async def __aenter__(self) -> BACnetClient: ...
    async def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> None: ...

    # --- Property operations ---

    async def read_property(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> PropertyValue:
        """Read a single property from a remote device."""
        ...

    async def write_property(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        value: PropertyValue,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
    ) -> None:
        """Write a single property on a remote device."""
        ...

    async def read_property_multiple(
        self,
        address: str,
        specs: list[
            tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]
        ],
    ) -> Any:
        """Read multiple properties from a remote device (ReadPropertyMultiple).

        ``specs`` is a list of ``(object_id, [(property_id, array_index), ...])`` tuples.
        Returns a nested dict structure of results.
        """
        ...

    async def write_property_multiple(
        self,
        address: str,
        specs: list[
            tuple[
                ObjectIdentifier,
                list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]],
            ]
        ],
    ) -> None:
        """Write multiple properties on a remote device (WritePropertyMultiple).

        ``specs`` is ``[(object_id, [(property_id, value, priority, array_index), ...]), ...]``.
        """
        ...

    # --- Multi-device batch operations ---

    async def read_property_from_devices(
        self,
        requests: list[
            tuple[int, ObjectIdentifier, PropertyIdentifier, Optional[int]]
        ],
        max_concurrent: Optional[int] = None,
    ) -> list[dict[str, Any]]:
        """Read a property from multiple discovered devices concurrently.

        ``requests`` is ``[(device_instance, object_id, property_id, array_index), ...]``.
        Returns ``[{"device_instance": int, "value": PropertyValue | None, "error": str | None}, ...]``.
        """
        ...

    async def read_property_multiple_from_devices(
        self,
        requests: list[
            tuple[
                int,
                list[
                    tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]
                ],
            ]
        ],
        max_concurrent: Optional[int] = None,
    ) -> list[dict[str, Any]]:
        """Read multiple properties from multiple devices concurrently (RPM batch).

        Returns ``[{"device_instance": int, "results": Any | None, "error": str | None}, ...]``.
        """
        ...

    async def write_property_to_devices(
        self,
        requests: list[
            tuple[int, ObjectIdentifier, PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]
        ],
        max_concurrent: Optional[int] = None,
    ) -> list[dict[str, Any]]:
        """Write a property to multiple devices concurrently.

        ``requests`` is ``[(device_instance, object_id, property_id, value, priority, array_index), ...]``.
        Returns ``[{"device_instance": int, "error": str | None}, ...]``.
        """
        ...

    # --- Discovery ---

    async def who_is(
        self,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> None:
        """Broadcast a Who-Is request. Responses are collected asynchronously;
        use ``discovered_devices()`` to retrieve them."""
        ...

    async def discover(
        self,
        timeout_ms: int = 3000,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> list[DiscoveredDevice]:
        """Convenience: send WhoIs, wait ``timeout_ms``, return discovered devices."""
        ...

    async def who_has_by_id(
        self,
        object_id: ObjectIdentifier,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> None:
        """Broadcast Who-Has by object identifier."""
        ...

    async def who_has_by_name(
        self,
        name: str,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> None:
        """Broadcast Who-Has by object name."""
        ...

    async def discovered_devices(self) -> list[DiscoveredDevice]:
        """Return all devices discovered so far."""
        ...

    async def get_device(self, instance: int) -> Optional[DiscoveredDevice]:
        """Look up a discovered device by instance number."""
        ...

    async def clear_devices(self) -> None:
        """Clear the discovered device table."""
        ...

    async def who_am_i(self) -> None:
        """Broadcast a Who-Am-I request."""
        ...

    async def who_is_directed(
        self,
        address: str,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> None:
        """Send a Who-Is to a specific device address (unicast)."""
        ...

    # --- Time synchronization ---

    async def time_synchronization(
        self,
        address: str,
        date: tuple[int, int, int, int],
        time: tuple[int, int, int, int],
    ) -> None:
        """Send a TimeSynchronization request (unconfirmed).

        ``date`` is ``(year, month, day, day_of_week)``; ``time`` is
        ``(hour, minute, second, hundredths)``. Year is the full year (e.g. 2026).
        """
        ...

    async def utc_time_synchronization(
        self,
        address: str,
        date: tuple[int, int, int, int],
        time: tuple[int, int, int, int],
    ) -> None:
        """Send a UTCTimeSynchronization request (unconfirmed)."""
        ...

    # --- Auto-routing (by device instance) ---

    async def read_property_from_device(
        self,
        device_instance: int,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> PropertyValue:
        """Read a property from a device by instance number (auto-routing)."""
        ...

    async def read_property_multiple_from_device(
        self,
        device_instance: int,
        specs: list[
            tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]
        ],
    ) -> Any:
        """Read multiple properties from a device by instance number (auto-routing)."""
        ...

    async def write_property_to_device(
        self,
        device_instance: int,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        value: PropertyValue,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
    ) -> None:
        """Write a property on a device by instance number (auto-routing)."""
        ...

    async def write_property_multiple_to_device(
        self,
        device_instance: int,
        specs: list[
            tuple[
                ObjectIdentifier,
                list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]],
            ]
        ],
    ) -> None:
        """Write multiple properties to a device by instance number (auto-routing)."""
        ...

    async def add_device(
        self, device_instance: int, address: str
    ) -> None:
        """Add a device to the discovery table manually (useful when address is known without WhoIs)."""
        ...

    # --- COV subscriptions ---

    async def subscribe_cov(
        self,
        address: str,
        subscriber_process_identifier: int,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: bool,
        lifetime: Optional[int] = None,
    ) -> None:
        """Subscribe to Change-of-Value notifications for an object."""
        ...

    async def unsubscribe_cov(
        self,
        address: str,
        subscriber_process_identifier: int,
        monitored_object_identifier: ObjectIdentifier,
    ) -> None:
        """Cancel a COV subscription."""
        ...

    async def subscribe_cov_property_multiple(
        self,
        address: str,
        subscriber_process_identifier: int,
        specs: list[
            tuple[
                ObjectIdentifier,
                list[tuple[PropertyIdentifier, Optional[int], Optional[float], bool]],
            ]
        ],
        issue_confirmed_notifications: bool,
        max_notification_delay: Optional[int] = None,
        lifetime: Optional[int] = None,
    ) -> None:
        """Subscribe to COV notifications for multiple properties on multiple objects.

        ``specs`` is ``[(object_id, [(property_id, array_index, cov_increment, timestamped), ...]), ...]``.
        ``issue_confirmed_notifications`` is required, including for cancellation requests.
        For subscriptions and re-subscriptions, ``lifetime`` and ``max_notification_delay`` are both required.
        For whole-context cancellations, pass an empty ``specs`` list and omit both fields.
        """
        ...

    async def cov_notifications(self) -> CovNotificationIterator:
        """Get an async iterator for incoming COV notifications."""
        ...

    # --- Object management ---

    async def delete_object(
        self, address: str, object_id: ObjectIdentifier
    ) -> None:
        """Delete an object on a remote device (DeleteObject service)."""
        ...

    async def create_object(
        self,
        address: str,
        object_specifier: Union[ObjectType, ObjectIdentifier],
        initial_values: Optional[
            list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]
        ] = None,
    ) -> bytes:
        """Create an object on a remote device (CreateObject service).

        ``object_specifier`` is an ``ObjectType`` (server assigns instance) or
        ``ObjectIdentifier`` (specific instance). ``initial_values`` is
        ``[(property_id, value, priority, array_index), ...]``.
        Returns the raw CreateObject-ACK response bytes.
        """
        ...

    # --- Device management ---

    async def device_communication_control(
        self,
        address: str,
        enable_disable: EnableDisable,
        time_duration: Optional[int] = None,
        password: Optional[str] = None,
    ) -> None:
        """Send DeviceCommunicationControl to a remote device."""
        ...

    async def reinitialize_device(
        self,
        address: str,
        reinitialized_state: ReinitializedState,
        password: Optional[str] = None,
    ) -> None:
        """Send ReinitializeDevice to a remote device."""
        ...

    # --- Alarms and events ---

    async def acknowledge_alarm_request(
        self,
        address: str,
        acknowledging_process_identifier: int,
        event_object_identifier: ObjectIdentifier,
        event_state_acknowledged: int,
        timestamp: BACnetTimeStamp,
        acknowledgment_source: str,
        time_of_acknowledgment: BACnetTimeStamp,
    ) -> None:
        """Acknowledge an alarm with exact caller-supplied BACnetTimeStamp values.

        ``timestamp`` must echo the original event notification timestamp;
        ``time_of_acknowledgment`` is selected by the caller.
        """
        ...

    async def acknowledge_alarm(
        self,
        address: str,
        acknowledging_process_identifier: int,
        event_object_identifier: ObjectIdentifier,
        event_state_acknowledged: int,
        acknowledgment_source: str,
    ) -> None:
        """Deprecated compatibility method.

        This method fabricates sequence-number zero for both timestamps. Use
        ``acknowledge_alarm_request`` for a lossless request.
        """
        ...

    async def get_event_information(
        self,
        address: str,
        last_received_object_identifier: Optional[ObjectIdentifier] = None,
    ) -> bytes:
        """Get event information from a remote device. Returns raw response bytes."""
        ...

    async def get_alarm_summary(self, address: str) -> list[dict[str, Any]]:
        """Get alarm summary from a remote device.

        Returns ``[{"object_id": ObjectIdentifier, "alarm_state": EventState,
        "acknowledged_transitions": {"unused_bits": int, "data": bytes}}, ...]``.
        """
        ...

    async def get_enrollment_summary(
        self,
        address: str,
        acknowledgment_filter: int = 0,
        event_state_filter: Optional[EnrollmentSummaryEventStateFilter] = None,
        event_type_filter: Optional[EventType] = None,
        min_priority: Optional[int] = None,
        max_priority: Optional[int] = None,
        notification_class_filter: Optional[int] = None,
    ) -> list[dict[str, Any]]:
        """Get enrollment summary from a remote device.

        Returns ``[{"object_id": ObjectIdentifier, "event_type": EventType,
        "event_state": EventState, "priority": int,
        "notification_class": Optional[int]}, ...]``.
        """
        ...

    # --- Range operations ---

    async def read_range(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
        range_type: Optional[str] = None,
        reference_index: Optional[int] = None,
        reference_seq: Optional[int] = None,
        count: Optional[int] = None,
    ) -> dict[str, Any]:
        """Read a range of items from a list or log object.

        ``range_type`` is ``"position"``, ``"sequence"``, or ``None``.
        Returns ``{"object_id": ObjectIdentifier, "property_id": PropertyIdentifier,
        "array_index": int | None, "result_flags": int, "item_count": int,
        "item_data": bytes}``.
        """
        ...

    # --- File operations ---

    async def atomic_read_file(
        self,
        address: str,
        file_identifier: ObjectIdentifier,
        access_method: str,
        start_position: int = 0,
        requested_octet_count: int = 0,
        start_record: int = 0,
        requested_record_count: int = 0,
    ) -> bytes:
        """Read from a file object. ``access_method`` is ``"stream"`` or ``"record"``.
        Returns raw response bytes."""
        ...

    async def atomic_write_file(
        self,
        address: str,
        file_identifier: ObjectIdentifier,
        access_method: str,
        start_position: int = 0,
        file_data: bytes = b"",
        start_record: int = 0,
        record_count: int = 0,
        file_record_data: Optional[list[bytes]] = None,
    ) -> bytes:
        """Write to a file object. ``access_method`` is ``"stream"`` or ``"record"``.
        Returns raw response bytes."""
        ...

    # --- List manipulation ---

    async def add_list_element(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        list_of_elements: bytes,
        array_index: Optional[int] = None,
    ) -> None:
        """Add elements to a list property (AddListElement service)."""
        ...

    async def remove_list_element(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        list_of_elements: bytes,
        array_index: Optional[int] = None,
    ) -> None:
        """Remove elements from a list property (RemoveListElement service)."""
        ...

    # --- Private transfer ---

    async def confirmed_private_transfer(
        self,
        address: str,
        vendor_id: int,
        service_number: int,
        service_parameters: Optional[bytes] = None,
    ) -> dict[str, Any]:
        """Send a ConfirmedPrivateTransfer request.

        Returns ``{"vendor_id": int, "service_number": int, "result_block": bytes | None}``.
        """
        ...

    async def unconfirmed_private_transfer(
        self,
        address: str,
        vendor_id: int,
        service_number: int,
        service_parameters: Optional[bytes] = None,
    ) -> None:
        """Send an UnconfirmedPrivateTransfer request."""
        ...

    # --- Text messaging ---

    async def confirmed_text_message(
        self,
        address: str,
        source_device: ObjectIdentifier,
        message_priority: MessagePriority,
        message: str,
        message_class_type: Optional[str] = None,
        message_class_value: Optional[Any] = None,
    ) -> None:
        """Send a ConfirmedTextMessage.

        ``message_class_type`` is ``"numeric"`` or ``"text"`` (or ``None`` for no class).
        """
        ...

    async def unconfirmed_text_message(
        self,
        address: str,
        source_device: ObjectIdentifier,
        message_priority: MessagePriority,
        message: str,
        message_class_type: Optional[str] = None,
        message_class_value: Optional[Any] = None,
    ) -> None:
        """Send an UnconfirmedTextMessage."""
        ...

    # --- Life safety ---

    async def life_safety_operation(
        self,
        address: str,
        requesting_process_identifier: int,
        requesting_source: str,
        operation: LifeSafetyOperation,
        object_identifier: Optional[ObjectIdentifier] = None,
    ) -> None:
        """Send a LifeSafetyOperation request."""
        ...

    # --- WriteGroup ---

    async def write_group(
        self,
        address: str,
        group_number: int,
        write_priority: int,
        change_list: list[tuple[Optional[ObjectIdentifier], Optional[int], bytes]],
        inhibit_delay: Optional[bool] = None,
    ) -> None:
        """Send a WriteGroup request (unconfirmed).

        ``change_list`` is ``[(channel_oid_or_none, override_priority_or_none, value_bytes), ...]``.
        ``write_priority`` must be 1-16.
        """
        ...

    # --- Virtual terminal ---

    async def vt_open(self, address: str, vt_class: int) -> int:
        """Open a virtual terminal session. Returns the remote session identifier."""
        ...

    async def vt_close(self, address: str, session_ids: list[int]) -> None:
        """Close one or more virtual terminal sessions."""
        ...

    async def vt_data(
        self,
        address: str,
        session_id: int,
        data: bytes,
        data_flag: bool,
    ) -> dict[str, Any]:
        """Send data over a virtual terminal session.

        Returns ``{"all_new_data_accepted": bool, "accepted_octet_count": int}``.
        """
        ...

    # --- Audit ---

    async def confirmed_audit_notification_typed(
        self,
        address: str,
        request: AuditNotificationRequestInput,
    ) -> None:
        """Send a validated mapping through the native confirmed Audit helper."""
        ...

    async def unconfirmed_audit_notification_typed(
        self,
        address: str,
        request: AuditNotificationRequestInput,
    ) -> None:
        """Send a validated mapping through the native unconfirmed Audit helper."""
        ...

    async def audit_log_query_typed(
        self,
        address: str,
        request: AuditLogQueryRequestInput,
    ) -> AuditLogQueryAck:
        """Send a typed Audit Log query and return a canonical decoded mapping."""
        ...

    async def confirmed_audit_notification(
        self, address: str, service_data: bytes
    ) -> None:
        """Send a pre-encoded Clause 21 ConfirmedAuditNotification payload.

        This is a raw escape hatch; the bundled server does not execute the
        service.
        """
        ...

    async def unconfirmed_audit_notification(
        self, address: str, service_data: bytes
    ) -> None:
        """Send a pre-encoded Clause 21 UnconfirmedAuditNotification payload.

        This is a raw escape hatch; the bundled server does not execute the
        service.
        """
        ...

    async def audit_log_query(
        self,
        address: str,
        service_data: bytes,
    ) -> bytes:
        """Send a pre-encoded Clause 21 AuditLogQuery payload.

        This raw escape hatch returns the peer's response payload. The bundled
        server does not execute the service.
        """
        ...

    # --- Lifecycle ---

    async def stop(self) -> None:
        """Explicitly stop the client and release resources."""
        ...


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------

class DccOutcomeCounters(TypedDict):
    """Independent u64 lifetime totals, saturating at 2**64-1; not an audit log."""
    accepted_total: int
    policy_denied_total: int
    password_failure_total: int
    deprecated_denied_total: int
    malformed_total: int

class RequestAdmissionCounters(TypedDict):
    """Independent counter samples, not a transactionally atomic aggregate."""
    recovery_active: int
    recovery_admitted_total: int
    recovery_overloaded_total: int
    confirmed_active: int
    confirmed_admitted_total: int
    confirmed_overloaded_total: int
    confirmed_global_overloaded_total: int
    confirmed_peer_overloaded_total: int
    confirmed_shutdown_rejected_total: int
    unconfirmed_active: int
    unconfirmed_admitted_total: int
    unconfirmed_overloaded_total: int
    unconfirmed_global_overloaded_total: int
    unconfirmed_peer_overloaded_total: int
    unconfirmed_shutdown_rejected_total: int
    abort_active: int
    abort_admitted_total: int
    confirmed_fallback_dropped_total: int
    abort_shutdown_rejected_total: int

class BACnetServer:
    """BACnet server that hosts objects and responds to client requests.

    For SC, sc_ca_cert, sc_client_cert, and sc_client_key must be nonempty
    paths; omission, None, or empty strings raise ValueError at construction.
    Their None defaults preserve non-SC use and positional layout only.
    start() loads the explicit site CA and operational cert/key before dialing
    or draining registrations. Local TLS configuration errors raise RuntimeError;
    repair the files and retry on the same server. Later startup failures do not
    have a general registration rollback guarantee.

    SC also requires keyword-only sc_device_uuid: exactly 16 bytes, not all zero,
    copied at construction. Missing, None, wrong length, or all-zero values
    raise ValueError after credential-presence validation, before file/network I/O.
    Provision before deployment and durably reuse the same UUID for the device's
    lifetime. No generation, persistence, change detection, or version/variant
    enforcement is provided. Non-SC transports ignore this option.

    Usage::

        server = BACnetServer(device_instance=1234, device_name="My Device")
        server.add_analog_input(1, "Temperature", units=62)
        await server.start()
    """

    def __init__(
        self,
        device_instance: int,
        device_name: str = "BACnet Device",
        interface: str = "0.0.0.0",
        port: int = 0xBAC0,
        broadcast_address: str = "255.255.255.255",
        transport: str = "bip",
        sc_hub: Optional[str] = None,
        sc_vmac: Optional[bytes] = None,
        sc_ca_cert: Optional[str] = None,
        sc_client_cert: Optional[str] = None,
        sc_client_key: Optional[str] = None,
        sc_heartbeat_interval_ms: Optional[int] = None,
        sc_heartbeat_timeout_ms: Optional[int] = None,
        ipv6_interface: Optional[str] = None,
        dcc_password: Optional[str] = None,
        reinit_password: Optional[str] = None,
        *,
        dcc_policy: str = "deny_all",
        dcc_source_restriction: list[tuple[int | None, bytes]] | None = None,
        dcc_disable_rate_limit: tuple[int, int] | None = None,
        serial_port: Optional[str] = None,
        mstp_baud: int = 38400,
        mstp_mac: int = 1,
        mstp_max_master: int = 127,
        mstp_max_info_frames: int = 1,
        max_confirmed_in_flight: int = 64,
        max_unconfirmed_in_flight: int = 32,
        max_confirmed_in_flight_per_peer: int = 16,
        max_unconfirmed_in_flight_per_peer: int = 8,
        confirmed_recovery_reserve: int = 4,
        max_recovery_in_flight_per_peer: int = 1,
        rpm_max_result_elements: int = 256,
        rpm_max_service_ack_bytes: int = 16384,
        alarm_summary_max_objects: int = 4096,
        alarm_summary_max_service_ack_bytes: int = 16384,
        enrollment_summary_max_objects: int = 4096,
        enrollment_summary_max_service_ack_bytes: int = 16384,
        atomic_read_file_max_requested_stream_octets: int = 16384,
        atomic_read_file_max_requested_records: int = 256,
        atomic_read_file_max_service_ack_bytes: int = 16384,
        atomic_write_file_max_stream_payload_octets: int = 16384,
        atomic_write_file_max_records: int = 256,
        atomic_write_file_max_record_payload_bytes: int = 16384,
        read_range_max_returned_items: int = 256,
        read_range_max_service_ack_bytes: int = 16384,
        event_information_max_objects: int = 4096,
        event_information_max_returned_summaries: int = 256,
        event_information_max_service_ack_bytes: int = 16384,
        sc_device_uuid: Optional[bytes | bytearray] = None,
    ) -> None: ...

    # --- Analog objects ---
    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_output(self, instance: int, name: str, units: int = 62) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62) -> None: ...

    # --- Binary objects ---
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_output(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str) -> None: ...

    # --- Multi-state objects ---
    def add_multistate_input(self, instance: int, name: str, number_of_states: int) -> None: ...
    def add_multistate_output(self, instance: int, name: str, number_of_states: int) -> None: ...
    def add_multistate_value(self, instance: int, name: str, number_of_states: int) -> None: ...

    # --- Date/time/pattern objects ---
    def add_calendar(self, instance: int, name: str) -> None: ...
    def add_schedule(self, instance: int, name: str) -> None: ...
    def add_date_value(self, instance: int, name: str) -> None: ...
    def add_time_value(self, instance: int, name: str) -> None: ...
    def add_date_time_value(self, instance: int, name: str) -> None: ...
    def add_date_pattern_value(self, instance: int, name: str) -> None: ...
    def add_time_pattern_value(self, instance: int, name: str) -> None: ...
    def add_date_time_pattern_value(self, instance: int, name: str) -> None: ...

    # --- Notification/logging ---
    def add_notification_class(self, instance: int, name: str, notification_class: int = 0) -> None: ...
    def add_trend_log(self, instance: int, name: str, buffer_size: int = 100) -> None: ...
    def add_trend_log_multiple(self, instance: int, name: str, buffer_size: int = 100) -> None: ...
    def add_event_log(self, instance: int, name: str, buffer_size: int = 100) -> None: ...
    def add_audit_log(self, instance: int, name: str, storage_path: str, buffer_size: int = 100) -> None: ...
    def add_device_binding(self, device_instance: int, address: str) -> None:
        """Register a direct B/IP (IPv4) Device binding on a transport="bip" server.

        Accept IPv4:port or exactly six hex bytes (four IPv4 octets and two port octets).
        Other transports or parsed address lengths raise ValueError without retention.
        The shared parser's broader grammar remains available to other APIs, not bindings.
        Invalid instance/address or duplicate Device raises ValueError. No overwrite,
        routing or discovery. Syntax validation precedes the frozen-state check; after
        start() consumes configuration, parseable calls raise RuntimeError before the
        transport/shape checks, including after stop. Broadcast validation stays in start().
        """
        ...
    def configure_audit_log_parent(
        self, instance: int, *, parent_device_instance: int, parent_audit_log_instance: int
    ) -> None:
        """Set a registered Audit Log's remote parent; valid pre-start calls replace it.

        Identifiers are integers in 0..=4194303, not bool. Invalid/missing/duplicate
        local logs or a local parent Device raise ValueError without mutation.
        Startup revalidates local identity; settings freeze at ownership transfer.
        Reapply on a new server after reopen. Only the selected inbound sink forwards,
        using a configured direct B/IP binding with the existing one-attempt confirmed
        policy, never deletion or a durable queue. IPv6/SC/MS/TP forwarding is not exposed.
        """
        ...
    def configure_audit_notification_sink(
        self, instance: int, *, policy: Literal["deny_all", "allow_all"]
    ) -> None:
        """Select one registered Audit Log before start(); omission denies receipt.

        Invalid policy/instance, missing or duplicate log raises ValueError without
        mutation. Startup revalidates before transport preparation/registration
        transfer. A running server raises RuntimeError. Payload Source_Device and
        Target_Device remain peer-reported, not verified origin. No Python callbacks.
        """
        ...
    def add_audit_reporter(self, instance: int, name: str) -> None: ...
    def configure_audit_recipient(self, recipient: AuditRecipientInput) -> None:
        """Provision the copied Device-owned recipient before startup.

        Accepts a concrete remote Device (not instance 4194303), or a supported
        direct unicast IPv4 BACnetAddress on B/IP. The selected target Reporter
        requires provision even at Audit_Level NONE. Add Device routes with
        add_device_binding independently. Missing Device routes start with
        CONFIGURATION_ERROR; live changes require usable old and new routes.
        This setter freezes at ownership transfer. Use Device property writes
        after start for atomic old/new delivery. No NULL recipient sentinel.
        """
        ...
    def configure_audit_reporter(
        self, instance: int, *,
        audit_level: Literal["none", "audit_config", "audit_all"],
        auditable_operations: int, issue_confirmed_notifications: bool,
        monitored_objects: list[ObjectIdentifier | ObjectType | None] | None = None,
        audit_priority_filter: int | None = None,
    ) -> None:
        """Configure one static target Reporter; add_audit_reporter alone stays inert.

        The first valid call fixes the Reporter identity. Later pre-start calls
        replace its settings; another Reporter raises ValueError.
        Reporter instances are non-bool integers in 0..=4194303.
        Operations is a non-bool u64 mask: bits 0..15 and 32..63 only. Wrong mask,
        level or confirmation types raise TypeError; invalid values/identities
        raise ValueError. Failures preserve prior settings and registrations.
        Configuration freezes at startup ownership transfer, including in-flight
        start and after stop (RuntimeError). Provision the Device recipient separately; configure its route
        with add_device_binding, in either order; an unresolved recipient permits
        startup but exposes CONFIGURATION_ERROR on an enabled Reporter's RELIABILITY.
        Monitored objects: None/omission removes the property (catch-all); an exact
        list selects exact ObjectIdentifiers or all instances of each ObjectType
        (including extensible values). None entries are ignored, empty/all-None
        selects no ordinary targets, and duplicates never duplicate records.
        Wrong container/element types raise TypeError. Priority filter is a non-bool
        u16 mask (0..65535): bit 0 selects priority 1, bit 15 priority 16 (also the
        default for omitted write priority). None/omission selects all priorities;
        zero is valid. Wrong types raise TypeError; out-of-range values ValueError.
        Priority filtering applies only to commandable-property writes; enabled
        Reporter-target writes retain their filter bypass. Replacement resets
        omitted options to their defaults. No runtime changes, source reporting,
        Python callbacks, retries or durable outbox.
        """
        ...

    # --- Control/PID ---
    def add_loop(self, instance: int, name: str, output_units: int = 62) -> None: ...
    def add_command(self, instance: int, name: str) -> None: ...
    def add_timer(self, instance: int, name: str) -> None: ...
    def add_load_control(self, instance: int, name: str) -> None: ...
    def add_program(self, instance: int, name: str) -> None: ...

    # --- Lighting ---
    def add_lighting_output(self, instance: int, name: str) -> None: ...
    def add_binary_lighting_output(self, instance: int, name: str) -> None: ...

    # --- Life safety ---
    def add_life_safety_point(self, instance: int, name: str) -> None: ...
    def add_life_safety_zone(self, instance: int, name: str) -> None: ...

    # --- Grouping/organization ---
    def add_group(self, instance: int, name: str) -> None: ...
    def add_global_group(self, instance: int, name: str) -> None: ...
    def add_structured_view(self, instance: int, name: str) -> None: ...

    # --- Access control ---
    def add_access_door(self, instance: int, name: str) -> None: ...
    def add_access_credential(self, instance: int, name: str) -> None: ...
    def add_access_point(self, instance: int, name: str) -> None: ...
    def add_access_rights(self, instance: int, name: str) -> None: ...
    def add_access_user(self, instance: int, name: str) -> None: ...
    def add_access_zone(self, instance: int, name: str) -> None: ...
    def add_credential_data_input(self, instance: int, name: str) -> None: ...
    def add_alert_enrollment(
        self, instance: int, name: str, initial_source: ObjectIdentifier
    ) -> None: ...
    def add_event_enrollment(self, instance: int, name: str, event_type: int = 0) -> None: ...

    # --- Building/transportation ---
    def add_elevator_group(self, instance: int, name: str) -> None: ...
    def add_escalator(self, instance: int, name: str) -> None: ...
    def add_lift(self, instance: int, name: str, num_floors: int) -> None: ...
    def add_staging(
        self,
        instance: int,
        name: str,
        present_value: float,
        min_present_value: float,
        units: int,
        priority_for_writing: int,
        stages: list[tuple[float, list[bool], float]],
        target_references: list[ObjectIdentifier],
        stage_names: list[str] | None = None,
    ) -> None:
        """Add a validated local-target Staging object before start()."""

    # --- Averaging ---
    def add_averaging(self, instance: int, name: str) -> None: ...

    # --- Value objects ---
    def add_integer_value(self, instance: int, name: str) -> None: ...
    def add_positive_integer_value(self, instance: int, name: str) -> None: ...
    def add_large_analog_value(self, instance: int, name: str) -> None: ...
    def add_character_string_value(self, instance: int, name: str) -> None: ...
    def add_octet_string_value(self, instance: int, name: str) -> None: ...
    def add_bit_string_value(self, instance: int, name: str) -> None: ...

    # --- Counters/accumulators ---
    def add_accumulator(self, instance: int, name: str, units: int = 62) -> None: ...
    def add_pulse_converter(self, instance: int, name: str, units: int = 62) -> None: ...

    # --- Files/network ---
    def add_file(self, instance: int, name: str, file_type: str = "application/octet-stream") -> None: ...
    def set_file_access_method(self, /, instance: int, access_method: str) -> None:
        """Select ``"stream"`` or ``"record"`` on a pending built-in File.

        This synchronous operation is valid only before ``start()``. Select the
        access method before loading the corresponding payload; changing modes
        does not convert stream data to records or vice versa.
        """
        ...
    def set_file_data(self, /, instance: int, data: bytes) -> None:
        """Copy bytes into a pending stream-access built-in File before ``start()``."""
        ...
    def get_file_data(self, /, instance: int) -> bytes:
        """Return a fresh bytes copy from a pending stream File before ``start()``."""
        ...
    def set_file_records(self, /, instance: int, records: list[bytes]) -> None:
        """Copy records into a pending record-access built-in File before ``start()``."""
        ...
    def get_file_records(self, /, instance: int) -> list[bytes]:
        """Return a fresh list and fresh bytes from a pending record File before ``start()``."""
        ...
    def set_max_file_size(self, /, instance: int, max_octets: int) -> int:
        """Set and return the effective octet growth cap before ``start()``.

        The built-in File clamp is authoritative. This does not truncate
        preloaded content.
        """
        ...
    def set_max_record_count(self, /, instance: int, max_records: int) -> int:
        """Set and return the effective record growth cap before ``start()``.

        The built-in File clamp is authoritative. This does not truncate
        preloaded records.
        """
        ...
    def add_network_port(self, instance: int, name: str, network_type: int = 0) -> None: ...

    # --- Server lifecycle ---
    async def start(self) -> None:
        """Start the server and begin accepting BACnet requests."""
        ...

    async def stop(self) -> None:
        """Stop the server and release resources."""
        ...

    async def local_address(self) -> str:
        """Get the local address the server is listening on."""
        ...

    # --- Server-side property access ---
    async def read_property(
        self,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> PropertyValue:
        """Read a property from a local object."""
        ...

    async def write_property_local(
        self,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        value: PropertyValue,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
    ) -> None:
        """Write a property on a local object."""
        ...

    async def set_present_value_local(
        self,
        object_id: ObjectIdentifier,
        value: PropertyValue,
    ) -> None:
        """Update Present_Value through the narrow, non-generic application Input authority.

        Accepted values are a finite REAL for Analog Input, logical Enumerated 0/1
        (INACTIVE/ACTIVE) for Binary Input, and Unsigned 1..=Number_Of_States for
        Multi-state Input. Binary values are BACnet logical values after Polarity,
        not raw hardware/interface levels. An Input with Out_Of_Service set rejects
        this update to preserve network simulation ownership. Other object types are
        not writable through this method.
        """
        ...

    async def comm_state(self) -> int:
        """Get the DeviceCommunicationControl state (0=Enable, 1=Disable, 2=DisableInitiation)."""
        ...

    async def dcc_outcome_counters(self) -> DccOutcomeCounters:
        """Sample completed admitted DCC handlers; zero on each new server lifetime.

        Accepted means state/timer commit, not response delivery. Independent
        samples are not an atomic aggregate. Excludes incomplete cancellation,
        duplicates, pre-handler rejection and timer expiry. No trace bridge or
        subscriber installation. Raises RuntimeError before start/after stop.
        """
        ...

    async def request_admission_counters(self) -> RequestAdmissionCounters:
        """Sample counters; RuntimeError before start and after stop.

        Admitted totals count registered work, not successful response sends.
        """
        ...


# ---------------------------------------------------------------------------
# SC Hub
# ---------------------------------------------------------------------------

class ScHubStatus(TypedDict):
    """Bounded hub snapshot: counts and kind labels only (no keys/VMAC maps)."""
    listening: bool
    max_clients: int
    max_handshakes: int
    client_count: int
    handshake_count: int
    admin_denied: int
    broadcast_sender_exhausted: int
    broadcast_global_exhausted: int

class ScHub:
    """BACnet/SC Hub for relaying messages between SC nodes.

    Usage::

        hub = ScHub("0.0.0.0:47809", "cert.pem", "key.pem", b"\\x02\\x00\\x00\\x00\\x00\\x01",
                    ca_cert="ca.pem", device_uuid=provisioned_hub_uuid)
        await hub.start()
        print(await hub.url())
        print(await hub.status())
        print(await hub.shutdown_gracefully())  # "graceful" or "forced"

    The hub also works as an async context manager (starts on entry,
    forcefully stops on exit)::

        async with ScHub(..., device_uuid=provisioned_hub_uuid) as hub:
            ...

    ``ca_cert`` must name trusted issuer CA PEM certificates for mutual TLS.
    Omitted, None, or empty values raise ValueError at construction. The None
    default retains the existing positional layout only; there is no insecure
    mode. Invalid/unreadable credentials raise BacnetError from start(), before
    binding. Connections require TLS 1.3 and a valid trusted client certificate.

    ``device_uuid`` is required, keyword-only and copied from bytes/bytearray into
    owned storage. Missing/None, wrong length or all-zero UUID raises ValueError
    before file I/O. The caller provisions this hosting device identity before
    deployment and durably reuses the same bytes for its entire lifetime, including
    stop/start and fresh objects. No generation/storage or UUID-bit validation is
    provided. The hosting port's ``vmac`` must be six bytes (RuntimeError for wrong
    length), neither all zero nor all ff (ValueError). CA presence remains first,
    then VMAC validation, then UUID. No certificate-to-identity binding is implied.

    Admission and timeout policy is constructor-validated, before bind:
    ``max_clients``/``max_handshakes`` caps (zero/overflowing raise ValueError,
    negative values raise OverflowError); ``admission_policy`` is the static
    string ``"allow_all"`` (default), ``"deny_all"`` or
    ``"deny_uuid_replacement"`` (unknown strings raise
    ValueError, non-strings including callables raise TypeError — no Python
    callback can run under the native registry lock). Replacement refusal is a
    local security policy before protocol acceptance: it preserves an incumbent
    when the same claimed UUID requests its current or another VMAC. Default
    mode retains Annex AB replacement; a different-UUID VMAC collision keeps its
    standard NAK. UUID equality is not certificate identity proof. Graceful per-peer ack /
    close / overall millisecond bounds and handshake TLS / WebSocket-upgrade /
    Connect-Request millisecond bounds (out-of-range values raise ValueError).
    ``stop()`` is forceful and idempotent; ``shutdown_gracefully()`` runs the
    Disconnect/Ack/close exchange and consumes the hub; dropping the hub
    without awaiting close only seals admission and cannot guarantee cleanup.
    """

    def __init__(
        self,
        listen: str,
        cert: str,
        key: str,
        vmac: bytes,
        ca_cert: Optional[str] = None,
        *,
        device_uuid: Optional[Union[bytes, bytearray]] = None,
        max_clients: int = 256,
        max_handshakes: int = 256,
        admission_policy: Literal["allow_all", "deny_all", "deny_uuid_replacement"] = "allow_all",
        graceful_disconnect_ack_ms: int = 5000,
        graceful_ws_close_ms: int = 5000,
        graceful_overall_ms: int = 15000,
        handshake_tls_ms: int = 10000,
        handshake_websocket_upgrade_ms: int = 10000,
        handshake_connect_request_ms: int = 10000,
    ) -> None: ...

    async def start(self) -> None:
        """Start the SC hub."""
        ...

    async def stop(self) -> None:
        """Stop the SC hub (forceful, idempotent)."""
        ...

    async def shutdown_gracefully(self) -> Literal["graceful", "forced"]:
        """Graceful shutdown; consumes the hub (RuntimeError if not started)."""
        ...

    async def status(self) -> ScHubStatus:
        """Bounded redacted snapshot (RuntimeError before start/after stop)."""
        ...

    async def __aenter__(self) -> ScHub: ...
    async def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> None: ...

    async def address(self) -> Optional[str]:
        """Get the address the hub is listening on (None before start)."""
        ...

    async def url(self) -> Optional[str]:
        """Get the WebSocket URL of the hub (None before start)."""
        ...


# ---------------------------------------------------------------------------
# Endpoint (RB-19): one transport above both roles
# ---------------------------------------------------------------------------

class EndpointStatus(TypedDict):
    """Bounded endpoint snapshot: liveness, identity, transport, leases, policy counts."""
    is_running: bool
    device_instance: int
    vendor_id: int
    max_apdu: int
    transport: str
    local_address: str
    active_leases: int
    ingress_policy: int
    no_server_role: int
    no_client_role: int
    unclaimed_terminal: int
    responder_declined: int

class EndpointClient:
    """Client role cloned from a running endpoint (no lifecycle).

    Initiates ``read_property`` through the owner's single transport.
    After the owner closes, calls fail closed with ``BacnetError``.
    """

    async def read_property(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> PropertyValue:
        """Read a property through the shared transport."""
        ...

    def service_scope(self) -> dict[str, Any]:
        """Narrow scope: initiates ``read_property`` only."""
        ...

class EndpointServer:
    """Server role cloned from a running endpoint (no lifecycle).

    The responder executes ``read_property`` automatically; this handle
    exposes liveness plus the MS/TP deferred-reply seam. No Python
    callbacks run under native locks.
    """

    def is_session_alive(self) -> bool:
        """True while the owning endpoint is alive and open."""
        ...

    def suspend_next_reply(self) -> None:
        """Arm one-shot deferred-reply suspension (MS/TP wiring)."""
        ...

    def service_scope(self) -> dict[str, Any]:
        """Narrow scope: executes ``read_property`` only."""
        ...

class BipEndpoint:
    """B/IP endpoint: one UDP socket that both initiates and executes.

    Two separately constructed objects (``BACnetClient`` + ``BACnetServer``)
    are two connections (two sockets/ports). The endpoint is the
    one-transport path: one socket serves both roles.

    Lifecycle: ``start()`` builds one transport; ``close()`` is idempotent;
    async context entry starts (idempotent when running) and exit closes.
    Dropping without awaiting close only seals forcefully and cannot
    guarantee awaited close. BIPv6/Ethernet have no endpoint owner.
    """

    def __init__(
        self,
        device_instance: int,
        device_name: str = "BACnet Device",
        vendor_id: int = 555,
        interface: str = "0.0.0.0",
        port: int = 0xBAC0,
        broadcast_address: str = "255.255.255.255",
        network_number: int = 0,
        network_port_instance: int = 1,
        max_apdu: int = 1476,
        segmentation: Optional[Segmentation] = None,
        services: Optional[list[int]] = None,
        device_uuid: Optional[Union[bytes, bytearray]] = None,
        queue_capacity: int = 16,
        apdu_timeout_ms: int = 6000,
        apdu_retries: int = 0,
    ) -> None: ...

    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62) -> None: ...
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str) -> None: ...

    async def start(self) -> None:
        """Start the endpoint (start-once; second start raises BacnetError)."""
        ...

    async def close(self) -> None:
        """Close the endpoint (idempotent)."""
        ...

    async def __aenter__(self) -> BipEndpoint: ...
    async def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> None: ...

    async def client(self) -> EndpointClient:
        """Clone the client role (RuntimeError before start/after close)."""
        ...

    async def server(self) -> EndpointServer:
        """Clone the server role (RuntimeError before start/after close)."""
        ...

    async def local_address(self) -> str:
        """Bound address as "ip:port" from the validated startup config."""
        ...

    async def status(self) -> EndpointStatus:
        """Bounded snapshot (RuntimeError before start/after close)."""
        ...

    async def broadcast_i_am(self) -> None:
        """Broadcast one I-Am consistent with the composed identity."""
        ...

    @property
    def device_instance(self) -> int: ...
    @property
    def vendor_id(self) -> int: ...

class ScEndpoint:
    """SC endpoint: one hub connection that both initiates and executes.

    ``sc_device_uuid`` is the single durable lifetime identity for both the
    hub dial and DEVICE_UUID. Lifecycle mirrors ``BipEndpoint``.
    """

    def __init__(
        self,
        device_instance: int,
        sc_hub: str,
        sc_vmac: Union[bytes, bytearray],
        sc_ca_cert: str,
        sc_client_cert: str,
        sc_client_key: str,
        *,
        sc_device_uuid: Union[bytes, bytearray],
        device_name: str = "BACnet Device",
        vendor_id: int = 555,
        sc_heartbeat_interval_ms: int = 30000,
        sc_heartbeat_timeout_ms: int = 60000,
        network_number: int = 0,
        network_port_instance: int = 2,
        max_apdu: int = 1476,
        segmentation: Optional[Segmentation] = None,
        services: Optional[list[int]] = None,
        queue_capacity: int = 16,
    ) -> None: ...

    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62) -> None: ...
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str) -> None: ...

    async def start(self) -> None:
        """Dial the hub and start (start-once)."""
        ...

    async def close(self) -> None:
        """Close the endpoint (idempotent)."""
        ...

    async def __aenter__(self) -> ScEndpoint: ...
    async def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> None: ...

    async def client(self) -> EndpointClient:
        """Clone the client role."""
        ...

    async def server(self) -> EndpointServer:
        """Clone the server role."""
        ...

    async def local_address(self) -> str:
        """VMAC hex for this SC node."""
        ...

    async def status(self) -> EndpointStatus:
        """Bounded snapshot (RuntimeError before start/after close)."""
        ...

    async def broadcast_i_am(self) -> None:
        """Broadcast one I-Am via the hub relay."""
        ...

    @property
    def device_instance(self) -> int: ...
    @property
    def vendor_id(self) -> int: ...

class MstpEndpoint:
    """MS/TP endpoint: one serial owner that both initiates and executes.

    Opens a real serial device via ``TokioSerialPort`` like the current
    wrappers. Lifecycle mirrors ``BipEndpoint``.
    """

    def __init__(
        self,
        device_instance: int,
        serial_port: str,
        device_name: str = "BACnet Device",
        vendor_id: int = 555,
        mstp_baud: int = 38400,
        mstp_mac: int = 1,
        mstp_max_master: int = 127,
        mstp_max_info_frames: int = 1,
        max_apdu: int = 480,
        segmentation: Optional[Segmentation] = None,
        services: Optional[list[int]] = None,
        device_uuid: Optional[Union[bytes, bytearray]] = None,
        queue_capacity: int = 16,
        apdu_timeout_ms: int = 6000,
        apdu_retries: int = 0,
    ) -> None: ...

    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62) -> None: ...
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str) -> None: ...

    async def start(self) -> None:
        """Open serial once and start (start-once)."""
        ...

    async def close(self) -> None:
        """Close the endpoint (idempotent)."""
        ...

    async def __aenter__(self) -> MstpEndpoint: ...
    async def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> None: ...

    async def client(self) -> EndpointClient:
        """Clone the client role."""
        ...

    async def server(self) -> EndpointServer:
        """Clone the server role."""
        ...

    async def local_address(self) -> str:
        """Station MAC as a decimal string."""
        ...

    async def status(self) -> EndpointStatus:
        """Bounded snapshot (RuntimeError before start/after close)."""
        ...

    async def broadcast_i_am(self) -> None:
        """Broadcast one I-Am (MS/TP local broadcast)."""
        ...

    @property
    def device_instance(self) -> int: ...
    @property
    def vendor_id(self) -> int: ...
