"""Type stubs for rusty_bacnet — Python bindings for the BACnet protocol stack (ASHRAE 135-2020).

Generated from the actual PyO3 source code. Enum classes expose all standard constants
as class attributes; vendor-proprietary values are available via ``from_raw()``.
"""

from __future__ import annotations

from typing import Any, Optional, Union


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

    Common properties listed below; all 500+ standard values are available at runtime
    via class attributes matching the BACnet constant names.
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

    All 139 standard codes are available as class attributes.
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

    @staticmethod
    def from_raw(value: int) -> ErrorCode: ...
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
    and BACnet/SC (``"sc"``) transports.

    Usage::

        async with BACnetClient() as client:
            await client.who_is()
            devices = await client.discovered_devices()
            value = await client.read_property("192.168.1.100:47808", oid, pid)
            print(value.tag, value.value)
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
        max_notification_delay: Optional[int] = None,
        issue_confirmed_notifications: Optional[bool] = None,
    ) -> None:
        """Subscribe to COV notifications for multiple properties on multiple objects.

        ``specs`` is ``[(object_id, [(property_id, array_index, cov_increment, timestamped), ...]), ...]``.
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

    async def acknowledge_alarm(
        self,
        address: str,
        acknowledging_process_identifier: int,
        event_object_identifier: ObjectIdentifier,
        event_state_acknowledged: int,
        acknowledgment_source: str,
    ) -> None:
        """Acknowledge an alarm on a remote device."""
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
        event_state_filter: Optional[EventState] = None,
        event_type_filter: Optional[EventType] = None,
        min_priority: Optional[int] = None,
        max_priority: Optional[int] = None,
        notification_class_filter: Optional[int] = None,
    ) -> list[dict[str, Any]]:
        """Get enrollment summary from a remote device.

        Returns ``[{"object_id": ObjectIdentifier, "event_type": EventType,
        "event_state": EventState, "priority": int, "notification_class": int}, ...]``.
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

    async def confirmed_audit_notification(
        self, address: str, service_data: bytes
    ) -> None:
        """Send a ConfirmedAuditNotification (raw service data)."""
        ...

    async def unconfirmed_audit_notification(
        self, address: str, service_data: bytes
    ) -> None:
        """Send an UnconfirmedAuditNotification (raw service data)."""
        ...

    async def audit_log_query(
        self,
        address: str,
        acknowledgment_filter: int,
        query_options_raw: bytes = b"",
    ) -> bytes:
        """Send an AuditLogQuery request. Returns raw response bytes."""
        ...

    # --- Lifecycle ---

    async def stop(self) -> None:
        """Explicitly stop the client and release resources."""
        ...


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------

class BACnetServer:
    """BACnet server that hosts objects and responds to client requests.

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
    def add_audit_log(self, instance: int, name: str, buffer_size: int = 100) -> None: ...
    def add_audit_reporter(self, instance: int, name: str) -> None: ...
    def add_notification_forwarder(self, instance: int, name: str) -> None: ...

    # --- Control/PID ---
    def add_loop(self, instance: int, name: str, output_units: int = 62) -> None: ...
    def add_command(self, instance: int, name: str) -> None: ...
    def add_timer(self, instance: int, name: str) -> None: ...
    def add_load_control(self, instance: int, name: str) -> None: ...
    def add_program(self, instance: int, name: str) -> None: ...

    # --- Lighting ---
    def add_lighting_output(self, instance: int, name: str) -> None: ...
    def add_binary_lighting_output(self, instance: int, name: str) -> None: ...
    def add_channel(self, instance: int, name: str, channel_number: int) -> None: ...

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
    def add_alert_enrollment(self, instance: int, name: str) -> None: ...
    def add_event_enrollment(self, instance: int, name: str, event_type: int = 0) -> None: ...

    # --- Building/transportation ---
    def add_elevator_group(self, instance: int, name: str) -> None: ...
    def add_escalator(self, instance: int, name: str) -> None: ...
    def add_lift(self, instance: int, name: str, num_floors: int) -> None: ...
    def add_staging(self, instance: int, name: str, num_stages: int) -> None: ...

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

    async def comm_state(self) -> int:
        """Get the DeviceCommunicationControl state (0=Enable, 1=Disable, 2=DisableInitiation)."""
        ...


# ---------------------------------------------------------------------------
# SC Hub
# ---------------------------------------------------------------------------

class ScHub:
    """BACnet/SC Hub for relaying messages between SC nodes.

    Usage::

        hub = ScHub("0.0.0.0:47809", "cert.pem", "key.pem", b"\\x00" * 6)
        await hub.start()
        print(await hub.url())
        await hub.stop()
    """

    def __init__(
        self,
        listen: str,
        cert: str,
        key: str,
        vmac: bytes,
        ca_cert: Optional[str] = None,
    ) -> None: ...

    async def start(self) -> None:
        """Start the SC hub."""
        ...

    async def stop(self) -> None:
        """Stop the SC hub."""
        ...

    async def address(self) -> str:
        """Get the address the hub is listening on."""
        ...

    async def url(self) -> str:
        """Get the WebSocket URL of the hub."""
        ...
