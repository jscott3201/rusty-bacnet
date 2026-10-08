"""Type stubs for rusty_bacnet — Python bindings for the BACnet protocol stack (ASHRAE 135-2020).

Generated from the actual PyO3 source code. Enum classes expose all standard constants
as class attributes; vendor-proprietary values are available via ``from_raw()``.
"""

from __future__ import annotations

import datetime
from typing import Any, Awaitable, Literal, NotRequired, Optional, TypedDict, Union

UNSPECIFIED: Literal[255]
"""What an unspecified date or time field holds, the year included: dates
are ``(year, month, day, day_of_week)`` with the full year, and times
``(hour, minute, second, hundredths)``. ``datetime.date(255, ...)`` is the
year 255 AD, not a wildcard."""


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    ADDITIONAL_REFERENCE_PORTS: PropertyIdentifier
    CERTIFICATE_SIGNING_REQUEST_FILE: PropertyIdentifier
    COMMAND_VALIDATION_RESULT: PropertyIdentifier
    ISSUER_CERTIFICATE_FILES: PropertyIdentifier
    COLOR_OVERRIDE: PropertyIdentifier
    COLOR_REFERENCE: PropertyIdentifier
    DEFAULT_COLOR: PropertyIdentifier
    DEFAULT_COLOR_TEMPERATURE: PropertyIdentifier
    OVERRIDE_COLOR_REFERENCE: PropertyIdentifier
    COLOR_COMMAND: PropertyIdentifier
    HIGH_END_TRIM: PropertyIdentifier
    LOW_END_TRIM: PropertyIdentifier
    TRIM_FADE_TIME: PropertyIdentifier

    @staticmethod
    def from_raw(value: int) -> PropertyIdentifier: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


class ErrorClass:
    """BACnet error class enumeration (Clause 18)."""

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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


class EnableDisable:
    """BACnet DeviceCommunicationControl enable/disable options (Clause 16.1)."""

    ENABLE: EnableDisable
    DISABLE: EnableDisable
    DISABLE_INITIATION: EnableDisable

    @staticmethod
    def from_raw(value: int) -> EnableDisable: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


class ReinitializedState:
    """BACnet ReinitializeDevice state options (Clause 16.4)."""

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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


class AcknowledgmentFilter:
    """GetEnrollmentSummary acknowledgment filter (Clause 13.11.1.1)."""

    ALL: AcknowledgmentFilter
    ACKED: AcknowledgmentFilter
    NOT_ACKED: AcknowledgmentFilter

    @staticmethod
    def from_raw(value: int) -> AcknowledgmentFilter: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


class VTClass:
    """BACnet virtual terminal class for VT-Open (Clause 17.2)."""

    DEFAULT_TERMINAL: VTClass
    ANSI_X3_64: VTClass
    DEC_VT52: VTClass
    DEC_VT100: VTClass
    DEC_VT220: VTClass
    HP_700_94: VTClass
    IBM_3130: VTClass

    @staticmethod
    def from_raw(value: int) -> VTClass: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


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
    def __reduce__(self) -> tuple[Any, tuple[int]]:
        """Rebuild with ``from_raw(to_raw())``, for ``copy`` and ``pickle``."""
        ...


# ---------------------------------------------------------------------------
# Core types
# ---------------------------------------------------------------------------

class ObjectIdentifier:
    """BACnet Object Identifier (10-bit type + 22-bit instance number).

    For u32-representable values, types above 1023 or instances above 4,194,303
    raise ValueError. Integers outside u32 raise OverflowError.
    Valid proprietary types and the wire wildcard instance are accepted.

    ``copy.copy``, ``copy.deepcopy`` and ``pickle`` (every protocol) rebuild
    an equal identifier through this constructor.
    """

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

    ``copy.copy``, ``copy.deepcopy`` and ``pickle`` (every protocol) rebuild
    an equal timestamp from its encoded CHOICE, so one read from a peer with
    a field outside those ranges copies too.
    """

    @staticmethod
    def sequence_number(value: int) -> BACnetTimeStamp:
        """Construct the Sequence Number CHOICE with a value in 0..65535;
        another integer raises OverflowError."""
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
        """Exact selected value, using a full year for the Date tuple (255
        when unspecified), as ``PropertyValue.date`` reads."""
        ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...


class ActionCommand(TypedDict):
    """One write in a Command object's action list (``BACnetActionCommand``).

    Unknown keys raise ValueError and wrong types raise TypeError. A
    ``device_identifier`` naming another Device sends that write there as a
    confirmed WriteProperty when the server has a binding for the Device;
    with none, the command fails when the list runs. A ``device_identifier``
    that isn't a Device raises ValueError. A read of Action gives each
    command in this form, with every key present.
    """

    object_identifier: ObjectIdentifier
    property_identifier: PropertyIdentifier
    property_value: PropertyValue
    property_array_index: NotRequired[int | None]
    # 1..=16; other values raise BacnetProtocolError (VALUE_OUT_OF_RANGE), and
    # values outside 0..=255 OverflowError.
    priority: NotRequired[int | None]
    # Seconds to wait after this write, before the next one or the end.
    post_delay: NotRequired[int | None]
    # A failed write with this set stops the rest of the list. Default False.
    quit_on_failure: NotRequired[bool]
    device_identifier: NotRequired[ObjectIdentifier | None]
    # A read of Action carries the flag a run sets. add_command accepts it so a
    # read mapping can be given back, but ignores it: a command starts False.
    write_successful: NotRequired[bool]


class DeviceObjectPropertyReference(TypedDict):
    """A property (``BACnetDeviceObjectPropertyReference``), in this device or
    the one ``device_identifier`` names: a Channel or Trend Log Multiple
    member, or the property whose value decides when an access rule applies,
    such as a Schedule's Present_Value. A member list also takes an
    ``(object, property)`` or ``(object, property, array_index)`` tuple for a
    property in this device, and stores a member naming the server's own
    Device in its local form.

    Unknown or missing keys and a ``device_identifier`` that isn't a Device
    raise ValueError, a ``property_array_index`` outside unsigned32
    OverflowError, and wrong types TypeError. The server reads only its own objects, so a Trend Log
    Multiple member naming another Device logs a failure instead of a value.
    A read of a Channel's, Schedule's or Trend Log Multiple's member list, or
    of a Global Group's Group_Members, gives each reference in this form,
    with every key present.
    """

    object_identifier: ObjectIdentifier
    property_identifier: PropertyIdentifier
    # 0..=4294967295; one element of an array property.
    property_array_index: NotRequired[int | None]
    device_identifier: NotRequired[ObjectIdentifier | None]


class AccessRule(TypedDict):
    """One element of an Access Rights rule array (``BACnetAccessRule``).

    A missing or ``None`` ``time_range`` means the rule applies at any time
    (ALWAYS), and a missing or ``None`` ``location`` that it covers every
    access point (ALL). A ``location`` is an Access Point or Access Zone, as
    an ``ObjectIdentifier`` in this device or a ``(device, object)`` pair.
    A read of Positive_Access_Rules or Negative_Access_Rules gives each
    rule in this form, with every key present.
    """

    enable: bool
    time_range: NotRequired[DeviceObjectPropertyReference | None]
    location: NotRequired[
        ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier] | None
    ]


# One Access Point Authentication_Policy_List element
# (``BACnetAuthenticationPolicy``): the ``(credential_data_input, index)``
# entries, each Credential Data Input an ``ObjectIdentifier`` in this device
# or a ``(device, object)`` pair, then whether the order is enforced and the
# timeout in seconds (0 for none). A read of the list gives each element in
# this form.
AuthenticationPolicy = tuple[
    list[tuple[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier], int]],
    bool,
    int,
]


class AuditReporterConfiguration(TypedDict):
    """Owned pre-start target Reporter settings; no Python callbacks."""
    instance: int
    audit_level: Literal["none", "audit_config", "audit_all"]
    auditable_operations: int
    issue_confirmed_notifications: bool
    monitored_objects: NotRequired[list[ObjectIdentifier | ObjectType | None] | None]
    audit_priority_filter: NotRequired[int | None]
    # Paired Maximum_Send_Delay/Send_Now: None absent; 0..3600 whole seconds.
    maximum_send_delay: NotRequired[int | None]

class AuditRecipientDevice(TypedDict):
    """Audit recipient selected by Device object identifier."""

    kind: Literal["device"]
    object_identifier: ObjectIdentifier


class AuditRecipientAddress(TypedDict):
    """Audit recipient selected by BACnet network and MAC address."""

    kind: Literal["address"]
    network_number: int
    mac_address: bytes


# A read of the Device's Audit_Notification_Recipient gives this form.
AuditRecipientInput = AuditRecipientDevice | AuditRecipientAddress


class Destination(TypedDict):
    """One Recipient_List destination (``BACnetDestination``) for
    ``add_notification_forwarder(recipients=...)``.

    Unknown keys, values outside BACnet's range and malformed time tuples
    raise ValueError, an integer outside its field's type (unsigned32 for
    ``process_identifier``, unsigned8 for the bit masks and time fields)
    OverflowError, and other wrong types TypeError. A key left out gives a
    destination active every day, all day, for every transition, with
    unconfirmed notifications. A read of Recipient_List gives each
    destination in this form, with every key present.
    """

    recipient: AuditRecipientInput
    process_identifier: int
    # Bit n is BACnetDaysOfWeek bit n: 1 << 0 is Monday, 1 << 6 Sunday. Default 0x7F.
    valid_days: NotRequired[int]
    # (hour, minute, second, hundredths); defaults (0, 0, 0, 0) and (23, 59, 59, 99).
    from_time: NotRequired[tuple[int, int, int, int]]
    to_time: NotRequired[tuple[int, int, int, int]]
    issue_confirmed_notifications: NotRequired[bool]
    # Bit 0 to-offnormal, bit 1 to-fault, bit 2 to-normal. Default 0x07.
    transitions: NotRequired[int]


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
    # BACnetLogStatus flags: 1 log-disabled, 2 buffer-purged, 4 log-interrupted.
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

    A read result (``read_property``, ``read_property_multiple``, a
    ``CovNotification`` value and ``BACnetServer.read_property``) keeps every
    element of the value; only broken framing raises:

    - A constructed property with a typed form reads typed. A whole read of
      a collection is a ``list`` of its elements, an indexed read one
      element, and a read of a single value that element, tagged with its
      production. Where the binding also takes the property as a typed
      value, the form is that value's. The properties and forms (see
      docs/python-api.md, "Typed constructed values"): Recipient_List
      (``"destination"``: a ``Destination`` with every key), Port_Filter
      (``"port_permission"``: ``(port_id, enabled)``), a Group's
      List_Of_Group_Members (``"read_access_specification"``:
      ``(object_id, [(property_id, array_index), ...])``) and Present_Value
      (``"read_access_result"``: a ``ReadAccessResult``), a Command's Action
      (``"action_list"``: a list of ``ActionCommand`` with every key), the
      object reference lists and Accompaniment
      (``"device_object_reference"``: an ``ObjectIdentifier``, or
      ``(device, object)``), Supported_Formats
      (``"authentication_factor_format"``), Authentication_Policy_List
      (``"authentication_policy"``: an ``AuthenticationPolicy``), Stages
      (``"stage_limit_value"``: ``(limit, values, deadband)``), Access Rights
      rules (``"access_rule"``:
      an ``AccessRule`` with every key), property reference lists
      (``"device_object_property_reference"``: a
      ``DeviceObjectPropertyReference`` with every key), a Global Group's
      Present_Value (``"property_access_result"``),
      Audit_Notification_Recipient (``"recipient"``: an
      ``AuditRecipientInput``), Weekly_Schedule (``"daily_schedule"``),
      Exception_Schedule (``"special_event"``), Effective_Period
      (``"date_range"``), Date_List (``"calendar_entry"``), timestamps
      (``"timestamp"``: a ``BACnetTimeStamp``), Active_COV_Subscriptions
      (``"cov_subscription"``), Device_Address_Binding
      (``"address_binding"``: ``device_identifier``, ``network_number`` and
      ``mac_address``), Value_Source and Value_Source_Array
      (``"value_source"``), and an Accumulator's Scale (``"scale"``: a
      ``float`` or an ``int``) and Prescale (``"prescale"``: ``(multiplier,
      modulo_divide)``). Tags elements are ``"name_value"`` mappings with
      ``"name": str`` and ``"value": None | PropertyValue``. Semantic tags
      have ``None``; valued NULL is ``PropertyValue.null()``. Date and Time
      are separate primitive values, not a combined pair. Whole Tags reads
      are lists, indexed reads one element, and index zero an unsigned count.
      Each element keeps its octets, so the value
      writes back unchanged. A value that isn't those elements, to the last
      octet, follows the rules below.
    - Other context-tagged content (a Load Control's shed levels, an Event
      Enrollment's Event_Parameters), or content this type has no form for
      (a UCS-4, DBCS or JIS string, bad UTF-8, an ENUMERATED past 32 bits),
      is ``application_data`` holding the octets exactly as served.
    - A whole read (no ``array_index``) of a property the stack's
      classification table marks as an array or list on that object type
      (every BACnetARRAY and BACnetLIST of the 2020 object tables) is a
      ``list`` at every length, 0 and 1 included.
    - Any other read is the bare value when it holds one element, and a
      ``list`` in wire order when it holds none or several (a date-time is
      a date and then a time). An indexed read is one element under these
      rules: ``Port_Filter[2]`` is a port_permission, ``Event_Time_Stamps[1]``
      a timestamp.

    Two values are equal when they have the same tag and value: numbers
    compare as numbers (``real(0.0) == real(-0.0)``, and a NaN equals
    nothing), a list item by item, and a typed read by its octets and
    element production. Equal values hash alike.

    ``copy.copy``, ``copy.deepcopy`` and ``pickle`` (every protocol) rebuild
    an equal value through the constructor its ``tag`` names, a list from
    its items as PropertyValues, and a typed constructed element from the
    octets it was read from.
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
        """Create a Date value, in the form ``.value`` reads it back.

        ``year`` is the full year, 1900..2154, or 255 for unspecified; any
        other year raises ValueError. Use 255 for any other unspecified
        field."""
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
        """Create a List (array) value from a list of PropertyValue items.
        Items that are all elements of one typed constructed collection (from
        indexed reads) make that collection, equal to its whole read. Lists
        nest at most 32 deep, the decoder's nesting limit; a deeper one
        raises ValueError."""
        ...
    @staticmethod
    def application_data(bytes: bytes) -> PropertyValue:
        """Create an ApplicationData value from pre-encoded application-layer bytes (e.g. a framed BACnetEventParameter)."""
        ...

    @property
    def tag(self) -> str:
        """Type tag: 'null', 'boolean', 'unsigned', 'signed', 'real', 'double',
        'octet_string', 'character_string', 'bit_string', 'enumerated',
        'date', 'time', 'object_identifier', 'list', 'application_data', or
        one typed constructed element: 'destination', 'port_permission',
        'read_access_specification', 'read_access_result', 'action_list',
        'device_object_reference', 'authentication_factor_format',
        'stage_limit_value', 'access_rule', 'device_object_property_reference',
        'property_access_result', 'recipient', 'daily_schedule',
        'special_event', 'calendar_entry', 'date_range', 'timestamp',
        'cov_subscription', 'value_source', 'scale', 'prescale',
        'address_binding'."""
        ...

    @property
    def value(self) -> Any:
        """The Python-native value (int, float, str, bytes, bool, dict, tuple,
        ObjectIdentifier, list, or None); ``application_data`` is ``bytes``,
        and a typed constructed element the form its typed write takes, or
        the form docs/python-api.md gives it. A date is ``(year, month, day,
        day_of_week)`` with the full year, or 255 for an unspecified year,
        as in every other date the binding reads."""
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
        """List of property value change entries: dicts with ``property_id``,
        ``array_index`` and ``value``. ``value`` is a ``PropertyValue`` shaped
        as a read result is, or ``bytes`` when the octets' framing is broken."""
        ...

    def __repr__(self) -> str: ...


class CovNotificationIterator:
    """Async iterator yielding ``CovNotification`` objects."""

    def __aiter__(self) -> CovNotificationIterator: ...
    def __anext__(self) -> Awaitable[CovNotification]: ...


# ---------------------------------------------------------------------------
# Exceptions
# ---------------------------------------------------------------------------

class BacnetError(Exception):
    """Base exception for all BACnet errors."""
    ...

class ObjectPropertyReference(TypedDict):
    """The object, property and array index a structured error names."""
    object_identifier: ObjectIdentifier
    property_identifier: PropertyIdentifier
    property_array_index: int | None


class BacnetProtocolError(BacnetError):
    """Raised on BACnet protocol-level errors (Error PDU).

    Some services answer with a structured error body that adds fields to the
    class and code. Each such attribute is None unless the device's error
    carried it.

    Attributes:
        error_class: The BACnet error class (integer).
        error_code: The BACnet error code (integer).
        first_failed_element_number: For an AddListElement or
            RemoveListElement ChangeList-Error, or a CreateObject-Error, the
            position (from 1) of the list element or initial value that
            failed, or 0 when the request failed for another reason. A
            server's write_local of a list whose object refuses one element
            sets it to that element's position in the list written.
        first_failed_write_attempt: For a WritePropertyMultiple-Error, the
            object, property and index of the first write that failed.
        first_failed_subscription: For a SubscribeCOVPropertyMultiple-Error
            about one COV reference, its object, property and index. None for
            a general failure of the request.
        vendor_id: For a ConfirmedPrivateTransfer-Error, the vendor of the
            private service.
        service_number: For a ConfirmedPrivateTransfer-Error, the private
            service number.
        error_parameters: For a ConfirmedPrivateTransfer-Error, the encoded
            vendor-defined error parameters, when the device sent any.
        vt_session_identifiers: For a VT-Close error that lists them, the
            local identifiers of the VT sessions that could not be closed.
    """
    error_class: int
    error_code: int
    first_failed_element_number: Optional[int]
    first_failed_write_attempt: Optional[ObjectPropertyReference]
    first_failed_subscription: Optional[ObjectPropertyReference]
    vendor_id: Optional[int]
    service_number: Optional[int]
    error_parameters: Optional[bytes]
    vt_session_identifiers: Optional[list[int]]

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

class BacnetLogNotAdvancingError(BacnetError):
    """Raised when a paged log read can't advance (#1530).

    The device answered a page by sequence number with records from before
    the one asked for, as bacnet-stack 1.6.1 does past its sequence wrap, or
    with none where its record counts say it holds that record. Read the log
    by position instead.

    Attributes:
        requested: The sequence number asked for.
        returned: What the device numbered the first record it sent; None
            when it sent none.
    """
    requested: int
    returned: int | None

class BacnetReadRangeViolationError(BacnetError):
    """Raised when a strict ``read_range`` refuses an answer that breaks a
    ReadRange rule (#1531). Read again with ``validation="lenient"`` to keep
    the answer and the rules it broke.

    Attributes:
        rule: The rule broken, in snake case, such as
            ``"zero_first_sequence_number"``.
    """
    rule: str

class BacnetTransportError(BacnetError, OSError):
    """Raised when a transport socket or I/O operation fails.

    Covers a failed bind or listen (B/IP, B/IPv6, ``ScHub.start``), a refused
    or failed dial, and other socket errors. It is also an ``OSError``:
    ``errno`` is the operating system's code when it reported one, else the
    code for the failure's kind (for example ``errno.EADDRINUSE``), else
    ``None``; ``strerror`` is the message. ``except OSError`` and
    ``except BacnetError`` both catch it. Unlike a bare ``OSError`` it is
    not narrowed to ``ConnectionRefusedError`` and the like, so test
    ``errno``.
    """
    errno: int | None
    strerror: str | None


# ---------------------------------------------------------------------------
# Serial ports
# ---------------------------------------------------------------------------

def decode_log_records(result: ReadRangeResult) -> list[dict[str, Any]]:
    """Decode the records of a ``read_range`` result of a log's Log_Buffer
    (#1534).

    Each record is a dict with ``timestamp`` as a ``(date, time)`` pair and
    ``datum``, a dict whose ``kind`` names the choice and whose key of the
    same name holds its value: ``log_status`` (bits: 0 log-disabled, 1
    buffer-purged, 2 log-interrupted), ``boolean``, ``real``,
    ``enumerated``, ``unsigned``, ``signed``, ``bitstring`` (``(unused_bits,
    bytes)``), ``null`` (no value), ``failure`` (``(error_class,
    error_code)``), ``time_change`` (seconds), ``any`` (the value's encoding),
    ``values`` (a Trend Log Multiple record's list of such dicts),
    ``notification`` (an Event Log record's event notification, its
    ``event_values`` as their encoding) or ``audit_notification`` (an Audit
    Log record, as ``audit_log_query`` shows it). A Trend Log record adds
    ``status_flags`` (bits: 0 in-alarm, 1 fault, 2 overridden, 3
    out-of-service), or None.

    Raises ValueError for a result that isn't a log's Log_Buffer, or whose
    item data doesn't decode as ``item_count`` records, naming the first
    record that fails by index and offset.
    """
    ...

def list_serial_ports() -> list[str]:
    """List the serial ports the operating system reports, as names to pass as
    ``serial_port=`` for ``transport="mstp"`` (``/dev/ttyUSB0``,
    ``/dev/cu.usbserial-1410``, ``COM3``).

    macOS lists them through IOKit, Windows through SetupAPI and the registry,
    and Linux from sysfs, so a port another program has open is listed too.
    An empty list means the system reports none. Raises OSError (or the
    subclass for the failure's kind) if the operating system can't be asked.
    """
    ...


# ---------------------------------------------------------------------------
# Client
# ---------------------------------------------------------------------------

# Private structural helpers used only by the installed stub; no runtime classes.
class _DeviceReadBatchResult(TypedDict):
    request_index: int
    device_instance: int
    value: PropertyValue | None
    error: BacnetError | None

class _DeviceRpmBatchResult(TypedDict):
    request_index: int
    device_instance: int
    results: list[ReadAccessResult] | None
    error: BacnetError | None

class _DeviceWriteBatchResult(TypedDict):
    request_index: int
    device_instance: int
    error: BacnetError | None


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
        share_port_by_address: bool = False,
        min_request_interval_ms: int = 0,
    ) -> None:
        """``share_port_by_address`` (B/IP only, default False): bind the
        interface address itself, so clients and devices on other addresses
        of this host can share the port. Needs an explicit interface and a
        nonzero port; broadcasts and unicast then arrive in no fixed order.

        ``min_request_interval_ms`` (default 0, no pacing; at most
        3,600,000, else ValueError) paces the confirmed requests to each
        destination: each waits until that long after the latest one sent to
        the same destination finished (reply, error or the caller giving up),
        or after it was sent while it is still outstanding, checking again
        when it wakes. Requests waiting together go one at a time, in no
        promised order. Paging a log or polling
        then leaves a slow device room for its other clients; requests to
        different destinations don't wait on each other. Behind a router the
        pause after a reply holds for requests made one after another only
        (#1535).
        """
        ...

    def __aenter__(self) -> Awaitable[BACnetClient]: ...
    def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> Awaitable[None]: ...

    # --- Property operations ---

    def read_property(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> Awaitable[PropertyValue]:
        """Read a single property from a remote device.

        ACK object/property/index must match. Device/Network Port instance 4194303
        requests accept a same-type concrete ACK. Malformed/mismatched ACKs raise
        BacnetError; this method returns the property value, not ACK metadata,
        shaped as ``PropertyValue`` describes for read results.
        """
        ...

    def write_property(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        value: PropertyValue,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
    ) -> Awaitable[None]:
        """Write a single property on a remote device.

        Priority is None or 1..16. Invalid u8 priorities raise ValueError
        synchronously; integers outside u8 raise OverflowError.
        """
        ...

    def read_property_multiple(
        self,
        address: str,
        specs: list[
            tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]
        ],
    ) -> Awaitable[list[ReadAccessResult]]:
        """Read multiple properties from a remote device (ReadPropertyMultiple).

        ``specs`` is a list of ``(object_id, [(property_id, array_index), ...])`` tuples.
        Returns ordered object/result dictionaries. Empty request/property lists
        are rejected; this standalone profile still permits property selectors.
        """
        ...

    def write_property_multiple(
        self,
        address: str,
        specs: list[
            tuple[
                ObjectIdentifier,
                list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]],
            ]
        ],
    ) -> Awaitable[None]:
        """Write multiple properties on a remote device (WritePropertyMultiple).

        ``specs`` is ``[(object_id, [(property_id, value, priority, array_index), ...]), ...]``.
        Empty request/property lists, ALL/REQUIRED/OPTIONAL targets and u8
        priorities outside 1..16 raise ValueError synchronously for the whole
        request. Priorities outside u8 raise OverflowError. None, index zero,
        proprietary properties, NULL and empty list values remain allowed.
        A device error raises BacnetProtocolError with
        first_failed_write_attempt set from its WritePropertyMultiple-Error.
        """
        ...

    # --- Multi-device batch operations ---

    def read_property_from_devices(
        self,
        requests: list[
            tuple[int, ObjectIdentifier, PropertyIdentifier, Optional[int]]
        ],
        max_concurrent: Optional[int] = None,
    ) -> Awaitable[list[_DeviceReadBatchResult]]:
        """Read a property from multiple discovered devices concurrently.

        ``requests`` is ``[(device_instance, object_id, property_id, array_index), ...]``.
        ``max_concurrent`` must be positive and fit the native usize; None uses 32.
        Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
        Returns an asyncio Future, accepted by await or asyncio.ensure_future,
        not a coroutine for asyncio.create_task. Results are returned in completion
        order; request_index is the zero-based
        input occurrence, including identical duplicates. Errors are BacnetError
        instances with their protocol attributes. Python construction failures
        raise from the whole call; cancellation returns no partial list.
        Returns ``[{"request_index": int, "device_instance": int, "value": PropertyValue | None, "error": BacnetError | None}, ...]``.
        """
        ...

    def read_property_multiple_from_devices(
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
    ) -> Awaitable[list[_DeviceRpmBatchResult]]:
        """Read multiple properties from multiple devices concurrently (RPM batch).

        ``max_concurrent`` must be positive and fit the native usize; None uses 32.
        Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
        Returns an asyncio Future, accepted by await or asyncio.ensure_future,
        not a coroutine for asyncio.create_task. Results are returned in completion
        order; request_index is the zero-based
        input occurrence, including identical duplicates. Errors are BacnetError
        instances with their protocol attributes. Python construction failures
        raise from the whole call; cancellation returns no partial list.
        Returns ``[{"request_index": int, "device_instance": int, "results": list[ReadAccessResult] | None, "error": BacnetError | None}, ...]``.
        """
        ...

    def write_property_to_devices(
        self,
        requests: list[
            tuple[int, ObjectIdentifier, PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]
        ],
        max_concurrent: Optional[int] = None,
    ) -> Awaitable[list[_DeviceWriteBatchResult]]:
        """Write a property to multiple devices concurrently.

        Every priority must be None or 1..16; invalid u8 priorities raise
        ValueError synchronously before any batch dispatch. Integers outside
        u8 raise OverflowError.

        ``requests`` is ``[(device_instance, object_id, property_id, value, priority, array_index), ...]``.
        ``max_concurrent`` must be positive and fit the native usize; None uses 32.
        Zero raises ValueError synchronously; out-of-range integers raise OverflowError.
        Returns an asyncio Future, accepted by await or asyncio.ensure_future,
        not a coroutine for asyncio.create_task. Results are returned in completion
        order; request_index is the zero-based
        input occurrence, including identical duplicates. Errors are BacnetError
        instances with their protocol attributes. Python construction failures
        raise from the whole call; cancellation returns no partial list.
        Returns ``[{"request_index": int, "device_instance": int, "error": BacnetError | None}, ...]``.
        """
        ...

    # --- Discovery ---

    def who_is(
        self,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> Awaitable[None]:
        """Broadcast a Who-Is request. Responses are collected asynchronously;
        use ``discovered_devices()`` to retrieve them.

        Give ``low_limit`` and ``high_limit`` together or not at all, each from
        0 to 4194303: one alone, ``low_limit`` above ``high_limit``, or a limit
        past 4194303 raises ``ValueError`` before anything is sent.
        """
        ...

    def discover(
        self,
        timeout_ms: int = 3000,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> Awaitable[list[DiscoveredDevice]]:
        """Convenience: send WhoIs, wait ``timeout_ms``, return discovered devices.

        Give ``low_limit`` and ``high_limit`` together or not at all, each from
        0 to 4194303: one alone, ``low_limit`` above ``high_limit``, or a limit
        past 4194303 raises ``ValueError`` before anything is sent.
        """
        ...

    def who_has_by_id(
        self,
        object_id: ObjectIdentifier,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> Awaitable[None]:
        """Broadcast Who-Has by object identifier.

        Give ``low_limit`` and ``high_limit`` together or not at all, each from
        0 to 4194303: one alone, ``low_limit`` above ``high_limit``, or a limit
        past 4194303 raises ``ValueError`` before anything is sent.
        """
        ...

    def who_has_by_name(
        self,
        name: str,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> Awaitable[None]:
        """Broadcast Who-Has by object name.

        Give ``low_limit`` and ``high_limit`` together or not at all, each from
        0 to 4194303: one alone, ``low_limit`` above ``high_limit``, or a limit
        past 4194303 raises ``ValueError`` before anything is sent.
        """
        ...

    def discovered_devices(self) -> Awaitable[list[DiscoveredDevice]]:
        """Return all devices discovered so far."""
        ...

    def get_device(self, instance: int) -> Awaitable[Optional[DiscoveredDevice]]:
        """Look up a discovered device by instance number."""
        ...

    def clear_devices(self) -> Awaitable[None]:
        """Clear the discovered device table."""
        ...

    def who_am_i(
        self,
        vendor_id: int,
        model_name: str,
        serial_number: str,
    ) -> Awaitable[None]:
        """Broadcast a Who-Am-I request announcing this device's identity.

        The three arguments should match the sender's Device object properties
        (``vendor_id`` is 0-65535). Raises ``ValueError`` if a string cannot be
        encoded, or ``OverflowError`` for a ``vendor_id`` that doesn't fit.
        """
        ...

    def who_is_directed(
        self,
        address: str,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
    ) -> Awaitable[None]:
        """Send a Who-Is to a specific device address (unicast).

        Give ``low_limit`` and ``high_limit`` together or not at all, each from
        0 to 4194303: one alone, ``low_limit`` above ``high_limit``, or a limit
        past 4194303 raises ``ValueError`` before anything is sent.
        """
        ...

    # --- Time synchronization ---

    def time_synchronization(
        self,
        address: str,
        date: tuple[int, int, int, int],
        time: tuple[int, int, int, int],
    ) -> Awaitable[None]:
        """Send a TimeSynchronization request (unconfirmed).

        ``date`` is ``(year, month, day, day_of_week)``; ``time`` is
        ``(hour, minute, second, hundredths)``. The request sets a clock, so
        both must be specific: a real day with the full year (1900..2154),
        month 1..12, day 1..31 and ``day_of_week`` that day's own weekday
        (1 = Monday), and every time field in range. A field that is
        ``UNSPECIFIED`` (255) or a pattern value (an odd month, the last
        day) raises ValueError before anything is sent.
        """
        ...

    def utc_time_synchronization(
        self,
        address: str,
        date: tuple[int, int, int, int],
        time: tuple[int, int, int, int],
    ) -> Awaitable[None]:
        """Send a UTCTimeSynchronization request (unconfirmed); arguments as
        for ``time_synchronization``."""
        ...

    # --- Auto-routing (by device instance) ---

    def read_property_from_device(
        self,
        device_instance: int,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> Awaitable[PropertyValue]:
        """Read a property from a device by instance number (auto-routing)."""
        ...

    def read_property_multiple_from_device(
        self,
        device_instance: int,
        specs: list[
            tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]
        ],
    ) -> Awaitable[Any]:
        """Read multiple properties from a device by instance number (auto-routing)."""
        ...

    def write_property_to_device(
        self,
        device_instance: int,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        value: PropertyValue,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
    ) -> Awaitable[None]:
        """Write a property on a device by instance number (auto-routing).

        Priority is None or 1..16. Invalid u8 priorities raise ValueError
        synchronously before lookup; integers outside u8 raise OverflowError.
        """
        ...

    def write_property_multiple_to_device(
        self,
        device_instance: int,
        specs: list[
            tuple[
                ObjectIdentifier,
                list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]],
            ]
        ],
    ) -> Awaitable[None]:
        """Write multiple properties to a device by instance number (auto-routing).

        Same whole-request validation as write_property_multiple, synchronously
        before discovery or dispatch: nonempty request/property lists, no special
        selectors, priority None or 1..16. Invalid u8 priorities/structure raise
        ValueError; priorities outside u8 raise OverflowError.
        """
        ...

    def add_device(
        self, device_instance: int, address: str
    ) -> Awaitable[None]:
        """Add a device to the discovery table manually (useful when address is known without WhoIs)."""
        ...

    # --- COV subscriptions ---

    def subscribe_cov(
        self,
        address: str,
        subscriber_process_identifier: int,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: bool,
        lifetime: Optional[int] = None,
    ) -> Awaitable[None]:
        """Subscribe to Change-of-Value notifications for an object.

        lifetime=None or 0 is indefinite; positive values are seconds.
        Use unsubscribe_cov for cancellation. confirmed always supplies the mode.
        """
        ...

    def unsubscribe_cov(
        self,
        address: str,
        subscriber_process_identifier: int,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Awaitable[None]:
        """Cancel a COV subscription."""
        ...

    def subscribe_cov_property_multiple(
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
    ) -> Awaitable[None]:
        """Subscribe to COV notifications for multiple properties on multiple objects.

        ``specs`` is ``[(object_id, [(property_id, array_index, cov_increment, timestamped), ...]), ...]``.
        ``issue_confirmed_notifications`` is required, including for cancellation requests.
        For subscriptions and re-subscriptions, ``lifetime`` and ``max_notification_delay`` are both required.
        For whole-context cancellations, pass an empty ``specs`` list and omit both fields.
        Complete timing and nested-reference validation raises ``ValueError`` synchronously,
        before an awaitable, address parsing, client-state access or I/O. Each supplied
        object needs references; ALL/REQUIRED/OPTIONAL selectors are prohibited and the
        cumulative reference limit is 10,000. Empty outer lists remain encodable with
        valid finite timing, without claiming bundled-server empty-context support.
        A device error about one COV reference raises BacnetProtocolError with
        first_failed_subscription set; a general failure leaves it None.
        """
        ...

    def cov_notifications(self) -> Awaitable[CovNotificationIterator]:
        """Get an async iterator for incoming COV notifications."""
        ...

    # --- Object management ---

    def delete_object(
        self, address: str, object_id: ObjectIdentifier
    ) -> Awaitable[None]:
        """Delete an object on a remote device (DeleteObject service)."""
        ...

    def create_object(
        self,
        address: str,
        object_specifier: Union[ObjectType, ObjectIdentifier],
        initial_values: Optional[
            list[tuple[PropertyIdentifier, PropertyValue, Optional[int], Optional[int]]]
        ] = None,
    ) -> Awaitable[bytes]:
        """Create an object on a remote device (CreateObject service).

        ``object_specifier`` is an ``ObjectType`` (server assigns instance) or
        ``ObjectIdentifier`` (specific instance). ``initial_values`` is
        ``[(property_id, value, priority, array_index), ...]``.
        Returns the raw CreateObject-ACK response bytes. A device error raises
        BacnetProtocolError with first_failed_element_number set when the
        device answers with a CreateObject-Error.
        """
        ...

    # --- Device management ---

    def device_communication_control(
        self,
        address: str,
        enable_disable: EnableDisable,
        time_duration: Optional[int] = None,
        password: Optional[str] = None,
    ) -> Awaitable[None]:
        """Send DeviceCommunicationControl to a remote device."""
        ...

    def reinitialize_device(
        self,
        address: str,
        reinitialized_state: ReinitializedState,
        password: Optional[str] = None,
    ) -> Awaitable[None]:
        """Send ReinitializeDevice to a remote device."""
        ...

    # --- Alarms and events ---

    def acknowledge_alarm_request(
        self,
        address: str,
        acknowledging_process_identifier: int,
        event_object_identifier: ObjectIdentifier,
        event_state_acknowledged: EventState,
        timestamp: BACnetTimeStamp,
        acknowledgment_source: str,
        time_of_acknowledgment: BACnetTimeStamp,
    ) -> Awaitable[None]:
        """Acknowledge an alarm with exact caller-supplied BACnetTimeStamp values.

        ``timestamp`` must echo the original event notification timestamp;
        ``time_of_acknowledgment`` is selected by the caller.
        """
        ...

    def acknowledge_alarm(
        self,
        address: str,
        acknowledging_process_identifier: int,
        event_object_identifier: ObjectIdentifier,
        event_state_acknowledged: EventState,
        acknowledgment_source: str,
    ) -> Awaitable[None]:
        """Deprecated compatibility method.

        This method fabricates sequence-number zero for both timestamps. Use
        ``acknowledge_alarm_request`` for a lossless request.
        """
        ...

    def get_event_information(
        self,
        address: str,
        last_received_object_identifier: Optional[ObjectIdentifier] = None,
    ) -> Awaitable[bytes]:
        """Get event information from a remote device. Returns raw response bytes."""
        ...

    def get_alarm_summary(self, address: str) -> Awaitable[list[dict[str, Any]]]:
        """Get alarm summary from a remote device.

        Returns ``[{"object_id": ObjectIdentifier, "alarm_state": EventState,
        "acknowledged_transitions": {"unused_bits": int, "data": bytes}}, ...]``.
        """
        ...

    def get_enrollment_summary(
        self,
        address: str,
        acknowledgment_filter: AcknowledgmentFilter = ...,
        event_state_filter: Optional[EnrollmentSummaryEventStateFilter] = None,
        event_type_filter: Optional[EventType] = None,
        min_priority: Optional[int] = None,
        max_priority: Optional[int] = None,
        notification_class_filter: Optional[int] = None,
    ) -> Awaitable[list[dict[str, Any]]]:
        """Get enrollment summary from a remote device.

        Returns ``[{"object_id": ObjectIdentifier, "event_type": EventType,
        "event_state": EventState, "priority": int,
        "notification_class": Optional[int]}, ...]``.
        """
        ...

    # --- Range operations ---

    def read_range(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
        range_type: Optional[Literal["position", "sequence", "time"]] = None,
        reference_index: Optional[int] = None,
        reference_seq: Optional[int] = None,
        count: Optional[int] = None,
        *,
        reference_time: ReferenceTime | None = None,
        validation: Literal["strict", "lenient"] = "strict",
    ) -> Awaitable[ReadRangeResult]:
        """Read a range of items from a list or log object.

        ``range_type`` is ``"position"``, ``"sequence"``, ``"time"`` or
        ``None`` (all-items). ``"time"`` reads the ``count`` items newer than
        ``reference_time`` (older, for a negative count): a naive
        ``datetime.datetime`` in the device's local time, or a
        ``((year, month, day, day_of_week), (hour, minute, second,
        hundredths))`` pair; an aware datetime raises ValueError, since the
        device's zone isn't known here (#1533). Invalid selectors, array index
        zero, missing or zero counts and a time that isn't specific raise
        ValueError before I/O; a count outside INTEGER16 (-32768..=32767)
        raises OverflowError before address parsing or I/O (#1360).
        Position/sequence reference zero is valid; omitted references default
        to zero.

        ``validation="strict"`` refuses an answer that breaks a ReadRange rule
        with BacnetReadRangeViolationError, whose ``rule`` names it;
        ``"lenient"`` keeps it and lists the rules in ``violations`` (#1531).
        Decode a log's records with ``decode_log_records``.
        """
        ...

    def read_log_page(
        self,
        address: str,
        object_id: ObjectIdentifier,
        cursor: LogCursor = None,
        page_size: int = 100,
    ) -> Awaitable[LogPage]:
        """Read one page of a log's Log_Buffer from ``cursor`` (#1530).

        ``cursor`` is ``None`` or ``"oldest"`` (the oldest record, found from
        Record_Count and Total_Record_Count), ``("sequence", n)``,
        ``("position", n)`` or ``("time", reference_time)``; lists work
        too, so a checkpoint survives JSON. Loop on ``page["next"]`` until
        ``page["done"]``, then keep ``next`` as the checkpoint. One request
        is outstanding at a time. A first record past the one asked for sets
        ``gap``; when the log drops the record asked for, the read starts
        over from the oldest. A page answered from before the one asked for,
        or none where the counts say records are, raises
        BacnetLogNotAdvancingError: such a device's sequence numbers are
        inconsistent, so read it from ``("position", 1)``. ``wrapped`` marks
        a page that reached the top of the sequence range. A page whose
        records don't decode raises BacnetError, dropping the records before
        the failing one; an answer that breaks a rule the reader can't
        tolerate, such as an echo that doesn't match, raises
        BacnetReadRangeViolationError. A non-log object or a ``page_size``
        outside 1..=32767 raises ValueError before I/O.
        """
        ...

    # --- File operations ---

    def atomic_read_file(
        self,
        address: str,
        file_identifier: ObjectIdentifier,
        access_method: str,
        start_position: int = 0,
        requested_octet_count: int = 0,
        start_record: int = 0,
        requested_record_count: int = 0,
    ) -> Awaitable[bytes]:
        """Read from a file object. ``access_method`` is ``"stream"`` or ``"record"``.
        Returns raw response bytes."""
        ...

    def atomic_write_file(
        self,
        address: str,
        file_identifier: ObjectIdentifier,
        access_method: str,
        start_position: int = 0,
        file_data: bytes = b"",
        start_record: int = 0,
        record_count: int = 0,
        file_record_data: Optional[list[bytes]] = None,
    ) -> Awaitable[bytes]:
        """Write to a file object. ``access_method`` is ``"stream"`` or ``"record"``.
        Returns raw response bytes."""
        ...

    # --- List manipulation ---

    def add_list_element(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        list_of_elements: bytes,
        array_index: Optional[int] = None,
    ) -> Awaitable[None]:
        """Add elements to a list property (AddListElement service).
        Requires nonempty, completely framed element bytes and a nonzero optional
        index. Invalid framing/index zero raises ValueError synchronously before
        client access; indexes outside u32 raise OverflowError. Empty-valued,
        constructed/context/vendor elements are retained without remote datatype
        validation. Shared limits: 1 MiB per tag, 32 context levels including the
        service wrapper; application Boolean has no payload bytes. A device
        error raises BacnetProtocolError with first_failed_element_number set
        when the device answers with a ChangeList-Error.
        """
        ...

    def remove_list_element(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        list_of_elements: bytes,
        array_index: Optional[int] = None,
    ) -> Awaitable[None]:
        """Remove elements from a list property (RemoveListElement service).
        Requires nonempty, completely framed element bytes and a nonzero optional
        index. Invalid framing/index zero raises ValueError synchronously before
        client access; indexes outside u32 raise OverflowError. Empty-valued,
        constructed/context/vendor elements are retained without remote datatype
        validation. Shared limits: 1 MiB per tag, 32 context levels including the
        service wrapper; application Boolean has no payload bytes. A device
        error raises BacnetProtocolError with first_failed_element_number set
        when the device answers with a ChangeList-Error.
        """
        ...

    # --- Private transfer ---

    def confirmed_private_transfer(
        self,
        address: str,
        vendor_id: int,
        service_number: int,
        service_parameters: Optional[bytes] = None,
    ) -> Awaitable[dict[str, Any]]:
        """Send a ConfirmedPrivateTransfer request.

        Returns ``{"vendor_id": int, "service_number": int, "result_block": bytes | None}``.
        A device error raises BacnetProtocolError with vendor_id,
        service_number and error_parameters set from its
        ConfirmedPrivateTransfer-Error. An ACK that is malformed, cut short
        or has octets after its last member raises BacnetError.
        """
        ...

    def unconfirmed_private_transfer(
        self,
        address: str,
        vendor_id: int,
        service_number: int,
        service_parameters: Optional[bytes] = None,
    ) -> Awaitable[None]:
        """Send an UnconfirmedPrivateTransfer request."""
        ...

    # --- Text messaging ---

    def confirmed_text_message(
        self,
        address: str,
        source_device: ObjectIdentifier,
        message_priority: MessagePriority,
        message: str,
        message_class_type: Optional[str] = None,
        message_class_value: Optional[Any] = None,
    ) -> Awaitable[None]:
        """Send a ConfirmedTextMessage.

        ``message_class_type`` is ``"numeric"`` or ``"text"`` (or ``None`` for no class).
        """
        ...

    def unconfirmed_text_message(
        self,
        address: str,
        source_device: ObjectIdentifier,
        message_priority: MessagePriority,
        message: str,
        message_class_type: Optional[str] = None,
        message_class_value: Optional[Any] = None,
    ) -> Awaitable[None]:
        """Send an UnconfirmedTextMessage."""
        ...

    # --- Life safety ---

    def life_safety_operation(
        self,
        address: str,
        requesting_process_identifier: int,
        requesting_source: str,
        operation: LifeSafetyOperation,
        object_identifier: Optional[ObjectIdentifier] = None,
    ) -> Awaitable[None]:
        """Send a LifeSafetyOperation request."""
        ...

    # --- WriteGroup ---

    def write_group(
        self,
        address: Optional[str],
        group_number: int,
        write_priority: int,
        change_list: list[tuple[int, Optional[int], Union[PropertyValue, bytes, bytearray]]],
        inhibit_delay: Optional[bool] = None,
        *,
        network: Optional[int] = None,
    ) -> Awaitable[None]:
        """Send a WriteGroup request (unconfirmed).

        With an ``address`` the request goes to that one device. With
        ``address=None`` it is broadcast: on the local network, or with
        ``network`` on that remote network (1-65534), or on every network when
        ``network`` is 65535. Give ``network`` only with ``address=None``.

        ``group_number`` is 1-4294967295 (group 0 is reserved) and
        ``write_priority`` is 1-16. ``change_list`` is a non-empty list of
        ``(channel, override_priority_or_none, value)`` tuples, where
        ``channel`` is a channel number 0-65535, the override priority is 1-16 or
        ``None``, and ``value`` is a ``PropertyValue``, which is encoded for you,
        or a ``bytes`` or ``bytearray`` holding one encoded BACnetChannelValue.
        Either way the value must be one BACnetChannelValue: a single
        application-tagged primitive (``PropertyValue.real(72.0)``), or a
        context-0 lighting command, context-1 xy colour or context-2 colour
        command (as ``bytes``, or the same octets in
        ``PropertyValue.application_data``), with no wrapper tag. Raises
        ``ValueError`` for anything else, a list for one, ``TypeError`` for a
        value of another type (a list of ints included), and ``OverflowError``
        for integers that don't fit.
        """
        ...

    # --- Virtual terminal ---

    def vt_open(
        self,
        address: str,
        vt_class: VTClass,
        local_vt_session_identifier: int,
    ) -> Awaitable[int]:
        """Open a virtual terminal session. Returns the remote session identifier.

        ``local_vt_session_identifier`` (0-255) is the caller's own number for
        the session, which the peer quotes when it sends data back.
        """
        ...

    def vt_close(self, address: str, session_ids: list[int]) -> Awaitable[None]:
        """Close one or more virtual terminal sessions.

        ``session_ids`` must not be empty and each identifier is 0-255. Raises
        ``ValueError`` for an empty list, or ``OverflowError`` for an integer that
        doesn't fit, before anything is sent. A device error that lists the
        sessions it could not close raises BacnetProtocolError with
        vt_session_identifiers set.
        """
        ...

    def vt_data(
        self,
        address: str,
        session_id: int,
        data: bytes,
        data_flag: bool,
    ) -> Awaitable[dict[str, Any]]:
        """Send data over a virtual terminal session.

        ``data_flag`` is the sequence flag that alternates on each new request
        for a session; it is sent as an Unsigned 0 or 1.

        Returns ``{"all_new_data_accepted": bool, "accepted_octet_count": int | None}``;
        the count is an ``int`` only when ``all_new_data_accepted`` is ``False``.
        """
        ...

    # --- Audit ---

    def confirmed_audit_notification_typed(
        self,
        address: str,
        request: AuditNotificationRequestInput,
    ) -> Awaitable[None]:
        """Send a validated mapping through the native confirmed Audit helper."""
        ...

    def unconfirmed_audit_notification_typed(
        self,
        address: str,
        request: AuditNotificationRequestInput,
    ) -> Awaitable[None]:
        """Send a validated mapping through the native unconfirmed Audit helper."""
        ...

    def audit_log_query_typed(
        self,
        address: str,
        request: AuditLogQueryRequestInput,
    ) -> Awaitable[AuditLogQueryAck]:
        """Send a typed Audit Log query and return a canonical decoded mapping."""
        ...

    def confirmed_audit_notification(
        self, address: str, service_data: bytes
    ) -> Awaitable[None]:
        """Send a pre-encoded Clause 21 ConfirmedAuditNotification payload.

        This is a raw escape hatch; the bundled server does not execute the
        service.
        """
        ...

    def unconfirmed_audit_notification(
        self, address: str, service_data: bytes
    ) -> Awaitable[None]:
        """Send a pre-encoded Clause 21 UnconfirmedAuditNotification payload.

        This is a raw escape hatch; the bundled server does not execute the
        service.
        """
        ...

    def audit_log_query(
        self,
        address: str,
        service_data: bytes,
    ) -> Awaitable[bytes]:
        """Send a pre-encoded Clause 21 AuditLogQuery payload.

        This raw escape hatch returns the peer's response payload. The bundled
        server does not execute the service.
        """
        ...

    # --- Lifecycle ---

    def stop(self) -> Awaitable[None]:
        """Explicitly stop the client and release resources."""
        ...


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------

class CovCounters(TypedDict):
    """COV telemetry, zero at each start; fields are sampled one by one, not atomically."""
    subscriptions_active: int
    subscriptions_created: int
    subscriptions_rejected_quota: int
    subscriptions_rejected_capacity: int
    subscriptions_rejected_indefinite: int
    subscriptions_cancelled: int
    subscriptions_purged: int
    notifications_sent: int
    notifications_confirmed: int
    notifications_unconfirmed: int
    notification_bytes_sent: int
    notifications_throttled_fanout: int
    notifications_throttled_peer: int
    timed_changes_dropped: int
    untimed_references_oversized: int

class CovPolicy(TypedDict, total=False):
    """BACnetServer(cov_policy=...) COV limits, checked at construction; omitted keys keep their defaults."""
    max_subscriptions_global: int
    max_subscriptions_per_peer: int
    reserved_capacity: int
    reserved_peers: list[bytes]
    reserved_recipients: list[tuple[int | None, bytes]]
    allow_indefinite_subscriptions: bool
    max_indefinite_per_peer: int
    max_notifications_per_event: int
    max_notification_bytes_per_event: int
    max_confirmed_in_flight_per_peer: int

class TimeSyncPolicy(TypedDict, total=False):
    """BACnetServer(time_sync_policy=...) inbound clock limits, checked at construction; omitted keys keep their defaults."""
    enabled: bool
    source_restriction: list[tuple[int | None, bytes]] | None
    max_step_ms: int | None
    per_source_rate: tuple[float, int] | None
    global_rate: tuple[float, int] | None
    coalesce_window_ms: int
    global_coalesce_window_ms: int
    max_sources: int

class ForwarderSaveCounters(TypedDict):
    """One Notification Forwarder's Subscribed_Recipients save totals, saturating at 2**64-1."""
    failed_saves: int

class EventNotificationCounters(TypedDict):
    """Undelivered event notifications, zero at each start; independent u64 totals, saturating at 2**64-1."""
    notification_class_missing: int
    recipient_list_unavailable: int
    recipient_list_invalid: int
    recipient_list_too_long: int
    device_recipient_unbound: int
    recipient_unroutable: int
    confirmed_broadcast_recipient: int
    confirmed_no_invoke_id: int
    confirmed_rejected: int
    confirmed_unanswered: int
    unconfirmed_send_failed: int
    apdu_too_large: int
    received_not_forwarded: int
    forwarding_cap_dropped: int
    received_not_logged: int

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

    Segmentation defaults to NONE. When enabled, apdu_segment_timeout_ms must
    be positive; it supplies Device APDU_SEGMENT_TIMEOUT, response SegmentACK
    waits, and the 4*Tseg receive inactivity timer. A separate 16-second
    no-progress resource limit aborts abandoned request transfers with OTHER.

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
        mutation_policy: Literal["permissive", "deny_all"] = "permissive",
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
        registered_network_port: Optional[int] = None,
        cov_policy: CovPolicy | None = None,
        time_sync_policy: TimeSyncPolicy | None = None,
        share_port_by_address: bool = False,
        segmentation_supported: Segmentation = Segmentation.NONE,
        apdu_segment_timeout_ms: int = 5000,
    ) -> None:
        """``share_port_by_address`` (B/IP only, default False): bind the
        interface address itself, so devices on other addresses of this host
        can share the port. Needs an explicit interface and a nonzero port;
        broadcasts and unicast then arrive in no fixed order."""
        ...

    # --- Analog objects ---
    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_output(self, instance: int, name: str, units: int = 62) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional AV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...

    # --- Binary objects ---
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_output(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional BV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...

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
    def add_notification_class(
        self,
        instance: int,
        name: str,
        notification_class: int = 0,
        storage_path: Optional[str] = None,
        *,
        recipients: Optional[list[Destination]] = None,
    ) -> None:
        """Add a Notification Class (Clause 12.21). With ``storage_path``, a
        Recipient_List a client writes is kept in that file across restarts; a
        write whose list cannot be saved is refused with DEVICE /
        OPERATIONAL_PROBLEM and the old list stays. Without it the list lives in
        memory only.

        ``storage_path`` is a ``str``; a ``pathlib.Path`` raises TypeError, as
        for ``add_notification_forwarder``. Give each class a file of its own:
        the file names the class it belongs to, so one that holds another
        object's list, or anything this backend did not write, makes this call
        raise BacnetError (BacnetProtocolError for a saved list a client's write
        would be refused).

        ``recipients`` seeds Recipient_List with ``Destination`` mappings, in
        order, with the checks and errors ``add_notification_forwarder``'s
        ``recipients`` has: more than 32, or an address MAC past 18 octets,
        raises BacnetProtocolError, and nothing is registered. With
        ``storage_path``, a Recipient_List a client wrote, once saved, wins:
        until a write sets the list, the seed applies at every start and is
        not saved."""
    def add_notification_forwarder(
        self,
        instance: int,
        name: str,
        process_identifier_filter: Optional[int] = None,
        local_forwarding_only: bool = False,
        storage_path: Optional[str] = None,
        *,
        recipients: list[Destination] | None = None,
        port_filter: list[tuple[int, bool]] | None = None,
    ) -> None:
        """Add a Notification Forwarder (Clause 12.51) that sends the event
        notifications this server receives on to its Recipient_List and
        Subscribed_Recipients. ``process_identifier_filter=None`` forwards every
        process identifier. With ``storage_path``, Recipient_List and
        Subscribed_Recipients are kept in that file across restarts.

        ``recipients`` seeds Recipient_List in order; more than 32 destinations,
        or an address MAC past 18 octets, raises BacnetProtocolError, as a
        client's write would be refused. With ``storage_path``, a
        Recipient_List a client wrote, once saved, wins over the seed. ``port_filter`` serves Port_Filter as
        ``(port_id, enabled)`` pairs; the server receives through Port_ID 0.
        Without it Port_Filter is absent."""
    def add_trend_log(
        self,
        instance: int,
        name: str,
        buffer_size: int = 100,
        *,
        total_record_count: int = 0,
    ) -> None:
        """Add a Trend Log (Clause 12.25) to the server (before starting).

        Log_DeviceObjectProperty reads as ``application_data`` holding the
        context-tagged reference; while unset it reads as Analog Input
        4194303's Present_Value, and writing a reference to instance 4194303
        unsets it. A null written to it succeeds and changes nothing.

        ``total_record_count`` seeds Total_Record_Count, an Unsigned32, so
        the log's first record is numbered one past it: seeded at
        4294967294, records number 4294967295, then 1, 2 and on, which lets
        a reader's handling of the wrap be tested. Out of range raises
        OverflowError.
        """
        ...
    def add_trend_log_multiple(
        self,
        instance: int,
        name: str,
        buffer_size: int = 100,
        *,
        members: Optional[
            list[
                tuple[ObjectIdentifier, PropertyIdentifier]
                | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]]
                | DeviceObjectPropertyReference
            ]
        ] = None,
        log_interval: int | None = None,
        logging_type: Literal["polled", "triggered"] | None = None,
        start_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
        stop_time: tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None = None,
        align_intervals: bool | None = None,
        interval_offset: int | None = None,
        total_record_count: int = 0,
    ) -> None:
        """Add a Trend Log Multiple (Clause 12.30) that the server polls or
        triggers, logging one value per member in each record.

        ``members`` fills Log_DeviceObjectProperty in order, each an
        ``(object, property)`` or ``(object, property, array_index)`` tuple
        or a ``DeviceObjectPropertyReference`` mapping; one naming this
        server's Device is kept in its local form. More than 64 raises
        BacnetProtocolError (NO_SPACE_TO_WRITE_PROPERTY), and a
        ``device_identifier`` that isn't a Device raises ValueError.
        ``log_interval`` is in hundredths of a second. ``logging_type``
        ``"polled"`` with no ``log_interval`` takes a one-minute interval;
        ``"triggered"`` zeroes Log_Interval and makes it read-only, so a
        ``log_interval`` with it raises BacnetProtocolError
        (WRITE_ACCESS_DENIED). ``"cov"`` raises BacnetProtocolError
        (VALUE_OUT_OF_RANGE), as a client's write of COV does; any other
        string raises ValueError. With neither, Log_Interval stays 0 and
        nothing is polled.

        ``start_time`` and ``stop_time`` bound when records are kept, each a
        ``((full_year, month, day, day_of_week), (hour, minute, second,
        hundredths))`` pair: every field 255 leaves that side open, 255
        seconds or hundredths count as zero, and anything else that isn't an
        actual date and time raises BacnetProtocolError (VALUE_OUT_OF_RANGE). Records are kept from the
        start up to, not including, the stop, and each opening and closing
        is logged. ``align_intervals`` aligns a polled log's acquisitions to
        the clock when Log_Interval divides a day, shifted by
        ``interval_offset`` hundredths (modulo Log_Interval).

        Peers can write each of these. To ask a triggered log for one
        record, write Trigger TRUE, from a peer or with
        ``write_property_local``; it reads TRUE until the poller acquires
        the record. Read the records with ``read_range``.

        ``total_record_count`` seeds Total_Record_Count as on
        ``add_trend_log``.
        """
        ...
    def add_event_log(
        self,
        instance: int,
        name: str,
        buffer_size: int = 100,
        log_received_notifications: bool = False,
        *,
        total_record_count: int = 0,
    ) -> None:
        """Register an Event Log of ``buffer_size`` records before start().

        The log records each event notification the server builds. With
        ``log_received_notifications`` it also records the Confirmed and
        UnconfirmedEventNotifications the server receives, unicast or
        broadcast, as they decoded; each source is held to a few records a
        second, and the rest show in ``event_notification_counters()`` as
        ``received_not_logged``.

        The log reports BUFFER_READY to its Notification Class once its
        Notification_Threshold is set (zero, the default, reports nothing);
        write it and Notification_Class with ``write_property_local`` or
        from a peer.

        ``total_record_count`` seeds Total_Record_Count as on
        ``add_trend_log``; BUFFER_READY counts from the seeded value.
        """
        ...
    def add_audit_log(self, instance: int, name: str, storage_path: str, buffer_size: int = 100) -> None: ...
    def add_device_binding(self, device_instance: int, address: str) -> None:
        """Register a direct B/IP (IPv4) Device binding on a transport="bip" server.

        Accept IPv4:port or exactly six hex bytes (four IPv4 octets and two port octets).
        Other transports or parsed address lengths raise ValueError without retention.
        The shared parser's broader grammar remains available to other APIs, not bindings.
        Invalid instance/address or duplicate Device raises ValueError. No overwrite,
        routing or discovery. Syntax validation precedes the frozen-state check; after
        start() consumes configuration, parseable calls raise RuntimeError before the
        transport/shape checks, including after stop. start() refuses a binding at a
        broadcast or other group address of the link (a multicast address,
        255.255.255.255, or the broadcast IP at another port), raising BacnetError
        naming the device and the address; bind each device at its unicast address.
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
    def configure_audit_reporters(self, reporters: list[AuditReporterConfiguration]) -> None:
        """Replace the complete target Reporter set before start (one through 64).

        Each configuration identifies a registered concrete Reporter (0..4194302).
        Invalid instance values (including bool and non-int), duplicate identities,
        invalid settings or missing objects raise ValueError. Wrong container types
        and wrong types for other setting fields raise TypeError. The entire input
        is copied and validated before pending changes. Startup freezes this API.
        Omitted optional fields reset to catch-all selectors, all priorities and
        absent delay/control properties. maximum_send_delay accepts None or an
        integer 0..3600: zero exposes immediate Maximum_Send_Delay/Send_Now;
        positive values enable bounded ordinary target batching. Bool/non-integer
        delay raises TypeError; an integer outside unsigned32 raises
        OverflowError and one past 3600 ValueError. An ``instance``,
        ``auditable_operations`` or ``audit_priority_filter`` outside its
        integer type (unsigned32, unsigned64, unsigned16) raises OverflowError
        too.
        Enabled nominal overlaps expose CONFIGURATION_ERROR on every affected
        Reporter's RELIABILITY; the lowest instance emits, before operation filters.
        Empty/all-None selectors select no nominal targets. Mandatory Reporter
        writes and Device recipient changes have separate bounded capture rules.
        All Reporters use the one provisioned Device recipient and share the same
        64-operation admission budget. Source reporting remains exactly one.
        """
        ...

    # --- Control/PID ---
    def add_loop(
        self,
        instance: int,
        name: str,
        output_units: int = 62,
        *,
        controlled_variable_units: Optional[int] = None,
        proportional_constant_units: Optional[int] = None,
        integral_constant_units: Optional[int] = None,
        derivative_constant_units: Optional[int] = None,
        priority_for_writing: Optional[int] = None,
    ) -> None:
        """Add a Loop (PID) object to the server (before starting).

        The keyword arguments set rows that are read-only over the network:
        Controlled_Variable_Units and the three gain units rows (NO_UNITS when
        omitted) and Priority_For_Writing (16 when omitted). Units above 65535
        or a priority outside 1..=16 raise BacnetProtocolError with
        VALUE_OUT_OF_RANGE; a priority outside 0..=255 raises OverflowError.
        Peers can write Action (DIRECT until written).

        Controlled_Variable_Reference and Manipulated_Variable_Reference read
        as ``application_data`` holding the context-tagged reference; while
        unset they name instance 4194303 (an Analog Input and an Analog Output
        respectively), and writing a reference to that instance unsets them.
        A null written to any Loop reference succeeds and changes nothing.
        """
        ...
    def add_command(
        self,
        instance: int,
        name: str,
        *,
        action: Optional[list[list[ActionCommand]]] = None,
        action_text: Optional[list[str]] = None,
    ) -> None:
        """Add a Command object; writing N to its Present_Value runs action[N-1].

        ``action_text`` serves Action_Text and needs one text per list. A
        command whose ``device_identifier`` isn't a Device raises ValueError.
        A priority outside 1..=16, a value with no encoding, or a text count
        that differs from the list count raises BacnetProtocolError with
        VALUE_OUT_OF_RANGE.
        """
        ...
    def add_timer(self, instance: int, name: str) -> None: ...
    def add_load_control(self, instance: int, name: str) -> None: ...
    def add_program(self, instance: int, name: str) -> None: ...

    # --- Lighting ---
    def add_lighting_output(self, instance: int, name: str) -> None:
        """Add a Lighting Output object.

        Its Lighting_Command reads and writes as ``application_data`` holding
        the context-tagged BACnetLightingCommand, operation NONE
        (``b"\\x09\\x00"``) until written. An ``octet_string`` or any other
        datatype is refused with INVALID_DATA_TYPE, and a command its operation
        can't take with VALUE_OUT_OF_RANGE. The object carries each command
        out: a fade or ramp puts its level in Present_Value at once and moves
        Tracking_Value there over time, with In_Progress showing FADE_ACTIVE
        or RAMP_ACTIVE; a step changes the level at once; and with
        Blink_Warn_Enable TRUE the warn commands hold the level for
        Egress_Time seconds before relinquishing or turning it off.

        A Present_Value or Relinquish_Default level above 0.0 and below 1.0
        is stored as 1.0, and one outside 0.0 to 100.0 is refused with
        VALUE_OUT_OF_RANGE, but for Present_Value's warn values -1.0 (WARN),
        -2.0 (WARN_RELINQUISH) and -3.0 (WARN_OFF). Tracking_Value reads the
        same level as Present_Value whenever no fade or ramp is running.
        """
        ...
    def add_binary_lighting_output(self, instance: int, name: str) -> None: ...
    def add_channel(
        self,
        instance: int,
        name: str,
        channel_number: int,
        members: Optional[
            list[
                tuple[ObjectIdentifier, PropertyIdentifier]
                | tuple[ObjectIdentifier, PropertyIdentifier, Optional[int]]
                | DeviceObjectPropertyReference
            ]
        ] = None,
        execution_delay: Optional[list[int]] = None,
        control_groups: Optional[list[int]] = None,
        *,
        allow_group_delay_inhibit: bool = False,
    ) -> None:
        """Add a Channel; a write of its Present_Value is passed on to ``members``.

        ``channel_number`` (0..=65535) is the number a WriteGroup names. Each
        member is an ``(object, property)`` or ``(object, property,
        array_index)`` tuple for a property in this device, or a
        ``DeviceObjectPropertyReference`` mapping; one naming this server's
        Device is kept in its local form, and one naming another Device is
        written there through the server's device bindings
        (``add_device_binding`` or a heard I-Am). ``execution_delay`` holds
        one delay in milliseconds per member (zeros when omitted),
        ``control_groups`` the WriteGroup groups the Channel is in (``[0]``,
        none, when omitted), and ``allow_group_delay_inhibit`` lets a
        WriteGroup that asks for no delays skip them. All of them are
        writable over the network too.

        A wrong shape or type raises TypeError. An unknown or missing mapping
        key or a device that isn't a Device raises ValueError. A channel
        number outside unsigned16, or an index (a tuple's or a mapping's), a
        delay or a group outside unsigned32, raises OverflowError. A delay
        count that differs from the member count or an empty group list
        raises BacnetProtocolError with VALUE_OUT_OF_RANGE; more than
        1024 members or 64 groups, NO_SPACE_TO_WRITE_PROPERTY. Nothing is
        registered after any of them.
        """
        ...

    # --- Life safety ---
    def add_life_safety_point(self, instance: int, name: str) -> None: ...
    def add_life_safety_zone(self, instance: int, name: str) -> None: ...

    # --- Grouping/organization ---
    def add_group(
        self,
        instance: int,
        name: str,
        members: Optional[
            list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]
        ] = None,
    ) -> None:
        """Group whose Present_Value is rebuilt from ``members`` on each read.

        ``members`` has the ``read_property_multiple`` spec shape and the
        endpoint ``add_group`` checks: a member with no properties, or one
        reporting a group's Present_Value, raises ValueError naming its
        position and the rule. Any property identifier is taken, those ASHRAE
        assigns above 4194303 included. Indexes outside unsigned32 raise
        OverflowError.
        """
        ...
    def add_global_group(self, instance: int, name: str) -> None: ...
    def add_structured_view(self, instance: int, name: str) -> None: ...

    # --- Access control ---
    def add_access_door(
        self,
        instance: int,
        name: str,
        *,
        door_members: Optional[
            list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]
        ] = None,
        alarm_values: Optional[list[int]] = None,
        fault_values: Optional[list[int]] = None,
        masked_alarm_values: Optional[list[int]] = None,
    ) -> None:
        """Add an Access Door object to the server (before starting).

        ``door_members`` sets Door_Members, the objects that make up the door
        (read-only over the network). Each element is an ``ObjectIdentifier``
        in this device or a ``(device, object)`` pair of identifiers for one
        in another device. A pair whose device isn't a Device object
        identifier raises ValueError, and nothing is registered.

        ``alarm_values``, ``fault_values`` and ``masked_alarm_values`` set the
        starting Alarm_Values, Fault_Values and Masked_Alarm_Values, as
        BACnetDoorAlarmState numbers other than NORMAL (1 to 8, or 256 to
        65535); peers can write all three. Door_Alarm_State stays NORMAL or a member of the
        alarm or fault values, never a masked state: a simulated value
        outside them is refused, and masking the current state returns the
        door to NORMAL. An alarm value raises a CHANGE_OF_STATE alarm after
        Time_Delay, and a fault value makes Reliability MULTI_STATE_FAULT. Any
        other number, NORMAL (0) included, raises BacnetProtocolError with
        VALUE_OUT_OF_RANGE, and nothing is registered.
        """
        ...
    def add_access_credential(self, instance: int, name: str) -> None: ...
    def add_access_point(
        self,
        instance: int,
        name: str,
        *,
        access_doors: Optional[
            list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]
        ] = None,
        number_of_authentication_policies: Optional[int] = None,
        authentication_policies: Optional[list[tuple[str, AuthenticationPolicy]]] = None,
        supported_authorization_modes: Optional[list[int]] = None,
        priority_for_writing: Optional[int] = None,
    ) -> None:
        """Add an Access Point object to the server (before starting).

        ``access_doors`` sets Access_Doors (read-only over the network), in the
        element forms ``add_access_door`` takes for ``door_members``. A pair
        whose device isn't a Device object identifier raises ValueError, and a
        reference to anything but an Access Door raises BacnetProtocolError
        with VALUE_OUT_OF_RANGE; either way nothing is registered.

        ``number_of_authentication_policies`` (1 when omitted, never 0) and
        ``priority_for_writing`` (16 when omitted, else 1 to 16; outside
        0..=255 OverflowError) set Number_Of_Authentication_Policies and
        Priority_For_Writing, which are read-only over the network. ``supported_authorization_modes`` lists
        the BACnetAuthorizationMode numbers the application carries out, the
        values a write of Authorization_Mode can take: AUTHORIZE (0) alone
        when omitted, and AUTHORIZE must be in any list given; proprietary
        modes run from 64 to 65535. A value outside those raises
        BacnetProtocolError with VALUE_OUT_OF_RANGE. Peers write
        Active_Authentication_Policy (1 to the policy count) and
        Authorization_Mode (one of the supported modes).

        ``authentication_policies`` sets Authentication_Policy_List and
        Authentication_Policy_Names as ``(name, policy)`` pairs, both
        read-only over the network, and the policy count to their number. A
        policy's entries name the Credential Data Inputs and the step each
        serves, from 1. An empty list, or more than 256 pairs, raises
        BacnetProtocolError with VALUE_OUT_OF_RANGE; an index or timeout
        outside 0..=4294967295 raises OverflowError. A policy with no
        entries, a reference to anything but a Credential Data Input, or
        indexes that don't start at 1 and climb by at most one is kept but
        can't be in effect: while it is, Active_Authentication_Policy reads 0
        until a peer writes a usable policy. While any such policy is listed,
        or the active policy is 0, Reliability reads CONFIGURATION_ERROR and
        the point takes no access events. A
        ``number_of_authentication_policies`` given as well is applied after
        and resizes both arrays, new policies empty and new names ``""``; it
        can be at most 256 with the pairs (VALUE_OUT_OF_RANGE above).
        """
        ...
    def add_access_rights(
        self,
        instance: int,
        name: str,
        *,
        positive_access_rules: Optional[list[AccessRule]] = None,
        negative_access_rules: Optional[list[AccessRule]] = None,
        enable: bool = True,
        accompaniment: Optional[
            ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]
        ] = None,
        storage_path: Optional[str] = None,
    ) -> None:
        """Add an Access Rights object to the server (before starting).

        ``positive_access_rules`` and ``negative_access_rules`` set the two
        rule arrays as ``AccessRule`` mappings, and ``enable`` sets Enable
        (``PropertyIdentifier.LOG_ENABLE``, property 133), whose FALSE
        disables every rule. Peers can write all three over the network. A
        wrong shape or type raises TypeError, an unknown or missing key or a
        device that isn't a Device raises ValueError, and a location naming
        anything but an Access Point or Access Zone raises BacnetProtocolError
        with VALUE_OUT_OF_RANGE, and so does a list of more than 1024 rules
        (NO_SPACE_TO_WRITE_PROPERTY). Nothing is registered after any of
        them. The server stores and serves the rules but doesn't evaluate
        them.

        ``accompaniment`` serves the optional Accompaniment row: the Access
        Rights, Access Credential or Access User object that a second
        credential, presented with the first, has to match. It takes an
        ``ObjectIdentifier`` in this device or a ``(device, object)`` pair,
        as ``door_members`` does; instance 4194303 asks for no
        accompaniment. Left out, the object has no Accompaniment row and
        clients can't add one; once served, clients can write it. A pair
        whose device isn't a Device raises ValueError, and any other object
        type raises BacnetProtocolError with VALUE_OUT_OF_RANGE. A read
        gives the reference back in the form the keyword takes.

        With ``storage_path``, a rule array, Enable or Accompaniment that a
        client writes is kept in that file across restarts, and wins over the
        keyword given for it at the next start, which is then checked but not
        applied; a saved Accompaniment is served even without the keyword.
        To lift a saved requirement, write the no-accompaniment reference
        (instance 4194303), which keeps the row; to drop the row, remove the
        storage file, which also drops the saved rules and Enable. A
        write that cannot be saved is refused with DEVICE /
        OPERATIONAL_PROBLEM and the old value stays. Without it, written
        values live in memory only. ``storage_path`` is a ``str``; a
        ``pathlib.Path`` raises TypeError. Give each object a file of its
        own: one that holds another object's state, or anything this backend
        did not write, makes this call raise BacnetError
        (BacnetProtocolError for a saved rule or Accompaniment the object
        would refuse).
        """
        ...
    def add_access_user(
        self,
        instance: int,
        name: str,
        *,
        credentials: Optional[
            list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]
        ] = None,
        members: Optional[
            list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]
        ] = None,
        member_of: Optional[
            list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]
        ] = None,
    ) -> None:
        """Add an Access User object to the server (before starting).

        ``credentials`` sets Credentials, the Access Credential objects the
        user holds, and ``members`` and ``member_of`` set Members and
        Member_Of, the Access Users one level below and above this one in a
        hierarchy of users. All three are read-only over the network and
        take the element forms ``add_access_door`` takes for
        ``door_members``. A pair whose device isn't a Device object
        identifier raises ValueError, and a reference to anything but an
        Access Credential in ``credentials``, or an Access User in the other
        two, raises BacnetProtocolError with VALUE_OUT_OF_RANGE; either way
        nothing is registered. A read of a list gives the references back in
        the forms the keywords take, or ``[]`` while it is empty.
        """
        ...
    def add_access_zone(
        self,
        instance: int,
        name: str,
        *,
        entry_points: Optional[
            list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]
        ] = None,
        exit_points: Optional[
            list[ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]]
        ] = None,
        alarm_values: Optional[list[int]] = None,
    ) -> None:
        """Add an Access Zone object to the server (before starting).

        ``entry_points`` and ``exit_points`` set Entry_Points and Exit_Points,
        the Access Points leading into and out of the zone (read-only over
        the network), in the element forms ``add_access_door`` takes for
        ``door_members``. A pair whose device isn't a Device object
        identifier raises ValueError, and a reference to anything but an
        Access Point raises BacnetProtocolError with VALUE_OUT_OF_RANGE;
        either way nothing is registered. A read of either list gives the
        references back in the forms the keywords take.

        ``alarm_values`` sets the starting Alarm_Values, the occupancy states
        the zone reports with a CHANGE_OF_STATE alarm after Time_Delay, as
        BACnetAccessZoneOccupancyState numbers other than NORMAL (1 to 6, or
        64 to 65535); peers can write it too. Any other number, NORMAL (0)
        included, raises BacnetProtocolError with VALUE_OUT_OF_RANGE and
        ``first_failed_element_number`` naming it, from 1, and nothing is
        registered. Left out, the list starts empty.
        """
        ...
    def add_credential_data_input(
        self,
        instance: int,
        name: str,
        *,
        supported_formats: Optional[
            list[tuple[int | tuple[int, int | None, int | None], int]]
        ] = None,
    ) -> None:
        """Add a Credential Data Input object to the server (before starting).

        ``supported_formats`` sets Supported_Formats and
        Supported_Format_Classes (read-only over the network) as
        ``(format, format_class)`` pairs. A format is a
        BACnetAuthenticationFactorType number, or a
        ``(format_type, vendor_id, vendor_format)`` triple whose vendor
        members may each be None (absent), as a read gives them; a CUSTOM
        format (2) needs both. A format outside the closed production, a CUSTOM
        format without its vendor members or a nonzero vendor member on
        another format raises BacnetProtocolError with VALUE_OUT_OF_RANGE; a
        vendor member above 65535 raises OverflowError. While Out_Of_Service is TRUE a client's simulated
        Present_Value must name one of these formats with its class, or be
        the UNDEFINED or ERROR factor with class 0.
        """
        ...
    def add_alert_enrollment(
        self, instance: int, name: str, initial_source: ObjectIdentifier
    ) -> None: ...
    def add_event_enrollment(
        self, instance: int, name: str, event_type: EventType = ...
    ) -> None:
        """Add an Event Enrollment (Clause 12.12) to the server (before starting).

        Object_Property_Reference, read-only to peers, reads as Analog Input
        4194303's Present_Value while the enrollment has no reference.
        Fault_Parameters reads as the context-tagged ``none`` choice while no
        fault algorithm is set, and writing that clears it; a null written to
        it succeeds and changes nothing.
        """
        ...

    # --- Building/transportation ---
    def add_elevator_group(
        self,
        instance: int,
        name: str,
        machine_room_id: Optional[ObjectIdentifier] = None,
    ) -> None:
        """Add an Elevator Group; machine_room_id must be a Positive Integer Value."""
        ...
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
    def add_averaging(
        self,
        instance: int,
        name: str,
        *,
        window_interval: int | None = None,
        window_samples: int | None = None,
    ) -> None:
        """Add an Averaging object before start().

        ``window_interval`` sets Window_Interval in seconds (900 when omitted)
        and ``window_samples`` sets Window_Samples (15 when omitted). Peers
        can write both, and each write discards the samples. An interval of 0,
        or a sample count of 0 or above 1440, raises VALUE_OUT_OF_RANGE.

        Object_Property_Reference reads as ``application_data`` holding the
        context-tagged reference; while unset it reads as Analog Input
        4194303's Present_Value, and writing a reference whose object or
        Device is at instance 4194303 unsets it. A null written to it succeeds
        and changes nothing.
        """

    # --- Value objects ---
    def add_integer_value(self, instance: int, name: str) -> None: ...
    def add_positive_integer_value(self, instance: int, name: str) -> None: ...
    def add_large_analog_value(self, instance: int, name: str) -> None: ...
    def add_character_string_value(self, instance: int, name: str) -> None: ...
    def add_octet_string_value(self, instance: int, name: str) -> None: ...
    def add_bit_string_value(self, instance: int, name: str) -> None: ...

    # --- Counters/accumulators ---
    def add_accumulator(
        self,
        instance: int,
        name: str,
        units: int = 62,
        *,
        scale: float | int | None = None,
        prescale: Optional[tuple[int, int]] = None,
    ) -> None:
        """Add an Accumulator (Clause 12.61) to the server (before starting).

        ``scale`` sets Scale: a ``float`` is a float scale, sent as a
        single-precision REAL that multiplies Present_Value, and an ``int``
        an integer scale, the power of ten Present_Value is multiplied by.
        Left out, Scale is the float scale 1.0. ``prescale``, a
        ``(multiplier, modulo_divide)`` pair of unsigned32 values, serves the
        optional Prescale; left out, the object doesn't serve it and a read
        of it is UNKNOWN_PROPERTY. Both read back as these values (tags
        ``"scale"`` and ``"prescale"``). A bool or another type raises
        TypeError, an integer outside its type OverflowError, a float that
        isn't finite as a REAL or a ``prescale`` of another length
        ValueError; nothing is registered then.
        """
        ...
    def add_pulse_converter(self, instance: int, name: str, units: int = 62) -> None:
        """Add a Pulse Converter (Clause 12.23) to the server (before starting).

        Input_Reference reads as ``application_data`` holding the
        context-tagged reference; while unset it reads as Accumulator
        4194303's Present_Value, and writing a reference to instance 4194303
        unsets it. A null written to it succeeds and changes nothing.
        While the reference names a missing object, or a property that isn't
        an Unsigned or INTEGER, Reliability reads CONFIGURATION_ERROR and
        Status_Flags FAULT; out of service, Reliability is writable for
        simulation. The running server counts each increase of the named
        property into Count at least once a second, across an Accumulator's
        wrap at Max_Pres_Value.
        """
        ...

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
    def add_bip_network_port(self, instance: int, name: str, *, ip_address: str = "0.0.0.0", udp_port: int = 47808, network_number: int = 0, apdu_length: int = 1476, subnet_mask: str = "0.0.0.0", default_gateway: str = "0.0.0.0", dns_servers: Optional[List[str]] = None) -> None: ...

    # --- Server lifecycle ---
    def start(self) -> Awaitable[None]:
        """Start the server and begin accepting BACnet requests."""
        ...

    def stop(self) -> Awaitable[None]:
        """Join admitted work and release the owned transport.

        Cancelling the returned Future retains shutdown ownership. Call stop()
        again to join it; a cleanup error also retains the owner for retry.
        Local mutation is rejected once shutdown starts.

        A Notification Forwarder, Notification Class or Audit Log write staged
        for a request that stop() cut short is dropped, and stop() waits, with
        no limit, until every save those objects queued has run, so storage
        holds what they serve. While storage holds it up, a warning naming the
        objects still saving is logged after 5 s and every 30 s after that.
        """
        ...

    def local_address(self) -> Awaitable[str]:
        """Get the retained server instance's last bound address snapshot.

        Available during interrupted shutdown; successful stop clears the
        instance and subsequent address queries raise RuntimeError.
        """
        ...

    # --- Server-side property access ---
    def read_property(
        self,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> Awaitable[PropertyValue]:
        """Read a property from a local object through the server's
        ReadProperty evaluator, the one network reads use.

        The result equals what a network ``read_property`` of the same
        property returns: a Group's Present_Value is rebuilt from its
        members, Device instance 4194303 names this server's Device, and the
        Device's COV subscription lists are live. Its Device_Address_Binding
        is a list with one ``"address_binding"`` element per device binding
        the server holds (each ``add_device_binding``, and each device whose
        I-Am it heard in the last ten minutes): a mapping of
        ``device_identifier``, ``network_number`` (0 on this network) and
        ``mac_address``. An unknown object or
        property raises ``BacnetProtocolError`` with the network error, and a
        Group whose member rows exceed ``rpm_max_result_elements`` raises
        ``BacnetAbortError`` (reason 9, OUT_OF_RESOURCES).
        """
        ...

    def write_property_local(
        self,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        value: PropertyValue,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
        *,
        source_object: Optional[ObjectIdentifier],
    ) -> Awaitable[None]:
        """Write locally with an explicit source: None selects the server Device.

        A source object must exist in the database. Tracked commands require a
        concrete server Device; corrections retain the original Device owner.
        The value is encoded and decoded as a network WriteProperty's would
        be, so any value a network write takes works, including whatever
        ``read_property`` returns, and array-index errors match the network's.
        """
        ...

    def set_present_value_local(
        self,
        object_id: ObjectIdentifier,
        value: PropertyValue,
    ) -> Awaitable[None]:
        """Update Present_Value through the narrow, non-generic application Input authority.

        Accepted values are a finite REAL for Analog Input and for a Loop's
        control output, logical Enumerated 0/1 (INACTIVE/ACTIVE) for Binary Input,
        and Unsigned 1..=Number_Of_States for Multi-state Input. Binary values are
        BACnet logical values after Polarity, not raw hardware/interface levels.
        An object with Out_Of_Service set rejects this update to preserve
        network simulation ownership. A Life Safety Point or Zone takes an
        Enumerated BACnetLifeSafetyState (standard, or 256..=65535) whether or
        not Out_Of_Service is set, since clients never write its Present_Value;
        Tracking_Value, Silenced and Operation_Expected stay as they are. Other
        object types are not writable through this method.
        """
        ...

    def report_access_event_local(
        self,
        object_id: ObjectIdentifier,
        event: int,
        tag: int,
        *,
        time: Optional[BACnetTimeStamp] = None,
        credential: Optional[
            ObjectIdentifier | tuple[ObjectIdentifier, ObjectIdentifier]
        ] = None,
        authentication_factor: Optional[tuple[int, int, bytes]] = None,
    ) -> Awaitable[None]:
        """Report an access event at an Access Point the server holds.

        Access_Event (``event``, a BACnetAccessEvent number),
        Access_Event_Tag (``tag``, the access transaction), Access_Event_Time
        (``time``, the Device clock's when omitted), Access_Event_Credential
        (``credential``, the no-credential reference when omitted) and
        Access_Event_Authentication_Factor (``authentication_factor`` as
        ``(format_type, format_class, value)``, the UNDEFINED factor when
        omitted) change together. A credential that isn't an Access
        Credential, a factor format outside the closed production, or an
        event that is neither a named BACnetAccessEvent nor a proprietary one
        from 512 to 65535, raises VALUE_OUT_OF_RANGE. While the point is out
        of service, or its Reliability isn't NO_FAULT_DETECTED, the event is
        refused with WRITE_ACCESS_DENIED and nothing changes. An unknown
        object raises UNKNOWN_OBJECT and any object other than an Access
        Point OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. A new time sends the
        point's COV report, and a time the point stamps itself always moves,
        so events of one transaction each send one.
        """
        ...

    def report_credential_read_local(
        self,
        object_id: ObjectIdentifier,
        factor: tuple[int, int, bytes],
        *,
        update_time: Optional[BACnetTimeStamp] = None,
    ) -> Awaitable[None]:
        """Report a factor a Credential Data Input's reader has read.

        Present_Value takes ``factor``, ``(format_type, format_class,
        value)``, and Update_Time ``update_time``, the Device clock's when
        omitted, together. The factor must name one of the reader's
        ``supported_formats`` with its class, or be the UNDEFINED (0) or
        ERROR (1) factor with class 0, else VALUE_OUT_OF_RANGE. While the
        reader is out of service the read is kept aside, in place of the
        reader's earlier one: a client's simulated values stay served, no
        COV report goes out, and the return to service serves the latest
        read. An unknown object raises UNKNOWN_OBJECT and any object other
        than a Credential Data Input OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. A
        new Update_Time sends the reader's COV report.
        """
        ...

    def report_door_state_local(
        self,
        object_id: ObjectIdentifier,
        *,
        door_status: Optional[int] = None,
        lock_status: Optional[int] = None,
        door_alarm_state: Optional[int] = None,
    ) -> Awaitable[None]:
        """Report an Access Door's hardware state.

        Whichever of Door_Status, Lock_Status and Door_Alarm_State is given,
        as BACnetDoorStatus, BACnetLockStatus and BACnetDoorAlarmState
        numbers, changes together; the others keep their values. A number
        outside its production, or an alarm state the door's Alarm_Values,
        Fault_Values and Masked_Alarm_Values don't admit, raises
        VALUE_OUT_OF_RANGE and nothing changes. While the door is out of
        service the values are kept aside, in place of the device's earlier
        ones: a client's simulated values stay served, no COV report or event
        follows, and the return to service serves the latest values
        reported. An unknown object raises UNKNOWN_OBJECT and any object
        other than an Access Door OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. In
        service a new Door_Alarm_State sends the door's COV report, and the
        door's event algorithm sees it at once.
        """
        ...

    def set_tracking_value_local(
        self,
        object_id: ObjectIdentifier,
        value: PropertyValue,
    ) -> Awaitable[None]:
        """Set a Life Safety Point's or Zone's Tracking_Value, the live state the application derived.

        The value is an Enumerated BACnetLifeSafetyState, standard or from
        256..=65535: another number raises VALUE_OUT_OF_RANGE and another
        datatype INVALID_DATA_TYPE. An unknown object raises UNKNOWN_OBJECT and
        any object other than a Life Safety Point or Zone
        OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Present_Value, Silenced and
        Operation_Expected stay as they are, so an application that latches
        Present_Value until reset keeps reporting the live state here. While
        Out_Of_Service is set the value is held for the return to service and
        a client's simulated Tracking_Value stays in place. In service a
        SubscribeCOVProperty on Tracking_Value is notified of the change; a
        SubscribeCOV report doesn't carry it.
        """
        ...

    def purge_audit_log(self, object_id: ObjectIdentifier) -> Awaitable[None]:
        """Purge an Audit Log: clear its records and append a BUFFER_PURGED status record.

        No peer can purge an Audit Log, since its Record_Count is read-only,
        so this is the application's route. The record is appended whether or
        not logging is enabled, and also carries LOG_DISABLED while logging is
        off; Total_Record_Count keeps counting. The purge reaches storage
        before the log serves it. An unknown object
        raises UNKNOWN_OBJECT, any object other than an Audit Log
        OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, and a missing clock or a failed
        commit DEVICE / OPERATIONAL_PROBLEM, leaving the log as it was. A
        server that is not running raises RuntimeError.
        """
        ...

    def set_controlled_variable_value_local(
        self,
        object_id: ObjectIdentifier,
        value: PropertyValue,
    ) -> Awaitable[None]:
        """Feed a Loop's measured Controlled_Variable_Value while the application runs its algorithm.

        The value must be a finite REAL: another datatype raises
        INVALID_DATA_TYPE and NaN or an infinity VALUE_OUT_OF_RANGE. An unknown
        object raises UNKNOWN_OBJECT and any object other than a Loop
        OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Unlike Present_Value, the update
        is accepted while Out_Of_Service is set. A SubscribeCOVProperty on the
        property is notified of the change; a SubscribeCOV on the Loop is not,
        and its next notification carries the new value. The property stays
        read-only over the network.
        """
        ...

    def add_averaging_sample_local(
        self,
        object_id: ObjectIdentifier,
        value: PropertyValue | None,
    ) -> Awaitable[None]:
        """Record one sample for an Averaging object, taken by the application.

        The server samples an object holding an Object_Property_Reference
        itself, every Window_Interval / Window_Samples seconds. For an object
        without one (its reference reads as the unset form, instance
        4194303), the application samples about that often and passes each
        result here; on one the server samples, a call is one more attempt and
        doesn't move the server's schedule. The value is a BOOLEAN (FALSE and
        TRUE count as 0 and 1), Signed, Unsigned, Enumerated or finite REAL;
        ``None`` records an attempt that produced no value, which counts
        toward Attempted_Samples but not Valid_Samples. Another datatype,
        Double included, raises INVALID_DATA_TYPE and NaN or an infinity
        VALUE_OUT_OF_RANGE, and a refused sample isn't counted. An unknown
        object raises UNKNOWN_OBJECT and any object other than an Averaging
        object OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Each call fills the next
        slot of the Window_Samples window, dropping the oldest once it is
        full, and Minimum_Value, Maximum_Value, Average_Value and the sample
        counts change together; with no valid sample in the window they read
        positive infinity, negative infinity and NaN. A SubscribeCOVProperty
        on one of them is notified when it changes (by the subscription's COV
        increment, if it gave one); SubscribeCOV on an Averaging object is
        refused.
        """
        ...

    def comm_state(self) -> Awaitable[EnableDisable]:
        """Get the DeviceCommunicationControl state.

        Returns ``EnableDisable.ENABLE`` or ``EnableDisable.DISABLE_INITIATION``;
        the server refuses the deprecated ``DISABLE``, so it never reports it.
        Raises RuntimeError before start and after stop.
        """
        ...

    def cov_counters(self) -> Awaitable[CovCounters]:
        """Sample COV subscription and notification telemetry; zero at each start.

        Every field of the Rust CovCounters under the same name.
        subscriptions_active is a gauge; the rest are running totals.
        timed_changes_dropped counts timestamped COV-multiple changes lost for
        good, in whole or in part, the running signal for a subscriber whose
        maximum APDU can't hold one timestamped value.
        untimed_references_oversized counts each report that left out an
        untimestamped reference too large for one notification. Raises
        RuntimeError before start and after stop.
        """
        ...

    def event_notification_counters(self) -> Awaitable[EventNotificationCounters]:
        """Sample the totals of event notifications the server did not deliver.

        Every field of the Rust EventNotificationCounters under the same name,
        zero at each start. The recipient-list fields count transitions whose
        Notification Class lookup failed closed: the class is missing, its
        Recipient_List can't be read or decoded, or it serves more than 32
        destinations. An empty or fully filtered list, DCC and Event_Enable are
        not counted. The next three count matched destinations skipped while
        their route was resolved: a Device recipient with no current binding,
        a recipient that can't be routed as configured, and a confirmed
        recipient at a group address (a broadcast or a multicast one). The
        confirmed fields count
        notifications to one recipient that found no free invoke ID, were
        answered with an Error, Reject or Abort, or drew no acknowledgment
        after the last retry. unconfirmed_send_failed counts unconfirmed
        notifications the transport refused to send, once per destination.
        apdu_too_large counts notifications not sent to one destination
        because they exceed the local APDU size, received_not_forwarded
        counts received event notifications no Notification Forwarder took,
        and forwarding_cap_dropped counts destinations dropped because one
        notification already had 64 across the forwarders.
        Fields are sampled independently. Raises
        RuntimeError before start and after stop.
        """
        ...

    def forwarder_save_counters(self) -> Awaitable[dict[int, ForwarderSaveCounters]]:
        """Sample each Notification Forwarder's save counters, keyed by instance.

        The totals belong to the objects: they count from registration, and
        a forwarder without ``storage_path`` stays at zero. Raises
        RuntimeError before start and after stop.
        """
        ...

    def dcc_outcome_counters(self) -> Awaitable[DccOutcomeCounters]:
        """Sample completed admitted DCC handlers; zero on each new server lifetime.

        Accepted means state/timer commit, not response delivery. Independent
        samples are not an atomic aggregate. Excludes incomplete cancellation,
        duplicates, pre-handler rejection and timer expiry. No trace bridge or
        subscriber installation. Raises RuntimeError before start/after stop.
        """
        ...

    def request_admission_counters(self) -> Awaitable[RequestAdmissionCounters]:
        """Sample counters; RuntimeError before start and after stop.

        Admitted totals count registered work, not successful response sends.
        """
        ...


# ---------------------------------------------------------------------------
# SC Hub
# ---------------------------------------------------------------------------

class ScHubOutcomeCounts(TypedDict):
    """Per-start saturating outcomes; selected refusals are not delivered NAK counts."""
    uuid_replacements: int
    vmac_collision_rejections: int
    registered_capacity_rejections: int
    total_active_accept_drops: int
    handshake_accept_drops: int
    tls_timeouts: int
    websocket_timeouts: int
    connect_timeouts: int
    unicast_no_target: int
    unicast_target_limit: int
    unicast_send_timeout: int
    unicast_send_error: int
    heartbeat_retirements: int

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
    outcomes: ScHubOutcomeCounts

class ScHubCertificateBinding:
    """Frozen installation group; exact leaf DER SHA-256, with redacted repr.

    Inputs are copied. UUID must be nonzero; VMAC and digest lists must be
    nonempty and distinct; reserved VMACs are invalid. Raises ValueError.
    """
    def __init__(
        self, *, uuid: bytes, allowed_vmacs: Sequence[bytes], leaf_sha256: Sequence[bytes],
    ) -> None: ...
    @property
    def uuid(self) -> bytes: ...
    @property
    def allowed_vmacs(self) -> tuple[bytes, ...]: ...
    @property
    def leaf_sha256(self) -> tuple[bytes, ...]: ...

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
    Optional Hub probes use ``probe_scan_interval_ms`` / ``probe_idle_age_ms`` /
    ``probe_ack_age_ms`` / ``probe_send_budget_ms`` (defaults 30000/60000/5000/5000).
    Strictly exceeded ages are checked at scans; ACK age starts at reservation,
    not completed send, and is not a hard closure deadline. Delayed ticks are
    skipped. Only matching valid ACKs refresh activity. Node keepalive is separate.
    ``relay_send_budget_ms`` bounds each NPDU/opaque unicast, concurrent broadcast
    recipient and forwarded BVLC-Result acquisition+send attempt (default 5000),
    without timeout-driven retirement, retry or fabricated Result. Probe, control,
    cleanup and graceful shutdown stay separate. Unicast counters keep their scope.
    These values must be positive whole milliseconds <=2**63-1 and fit the native
    monotonic clock. Broadcast sender/global burst/per-second settings use native
    token buckets: bounds 1..=(2**64-1)//1_000_000_000, silent drops, existing counters.
    All are constructor-validated before I/O. Negative/over-u64 integers raise
    OverflowError, nonintegers TypeError, other invalid bounds ValueError.
    ``certificate_bindings=None`` retains CA-valid admission. A nonempty group
    sequence enables mapped-only UUID/VMAC reservations, including offline nodes;
    existing admission_policy can further deny. Shared native validation rejects
    overlapping ownership and local Hub VMAC conflicts before PEM I/O/bind.
    Inputs are copied; status/repr/errors do not expose certificate contents.
    This is installation policy, not end-to-end relayed authentication.
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
        probe_scan_interval_ms: int = 30000,
        probe_idle_age_ms: int = 60000,
        probe_ack_age_ms: int = 5000,
        probe_send_budget_ms: int = 5000,
        broadcast_sender_burst: int = 1024,
        broadcast_sender_per_second: int = 128,
        broadcast_global_burst: int = 4096,
        broadcast_global_per_second: int = 512,
        relay_send_budget_ms: int = 5000,
        certificate_bindings: Sequence[ScHubCertificateBinding] | None = None,
    ) -> None: ...

    def start(self) -> Awaitable[None]:
        """Start the SC hub."""
        ...

    def stop(self) -> Awaitable[None]:
        """Stop the SC hub (forceful, idempotent)."""
        ...

    def shutdown_gracefully(self) -> Awaitable[Literal["graceful", "forced"]]:
        """Graceful shutdown; consumes the hub (RuntimeError if not started)."""
        ...

    def status(self) -> Awaitable[ScHubStatus]:
        """Bounded redacted snapshot (RuntimeError before start/after stop)."""
        ...

    def __aenter__(self) -> Awaitable[ScHub]: ...
    def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> Awaitable[None]: ...

    def address(self) -> Awaitable[Optional[str]]:
        """Get the address the hub is listening on (None before start)."""
        ...

    def url(self) -> Awaitable[Optional[str]]:
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

class ReadRangeResult(TypedDict):
    object_id: ObjectIdentifier
    property_id: PropertyIdentifier
    array_index: int | None
    result_flags: tuple[bool, bool, bool]
    item_count: int
    item_data: bytes
    first_sequence_number: int | None
    violations: list[str]

ReferenceTime = Union[
    datetime.datetime,
    tuple[tuple[int, int, int, int], tuple[int, int, int, int]],
]
"""A ByTime reference: a naive datetime in the device's local time, or a
``((year, month, day, day_of_week), (hour, minute, second, hundredths))``
pair."""

LogCursor = Union[
    None,
    Literal["oldest"],
    tuple[Literal["sequence"], int],
    tuple[Literal["position"], int],
    tuple[Literal["time"], ReferenceTime],
]
"""Where ``read_log_page`` starts; a list of the same two items works too."""

class LogGap(TypedDict):
    """A page whose first record isn't the one asked for."""
    expected: int
    first: int
    skipped: int | None

class LogPage(TypedDict):
    """One page of ``read_log_page``.

    ``records`` are dicts, oldest first, as ``decode_log_records`` returns
    them. ``next`` is never ``None`` or ``"oldest"``.
    """
    records: list[dict[str, Any]]
    first_sequence_number: int | None
    result_flags: tuple[bool, bool, bool]
    gap: LogGap | None
    violations: list[str]
    next: LogCursor
    done: bool
    wrapped: bool

class ReadPropertyResult(TypedDict):
    property_id: PropertyIdentifier
    array_index: int | None
    value: PropertyValue | bytes | None
    error: tuple[ErrorClass, ErrorCode] | None

class ReadAccessResult(TypedDict):
    object_id: ObjectIdentifier
    results: list[ReadPropertyResult]

class EndpointClient:
    """Client role cloned from a running endpoint (no lifecycle).

    Initiates ``read_property``, ``read_range``, ``read_property_multiple`` and direct B/IP ``write_property`` through the owner's single transport.
    After the owner closes, calls fail closed with ``BacnetError``.
    """

    def read_property(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> Awaitable[PropertyValue]:
        """Read a property through the shared transport.

        ACK object/property/index must match. Device/Network Port instance 4194303
        requests accept a same-type concrete ACK. Malformed/mismatched ACKs raise
        BacnetError; this method returns the property value, not ACK metadata,
        shaped as ``PropertyValue`` describes for read results.
        """
        ...

    def write_property(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        value: PropertyValue,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
        *,
        commandability: Literal["commandable", "noncommandable"],
    ) -> Awaitable[None]:
        """Write to a direct IPv4 B/IP peer through the shared requester.

        Commandability is required even without a source Reporter. It controls
        Audit filtering, never the wire priority. Omitted commandable priority
        means effective 16, including NULL. Invalid literals, priority, raw TLV
        framing and APDU size raise ValueError synchronously (out-of-u8 priorities
        raise OverflowError). Empty encoded lists are valid. Eligible source
        records preserve complete 0–32-byte values and omit longer values.
        After source admission, caller cancellation does not cancel the attempt.
        """
        ...

    def read_property_multiple(
        self,
        address: str,
        specs: list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]],
    ) -> Awaitable[list[ReadAccessResult]]:
        """Read 1–64 explicit property occurrences on concrete objects.

        Empty lists, ALL/REQUIRED/OPTIONAL and wildcard instances raise ValueError
        before address parsing or I/O. Index zero is valid. Complete ordered ACK
        correlation permits an inline error to omit its requested index; successful
        values must echo it. Byte limits and malformed/mismatched ACKs raise
        BacnetError. Returned dictionaries share standalone RPM conversion.
        """
        ...

    def read_range(
        self,
        address: str,
        object_id: ObjectIdentifier,
        property_id: PropertyIdentifier,
        array_index: Optional[int] = None,
        range_type: Optional[Literal["position", "sequence", "time"]] = None,
        reference_index: Optional[int] = None,
        reference_seq: Optional[int] = None,
        count: Optional[int] = None,
        *,
        reference_time: ReferenceTime | None = None,
        validation: Literal["strict", "lenient"] = "strict",
    ) -> Awaitable[ReadRangeResult]:
        """Read a range of items from a list or log object.

        ``range_type`` is ``"position"``, ``"sequence"``, ``"time"`` or
        ``None`` (all-items). ``"time"`` reads the ``count`` items newer than
        ``reference_time`` (older, for a negative count): a naive
        ``datetime.datetime`` in the device's local time, or a
        ``((year, month, day, day_of_week), (hour, minute, second,
        hundredths))`` pair; an aware datetime raises ValueError, since the
        device's zone isn't known here (#1533). Invalid selectors, array index
        zero, missing or zero counts and a time that isn't specific raise
        ValueError before I/O; a count outside INTEGER16 (-32768..=32767)
        raises OverflowError before address parsing or I/O (#1360).
        Position/sequence reference zero is valid; omitted references default
        to zero.

        ``validation="strict"`` refuses an answer that breaks a ReadRange rule
        with BacnetReadRangeViolationError, whose ``rule`` names it;
        ``"lenient"`` keeps it and lists the rules in ``violations`` (#1531).
        Decode a log's records with ``decode_log_records``.
        """
        ...

    def read_log_page(
        self,
        address: str,
        object_id: ObjectIdentifier,
        cursor: LogCursor = None,
        page_size: int = 100,
    ) -> Awaitable[LogPage]:
        """Read one page of a log's Log_Buffer from ``cursor`` (#1530).

        ``cursor`` is ``None`` or ``"oldest"`` (the oldest record, found from
        Record_Count and Total_Record_Count), ``("sequence", n)``,
        ``("position", n)`` or ``("time", reference_time)``; lists work
        too, so a checkpoint survives JSON. Loop on ``page["next"]`` until
        ``page["done"]``, then keep ``next`` as the checkpoint. One request
        is outstanding at a time. A first record past the one asked for sets
        ``gap``; when the log drops the record asked for, the read starts
        over from the oldest. A page answered from before the one asked for,
        or none where the counts say records are, raises
        BacnetLogNotAdvancingError: such a device's sequence numbers are
        inconsistent, so read it from ``("position", 1)``. ``wrapped`` marks
        a page that reached the top of the sequence range. A page whose
        records don't decode raises BacnetError, dropping the records before
        the failing one; an answer that breaks a rule the reader can't
        tolerate, such as an echo that doesn't match, raises
        BacnetReadRangeViolationError. A non-log object or a ``page_size``
        outside 1..=32767 raises ValueError before I/O.
        """
        ...

    def service_scope(self) -> dict[str, Any]:
        """Narrow scope: initiates reads and direct B/IP ``write_property``."""
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

    Admission is lifecycle-lock acquisition, not Future creation. Explicit
    second start raises BacnetError; context reentry returns this endpoint.
    Close cancels pending connection preparation, then joins earlier admitted
    startup and teardown. A later start may restart without old registrations.
    Cancelled waiters do not abandon admitted session startup or cleanup;
    cancellation can race publication, so always await close for cleanup.
    Setup errors are delivered by awaiting start/context entry. BIPv6/Ethernet
    have no endpoint owner.
    """

    # network_port_instance declares a snapshot; registered_network_port explicitly
    # selects it for one concrete-interface NORMAL B/IP bind. None stays unbound.
    # NORMAL B/IP answers/learns local Network Number controls. Only explicit
    # registration supplies configured provenance; an unregistered owner starts unknown.

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
        registered_network_port: Optional[int] = None,
        *,
        read_work_limit: int = 256,
        share_port_by_address: bool = False,
        min_request_interval_ms: int = 0,
    ) -> None:
        """``read_work_limit``: result rows one ReadProperty served by the
        server role may expand, a Group's member rows included; a read past it
        is aborted with OUT_OF_RESOURCES. Zero raises ValueError.
        ``share_port_by_address`` (default False): bind the interface address
        itself, so endpoints on other addresses of this host can share the
        port. Needs an explicit interface and a nonzero port; broadcasts and
        unicast then arrive in no fixed order.
        ``min_request_interval_ms`` (default 0, no pacing; at most 3,600,000,
        else ValueError) paces the client role's confirmed requests to each
        destination exactly as ``BACnetClient``'s keyword does (#1542). A
        retry keeps its request's turn; replies, notifications and
        unconfirmed requests aren't paced."""
        ...

    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional AV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional BV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...
    def add_group(
        self,
        instance: int,
        name: str,
        members: Optional[
            list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]
        ] = None,
    ) -> None:
        """Group whose Present_Value is rebuilt from ``members`` on each read.

        ``members`` has the ``read_property_multiple`` spec shape. A member
        with no properties, or one reporting a group's Present_Value, raises
        ValueError naming its position and the rule. Any property identifier
        is taken, those ASHRAE assigns above 4194303 included. Indexes
        outside unsigned32 raise OverflowError.
        """
        ...

    def start(self) -> Awaitable[None]:
        """Admit one startup; second start while running raises BacnetError."""
        ...

    def close(self) -> Awaitable[None]:
        """Join earlier startup and cleanup; idempotent and safe after cancellation."""
        ...

    def __aenter__(self) -> Awaitable[BipEndpoint]: ...
    def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> Awaitable[None]: ...

    def client(self) -> Awaitable[EndpointClient]:
        """Clone the client role (RuntimeError before start/after close)."""
        ...

    def server(self) -> Awaitable[EndpointServer]:
        """Clone the server role (RuntimeError before start/after close)."""
        ...

    def local_address(self) -> Awaitable[str]:
        """Active announced IP and actual UDP port; RuntimeError outside active state."""
        ...

    def status(self) -> Awaitable[EndpointStatus]:
        """Bounded snapshot (RuntimeError before start/after close)."""
        ...

    def broadcast_i_am(self) -> Awaitable[None]:
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
        read_work_limit: int = 256,
        min_request_interval_ms: int = 0,
    ) -> None:
        """``read_work_limit`` and ``min_request_interval_ms`` are as for
        ``BipEndpoint``."""
        ...

    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional AV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional BV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...
    def add_group(
        self,
        instance: int,
        name: str,
        members: Optional[
            list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]
        ] = None,
    ) -> None:
        """Group whose Present_Value is rebuilt from ``members`` on each read.

        ``members`` has the ``read_property_multiple`` spec shape. A member
        with no properties, or one reporting a group's Present_Value, raises
        ValueError naming its position and the rule. Any property identifier
        is taken, those ASHRAE assigns above 4194303 included. Indexes
        outside unsigned32 raise OverflowError.
        """
        ...

    def start(self) -> Awaitable[None]:
        """Admit one hub connection; second start while running raises BacnetError."""
        ...

    def close(self) -> Awaitable[None]:
        """Join earlier startup and cleanup; idempotent and safe after cancellation."""
        ...

    def __aenter__(self) -> Awaitable[ScEndpoint]: ...
    def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> Awaitable[None]: ...

    def client(self) -> Awaitable[EndpointClient]:
        """Clone the client role."""
        ...

    def server(self) -> Awaitable[EndpointServer]:
        """Clone the server role."""
        ...

    def local_address(self) -> Awaitable[str]:
        """VMAC hex for this SC node."""
        ...

    def status(self) -> Awaitable[EndpointStatus]:
        """Bounded snapshot (RuntimeError before start/after close)."""
        ...

    def broadcast_i_am(self) -> Awaitable[None]:
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
        *,
        read_work_limit: int = 256,
        min_request_interval_ms: int = 0,
    ) -> None:
        """``read_work_limit`` and ``min_request_interval_ms`` are as for
        ``BipEndpoint``."""
        ...

    def add_analog_input(self, instance: int, name: str, units: int = 62, present_value: float = 0.0) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int = 62, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional AV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str, *, audit_level: Literal["default", "none", "audit_config", "audit_all"] | None = None, auditable_operations: int | None = None, audit_priority_filter: int | Literal["inherit"] | None = None) -> None:
        """Optional BV Audit rows: None is absent; priority 'inherit' is present NULL."""
        ...
    def add_group(
        self,
        instance: int,
        name: str,
        members: Optional[
            list[tuple[ObjectIdentifier, list[tuple[PropertyIdentifier, Optional[int]]]]]
        ] = None,
    ) -> None:
        """Group whose Present_Value is rebuilt from ``members`` on each read.

        ``members`` has the ``read_property_multiple`` spec shape. A member
        with no properties, or one reporting a group's Present_Value, raises
        ValueError naming its position and the rule. Any property identifier
        is taken, those ASHRAE assigns above 4194303 included. Indexes
        outside unsigned32 raise OverflowError.
        """
        ...

    def start(self) -> Awaitable[None]:
        """Admit one serial owner; second start while running raises BacnetError."""
        ...

    def close(self) -> Awaitable[None]:
        """Join earlier startup and cleanup; idempotent and safe after cancellation."""
        ...

    def __aenter__(self) -> Awaitable[MstpEndpoint]: ...
    def __aexit__(
        self,
        _exc_type: Any = None,
        _exc_val: Any = None,
        _exc_tb: Any = None,
    ) -> Awaitable[None]: ...

    def client(self) -> Awaitable[EndpointClient]:
        """Clone the client role."""
        ...

    def server(self) -> Awaitable[EndpointServer]:
        """Clone the server role."""
        ...

    def local_address(self) -> Awaitable[str]:
        """Station MAC as a decimal string."""
        ...

    def status(self) -> Awaitable[EndpointStatus]:
        """Bounded snapshot (RuntimeError before start/after close)."""
        ...

    def broadcast_i_am(self) -> Awaitable[None]:
        """Broadcast one I-Am (MS/TP local broadcast)."""
        ...

    @property
    def device_instance(self) -> int: ...
    @property
    def vendor_id(self) -> int: ...
