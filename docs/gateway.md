# BACnet Gateway

HTTP REST API and MCP (Model Context Protocol) server gateway for BACnet networks.

The gateway bridges BACnet/IP networks to modern HTTP and MCP interfaces, enabling web applications, AI agents, and automation tools to discover devices, read/write properties, and manage a local BACnet object database.

## Table of Contents

- [Overview](#overview)
- [Installation and Building](#installation-and-building)
- [Configuration Reference](#configuration-reference)
- [CLI Reference](#cli-reference)
- [REST API Reference](#rest-api-reference)
- [MCP Server Reference](#mcp-server-reference)
- [Authentication](#authentication)
- [Read-Only Mode](#read-only-mode)
- [Feature Flags](#feature-flags)
- [Example Configurations](#example-configurations)

---

## Overview

The `bacnet-gateway` crate provides two optional server interfaces on top of the Rusty BACnet stack:

- **REST API** -- An Axum-based HTTP API under `/api/v1/` for device discovery, property read/write, and local object CRUD.
- **MCP Server** -- A Model Context Protocol server at `/mcp` exposing BACnet operations as tools and BACnet knowledge as resources, designed for LLM agents.

Both interfaces share the same `GatewayState`, which wraps a BACnet client (for remote device operations) and a local `ObjectDatabase` (shared with the BACnet server).

**Nothing compiles by default.** All HTTP, MCP, and binary dependencies are gated behind feature flags. The core modules (`config`, `state`, `builder`, `parse`) have no web dependencies and are always available.

### Architecture

```
                       +------------------+
                       |   bacnet-gateway |
                       |   (Axum server)  |
                       +--------+---------+
                                |
              +-----------------+-----------------+
              |                                   |
     /api/v1/* (REST)                     /mcp (MCP/SSE)
              |                                   |
              +--------+     +--------------------+
                       |     |
                  GatewayState
                  (Arc-wrapped)
                       |
           +-----------+-----------+
           |                       |
    BACnetClient             ObjectDatabase
    (remote ops)             (local objects,
                              shared with server)
           |
      BipTransport
      (UDP/47808)
```

---

## Installation and Building

The gateway binary requires the `bin` feature, which enables both `http` and `mcp` plus CLI dependencies.

```bash
# Check compilation (no linkage)
cargo check -p bacnet-gateway --features bin

# Build the binary
cargo build -p bacnet-gateway --features bin

# Build release binary
cargo build -p bacnet-gateway --features bin --release

# Run tests (requires features to be enabled)
cargo test -p bacnet-gateway --features http,mcp

# Check only the HTTP API (no MCP)
cargo check -p bacnet-gateway --features http

# Check only the MCP server (no HTTP)
cargo check -p bacnet-gateway --features mcp
```

The binary is output at `target/debug/bacnet-gateway` (or `target/release/bacnet-gateway`).

---

## Configuration Reference

The gateway is configured via a TOML file (default: `gateway.toml`). The top-level structure is:

```toml
[server]           # HTTP/MCP server settings (optional, has defaults)
[device]           # Local BACnet device identity (required)
[transports.bip]   # BACnet/IP transport (optional)
[transports.sc]    # BACnet/SC transport (optional)
[transports.mstp]  # MS/TP transport (optional, Linux only)
[bbmd]             # BBMD configuration (optional)
[foreign_device]   # Foreign device registration (optional)
[[routes]]         # Static routing table entries (optional, repeatable)
[[objects]]        # Pre-populated local objects (optional, repeatable)
```

### `[server]` -- HTTP/MCP Server Settings

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `bind` | `String` | `"127.0.0.1:3000"` | Bind address for the HTTP server (ip:port). |
| `api_key` | `String?` | `None` | API key for bearer token authentication. If omitted, no auth is applied. |
| `read_only` | `bool` | `false` | When `true`, all write operations are rejected. |

### `[device]` -- Local BACnet Device Identity (required)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `instance` | `u32` | *(required)* | Device instance number. Must be 0--4194302. |
| `name` | `String` | *(required)* | Device object name. |
| `vendor_id` | `u16` | `999` | Vendor identifier. |
| `description` | `String` | `""` | Device description. |

### `[transports.bip]` -- BACnet/IP Transport

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `interface` | `String` | `"0.0.0.0"` | Bind interface address. |
| `port` | `u16` | `47808` | UDP port. |
| `broadcast` | `String` | *(required)* | Subnet broadcast address (e.g., `"192.168.1.255"`). |
| `network_number` | `u16` | *(required)* | Network number for this transport. Must be 1--65534, unique across transports. |

### `[transports.sc]` -- BACnet/SC Transport

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `hub_uri` | `String` | *(required)* | WebSocket hub URI (e.g., `"wss://hub.example.com"`). |
| `cert` | `String` | *(required)* | Path to TLS client certificate (PEM). |
| `key` | `String` | *(required)* | Path to TLS private key (PEM). |
| `ca` | `String?` | `None` | Path to CA certificate (PEM). Optional. |
| `network_number` | `u16` | *(required)* | Network number for this transport. Must be 1--65534, unique. |

### `[transports.mstp]` -- MS/TP Transport (Linux only)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `serial_port` | `String` | *(required)* | Serial port path (e.g., `"/dev/ttyUSB0"`). |
| `baud_rate` | `u32` | `76800` | Baud rate. |
| `station_address` | `u8` | *(required)* | Station address (0--254). |
| `max_master` | `u8` | `127` | Max master station address. |
| `network_number` | `u16` | *(required)* | Network number for this transport. Must be 1--65534, unique. |
| `rs485` | `Rs485Config?` | `None` | RS-485 direction control (see below). If omitted, assumes hardware auto-direction. |

### `[transports.mstp.rs485]` -- RS-485 Direction Control (optional)

Most USB RS-485 adapters handle direction switching automatically and need no configuration. For RS-485 hats or transceivers with manual DE/RE control, configure one of these modes:

**GPIO mode** -- for RS-485 hats with a GPIO direction pin (e.g., Seeed Studio RS-485 Shield):

```toml
[transports.mstp.rs485]
mode = "gpio"
gpio_chip = "/dev/gpiochip0"  # default
gpio_line = 18                 # GPIO pin number for DE/RE
active_high = true             # true = HIGH enables transmitter (default)
post_tx_delay_us = 200         # microseconds after TX before switching to RX (default: 200)
```

**Kernel RTS mode** -- for setups where DE/RE is wired to the UART's RTS pin:

```toml
[transports.mstp.rs485]
mode = "kernel-rts"
invert_rts = false             # true if DE is active-low (default: false)
delay_before_send_us = 0       # microseconds delay before TX (default: 0)
delay_after_send_us = 0        # microseconds delay after TX (default: 0)
```

### `[bbmd]` -- BBMD Configuration

Mutually exclusive with `[foreign_device]`. Requires `[transports.bip]` when `enabled = true`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | `bool` | `false` | Enable BBMD on the BIP transport. |
| `bdt` | `[String]` | `[]` | Initial Broadcast Distribution Table entries (IP:port strings). |

### `[foreign_device]` -- Foreign Device Registration

Mutually exclusive with `[bbmd]`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `bbmd` | `String` | *(required)* | BBMD address to register with (IP:port). |
| `ttl` | `u16` | *(required)* | Time-to-live in seconds. |

### `[[routes]]` -- Static Route Entries (repeatable)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `network` | `u16` | *(required)* | Destination network number. |
| `via_transport` | `String` | *(required)* | Transport to route through (`"bip"`, `"sc"`, `"mstp"`). |
| `next_hop` | `String?` | `None` | Next hop address (optional, for routed networks). |

### `[[objects]]` -- Pre-populated Local Objects (repeatable)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `type` | `String` | *(required)* | Object type name (e.g., `"analog-value"`, `"binary-input"`). |
| `instance` | `u32` | *(required)* | Object instance number. |
| `name` | `String` | *(required)* | Object name. |
| `units` | `String?` | `None` | Engineering units (optional). |

### Validation Rules

- `device.instance` must be 0--4194302.
- `[bbmd]` and `[foreign_device]` are mutually exclusive.
- `[bbmd]` with `enabled = true` requires `[transports.bip]`.
- `[transports.mstp]` is only available on Linux.
- Network numbers must be 1--65534 (0 is reserved for local-only, 65535 is reserved for broadcast).
- Network numbers must be unique across all configured transports.

### Complete Example Config

```toml
[server]
bind = "0.0.0.0:3000"
api_key = "my-secret-api-key"
read_only = false

[device]
instance = 389001
name = "Rusty Gateway"
vendor_id = 555
description = "BACnet HTTP/MCP Gateway"

[transports.bip]
interface = "0.0.0.0"
port = 47808
broadcast = "192.168.1.255"
network_number = 1

[transports.sc]
hub_uri = "wss://hub.example.com"
cert = "certs/client.pem"
key = "certs/client.key"
ca = "certs/ca.pem"
network_number = 2

[[routes]]
network = 4
via_transport = "bip"
next_hop = "192.168.1.100:47808"

[[objects]]
type = "analog-value"
instance = 1
name = "Zone Temperature"
units = "degrees-celsius"

[[objects]]
type = "binary-value"
instance = 1
name = "Occupancy Status"
```

---

## CLI Reference

```
bacnet-gateway [OPTIONS]
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config <PATH>` | `-c` | `gateway.toml` | Config file path. |
| `--bind <ADDR>` | `-b` | *(from config)* | Override server bind address (e.g., `0.0.0.0:8080`). |
| `--api-key <KEY>` | `-k` | *(from config)* | Override API key. Also reads `BACNET_GATEWAY_API_KEY` env var. |
| `--verbose` | `-v` | 0 | Increase log verbosity. `-v` = info, `-vv` = debug, `-vvv` = trace. |
| `--quiet` | `-q` | `false` | Suppress all output except errors. |
| `--no-mcp` | | `false` | Disable the MCP endpoint (REST only). |
| `--no-api` | | `false` | Disable the REST API (MCP only). Cannot combine with `--no-mcp`. |
| `--read-only` | | `false` | Disable all write operations (overrides config). |
| `--print-config` | | `false` | Print resolved config and exit (useful for debugging). |

### Log Level Resolution

The log level is determined by the following priority:

1. `RUST_LOG` environment variable (if set, takes precedence).
2. `--quiet` flag: sets level to `error`.
3. `--verbose` count: 0 = `warn` (default), 1 = `info`, 2 = `debug`, 3+ = `trace`.

### API Key Resolution Order

1. `--api-key` CLI flag (highest priority).
2. `api_key` in the TOML config file.
3. `BACNET_GATEWAY_API_KEY` environment variable (lowest priority, only if neither of the above is set).

### Shutdown

The gateway handles graceful shutdown via `SIGINT` (Ctrl+C) and `SIGTERM` (Unix only).

---

## REST API Reference

All API routes are under `/api/v1/`. When authentication is configured, all routes except `/api/v1/health` require a valid bearer token.

### Error Response Format

All error responses follow a consistent structure:

```json
{
  "error": {
    "class": "services",
    "code": "invalid-parameter",
    "message": "Human-readable description"
  }
}
```

Common error codes:

| HTTP Status | class | code | Meaning |
|-------------|-------|------|---------|
| 400 | `services` | `invalid-parameter` | Bad request (invalid specifier, property, value). |
| 401 | -- | -- | Missing or invalid authentication token. |
| 404 | `object` | `unknown-object` | Object or device not found. |
| 500 | `protocol` | `error` | BACnet protocol error. |
| 503 | `device` | `internal-error` | BACnet client not started. |

### Health Check

#### `GET /api/v1/health`

Returns gateway health status. **Always unprotected** (no auth required).

**Response:**
```json
{
  "status": "ok"
}
```

---

### Device Discovery

#### `POST /api/v1/devices/discover`

Sends a WhoIs broadcast and waits for IAm responses. Returns all discovered devices.

**Request body** (optional):
```json
{
  "low_instance": 0,
  "high_instance": 4194302,
  "timeout_seconds": 5
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `low_instance` | `u32?` | `null` | Minimum device instance to discover. |
| `high_instance` | `u32?` | `null` | Maximum device instance to discover. |
| `timeout_seconds` | `u64?` | `3` | Seconds to wait for responses. Capped at 30. |

**Response:**
```json
{
  "devices": [
    {
      "instance": 1234,
      "mac": "[c0, a8, 01, 64, ba, c0]",
      "vendor_id": 555,
      "max_apdu_length": 1476,
      "network": null
    }
  ]
}
```

#### `GET /api/v1/devices`

List all previously discovered devices from the device table. No network traffic.

**Response:**
```json
{
  "devices": [
    {
      "instance": 1234,
      "mac": "[c0, a8, 01, 64, ba, c0]",
      "vendor_id": 555,
      "max_apdu_length": 1476,
      "network": null
    }
  ]
}
```

#### `GET /api/v1/devices/{instance}`

Get detailed info about a specific device by reading its Device object properties (object-name, vendor-name, vendor-identifier, model-name, firmware-revision, application-software-version, protocol-version, protocol-revision).

**Path parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `instance` | `u32` | Device instance number. |

**Response:**
```json
{
  "instance": 1234,
  "mac": "[c0, a8, 01, 64, ba, c0]",
  "vendor_id": 555,
  "object-name": { "type": "string", "value": "HVAC Controller" },
  "vendor-name": { "type": "string", "value": "Acme Controls" },
  "model-name": { "type": "string", "value": "AC-2000" },
  "firmware-revision": { "type": "string", "value": "3.1.0" }
}
```

---

### Local Object Management

Object specifiers use the format `{type}:{instance}`, for example `analog-value:1`, `binary-input:3`, `device:389001`.

#### `GET /api/v1/objects`

List all objects in the local database.

**Query parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `type` | `String?` | Filter by object type (e.g., `analog-value`). |

**Response:**
```json
{
  "objects": [
    {
      "identifier": "device:389001",
      "type": "device",
      "instance": 389001,
      "name": "Rusty Gateway"
    },
    {
      "identifier": "analog-value:1",
      "type": "analog-value",
      "instance": 1,
      "name": "Zone Temperature"
    }
  ]
}
```

#### `GET /api/v1/objects/{specifier}`

Get a specific local object with all its readable properties.

**Path parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `specifier` | `String` | Object specifier (e.g., `analog-value:1`). |

**Response:**
```json
{
  "identifier": "analog-value:1",
  "name": "Zone Temperature",
  "properties": [
    { "property": "object-name", "value": { "type": "string", "value": "Zone Temperature" } },
    { "property": "present-value", "value": { "type": "real", "value": 72.5 } },
    { "property": "status-flags", "value": { "type": "bit-string", "unused_bits": 4, "value": "00" } }
  ]
}
```

#### `POST /api/v1/objects`

Create a new object in the local database. **Blocked in read-only mode.**

**Request body:**
```json
{
  "type": "analog-value",
  "instance": 2,
  "name": "Setpoint",
  "number_of_states": null
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `type` | `String` | *(required)* | Object type name. |
| `instance` | `u32` | *(required)* | Object instance number. |
| `name` | `String` | *(required)* | Object name. |
| `number_of_states` | `u32?` | `2` | Number of states for multi-state objects. Ignored for other types. |

Supported object types for creation: `analog-input`, `analog-output`, `analog-value`, `binary-input`, `binary-output`, `binary-value`, `multi-state-input`, `multi-state-output`, `multi-state-value`, `integer-value`, `positive-integer-value`, `large-analog-value`, `characterstring-value`.

**Response (201 Created):**
```json
{
  "identifier": "analog-value:2",
  "status": "created"
}
```

#### `DELETE /api/v1/objects/{specifier}`

Delete a local object. The Device object cannot be deleted. **Blocked in read-only mode.**

**Response:**
```json
{
  "status": "deleted"
}
```

#### `GET /api/v1/objects/{specifier}/properties/{property}`

Read a single property from a local object.

**Path parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `specifier` | `String` | Object specifier (e.g., `analog-value:1`). |
| `property` | `String` | Property name (e.g., `present-value`). |

**Query parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `index` | `u32?` | Array index for array properties. |

**Response:**
```json
{
  "property": "present-value",
  "value": { "type": "real", "value": 72.5 }
}
```

#### `PUT /api/v1/objects/{specifier}/properties/{property}`

Write a value to a local object property. **Blocked in read-only mode.**

**Request body:**
```json
{
  "value": 72.5,
  "priority": 8,
  "index": null
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `value` | `any` | *(required)* | Value to write. JSON `null`, `boolean`, `number`, or `string`. |
| `priority` | `u8?` | `null` | Command priority 1--16 (for commandable properties). |
| `index` | `u32?` | `null` | Array index (for array properties). |

**Response:**
```json
{
  "status": "ok"
}
```

---

### Remote Device Property Access

These endpoints read/write properties on remote BACnet devices over the network. Devices must be in the device table (from a prior discovery).

#### `GET /api/v1/devices/{instance}/objects/{specifier}/properties/{property}`

Read a property from a remote device.

**Path parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `instance` | `u32` | Remote device instance number. |
| `specifier` | `String` | Object specifier (e.g., `analog-input:1`). |
| `property` | `String` | Property name (e.g., `present-value`). |

**Query parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `index` | `u32?` | Array index for array properties. |

**Response:**
```json
{
  "device": 1234,
  "object": "analog-input:1",
  "property": "present-value",
  "value": { "type": "real", "value": 72.5 }
}
```

#### `PUT /api/v1/devices/{instance}/objects/{specifier}/properties/{property}`

Write a value to a property on a remote device. **Blocked in read-only mode.**

**Request body:**
```json
{
  "value": 72.5,
  "priority": 8,
  "index": null
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `value` | `any` | *(required)* | Value to write. |
| `priority` | `u8?` | `null` | Command priority 1--16. |
| `index` | `u32?` | `null` | Array index. |

**Response:**
```json
{
  "status": "ok"
}
```

---

### Property Value Encoding

JSON values are mapped to BACnet types as follows:

| JSON Type | BACnet Type | Notes |
|-----------|-------------|-------|
| `null` | Null | Relinquishes a commandable property. |
| `true`/`false` | Boolean | |
| Integer (e.g., `42`) | Unsigned or Signed | Unsigned if non-negative, Signed if negative. |
| Float (e.g., `72.5`) | Real (f32) | |
| `"string"` | CharacterString | |

Response values include a `type` field for unambiguous decoding:

```json
{ "type": "real", "value": 72.5 }
{ "type": "unsigned", "value": 42 }
{ "type": "boolean", "value": true }
{ "type": "string", "value": "Zone 1" }
{ "type": "enumerated", "value": 0 }
{ "type": "object-identifier", "value": "analog-input:1" }
{ "type": "bit-string", "unused_bits": 4, "value": "00" }
{ "type": "octet-string", "value": "deadbeef" }
{ "type": "date", "value": "..." }
{ "type": "time", "value": "..." }
{ "type": "list", "value": [ ... ] }
```

---

## MCP Server Reference

The MCP (Model Context Protocol) server is available at `/mcp` and uses the Streamable HTTP transport. It exposes BACnet operations as tools and BACnet knowledge as resources.

### Server Info

The server announces the following capabilities:
- **Tools**: enabled
- **Resources**: enabled

**Instructions** (sent to LLM clients):
> BACnet gateway MCP server. Use tools to discover devices, read/write properties, and manage the local object database. Read reference resources (bacnet://reference/*) to learn about BACnet object types, properties, networking, and troubleshooting.

---

### Tools

#### `discover_devices`

Discover BACnet devices on the network by sending a WhoIs broadcast. Returns a list of devices that respond with IAm.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `low_instance` | `u32` | No | Minimum device instance number to search for (0--4194302). |
| `high_instance` | `u32` | No | Maximum device instance number to search for (0--4194302). |
| `timeout_seconds` | `u64` | No | Seconds to wait for IAm responses (default: 3, max: 30). |

**Example result:**
```
Discovered 2 device(s):
  - Instance 1234, vendor ID 555, max APDU 1476, MAC [c0, a8, 01, 64, ba, c0]
  - Instance 5678, vendor ID 7, max APDU 480, MAC [0a, 00, 01, 0a, ba, c0], network 2
```

#### `list_known_devices`

List all previously discovered BACnet devices from the device table. No network traffic is generated.

*No parameters.*

**Example result:**
```
2 known device(s):
  - Instance 1234, vendor ID 555, MAC [c0, a8, 01, 64, ba, c0]
  - Instance 5678, vendor ID 7, MAC [0a, 00, 01, 0a, ba, c0], network 2
```

#### `get_device_info`

Get detailed information about a specific BACnet device by reading its Device object properties (name, vendor, model, firmware, etc.).

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `device_instance` | `u32` | Yes | Device instance number (must be in the device table from a prior discover). |

**Example result:**
```
Device 1234 info:
  MAC: [c0, a8, 01, 64, ba, c0]
  Vendor ID: 555
  Max APDU: 1476
  object-name: "HVAC Controller"
  vendor-name: "Acme Controls"
  model-name: "AC-2000"
  firmware-revision: "3.1.0"
  description: "Main building controller"
```

#### `read_property`

Read a property from a remote BACnet device. Specify the device instance, object type and instance, and property name.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `device_instance` | `u32` | Yes | Device instance number (e.g., 1234). |
| `object_type` | `String` | Yes | Object type name (e.g., `analog-input`, `binary-value`, `device`). |
| `object_instance` | `u32` | Yes | Object instance number (e.g., 1). |
| `property` | `String` | Yes | Property name (e.g., `present-value`, `object-name`, `status-flags`). |
| `array_index` | `u32` | No | Array index for array properties. |

**Example result:**
```
analog-input:1 present-value = 72.5
```

#### `write_property`

Write a value to a property on a remote BACnet device. **Blocked in read-only mode.**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `device_instance` | `u32` | Yes | Device instance number. |
| `object_type` | `String` | Yes | Object type name (e.g., `analog-output`, `binary-value`). |
| `object_instance` | `u32` | Yes | Object instance number. |
| `property` | `String` | Yes | Property name (e.g., `present-value`). |
| `value` | `any` | Yes | Value to write: number (72.5), boolean (true/false), string, or null. |
| `priority` | `u8` | No | Command priority 1--16 (for commandable properties like present-value on outputs). |

**Example result:**
```
Successfully wrote 72.5 to analog-output:1 present-value
```

#### `list_local_objects`

List objects in the gateway's local BACnet object database. Optionally filter by object type.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `object_type` | `String` | No | Filter by object type (e.g., `analog-value`, `binary-input`). |

**Example result:**
```
3 local object(s):
  - device:389001 "Rusty Gateway"
  - analog-value:1 "Zone Temperature"
  - binary-value:1 "Occupancy Status"
```

#### `read_local_property`

Read a property from the gateway's local object database. No network traffic.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `object_type` | `String` | Yes | Object type (e.g., `analog-value`, `device`). |
| `object_instance` | `u32` | Yes | Object instance number. |
| `property` | `String` | Yes | Property name (e.g., `present-value`, `object-name`). |

**Example result:**
```
analog-value:1 present-value = 72.5
```

#### `write_local_property`

Write a value to a property in the gateway's local object database. No network traffic. **Blocked in read-only mode.**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `object_type` | `String` | Yes | Object type (e.g., `analog-value`). |
| `object_instance` | `u32` | Yes | Object instance number. |
| `property` | `String` | Yes | Property name (e.g., `present-value`). |
| `value` | `any` | Yes | Value to write: number, boolean, string, or null. |

**Example result:**
```
Successfully wrote 72.5 to local analog-value:1 present-value
```

#### `create_local_object`

Create a new object in the gateway's local BACnet database. Supports analog, binary, multi-state, and value types. **Blocked in read-only mode.**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `object_type` | `String` | Yes | Object type (e.g., `analog-value`, `binary-input`, `multi-state-value`). |
| `object_instance` | `u32` | Yes | Object instance number. |
| `object_name` | `String` | Yes | Human-readable object name. |
| `number_of_states` | `u32` | No | Number of states for multi-state objects (default: 2, ignored for other types). |

**Example result:**
```
Created local object analog-value:2 "Setpoint"
```

#### `delete_local_object`

Delete an object from the gateway's local BACnet database. Cannot delete the Device object. **Blocked in read-only mode.**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `object_type` | `String` | Yes | Object type (e.g., `analog-value`). |
| `object_instance` | `u32` | Yes | Object instance number. |

**Example result:**
```
Deleted local object analog-value:2
```

---

### Resources

Resources are readable via `resources/read` with a URI. They fall into two categories: static reference material (compiled into the binary) and live state resources (read from the running gateway).

#### Static Reference Resources

| URI | Name | Description |
|-----|------|-------------|
| `bacnet://reference/object-types` | BACnet Object Types | Index of all 65 BACnet object types with name, category, and purpose. |
| `bacnet://reference/properties` | BACnet Properties | Common properties: present-value, status-flags, reliability, out-of-service, event-state, priority-array. |
| `bacnet://reference/units` | BACnet Engineering Units | All ~256 BACnet engineering units with descriptions. |
| `bacnet://reference/errors` | BACnet Error Codes | Error classes and codes with common causes and next steps. |
| `bacnet://reference/reliability` | BACnet Reliability Values | Reliability enum values: meaning, when they occur, how to clear. |
| `bacnet://reference/priority-array` | BACnet Priority Array | 16-level command priority scheme: what each level is for, relinquish-default, common pitfalls. |
| `bacnet://reference/networking` | BACnet Networking Guide | Conceptual guide: networks, routers, BBMDs, foreign devices, broadcast domains. |
| `bacnet://reference/services` | BACnet Services | When to use each service: ReadProperty vs ReadPropertyMultiple, COV vs polling, confirmed vs unconfirmed. |
| `bacnet://reference/troubleshooting` | BACnet Troubleshooting | Common problem patterns, diagnostic steps, and resolution guides. |

#### Resource Templates

| URI Template | Name | Description |
|--------------|------|-------------|
| `bacnet://reference/object-types/{type}` | BACnet Object Type Detail | Detailed reference for a specific BACnet object type: purpose, key properties, common configurations, troubleshooting. |

Example: `bacnet://reference/object-types/analog-input` returns detailed documentation for the Analog Input object type.

#### Live State Resources

| URI | Name | Description |
|-----|------|-------------|
| `bacnet://state/devices` | Discovered Devices | Current device table -- discovered devices with instance, vendor, MAC. |
| `bacnet://state/local-objects` | Local Objects | Objects in the gateway's local BACnet database. |
| `bacnet://state/config` | Gateway Configuration | Current gateway configuration (sanitized, no secrets). |

---

## Authentication

### Bearer Token

When an API key is configured, all protected endpoints require an `Authorization: Bearer <token>` header. The token is compared using constant-time comparison (via the `subtle` crate) to prevent timing attacks.

**Configuring authentication (in order of precedence):**

1. CLI flag: `--api-key my-secret-key` or `-k my-secret-key`
2. Config file: `server.api_key = "my-secret-key"`
3. Environment variable: `BACNET_GATEWAY_API_KEY=my-secret-key`

**Example request:**
```bash
curl -H "Authorization: Bearer my-secret-key" http://localhost:3000/api/v1/devices
```

### What Is Protected

- **REST API**: All routes under `/api/v1/` are protected **except** `/api/v1/health`.
- **MCP endpoint**: The `/mcp` endpoint is protected when auth is configured.
- **Health check**: `/api/v1/health` is always unprotected.

### Auth Error Responses

| Scenario | HTTP Status | Message |
|----------|-------------|---------|
| Missing `Authorization` header | 401 | `missing Authorization header` |
| Wrong scheme (not `Bearer`) | 401 | `invalid Authorization header format, expected: Bearer <token>` |
| Invalid token | 401 | `invalid token` |

### Pluggable Authenticator Trait

The authentication system is pluggable via the `Authenticator` trait:

```rust
pub trait Authenticator: Send + Sync + 'static {
    fn authenticate(&self, headers: &HeaderMap) -> Result<(), AuthError>;
}
```

The built-in `BearerTokenAuth` implements this trait. Custom authenticators can be created by implementing `Authenticator` and passing them to `api_router()`.

---

## Read-Only Mode

Read-only mode prevents all write operations through both the REST API and MCP tools.

### Enabling Read-Only Mode

- **Config file**: `server.read_only = true`
- **CLI flag**: `--read-only` (overrides config to `true`)

### Blocked Operations

When read-only mode is active, the following operations return an error:

| Interface | Operation |
|-----------|-----------|
| REST API | `PUT /api/v1/objects/{specifier}/properties/{property}` |
| REST API | `POST /api/v1/objects` |
| REST API | `DELETE /api/v1/objects/{specifier}` |
| REST API | `PUT /api/v1/devices/{instance}/objects/{specifier}/properties/{property}` |
| MCP | `write_property` tool |
| MCP | `write_local_property` tool |
| MCP | `create_local_object` tool |
| MCP | `delete_local_object` tool |

All read operations (discovery, list, get, read) remain available.

**Error message**: `"Gateway is in read-only mode. Write operations are disabled."`

---

## Feature Flags

| Feature | Enables | Key Dependencies |
|---------|---------|------------------|
| *(none)* | `config`, `state`, `builder`, `parse` modules only. No web dependencies. | `serde`, `toml`, `tokio`, bacnet crates |
| `http` | REST API module (`api`) and auth middleware (`auth`). | `axum`, `tower`, `subtle` |
| `mcp` | MCP server module (`mcp`) with tools, resources, and knowledge base. | `rmcp`, `schemars` |
| `bin` | Binary entry point (`main.rs`). Implies `http` + `mcp`. | `clap`, `tracing-subscriber`, `tokio-util` |
| `sc-tls` | BACnet/SC TLS transport support (pass-through to `bacnet-transport`). | `rustls`, `tokio-tungstenite` |
| `serial` | MS/TP serial transport support (pass-through to `bacnet-transport`, Linux only). | `serialport` |

The `default` feature set is empty. To build the binary, use `--features bin`. To embed just the REST API in another application, use `--features http`. To embed just the MCP server, use `--features mcp`.

---

## Example Configurations

### Minimal (BIP only, no auth)

```toml
[device]
instance = 1234
name = "Minimal Gateway"

[transports.bip]
broadcast = "192.168.1.255"
network_number = 1
```

This uses all defaults: binds to `127.0.0.1:3000`, no authentication, read-write mode, BIP on `0.0.0.0:47808`.

### BIP with Authentication

```toml
[server]
bind = "0.0.0.0:3000"
api_key = "change-me-in-production"

[device]
instance = 389001
name = "Secure Gateway"
vendor_id = 555

[transports.bip]
interface = "192.168.1.50"
port = 47808
broadcast = "192.168.1.255"
network_number = 1
```

### Read-Only Monitor

```toml
[server]
bind = "0.0.0.0:8080"
api_key = "monitor-key"
read_only = true

[device]
instance = 900001
name = "Monitoring Gateway"

[transports.bip]
broadcast = "10.0.0.255"
network_number = 1

[[objects]]
type = "analog-value"
instance = 1
name = "Building Energy"
units = "kilowatt-hours"
```

### Multi-Transport with Routes

```toml
[server]
bind = "0.0.0.0:3000"
api_key = "multi-transport-key"

[device]
instance = 100001
name = "Multi-Transport Gateway"
vendor_id = 555
description = "Routes between BIP and SC networks"

[transports.bip]
interface = "0.0.0.0"
port = 47808
broadcast = "192.168.1.255"
network_number = 1

[transports.sc]
hub_uri = "wss://sc-hub.building.local:8443"
cert = "/etc/bacnet/client.pem"
key = "/etc/bacnet/client.key"
ca = "/etc/bacnet/ca.pem"
network_number = 2

[[routes]]
network = 10
via_transport = "bip"
next_hop = "192.168.1.100:47808"

[[routes]]
network = 20
via_transport = "sc"
```

### Foreign Device Registration

```toml
[server]
bind = "127.0.0.1:3000"

[device]
instance = 50001
name = "Remote Gateway"

[transports.bip]
interface = "0.0.0.0"
port = 47808
broadcast = "10.0.0.255"
network_number = 1

[foreign_device]
bbmd = "10.0.0.1:47808"
ttl = 300
```

### BBMD with Broadcast Distribution Table

```toml
[server]
bind = "0.0.0.0:3000"

[device]
instance = 60001
name = "BBMD Gateway"

[transports.bip]
interface = "0.0.0.0"
port = 47808
broadcast = "192.168.1.255"
network_number = 1

[bbmd]
enabled = true
bdt = [
    "192.168.2.1:47808",
    "10.0.0.1:47808",
]
```

### MS/TP on Linux (USB RS-485 adapter)

```toml
[device]
instance = 70001
name = "MS/TP Gateway"

[transports.mstp]
serial_port = "/dev/ttyUSB0"
baud_rate = 76800
station_address = 1
max_master = 127
network_number = 3
# No rs485 section needed — USB adapters handle direction automatically

[transports.bip]
broadcast = "192.168.1.255"
network_number = 1
```

### MS/TP on Raspberry Pi (RS-485 Hat with GPIO)

```toml
[device]
instance = 70002
name = "Pi MS/TP Gateway"

[transports.mstp]
serial_port = "/dev/ttyS0"
baud_rate = 76800
station_address = 1
max_master = 127
network_number = 3

# Seeed Studio RS-485 Shield: DE/RE on GPIO18
[transports.mstp.rs485]
mode = "gpio"
gpio_line = 18

[transports.bip]
broadcast = "192.168.1.255"
network_number = 1
```

### Pre-populated Objects

```toml
[device]
instance = 80001
name = "Data Gateway"

[transports.bip]
broadcast = "192.168.1.255"
network_number = 1

[[objects]]
type = "analog-value"
instance = 1
name = "Outdoor Temperature"
units = "degrees-fahrenheit"

[[objects]]
type = "analog-value"
instance = 2
name = "Humidity"
units = "percent-relative-humidity"

[[objects]]
type = "binary-value"
instance = 1
name = "Alarm Active"

[[objects]]
type = "multi-state-value"
instance = 1
name = "Operating Mode"
```
