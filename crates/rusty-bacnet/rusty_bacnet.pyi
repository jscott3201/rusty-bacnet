"""Type stubs for rusty_bacnet — Python bindings for the BACnet protocol stack (ASHRAE 135-2020)."""

from __future__ import annotations

from typing import Any, AsyncIterator, Literal, Optional, Sequence


# ---------------------------------------------------------------------------
# Enum types
# ---------------------------------------------------------------------------

class ObjectType:
    """BACnet object type enumeration."""
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
    STRUCTURED_VIEW: ObjectType
    LOAD_CONTROL: ObjectType
    NETWORK_PORT: ObjectType
    LIGHTING_OUTPUT: ObjectType
    BINARY_LIGHTING_OUTPUT: ObjectType
    TIMER: ObjectType
    STAGING: ObjectType
    AUDIT_LOG: ObjectType

    @staticmethod
    def from_raw(value: int) -> ObjectType: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class PropertyIdentifier:
    """BACnet property identifier enumeration."""
    PRESENT_VALUE: PropertyIdentifier
    OBJECT_NAME: PropertyIdentifier
    OBJECT_TYPE: PropertyIdentifier
    OBJECT_IDENTIFIER: PropertyIdentifier
    DESCRIPTION: PropertyIdentifier
    STATUS_FLAGS: PropertyIdentifier
    EVENT_STATE: PropertyIdentifier
    OUT_OF_SERVICE: PropertyIdentifier
    UNITS: PropertyIdentifier
    PRIORITY_ARRAY: PropertyIdentifier
    RELINQUISH_DEFAULT: PropertyIdentifier
    COV_INCREMENT: PropertyIdentifier
    OBJECT_LIST: PropertyIdentifier
    PROPERTY_LIST: PropertyIdentifier
    RELIABILITY: PropertyIdentifier
    SYSTEM_STATUS: PropertyIdentifier
    VENDOR_NAME: PropertyIdentifier
    MODEL_NAME: PropertyIdentifier
    FIRMWARE_REVISION: PropertyIdentifier
    PROTOCOL_VERSION: PropertyIdentifier
    PROTOCOL_REVISION: PropertyIdentifier
    MAX_APDU_LENGTH_ACCEPTED: PropertyIdentifier
    SEGMENTATION_SUPPORTED: PropertyIdentifier
    NOTIFICATION_CLASS: PropertyIdentifier
    HIGH_LIMIT: PropertyIdentifier
    LOW_LIMIT: PropertyIdentifier
    DEADBAND: PropertyIdentifier
    LOG_BUFFER: PropertyIdentifier
    ACTIVE_TEXT: PropertyIdentifier
    INACTIVE_TEXT: PropertyIdentifier
    NUMBER_OF_STATES: PropertyIdentifier
    STATE_TEXT: PropertyIdentifier

    @staticmethod
    def from_raw(value: int) -> PropertyIdentifier: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class ErrorClass:
    """BACnet error class enumeration."""
    DEVICE: ErrorClass
    OBJECT: ErrorClass
    PROPERTY: ErrorClass
    RESOURCES: ErrorClass
    SECURITY: ErrorClass
    SERVICES: ErrorClass
    COMMUNICATION: ErrorClass

    @staticmethod
    def from_raw(value: int) -> ErrorClass: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class ErrorCode:
    """BACnet error code enumeration."""
    OTHER: ErrorCode
    UNKNOWN_OBJECT: ErrorCode
    UNKNOWN_PROPERTY: ErrorCode
    WRITE_ACCESS_DENIED: ErrorCode
    READ_ACCESS_DENIED: ErrorCode
    VALUE_OUT_OF_RANGE: ErrorCode
    PASSWORD_FAILURE: ErrorCode
    SERVICE_REQUEST_DENIED: ErrorCode
    INVALID_DATA_TYPE: ErrorCode
    OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED: ErrorCode

    @staticmethod
    def from_raw(value: int) -> ErrorCode: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class EnableDisable:
    """BACnet DeviceCommunicationControl enable/disable options."""
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
    """BACnet ReinitializeDevice state options."""
    COLDSTART: ReinitializedState
    WARMSTART: ReinitializedState
    START_BACKUP: ReinitializedState
    END_BACKUP: ReinitializedState
    START_RESTORE: ReinitializedState
    END_RESTORE: ReinitializedState
    ABORT_RESTORE: ReinitializedState

    @staticmethod
    def from_raw(value: int) -> ReinitializedState: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class Segmentation:
    """BACnet segmentation support options."""
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
    """BACnet event state enumeration."""
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
    """BACnet event type enumeration."""
    CHANGE_OF_BITSTRING: EventType
    CHANGE_OF_STATE: EventType
    CHANGE_OF_VALUE: EventType
    COMMAND_FAILURE: EventType
    FLOATING_LIMIT: EventType
    OUT_OF_RANGE: EventType

    @staticmethod
    def from_raw(value: int) -> EventType: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class MessagePriority:
    """BACnet text message priority."""
    NORMAL: MessagePriority
    URGENT: MessagePriority

    @staticmethod
    def from_raw(value: int) -> MessagePriority: ...
    def to_raw(self) -> int: ...
    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


class LifeSafetyOperation:
    """BACnet life safety operation codes."""
    NONE: LifeSafetyOperation
    SILENCE: LifeSafetyOperation
    SILENCE_AUDIBLE: LifeSafetyOperation
    SILENCE_VISUAL: LifeSafetyOperation
    RESET: LifeSafetyOperation
    RESET_ALARM: LifeSafetyOperation

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
    """A decoded BACnet property value with tag and Python-native value."""

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


class DiscoveredDevice:
    """A device discovered via Who-Is / I-Am."""

    @property
    def instance(self) -> int: ...

    @property
    def mac(self) -> bytes: ...

    @property
    def max_apdu_length(self) -> int: ...

    @property
    def segmentation_supported(self) -> int: ...

    @property
    def vendor_id(self) -> int: ...

    def __repr__(self) -> str: ...


class CovNotificationIterator:
    """Async iterator yielding COV notification dicts."""

    def __aiter__(self) -> CovNotificationIterator: ...
    async def __anext__(self) -> dict[str, Any]: ...


class BdtEntry:
    """A Broadcast Distribution Table entry from a BBMD."""

    @property
    def ip(self) -> str:
        """IP address as dotted quad string."""
        ...

    @property
    def port(self) -> int: ...

    @property
    def mask(self) -> str:
        """Broadcast distribution mask as dotted quad string."""
        ...

    def __repr__(self) -> str: ...


class FdtEntry:
    """A Foreign Device Table entry from a BBMD."""

    @property
    def ip(self) -> str: ...

    @property
    def port(self) -> int: ...

    @property
    def ttl(self) -> int:
        """Time-to-live in seconds."""
        ...

    @property
    def seconds_remaining(self) -> int:
        """Seconds remaining before expiry."""
        ...

    def __repr__(self) -> str: ...


class RouterInfo:
    """A discovered BACnet router and the networks it serves."""

    @property
    def address(self) -> str:
        """Router address as 'ip:port'."""
        ...

    @property
    def networks(self) -> list[int]:
        """Network numbers reachable through this router."""
        ...

    def __repr__(self) -> str: ...


# ---------------------------------------------------------------------------
# Exceptions
# ---------------------------------------------------------------------------

class BACnetError(Exception):
    """Base exception for BACnet protocol errors."""
    ...

class BACnetTimeoutError(BACnetError):
    """Raised when a BACnet operation times out."""
    ...

class BACnetProtocolError(BACnetError):
    """Raised on BACnet protocol-level errors (Error PDU)."""
    error_class: int
    error_code: int


# ---------------------------------------------------------------------------
# Client
# ---------------------------------------------------------------------------

class BACnetClient:
    """Async BACnet client for reading/writing properties on remote devices.

    Supports BACnet/IP (``"bip"``), BACnet/IPv6 (``"ipv6"``),
    and BACnet/SC (``"sc"``) transports.

    Usage::

        async with BACnetClient("0.0.0.0", 47808) as client:
            value = await client.read_property("192.168.1.100:47808", oid, pid)
            print(value.tag, value.value)
    """

    def __init__(
        self,
        interface: str = "0.0.0.0",
        port: int = 0xBAC0,
        broadcast_address: str = "255.255.255.255",
        apdu_timeout_ms: int = 6000,
        transport: Literal["bip", "ipv6", "sc"] = "bip",
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
    async def __aexit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None: ...

    async def read_property(
        self,
        address: str,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> PropertyValue:
        """Read a single property from a remote device."""
        ...

    async def write_property(
        self,
        address: str,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        value: Any,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
    ) -> None:
        """Write a single property on a remote device."""
        ...

    async def who_is(
        self,
        low_limit: Optional[int] = None,
        high_limit: Optional[int] = None,
        address: Optional[str] = None,
    ) -> list[DiscoveredDevice]:
        """Broadcast a Who-Is request and collect I-Am responses."""
        ...

    async def read_property_multiple(
        self,
        address: str,
        specs: list[dict[str, Any]],
    ) -> dict[str, Any]:
        """Read multiple properties from a remote device (ReadPropertyMultiple)."""
        ...

    async def write_property_multiple(
        self,
        address: str,
        specs: list[dict[str, Any]],
    ) -> None:
        """Write multiple properties on a remote device (WritePropertyMultiple)."""
        ...

    async def subscribe_cov(
        self,
        address: str,
        object_identifier: ObjectIdentifier,
        process_id: int = 1,
        confirmed: bool = False,
        lifetime: Optional[int] = None,
    ) -> None:
        """Subscribe to Change-of-Value notifications for an object."""
        ...

    async def unsubscribe_cov(
        self,
        address: str,
        object_identifier: ObjectIdentifier,
        process_id: int = 1,
    ) -> None:
        """Cancel a COV subscription."""
        ...

    async def cov_notifications(self) -> CovNotificationIterator:
        """Get an async iterator for incoming COV notifications."""
        ...

    async def who_has_by_id(
        self, object_identifier: ObjectIdentifier, address: Optional[str] = None
    ) -> Optional[dict[str, Any]]:
        """Broadcast Who-Has by object identifier."""
        ...

    async def who_has_by_name(
        self, object_name: str, address: Optional[str] = None
    ) -> Optional[dict[str, Any]]:
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

    async def delete_object(
        self, address: str, object_identifier: ObjectIdentifier
    ) -> None:
        """Delete an object on a remote device (DeleteObject service)."""
        ...

    async def create_object(
        self,
        address: str,
        object_type: ObjectType,
        instance: Optional[int] = None,
        name: Optional[str] = None,
        initial_values: Optional[dict[str, Any]] = None,
    ) -> ObjectIdentifier:
        """Create an object on a remote device (CreateObject service)."""
        ...

    async def device_communication_control(
        self,
        address: str,
        enable_disable: EnableDisable,
        duration: Optional[int] = None,
        password: Optional[str] = None,
    ) -> None:
        """Send DeviceCommunicationControl to a remote device."""
        ...

    async def reinitialize_device(
        self,
        address: str,
        state: ReinitializedState,
        password: Optional[str] = None,
    ) -> None:
        """Send ReinitializeDevice to a remote device."""
        ...

    async def acknowledge_alarm(
        self,
        address: str,
        process_id: int,
        object_identifier: ObjectIdentifier,
        event_state: int,
        source: str,
    ) -> None:
        """Acknowledge an alarm on a remote device."""
        ...

    async def get_event_information(
        self, address: str, last_object: Optional[ObjectIdentifier] = None
    ) -> dict[str, Any]:
        """Get event information from a remote device."""
        ...

    async def read_range(
        self,
        address: str,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        index: Optional[int] = None,
        count: Optional[int] = None,
        range_type: Optional[str] = None,
        reference: Optional[int] = None,
    ) -> dict[str, Any]:
        """Read a range of list items from a remote device."""
        ...

    async def atomic_read_file(
        self,
        address: str,
        file_object: ObjectIdentifier,
        start: int,
        length: int,
        stream: bool = True,
    ) -> dict[str, Any]:
        """Read file data from a remote device (AtomicReadFile)."""
        ...

    async def atomic_write_file(
        self,
        address: str,
        file_object: ObjectIdentifier,
        start: int,
        data: bytes,
        stream: bool = True,
    ) -> int:
        """Write file data to a remote device (AtomicWriteFile). Returns start position."""
        ...

    async def add_list_element(
        self,
        address: str,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        elements: bytes,
    ) -> None:
        """Add elements to a list property (AddListElement service)."""
        ...

    async def remove_list_element(
        self,
        address: str,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        elements: bytes,
    ) -> None:
        """Remove elements from a list property (RemoveListElement service)."""
        ...

    async def confirmed_private_transfer(
        self,
        address: str,
        vendor_id: int,
        service_number: int,
        data: Optional[bytes] = None,
    ) -> Optional[bytes]:
        """Send a ConfirmedPrivateTransfer request."""
        ...

    async def unconfirmed_private_transfer(
        self,
        address: str,
        vendor_id: int,
        service_number: int,
        data: Optional[bytes] = None,
    ) -> None:
        """Send an UnconfirmedPrivateTransfer request."""
        ...

    async def confirmed_text_message(
        self,
        address: str,
        source_device: ObjectIdentifier,
        message: str,
        priority: MessagePriority,
        message_class: Optional[str | int] = None,
    ) -> None:
        """Send a ConfirmedTextMessage."""
        ...

    async def unconfirmed_text_message(
        self,
        address: str,
        source_device: ObjectIdentifier,
        message: str,
        priority: MessagePriority,
        message_class: Optional[str | int] = None,
    ) -> None:
        """Send an UnconfirmedTextMessage."""
        ...

    async def life_safety_operation(
        self,
        address: str,
        process_id: int,
        operation: LifeSafetyOperation,
        target: ObjectIdentifier,
        source: Optional[str] = None,
    ) -> None:
        """Send a LifeSafetyOperation request."""
        ...

    async def get_enrollment_summary(
        self,
        address: str,
        ack_filter: int = 0,
        event_state: Optional[EventState] = None,
        event_type: Optional[EventType] = None,
        min_priority: Optional[int] = None,
        max_priority: Optional[int] = None,
        notification_class: Optional[int] = None,
    ) -> list[dict[str, Any]]:
        """Get enrollment summary from a remote device."""
        ...

    async def get_alarm_summary(self, address: str) -> list[dict[str, Any]]:
        """Get alarm summary from a remote device."""
        ...

    async def who_am_i(self) -> None:
        """Broadcast Who-Am-I."""
        ...

    async def write_group(
        self,
        address: str,
        group_number: int,
        write_priority: int,
        change_list: list[dict[str, Any]],
    ) -> None:
        """Send a WriteGroup request."""
        ...

    async def read_bdt(self, address: str) -> list[BdtEntry]:
        """Read the Broadcast Distribution Table from a BBMD."""
        ...

    async def read_fdt(self, address: str) -> list[FdtEntry]:
        """Read the Foreign Device Table from a BBMD."""
        ...

    async def who_is_router_to_network(
        self,
        network: Optional[int] = None,
        timeout_ms: int = 3000,
    ) -> list[RouterInfo]:
        """Broadcast Who-Is-Router-To-Network and collect responses."""
        ...

    async def stop(self) -> None:
        """Stop the client and release resources."""
        ...


# ---------------------------------------------------------------------------
# Server
# ---------------------------------------------------------------------------

class BACnetServer:
    """BACnet server that hosts objects and responds to client requests.

    Usage::

        server = BACnetServer(
            device_instance=1234,
            device_name="My Device",
            interface="0.0.0.0",
            port=47808,
        )
        server.add_analog_input(1, "Temperature", 62)
        await server.start()
    """

    def __init__(
        self,
        device_instance: int,
        device_name: str,
        interface: str = "0.0.0.0",
        port: int = 0xBAC0,
        broadcast_address: str = "255.255.255.255",
        vendor_name: str = "Rusty BACnet",
        vendor_id: int = 0,
        model_name: str = "rusty-bacnet",
        firmware_revision: str = "0.7.0",
        application_software_version: str = "0.7.0",
        max_apdu_length: int = 1476,
        segmentation_supported: Optional[Segmentation] = None,
        apdu_timeout: int = 6000,
        apdu_retries: int = 3,
        dcc_password: Optional[str] = None,
        reinit_password: Optional[str] = None,
        transport: Literal["bip", "ipv6"] = "bip",
        ipv6_interface: Optional[str] = None,
    ) -> None: ...

    # Object creation methods
    def add_analog_input(self, instance: int, name: str, units: int) -> None: ...
    def add_analog_output(self, instance: int, name: str, units: int) -> None: ...
    def add_analog_value(self, instance: int, name: str, units: int) -> None: ...
    def add_binary_input(self, instance: int, name: str) -> None: ...
    def add_binary_output(self, instance: int, name: str) -> None: ...
    def add_binary_value(self, instance: int, name: str) -> None: ...
    def add_multistate_input(self, instance: int, name: str, number_of_states: int) -> None: ...
    def add_multistate_output(self, instance: int, name: str, number_of_states: int) -> None: ...
    def add_multistate_value(self, instance: int, name: str, number_of_states: int) -> None: ...
    def add_calendar(self, instance: int, name: str) -> None: ...
    def add_schedule(self, instance: int, name: str) -> None: ...
    def add_notification_class(self, instance: int, name: str, priority: list[int] | None = None) -> None: ...
    def add_trend_log(self, instance: int, name: str, buffer_size: int) -> None: ...
    def add_loop(self, instance: int, name: str, output_units: int) -> None: ...
    def add_file(self, instance: int, name: str, file_type: str) -> None: ...
    def add_network_port(self, instance: int, name: str, network_type: int) -> None: ...
    def add_event_enrollment(self, instance: int, name: str, event_type: int) -> None: ...
    def add_program(self, instance: int, name: str) -> None: ...
    def add_command(self, instance: int, name: str) -> None: ...
    def add_timer(self, instance: int, name: str) -> None: ...
    def add_load_control(self, instance: int, name: str) -> None: ...
    def add_lighting_output(self, instance: int, name: str) -> None: ...
    def add_binary_lighting_output(self, instance: int, name: str) -> None: ...
    def add_life_safety_point(self, instance: int, name: str) -> None: ...
    def add_life_safety_zone(self, instance: int, name: str) -> None: ...
    def add_group(self, instance: int, name: str) -> None: ...
    def add_global_group(self, instance: int, name: str) -> None: ...
    def add_structured_view(self, instance: int, name: str) -> None: ...
    def add_channel(self, instance: int, name: str, channel_number: int) -> None: ...
    def add_staging(self, instance: int, name: str, num_stages: int) -> None: ...
    def add_accumulator(self, instance: int, name: str, units: int) -> None: ...
    def add_pulse_converter(self, instance: int, name: str, units: int) -> None: ...
    def add_audit_log(self, instance: int, name: str, buffer_size: int) -> None: ...
    def add_audit_reporter(self, instance: int, name: str) -> None: ...
    def add_event_log(self, instance: int, name: str, buffer_size: int) -> None: ...
    def add_trend_log_multiple(self, instance: int, name: str, buffer_size: int) -> None: ...
    def add_integer_value(self, instance: int, name: str) -> None: ...
    def add_positive_integer_value(self, instance: int, name: str) -> None: ...
    def add_large_analog_value(self, instance: int, name: str) -> None: ...
    def add_character_string_value(self, instance: int, name: str) -> None: ...
    def add_octet_string_value(self, instance: int, name: str) -> None: ...
    def add_bit_string_value(self, instance: int, name: str) -> None: ...
    def add_date_value(self, instance: int, name: str) -> None: ...
    def add_time_value(self, instance: int, name: str) -> None: ...
    def add_date_time_value(self, instance: int, name: str) -> None: ...
    def add_date_pattern_value(self, instance: int, name: str) -> None: ...
    def add_time_pattern_value(self, instance: int, name: str) -> None: ...
    def add_date_time_pattern_value(self, instance: int, name: str) -> None: ...
    def add_lift(self, instance: int, name: str, num_floors: int) -> None: ...
    def add_escalator(self, instance: int, name: str) -> None: ...
    def add_averaging(self, instance: int, name: str) -> None: ...

    # Server lifecycle
    async def start(self) -> None:
        """Start the server and begin accepting BACnet requests."""
        ...

    async def stop(self) -> None:
        """Stop the server and release resources."""
        ...

    async def local_address(self) -> str:
        """Get the local address the server is listening on."""
        ...

    # Server-side property access
    async def read_property(
        self,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        array_index: Optional[int] = None,
    ) -> PropertyValue:
        """Read a property from a local object."""
        ...

    async def write_property_local(
        self,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        value: Any,
        priority: Optional[int] = None,
        array_index: Optional[int] = None,
    ) -> None:
        """Write a property on a local object."""
        ...

    async def comm_state(self) -> int:
        """Get the current DeviceCommunicationControl state (0=Enable, 1=Disable, 2=DisableInitiation)."""
        ...


# ---------------------------------------------------------------------------
# SC Hub
# ---------------------------------------------------------------------------

class ScHub:
    """BACnet/SC Hub for relaying messages between SC nodes.

    Usage::

        hub = await ScHub.start("0.0.0.0:47809", cert_path, key_path, ca_path, vmac)
        # ... hub is running ...
        await hub.stop()
    """

    @staticmethod
    async def start(
        bind_address: str,
        cert_path: str,
        key_path: str,
        ca_path: str,
        vmac: bytes,
    ) -> ScHub:
        """Start the SC hub on the given address with TLS configuration."""
        ...

    def local_address(self) -> str:
        """Get the address the hub is listening on."""
        ...

    async def stop(self) -> None:
        """Stop the hub."""
        ...
