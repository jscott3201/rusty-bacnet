//! Static BACnet reference content compiled into the binary.
//!
//! All content is original reference material (not spec text).

use bacnet_types::enums::ObjectType;

pub const OBJECT_TYPES_INDEX: &str = "\
BACnet Object Types — 65 standard types defined in ASHRAE 135-2020.

Input Objects (physical sensors/contacts):
  analog-input     — Analog sensor (temperature, pressure, humidity, CO2, etc.)
  binary-input     — Digital contact (switch, relay status, occupancy sensor)
  multi-state-input — Multi-position sensor (fan speed selector, damper position)

Output Objects (physical actuators):
  analog-output    — Analog actuator (valve, damper, VFD speed)
  binary-output    — Digital actuator (relay, contactor, on/off valve)
  multi-state-output — Multi-position actuator (fan speed command)

Value Objects (software points, no physical I/O):
  analog-value     — Floating-point setpoint or calculation result
  binary-value     — Boolean flag or mode switch
  multi-state-value — Enumerated mode or state
  integer-value    — Integer setpoint or counter
  large-analog-value — Double-precision floating point
  positive-integer-value — Unsigned integer value
  datetime-value   — Date/time value
  characterstring-value — Text string value
  octetstring-value — Raw byte string value
  bitstring-value  — Bit field value
  time-value       — Time-of-day value
  date-value       — Date value
  datepattern-value — Date pattern for scheduling
  timepattern-value — Time pattern for scheduling
  datetimepattern-value — Combined date+time pattern for scheduling

Device & Infrastructure:
  device           — Represents the BACnet device itself (required in every device)
  network-port     — Network interface configuration (BIP port, MS/TP port, etc.)

Scheduling:
  schedule         — Weekly schedule with exception dates
  calendar         — List of date patterns for holidays/exceptions

Trending:
  trend-log        — Historical data logging (samples at intervals or on COV)
  trend-log-multiple — Multi-object trend logging

Alarming & Events:
  notification-class — Alarm routing configuration (who gets notified)
  event-enrollment — Event detection rules (out-of-range, change-of-state, etc.)
  event-log        — Historical event/alarm log
  notification-forwarder — Forwards alarm notifications between networks

File & Data:
  file             — File transfer object (firmware upload, config backup)
  pulse-converter  — Converts pulse count input to analog value
  accumulator      — Pulse counter with prescale
  averaging        — Statistical averaging over time period

Advanced:
  loop             — PID control loop
  program          — Programmable logic program
  command          — Macro command (write multiple values atomically)
  group            — Object grouping for bulk reads
  structured-view  — Hierarchical organization of objects
  global-group     — Cross-device object grouping
  access-door      — Door access control
  access-point     — Access control point
  access-zone      — Access control zone
  access-credential — Access credential
  credential-data-input — Access control credential reader
  access-rights    — Access rights definition
  access-user      — Access control user
  life-safety-point — Life safety input (smoke, fire, sprinkler)
  life-safety-zone — Life safety zone grouping
  load-control     — Demand response / load shedding
  lighting-output  — Lighting control with dimming
  binary-lighting-output — On/off lighting control
  channel          — Lighting channel grouping
  color            — Color control (RGB/CIE)
  color-temperature — Color temperature control
  elevator-group   — Elevator bank grouping
  escalator        — Escalator monitoring
  lift             — Elevator/lift monitoring
  staging          — Multi-stage equipment sequencing
  timer            — Countdown/countup timer
  audit-log        — Security audit trail
  audit-reporter   — Audit event reporter
  alert-enrollment — Alert detection enrollment
  network-security — Network security configuration

Use bacnet://reference/object-types/{type-name} to get detailed info on any type.
";

pub const PROPERTIES: &str = "\
BACnet Properties — Common properties found on most object types.

present-value: The current value of the object. For inputs, this is the sensor reading. \
For outputs, this is the commanded value. For values, this is the software point value. \
Type depends on the object: Real for analog, Enumerated for binary (0=inactive, 1=active), \
Unsigned for multi-state.

object-name: Human-readable name string. Must be unique within a device. \
Used for discovery (WhoHas service).

object-type: Enumerated value identifying the type (0=analog-input, 1=analog-output, etc.).

object-identifier: Combination of object-type and instance-number. Globally unique within a device.

status-flags: 4-bit field indicating object state:
  Bit 0 — IN_ALARM: an alarm condition is active
  Bit 1 — FAULT: a fault has been detected (see reliability property)
  Bit 2 — OVERRIDDEN: value is being overridden by hardware or external system
  Bit 3 — OUT_OF_SERVICE: object is disconnected from physical I/O

out-of-service: Boolean. When true, present-value is decoupled from physical I/O. \
The value can be written manually for testing. Status-flags bit 3 mirrors this.

reliability: Indicates fault condition:
  0 = no-fault-detected (normal)
  2 = over-range (sensor reading above max-pres-value)
  3 = under-range (sensor reading below min-pres-value)
  7 = unreliable-other
  See bacnet://reference/reliability for the complete list.

event-state: Current alarm state:
  0 = normal
  1 = fault
  2 = offnormal
  3 = high-limit (analog exceeded high-limit)
  4 = low-limit (analog exceeded low-limit)
  5 = life-safety-alarm (life safety system triggered)

units: Engineering units for analog objects (e.g., degrees-fahrenheit=62, percent=98). \
See bacnet://reference/units for the complete list.

priority-array: Array of 16 command slots for commandable objects (outputs and values). \
See bacnet://reference/priority-array for how priorities work.

description: Free-text description of the object's purpose.

cov-increment: For analog objects, the minimum change in present-value that triggers \
a COV (Change of Value) notification. Smaller values = more notifications.

high-limit / low-limit: Alarm thresholds for analog objects.

deadband: Hysteresis value for returning to normal from an alarm state.

polarity: For binary objects, whether the physical state is normal (0) or reversed (1).

relinquish-default: The fallback value used when all 16 priority slots are null.
";

pub const UNITS: &str = "\
BACnet Engineering Units — Common units for analog objects.

Temperature: degrees-celsius (62), degrees-fahrenheit (64), degrees-kelvin (63)
Pressure: pascals (53), kilopascals (54), bars (55), psi (56), centimeters-of-water (57)
Flow: liters-per-second (87), cubic-meters-per-hour (135), cubic-feet-per-minute (84)
Humidity: percent-relative-humidity (29)
Speed: rpm (104), meters-per-second (74)
Electrical: volts (5), amperes (3), watts (47), kilowatts (48), kilowatt-hours (19)
Light: lux (37), foot-candles (38)
Concentration: ppm (96), percent (98)
Time: seconds (73), minutes (72), hours (71), days (70)
Dimensionless: no-units (95), percent (98)

The 'units' property on an analog object is an enumerated value from this list. \
When reading a present-value, always check the units property to understand what the number means.
";

pub const ERRORS: &str = "\
BACnet Errors — Error classes and codes returned by devices.

Error Class: DEVICE (device-level issues)
  operational-problem — device is in a state that prevents the operation
  configuration-error — device configuration issue
  internal-error — unexpected device-internal failure

Error Class: OBJECT (object-level issues)
  unknown-object — the requested object does not exist on this device
  object-identifier-already-exists — CreateObject failed, object already exists
  no-space-for-object — device cannot create more objects (resource limit)
  dynamic-creation-not-supported — device doesn't support CreateObject

Error Class: PROPERTY (property-level issues)
  unknown-property — this object type does not have the requested property
  read-access-denied — property exists but cannot be read
  write-access-denied — property exists but cannot be written
  value-out-of-range — the written value is outside the acceptable range
  not-writable — this property is read-only on this device

Error Class: SERVICES (service-level issues)
  inconsistent-parameters — request parameters are contradictory
  invalid-parameter-data-type — wrong data type for this property
  service-request-denied — device rejected the request
  other — unspecified error

Error Class: RESOURCES
  no-space-for-object — out of memory/storage for new objects
  no-space-to-write-property — out of storage for property data

Error Class: SECURITY
  authentication-failed — access credentials rejected
  not-configured — security not configured on this device

Common troubleshooting:
  unknown-object → verify the object type and instance number exist on the target device
  unknown-property → the device may not support this property; check the property-list
  write-access-denied → the property may be read-only, or a password/authentication is required
  value-out-of-range → check min/max constraints; for analog writes, verify units match
  service-request-denied → device may be in DCC disable state or the service is not supported
";

pub const RELIABILITY: &str = "\
BACnet Reliability Values — Indicates why an object is in a fault state.

0 = no-fault-detected — Normal operation, no issues.
1 = no-sensor — Physical sensor is not connected or not responding.
2 = over-range — Sensor reading exceeds the max-pres-value. Check physical sensor.
3 = under-range — Sensor reading is below the min-pres-value. Check physical sensor.
4 = open-loop — Control loop is open (feedback missing for output objects).
5 = shorted-loop — Control loop is shorted (output objects).
6 = no-output — Output device is not responding.
7 = unreliable-other — Unspecified reliability issue.
8 = process-error — Internal processing error.
9 = multi-state-fault — Multi-state object in an invalid state number.
10 = configuration-error — Object is misconfigured.
12 = communication-failure — Communication with a monitored device or service has failed.
13 = member-fault — A member of a group or collection is in fault.
14 = monitored-object-fault — The object being monitored is in fault.
15 = tripped — A protective device (breaker, fuse) has tripped.
16 = lamp-failure — A lamp has burned out or failed.
17 = activation-failure — Failed to activate (e.g., damper actuator stuck).
18 = renew-dhcp-failure — DHCP lease renewal failed.
19 = renew-fd-registration-failure — Foreign device registration renewal failed.
20 = restart-auto-negotiation-failure — Network auto-negotiation failed after restart.
21 = restart-failure — Device restart failed.
22 = proprietary-command-failure — A vendor-specific command failed.
23 = faults-listed — Multiple faults are active (check fault-values property for list).
24 = referenced-object-fault — A referenced object is in fault.

When reliability is non-zero:
  - status-flags.fault will be true
  - event-state will typically be 'fault'
  - present-value may be stale or invalid

To clear a fault:
  - Fix the underlying physical or configuration issue
  - For some faults, writing out-of-service=true then false can reset the state
  - Check the device's event/alarm log for more context
";

pub const PRIORITY_ARRAY: &str = "\
BACnet Priority Array — 16-level command priority scheme for outputs and commandable values.

Priority 1:  Manual-Life-Safety     (highest — fire, smoke, emergency override)
Priority 2:  Automatic-Life-Safety  (automatic fire/safety systems)
Priority 3:  (available)
Priority 4:  (available)
Priority 5:  Critical-Equipment-Control (critical equipment protection)
Priority 6:  Minimum-On/Off         (minimum runtime protection)
Priority 7:  (available)
Priority 8:  Manual-Operator        (operator manual override from workstation)
Priority 9:  (available)
Priority 10: (available)
Priority 11: (available)
Priority 12: (available)
Priority 13: (available)
Priority 14: (available)
Priority 15: (available)
Priority 16: (available — lowest, often used for scheduling/default)

How it works:
  - Each priority level is a slot that can hold a value or be null.
  - The present-value is determined by the highest (lowest-numbered) non-null slot.
  - Writing present-value with a priority sets that slot; writing null at a priority relinquishes it.
  - If ALL 16 slots are null, present-value falls back to relinquish-default.

Common patterns:
  - BAS schedules write at priority 16
  - Operator overrides write at priority 8
  - Safety systems write at priority 1 or 2
  - To release an override: write null at the override's priority level

Pitfalls:
  - Writing without specifying a priority defaults to priority 16 (lowest)
  - A value stuck at a high priority blocks lower-priority commands
  - Reading priority-array shows all 16 slots; look for non-null entries to find active commands
";

pub const NETWORKING: &str = "\
BACnet Networking — How devices communicate across networks.

Network Numbers:
  Every BACnet network segment has a unique network number (1-65534).
  Devices on the same physical segment share a network number.
  Network 0 means 'local network' (no routing needed).

Transports:
  BACnet/IP (BIP) — UDP/IP, uses BVLL framing (Annex J). Most common.
  BACnet/SC — WebSocket over TLS, hub-and-spoke topology. Modern/secure.
  MS/TP — RS-485 serial token-passing. Common for field devices.
  BACnet/IPv6 — UDP over IPv6 with virtual MAC addresses.
  Ethernet — Raw IEEE 802.3 LLC frames.

Routing:
  BACnet routers connect different network segments.
  Each router port is assigned to a network number.
  The router forwards messages between networks based on the destination network in the NPDU header.
  Routing is transparent — devices don't need to know about routers for basic communication.
  Who-Is-Router-To-Network discovers which router can reach a given network.
  I-Am-Router-To-Network announces reachability.

BBMDs (BACnet Broadcast Management Devices):
  On BACnet/IP, UDP broadcasts don't cross IP subnets.
  BBMDs solve this by forwarding broadcasts between subnets.
  Each BBMD maintains a BDT (Broadcast Distribution Table) listing all BBMDs.
  BBMDs forward Original-Broadcast-NPDU to all BDT peers.

Foreign Devices:
  A device on a remote subnet that doesn't have its own BBMD.
  Registers with a BBMD, which forwards broadcasts to it.
  Must periodically re-register (TTL-based).
  The BBMD maintains an FDT (Foreign Device Table) of registered devices.

Common Issues:
  - Devices not discovered → check BBMD configuration, verify BDT entries
  - Cross-subnet communication fails → BBMDs not configured or BDT incomplete
  - Intermittent connectivity → foreign device TTL expiring, re-registration failing
  - MS/TP devices unreachable → check router, verify RS-485 wiring and baud rate
";

pub const SERVICES: &str = "\
BACnet Services — When to use each one.

Property Access:
  ReadProperty — Read one property from one object. Simple, low overhead.
  ReadPropertyMultiple (RPM) — Read multiple properties from multiple objects in one request. \
    Much more efficient for bulk reads. Use this when reading more than 2-3 properties.
  WriteProperty — Write one property value with optional priority.
  WritePropertyMultiple (WPM) — Write multiple properties in one request.

Discovery:
  WhoIs — Broadcast to discover devices. Optionally filter by instance range.
  IAm — Response to WhoIs. Contains device instance, vendor ID, max APDU, segmentation support.
  WhoHas — Find devices that have an object with a specific name or identifier.
  IHave — Response to WhoHas.

Change of Value (COV):
  SubscribeCOV — Subscribe to value changes. More efficient than polling.
    Confirmed: device sends confirmed notification (reliable, device retries).
    Unconfirmed: device sends unconfirmed notification (fire-and-forget).
  SubscribeCOVProperty — Subscribe to a specific property (vs. all COV properties).
  Lifetime: subscriptions expire. Client must re-subscribe before lifetime ends.

Object Management:
  CreateObject — Create a new object on a remote device.
  DeleteObject — Delete an object from a remote device.
  AddListElement / RemoveListElement — Modify list properties.

Device Management:
  DeviceCommunicationControl (DCC) — Enable/disable a device's communication.
  ReinitializeDevice — Trigger a warm/cold restart.
  TimeSynchronization — Set the device's clock.

Alarms & Events:
  GetEventInformation — Query active alarms/events.
  AcknowledgeAlarm — Acknowledge an alarm condition.
  GetEnrollmentSummary — List event enrollment objects and their states.

File Access:
  AtomicReadFile — Read from a file object (firmware, logs, config).
  AtomicWriteFile — Write to a file object.
  ReadRange — Read a range of records from a list (trend logs, event logs).
";

pub const TROUBLESHOOTING: &str = "\
BACnet Troubleshooting — Common problems and diagnostic steps.

Device Not Responding:
  1. Verify the device is powered on and connected to the network
  2. Check the network number — is it on a different segment requiring a router?
  3. Try discover_devices tool to see if the device responds to WhoIs
  4. Use ping_device tool to check reachability of a specific device
  5. Check if the device is in DCC (DeviceCommunicationControl) disabled state
  6. Verify the MAC address is correct (use list_known_devices to check cached MAC)
  7. For MS/TP: check RS-485 wiring, baud rate, and station address

Property Read Returns Error:
  unknown-object → the object doesn't exist; list the device's objects first
  unknown-property → this property isn't supported; read property-list to see what's available
  read-access-denied → security restrictions; may need authentication

Cannot Write to Property:
  write-access-denied → property may be read-only, or higher-priority command is active
  value-out-of-range → value is outside the object's acceptable range
  For commandable objects: check priority-array for higher-priority overrides

COV Notifications Not Arriving:
  1. Verify the subscription is active (list_cov_subscriptions)
  2. Check if the subscription lifetime has expired
  3. For confirmed COV: the server may be in DCC disabled-initiation state
  4. Check cov-increment — if too large, small changes won't trigger
  5. Verify network connectivity between subscriber and notifier

Cross-Network Communication Fails:
  1. Check routing table (use get_routing_table tool) — is there a route to the destination network?
  2. Use who_is_router_to_network tool to discover available routers
  3. Verify router ports are active (use read_router_network_ports tool on the router device)
  4. For BIP across subnets: check BBMD BDT configuration (use read_bdt tool)
  5. For MS/TP: verify the router's serial port is active and baud rate matches

BBMD Issues:
  1. Read BDT from each BBMD (use read_bdt tool) — all should have matching entries
  2. Verify BBMDs can reach each other (check for firewall rules on port 47808)
  3. For foreign devices: verify registration is active (use read_fdt tool)
  4. Check if foreign device TTL is expiring before re-registration
";

/// Generate detailed reference content for a specific object type.
pub fn object_type_detail(type_name: &str) -> Option<String> {
    let normalized = type_name.to_lowercase().replace('-', "_");

    // Find the matching ObjectType.
    let obj_type = ObjectType::ALL_NAMED
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&normalized))
        .map(|(_, val)| *val)?;

    let display_name = type_name.replace('_', "-").to_lowercase();

    // Generate content based on the type category.
    let content = match obj_type {
        ObjectType::ANALOG_INPUT => format!(
            "\
{display_name} (type {})

Category: Input / Sensor
Commandable: No (read-only from physical sensor)
COV Support: Yes (triggers on present-value change exceeding cov-increment)

Purpose:
Represents a physical analog sensor — temperature, pressure, humidity, flow rate, CO2 level, etc. \
The present-value reflects the current sensor reading in the configured engineering units.

Key Properties:
  present-value — Current sensor reading (Real)
  units — Engineering units (e.g., degrees-fahrenheit, pascals)
  out-of-service — When true, present-value is manually set (not from sensor)
  status-flags — [in-alarm, fault, overridden, out-of-service]
  reliability — no-fault-detected, over-range, under-range, open-loop, etc.
  high-limit / low-limit — Alarm thresholds
  deadband — Hysteresis for alarm return-to-normal
  cov-increment — Minimum change to trigger COV notification
  min-pres-value / max-pres-value — Expected sensor range
  event-state — normal, high-limit, low-limit, fault

Common Configurations:
  Temperature sensor: units=degrees-fahrenheit, high-limit=90, low-limit=55
  Pressure sensor: units=pascals, cov-increment=100
  Humidity sensor: units=percent-relative-humidity

Troubleshooting:
  reliability=over-range → sensor reading above max-pres-value, check physical sensor
  reliability=under-range → sensor reading below min-pres-value, check wiring
  status-flags.fault=true → check reliability value for specific fault type
  present-value not updating → check out-of-service flag
",
            obj_type.to_raw()
        ),

        ObjectType::ANALOG_OUTPUT => format!(
            "\
{display_name} (type {})

Category: Output / Actuator
Commandable: Yes (via 16-level priority array)
COV Support: Yes

Purpose:
Represents a physical analog actuator — valve position, damper position, VFD speed, dimmer level. \
The present-value is the commanded output percentage or engineering value.

Key Properties:
  present-value — Commanded output value (Real)
  priority-array — 16 command priority slots (see bacnet://reference/priority-array)
  relinquish-default — Fallback value when all priority slots are null
  units — Engineering units for the output value
  min-pres-value / max-pres-value — Output range limits
  out-of-service — When true, output is disconnected from physical actuator

Troubleshooting:
  Output not responding → check priority-array for higher-priority overrides
  Value stuck → look for non-null entries in priority-array above your write priority
  To release an override → write null at that priority level
",
            obj_type.to_raw()
        ),

        ObjectType::ANALOG_VALUE => format!(
            "\
{display_name} (type {})

Category: Value / Software Point
Commandable: Yes (via 16-level priority array)
COV Support: Yes

Purpose:
A software analog point with no physical I/O. Used for setpoints, calculation results, \
intermediate values, and configuration parameters. Commonly used for zone temperature setpoints, \
schedule outputs, and energy calculations.

Key Properties:
  present-value — Current value (Real)
  priority-array — 16 command priority slots
  relinquish-default — Fallback when all priorities null
  units — Engineering units (often degrees or percent)
  cov-increment — Minimum change for COV notification

Common Uses:
  Zone temperature setpoint: units=degrees-fahrenheit, relinquish-default=72.0
  Calculated value: energy totalization, average, etc.
  Configuration parameter: PID tuning constants
",
            obj_type.to_raw()
        ),

        ObjectType::BINARY_INPUT => format!(
            "\
{display_name} (type {})

Category: Input / Sensor
Commandable: No
COV Support: Yes (triggers on any state change)

Purpose:
Represents a physical digital input — door switch, relay status, occupancy sensor, \
limit switch, or any on/off sensor. Present-value is 0 (inactive) or 1 (active).

Key Properties:
  present-value — Current state: inactive (0) or active (1)
  polarity — Normal (0) or Reverse (1). Reverse inverts the physical input.
  out-of-service — When true, present-value is decoupled from physical input
  status-flags — [in-alarm, fault, overridden, out-of-service]
",
            obj_type.to_raw()
        ),

        ObjectType::BINARY_OUTPUT => format!(
            "\
{display_name} (type {})

Category: Output / Actuator
Commandable: Yes (via 16-level priority array)
COV Support: Yes

Purpose:
Represents a physical digital output — relay, contactor, on/off valve, fan start/stop. \
Present-value is inactive (0) or active (1).

Key Properties:
  present-value — Commanded state: inactive (0) or active (1)
  priority-array — 16 command priority slots
  relinquish-default — Fallback when all priorities null
  polarity — Normal or Reverse
  minimum-on-time / minimum-off-time — Minimum runtime protection (seconds)
",
            obj_type.to_raw()
        ),

        ObjectType::BINARY_VALUE => format!(
            "\
{display_name} (type {})

Category: Value / Software Point
Commandable: Yes (via 16-level priority array)
COV Support: Yes

Purpose:
A software boolean point with no physical I/O. Used for mode flags, enable/disable switches, \
and boolean logic results. Present-value is inactive (0) or active (1).

Key Properties:
  present-value — Current state: inactive (0) or active (1)
  priority-array — 16 command priority slots
  relinquish-default — Fallback when all priorities null
",
            obj_type.to_raw()
        ),

        ObjectType::DEVICE => format!(
            "\
{display_name} (type {})

Category: Infrastructure (required — one per device)
Commandable: No
COV Support: No

Purpose:
The Device object represents the BACnet device itself. Every device must have exactly one. \
It exposes device identity, protocol support, and configuration.

Key Properties:
  object-name — Device name (human-readable)
  system-status — operational(0), non-operational, etc.
  vendor-name — Manufacturer name
  vendor-identifier — ASHRAE vendor ID number
  model-name — Device model
  firmware-revision — Firmware version string
  application-software-version — Application version
  protocol-version — BACnet protocol version (typically 1)
  protocol-revision — Protocol revision (higher = newer features)
  max-apdu-length-accepted — Maximum message size this device handles
  segmentation-supported — Whether device supports segmented messages
  object-list — Array of all object identifiers in this device
  protocol-services-supported — Bitstring of supported services
  protocol-object-types-supported — Bitstring of supported object types
",
            obj_type.to_raw()
        ),

        ObjectType::SCHEDULE => format!(
            "\
{display_name} (type {})

Category: Scheduling
Commandable: No
COV Support: Yes

Purpose:
Implements a weekly time schedule with exception dates (holidays). Writes a value to one or more \
objects at specified times. The present-value reflects the current scheduled output.

Key Properties:
  present-value — Current schedule output value
  weekly-schedule — Array of 7 daily schedules (Monday–Sunday), each with time/value pairs
  exception-schedule — Special dates that override the weekly schedule
  schedule-default — Value when no schedule entry is active
  list-of-object-property-references — Objects that receive the scheduled value
  effective-period — Date range when the schedule is active
",
            obj_type.to_raw()
        ),

        ObjectType::TREND_LOG => format!(
            "\
{display_name} (type {})

Category: Trending / Data Logging
Commandable: No
COV Support: No

Purpose:
Records historical data for an object property. Samples at fixed intervals (polling) \
or on change-of-value (COV). Stores timestamped records in a circular buffer.

Key Properties:
  log-device-object-property — The object/property being logged
  logging-type — Polled (1), COV (2), or Triggered (3)
  log-interval — Polling interval in centiseconds (e.g., 6000 = 60 seconds)
  stop-when-full — false = circular buffer, true = stops when full
  buffer-size — Maximum number of records
  record-count — Current number of records
  total-record-count — Total records ever logged (may exceed buffer-size)
  enable — true to start logging
",
            obj_type.to_raw()
        ),

        ObjectType::NOTIFICATION_CLASS => format!(
            "\
{display_name} (type {})

Category: Alarming
Commandable: No
COV Support: No

Purpose:
Defines how alarm notifications are routed. Specifies recipients, priorities, \
and which transitions (to-offnormal, to-fault, to-normal) generate notifications.

Key Properties:
  notification-class — The class number (referenced by event-enabled objects)
  recipient-list — List of (device, address) recipients for each transition
  priority — Array of 3 priorities (to-offnormal, to-fault, to-normal)
  ack-required — Which transitions require acknowledgment
",
            obj_type.to_raw()
        ),

        ObjectType::NETWORK_PORT => format!(
            "\
{display_name} (type {})

Category: Infrastructure / Network Configuration
Commandable: No
COV Support: No

Purpose:
Represents a network interface on the device. Exposes configuration for BACnet/IP ports, \
MS/TP ports, BACnet/SC connections, etc. One Network Port object per physical or logical interface.

Key Properties:
  network-type — Port type: ipv4 (5), mstp (9), sc (14), etc.
  network-number — BACnet network number assigned to this port
  mac-address — This port's MAC address
  link-speed — Physical link speed
  ip-address / ip-subnet-mask — For BIP ports
  bdt-table — For BIP BBMD ports
  fd-bbmd-address — For foreign device ports
  bacnet-ip-udp-port — UDP port (default 47808)
  routing-table — For routers, the routing table entries
  command — restart/other management commands
",
            obj_type.to_raw()
        ),

        ObjectType::MULTI_STATE_INPUT => format!(
            "\
{display_name} (type {})

Category: Input / Sensor
Commandable: No
COV Support: Yes (triggers on any state change)

Purpose:
Represents a multi-position physical input — fan speed selector, damper position switch, \
or any sensor with discrete states. Present-value is an unsigned integer (1-based) \
representing the current state.

Key Properties:
  present-value — Current state number (Unsigned, 1 to number-of-states)
  number-of-states — How many valid states this input has
  state-text — Optional array of human-readable names for each state
  out-of-service — When true, present-value is manually set
  status-flags — [in-alarm, fault, overridden, out-of-service]

Common Configurations:
  Fan speed: number-of-states=3, state-text=[\"Off\", \"Low\", \"High\"]
  Damper position: number-of-states=3, state-text=[\"Closed\", \"Partially Open\", \"Fully Open\"]
",
            obj_type.to_raw()
        ),

        ObjectType::MULTI_STATE_OUTPUT => format!(
            "\
{display_name} (type {})

Category: Output / Actuator
Commandable: Yes (via 16-level priority array)
COV Support: Yes

Purpose:
Represents a multi-position physical output — fan speed command, multi-stage equipment, \
or any actuator with discrete positions. Present-value is an unsigned integer (1-based).

Key Properties:
  present-value — Commanded state number (Unsigned, 1 to number-of-states)
  priority-array — 16 command priority slots
  relinquish-default — Fallback state when all priorities null
  number-of-states — How many valid states
  state-text — Optional array of names for each state
",
            obj_type.to_raw()
        ),

        ObjectType::MULTI_STATE_VALUE => format!(
            "\
{display_name} (type {})

Category: Value / Software Point
Commandable: Yes (via 16-level priority array)
COV Support: Yes

Purpose:
A software multi-state point with no physical I/O. Used for mode selection, \
operating modes, and enumerated configuration values.

Key Properties:
  present-value — Current state number (Unsigned, 1 to number-of-states)
  priority-array — 16 command priority slots
  relinquish-default — Fallback state
  number-of-states — How many valid states
  state-text — Optional array of names for each state

Common Uses:
  Operating mode: number-of-states=4, state-text=[\"Off\", \"Heat\", \"Cool\", \"Auto\"]
  Occupancy mode: number-of-states=3, state-text=[\"Unoccupied\", \"Occupied\", \"Standby\"]
",
            obj_type.to_raw()
        ),

        ObjectType::INTEGER_VALUE => format!(
            "\
{display_name} (type {})

Category: Value / Software Point
Commandable: Yes (via 16-level priority array)
COV Support: Yes

Purpose:
A software integer point. Used for counters, integer setpoints, and configuration \
values that require whole numbers. Present-value is a signed 32-bit integer.

Key Properties:
  present-value — Current value (Signed integer)
  priority-array — 16 command priority slots
  relinquish-default — Fallback value
  units — Engineering units
  min-pres-value / max-pres-value — Valid range
",
            obj_type.to_raw()
        ),

        ObjectType::FILE => format!(
            "\
{display_name} (type {})

Category: File / Data Transfer
Commandable: No
COV Support: No

Purpose:
Represents a file on the device — firmware image, configuration backup, log export, etc. \
Used with AtomicReadFile and AtomicWriteFile services for reliable file transfer.

Key Properties:
  file-type — MIME-like type string
  file-size — Size in bytes (may be approximate for stream files)
  modification-date — Last modified timestamp
  archive — Whether the file has been modified since last backup
  read-only — Whether the file can be written
  file-access-method — record-access (fixed records) or stream-access (byte stream)

Usage:
  Use AtomicReadFile to download file contents
  Use AtomicWriteFile to upload new contents
  For record-access files, specify start-record and record-count
  For stream-access files, specify start-position and byte-count
",
            obj_type.to_raw()
        ),

        ObjectType::CALENDAR => format!(
            "\
{display_name} (type {})

Category: Scheduling
Commandable: No
COV Support: Yes

Purpose:
A list of dates or date patterns — holidays, special events, maintenance windows. \
Referenced by Schedule objects as exception schedules.

Key Properties:
  present-value — Boolean: true if today matches any entry in the date-list
  date-list — Array of date entries (specific dates, date ranges, or weekly patterns)

Usage:
  Create a Calendar with holiday dates
  Reference it from a Schedule's exception-schedule
  The Schedule uses the Calendar to override its weekly schedule on matching days
",
            obj_type.to_raw()
        ),

        ObjectType::EVENT_ENROLLMENT => format!(
            "\
{display_name} (type {})

Category: Alarming / Event Detection
Commandable: No
COV Support: No

Purpose:
Defines an event detection rule — monitors a property on another object and triggers \
alarm notifications when conditions are met (out-of-range, change-of-state, etc.).

Key Properties:
  event-type — Type of detection: change-of-bitstring (0), change-of-state (1), \
    change-of-value (2), floating-limit (4), out-of-range (5), etc.
  object-property-reference — The object+property being monitored
  notification-class — Which Notification Class handles the alarm routing
  event-parameters — Detection parameters (thresholds, deadbands, time delays)
  event-state — Current state: normal, offnormal, fault, high-limit, low-limit
  event-enable — Which transitions are enabled (to-offnormal, to-fault, to-normal)
  acked-transitions — Which transitions have been acknowledged

Troubleshooting:
  event-state stuck in offnormal → check if the monitored property has returned to normal range
  No notifications → check event-enable bits and notification-class recipient-list
",
            obj_type.to_raw()
        ),

        ObjectType::LOOP => format!(
            "\
{display_name} (type {})

Category: Control
Commandable: No
COV Support: Yes

Purpose:
A PID (Proportional-Integral-Derivative) control loop. Reads a process variable, \
compares it to a setpoint, and writes a control output to maintain the setpoint.

Key Properties:
  present-value — Current output value (0-100% typically)
  controlled-variable-reference — The input (process variable) object+property
  controlled-variable-value — Current process variable reading
  setpoint-reference — The setpoint object+property
  setpoint — Current setpoint value
  manipulated-variable-reference — The output object+property being controlled
  action — direct (output increases when PV > SP) or reverse
  proportional-constant — P gain
  integral-constant — I gain (minutes)
  derivative-constant — D gain (minutes)
  output-units — Engineering units for the output
",
            obj_type.to_raw()
        ),

        // Generic fallback for types without detailed descriptions.
        _ => format!(
            "\
{display_name} (type {})

A standard BACnet object type defined in ASHRAE 135-2020. \
Use read_property to examine its properties, or read the property-list \
property to see which properties this object supports.

Use list_local_objects or read_property with object-list on the Device object \
to find instances of this type on a device.
",
            obj_type.to_raw()
        ),
    };

    Some(content)
}
