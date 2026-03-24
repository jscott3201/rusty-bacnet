# WASM/JavaScript API Reference

BACnet/SC thin client for browsers via WebAssembly.

**npm package**: `@jscott3201/bacnet-wasm`
**Version**: 0.8.0
**License**: MIT
**Repository**: [github.com/jscott3201/rusty-bacnet](https://github.com/jscott3201/rusty-bacnet)

## Overview

The `bacnet-wasm` crate provides a BACnet/SC (Secure Connect) client for browser environments. It compiles the Rusty BACnet protocol stack to WebAssembly via `wasm-bindgen`, exposing a high-level async JavaScript API that handles:

- WebSocket-based BACnet/SC hub connections (Annex AB)
- ReadProperty and WriteProperty services
- Who-Is discovery with I-Am callback handling
- COV (Change of Value) subscriptions and notification callbacks
- BACnet APDU encoding/decoding for common data types
- Automatic heartbeat response to keep connections alive

The client connects to a BACnet/SC hub over `wss://` WebSocket, performs the SC Connect handshake, and then sends/receives BACnet service requests encapsulated in SC frames.

## Installation

### npm

```bash
npm install @jscott3201/bacnet-wasm
```

### Vite

Vite supports WASM modules natively. Import and initialize:

```javascript
import init, { BACnetScClient } from '@jscott3201/bacnet-wasm';

await init();
```

Vite automatically resolves the `.wasm` file from the package. No additional configuration is needed.

### webpack

For webpack 5+, add WASM support to your configuration:

```javascript
// webpack.config.js
module.exports = {
  experiments: {
    asyncWebAssembly: true,
  },
};
```

Then import normally:

```javascript
import init, { BACnetScClient } from '@jscott3201/bacnet-wasm';

await init();
```

### CDN / Script Tag

Load directly from a CDN that serves npm packages (e.g., unpkg, jsDelivr):

```html
<script type="module">
  import init, { BACnetScClient } from 'https://unpkg.com/@jscott3201/bacnet-wasm/bacnet_wasm.js';

  await init();
  const client = new BACnetScClient();
</script>
```

### Package Contents

The npm package contains four files:

| File | Description |
|------|-------------|
| `bacnet_wasm.js` | ES module entry point (main) |
| `bacnet_wasm_bg.js` | Generated JS glue code |
| `bacnet_wasm_bg.wasm` | Compiled WebAssembly binary |
| `bacnet_wasm.d.ts` | TypeScript type definitions |

## Quick Start

```javascript
import init, {
  BACnetScClient,
  ObjectTypes,
  PropertyIds,
  encodeReal,
} from '@jscott3201/bacnet-wasm';

// Initialize the WASM module (required once before any API calls)
await init();

// Create a client (generates a random 6-byte VMAC automatically)
const client = new BACnetScClient();

// Connect to a BACnet/SC hub
await client.connect("wss://hub.example.com:1234");

// Register callbacks for unsolicited messages
client.onIAm((data) => {
  console.log("I-Am received:", data);
});

// Read present-value from Analog Input 1
const value = await client.readProperty(
  ObjectTypes.analog_input(),  // object type (0)
  1,                           // instance number
  PropertyIds.present_value(), // property (85)
);
console.log("Value:", value);

// Write a new present-value to Analog Output 1
await client.writeProperty(
  ObjectTypes.analog_output(), // object type (1)
  1,                           // instance number
  PropertyIds.present_value(), // property (85)
  encodeReal(72.5),            // encoded value bytes
  8,                           // priority (optional)
);

// Discover devices on the network
client.whoIs();

// Disconnect gracefully
await client.disconnect();
```

## BACnetScClient Class

The main client class for BACnet/SC communication. Uses browser WebSocket to connect to a BACnet/SC hub and provides async methods for BACnet service requests.

### Constructor

```typescript
new BACnetScClient(): BACnetScClient
```

Creates a new BACnet/SC client with a randomly generated 6-byte VMAC (Virtual MAC address). Uses `crypto.getRandomValues()` when available, falling back to `Math.random()`.

**Example:**

```javascript
const client = new BACnetScClient();
```

---

### connect

```typescript
connect(url: string): Promise<void>
```

Connect to a BACnet/SC hub via WebSocket.

Opens a WebSocket connection to the specified URL using the `hub.bsc.bacnet.org` subprotocol, sends a ConnectRequest with the client's VMAC and device UUID, and waits for a ConnectAccept response from the hub. Once connected, starts a background receive loop that dispatches incoming messages to pending request promises and event callbacks.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `url` | `string` | WebSocket URL of the BACnet/SC hub (e.g., `"wss://hub.example.com:1234"`) |

**Returns:** `Promise<void>` -- Resolves on successful connection, rejects on failure.

**Throws:**
- WebSocket connection failure (network error, refused, etc.)
- ConnectAccept not received or invalid (hub rejected the connection)

**Example:**

```javascript
try {
  await client.connect("wss://hub.example.com:1234");
  console.log("Connected:", client.connected);
} catch (e) {
  console.error("Connection failed:", e.message);
}
```

---

### readProperty

```typescript
readProperty(
  objectType: number,
  instance: number,
  propertyId: number,
  arrayIndex?: number
): Promise<any>
```

Read a property from a remote BACnet device.

Sends a ReadProperty confirmed request and waits for the response. On success, the ReadProperty-ACK payload is decoded and returned as a JavaScript object. On failure, the promise rejects with the BACnet error, reject, or abort reason.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `objectType` | `number` | BACnet object type (e.g., `0` for Analog Input). Use `ObjectTypes` constants. |
| `instance` | `number` | Object instance number (0 to 4194303). |
| `propertyId` | `number` | BACnet property identifier (e.g., `85` for Present-Value). Use `PropertyIds` constants. |
| `arrayIndex` | `number \| undefined` | Optional array index for array properties. Omit to read the entire property. |

**Returns:** `Promise<any>` -- The decoded property value from the ReadProperty-ACK. For ReadProperty responses, this is a JS object with `object_type`, `instance`, `property_id`, `array_index`, and `value_bytes` fields. For other confirmed service ACKs, returns `true`.

**Throws:**
- `"not connected"` -- Client is not connected to a hub.
- `"BACnet error: class=X code=Y"` -- Remote device returned a BACnet Error PDU.
- `"BACnet reject: reason=X"` -- Remote device rejected the request.
- `"BACnet abort: reason=X"` -- Remote device aborted the transaction.

**Example:**

```javascript
// Read present-value from Analog Input 1
const result = await client.readProperty(0, 1, 85);
console.log("Value bytes:", result.value_bytes);

// Read object-name from Device 1234
const name = await client.readProperty(8, 1234, 77);

// Read element 3 of an array property
const element = await client.readProperty(0, 1, 87, 3);
```

---

### writeProperty

```typescript
writeProperty(
  objectType: number,
  instance: number,
  propertyId: number,
  valueBytes: Uint8Array,
  priority?: number
): Promise<void>
```

Write a property on a remote BACnet device.

Sends a WriteProperty confirmed request with pre-encoded application-tagged value bytes. Use the `encodeReal()`, `encodeUnsigned()`, `encodeBoolean()`, or `encodeEnumerated()` functions to produce the value bytes.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `objectType` | `number` | BACnet object type. Use `ObjectTypes` constants. |
| `instance` | `number` | Object instance number (0 to 4194303). |
| `propertyId` | `number` | BACnet property identifier. Use `PropertyIds` constants. |
| `valueBytes` | `Uint8Array` | Pre-encoded application-tagged value data. Use the `encode*()` functions. |
| `priority` | `number \| undefined` | Optional write priority (1-16). Omit for non-commandable properties. |

**Returns:** `Promise<void>` -- Resolves on SimpleAck, rejects on error.

**Throws:**
- `"not connected"` -- Client is not connected to a hub.
- `"BACnet error: class=X code=Y"` -- Remote device returned a BACnet Error PDU.
- `"BACnet reject: reason=X"` -- Remote device rejected the request.
- `"BACnet abort: reason=X"` -- Remote device aborted the transaction.

**Example:**

```javascript
// Write a Real value to Analog Output 1, present-value, priority 8
await client.writeProperty(1, 1, 85, encodeReal(72.5), 8);

// Write a Boolean to Binary Value 1, out-of-service
await client.writeProperty(5, 1, 81, encodeBoolean(true));

// Write an Enumerated value (e.g., polarity = 1)
await client.writeProperty(3, 1, 84, encodeEnumerated(1));
```

---

### whoIs

```typescript
whoIs(low?: number, high?: number): void
```

Send a Who-Is broadcast request to discover devices on the network.

This is an unconfirmed service -- it does not return a result directly. Responses arrive as I-Am broadcasts, which are delivered to the callback registered via `onIAm()`.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `low` | `number \| undefined` | Optional low device instance range limit. |
| `high` | `number \| undefined` | Optional high device instance range limit. |

**Returns:** `void`

**Throws:**
- `"not connected"` -- Client is not connected to a hub.

**Example:**

```javascript
// Discover all devices
client.whoIs();

// Discover devices in range 1000-2000
client.whoIs(1000, 2000);

// Discover a specific device
client.whoIs(1234, 1234);
```

---

### subscribeCov

```typescript
subscribeCov(
  processId: number,
  objectType: number,
  instance: number,
  confirmed: boolean,
  lifetime?: number
): Promise<void>
```

Subscribe to COV (Change of Value) notifications for an object.

Sends a SubscribeCOV confirmed request to a remote device. Once subscribed, COV notifications are delivered to the callback registered via `onCovNotification()`.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `processId` | `number` | Subscriber process identifier. Use a unique value per subscription. |
| `objectType` | `number` | BACnet object type of the monitored object. |
| `instance` | `number` | Instance number of the monitored object. |
| `confirmed` | `boolean` | `true` for confirmed notifications, `false` for unconfirmed. |
| `lifetime` | `number \| undefined` | Optional subscription lifetime in seconds. Omit for no expiration. |

**Returns:** `Promise<void>` -- Resolves on SimpleAck from the remote device.

**Throws:**
- `"not connected"` -- Client is not connected to a hub.
- `"BACnet error: class=X code=Y"` -- Remote device returned a BACnet Error PDU.

**Example:**

```javascript
// Subscribe to COV on Analog Input 1, unconfirmed, 5-minute lifetime
await client.subscribeCov(1, 0, 1, false, 300);

// Subscribe to COV on Binary Value 2, confirmed, no expiration
await client.subscribeCov(2, 5, 2, true);
```

---

### onIAm

```typescript
onIAm(callback: (data: Uint8Array) => void): void
```

Register a callback for I-Am responses.

The callback receives the raw I-Am service request bytes as a `Uint8Array`. This includes the encoded object identifier, max APDU length, segmentation supported, and vendor ID fields.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `callback` | `(data: Uint8Array) => void` | Function called when an I-Am is received. |

**Example:**

```javascript
client.onIAm((data) => {
  console.log("I-Am received, raw bytes:", data);
  // Decode the I-Am payload using decodeApdu or manual parsing
});

// Trigger I-Am responses
client.whoIs();
```

---

### onCovNotification

```typescript
onCovNotification(callback: (data: Uint8Array) => void): void
```

Register a callback for unconfirmed COV notification messages.

The callback receives the raw UnconfirmedCOVNotification service request bytes as a `Uint8Array`.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `callback` | `(data: Uint8Array) => void` | Function called when a COV notification is received. |

**Example:**

```javascript
client.onCovNotification((data) => {
  console.log("COV notification, raw bytes:", data);
});

// Subscribe to trigger future notifications
await client.subscribeCov(1, 0, 1, false, 300);
```

---

### disconnect

```typescript
disconnect(): Promise<void>
```

Disconnect from the BACnet/SC hub.

Sends a DisconnectRequest to the hub (if connected), closes the WebSocket, and sets the connection state to Disconnected.

**Returns:** `Promise<void>` -- Always resolves (errors during disconnect are silently ignored).

**Example:**

```javascript
await client.disconnect();
console.log("Connected:", client.connected); // false
```

---

### connected (getter)

```typescript
readonly connected: boolean
```

Check if the client is currently connected to a BACnet/SC hub.

Returns `true` when the SC connection state is `Connected` (after a successful `connect()` and before `disconnect()` or a connection drop).

**Example:**

```javascript
if (client.connected) {
  const value = await client.readProperty(0, 1, 85);
}
```

## Codec Functions

Standalone functions for encoding BACnet service requests and decoding responses. These are exported at the module level alongside `BACnetScClient`.

### Service Encoders

These functions produce complete NPDU+APDU byte sequences ready for SC framing. They are used internally by `BACnetScClient` but are also exported for advanced use cases (e.g., building custom transport layers).

---

#### encodeReadProperty

```typescript
encodeReadProperty(
  invokeId: number,
  objectType: number,
  instance: number,
  propertyId: number,
  arrayIndex?: number
): Uint8Array
```

Encode a ReadProperty request into NPDU+APDU bytes.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `invokeId` | `number` | Invoke ID for the confirmed request (0-255). |
| `objectType` | `number` | BACnet object type. |
| `instance` | `number` | Object instance number. |
| `propertyId` | `number` | BACnet property identifier. |
| `arrayIndex` | `number \| undefined` | Optional array index. |

**Returns:** `Uint8Array` -- Encoded NPDU+APDU bytes.

**Throws:** Object identifier validation error if instance exceeds 4194303.

---

#### encodeWriteProperty

```typescript
encodeWriteProperty(
  invokeId: number,
  objectType: number,
  instance: number,
  propertyId: number,
  valueBytes: Uint8Array,
  priority?: number
): Uint8Array
```

Encode a WriteProperty request into NPDU+APDU bytes.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `invokeId` | `number` | Invoke ID for the confirmed request (0-255). |
| `objectType` | `number` | BACnet object type. |
| `instance` | `number` | Object instance number. |
| `propertyId` | `number` | BACnet property identifier. |
| `valueBytes` | `Uint8Array` | Pre-encoded application-tagged value data. |
| `priority` | `number \| undefined` | Optional write priority (1-16). |

**Returns:** `Uint8Array` -- Encoded NPDU+APDU bytes.

**Throws:** Object identifier validation error if instance exceeds 4194303.

---

#### encodeWhoIs

```typescript
encodeWhoIs(low?: number, high?: number): Uint8Array
```

Encode a Who-Is request into NPDU+APDU bytes.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `low` | `number \| undefined` | Optional low device instance range limit. |
| `high` | `number \| undefined` | Optional high device instance range limit. |

**Returns:** `Uint8Array` -- Encoded NPDU+APDU bytes.

---

#### encodeSubscribeCov

```typescript
encodeSubscribeCov(
  invokeId: number,
  processId: number,
  objectType: number,
  instance: number,
  confirmed: boolean,
  lifetime?: number
): Uint8Array
```

Encode a SubscribeCOV request into NPDU+APDU bytes.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `invokeId` | `number` | Invoke ID for the confirmed request (0-255). |
| `processId` | `number` | Subscriber process identifier. |
| `objectType` | `number` | BACnet object type of the monitored object. |
| `instance` | `number` | Instance number of the monitored object. |
| `confirmed` | `boolean` | `true` for confirmed notifications. |
| `lifetime` | `number \| undefined` | Optional subscription lifetime in seconds. |

**Returns:** `Uint8Array` -- Encoded NPDU+APDU bytes.

**Throws:** Object identifier validation error if instance exceeds 4194303.

### Value Encoders

These functions produce application-tagged BACnet value bytes for use with `writeProperty()` and `encodeWriteProperty()`.

---

#### encodeReal

```typescript
encodeReal(value: number): Uint8Array
```

Encode a floating-point number as a BACnet Real (IEEE 754 single-precision, application tag 4).

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `value` | `number` | The floating-point value to encode. |

**Returns:** `Uint8Array` -- Application-tagged Real value bytes.

**Example:**

```javascript
const bytes = encodeReal(72.5);
await client.writeProperty(1, 1, 85, bytes, 8);
```

---

#### encodeUnsigned

```typescript
encodeUnsigned(value: number): Uint8Array
```

Encode an integer as a BACnet Unsigned Integer (application tag 2). The value is treated as a 32-bit unsigned integer.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `value` | `number` | The unsigned integer value to encode (0 to 4294967295). |

**Returns:** `Uint8Array` -- Application-tagged Unsigned value bytes.

**Example:**

```javascript
const bytes = encodeUnsigned(300);
await client.writeProperty(0, 1, 117, bytes); // write units
```

---

#### encodeBoolean

```typescript
encodeBoolean(value: boolean): Uint8Array
```

Encode a boolean as a BACnet Boolean value (application tag 1).

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `value` | `boolean` | The boolean value to encode. |

**Returns:** `Uint8Array` -- Application-tagged Boolean value bytes.

**Example:**

```javascript
const bytes = encodeBoolean(true);
await client.writeProperty(5, 1, 81, bytes); // write out-of-service
```

---

#### encodeEnumerated

```typescript
encodeEnumerated(value: number): Uint8Array
```

Encode an integer as a BACnet Enumerated value (application tag 9).

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `value` | `number` | The enumerated value to encode. |

**Returns:** `Uint8Array` -- Application-tagged Enumerated value bytes.

**Example:**

```javascript
const bytes = encodeEnumerated(1); // e.g., binary active
await client.writeProperty(4, 1, 85, bytes, 8);
```

### Decoders

---

#### decodeApdu

```typescript
decodeApdu(data: Uint8Array): DecodedApdu
```

Decode a raw APDU from bytes into a structured JavaScript object.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `data` | `Uint8Array` | Raw APDU bytes. |

**Returns:** A `DecodedApdu` object with the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `pdu_type` | `string` | One of: `"confirmed-ack"`, `"simple-ack"`, `"error"`, `"reject"`, `"abort"`, `"unconfirmed"`, `"confirmed-request"`, `"segment-ack"` |
| `invoke_id` | `number` | Invoke ID of the PDU (0 for unconfirmed/segment-ack). |
| `service_choice` | `number` | Service choice code (0 for reject/abort/segment-ack). |
| `payload` | `Uint8Array \| undefined` | Service-specific payload bytes. Present for `confirmed-ack`, `unconfirmed`, and `confirmed-request`. |
| `error_class` | `number \| undefined` | Error class. Present only for `error` PDU type. |
| `error_code` | `number \| undefined` | Error code. Present only for `error` PDU type. |
| `reason` | `number \| undefined` | Reject or abort reason. Present only for `reject` and `abort` PDU types. |

**Throws:** Decoding error if the APDU bytes are malformed.

**Example:**

```javascript
const apdu = decodeApdu(rawBytes);
if (apdu.pdu_type === "confirmed-ack" && apdu.service_choice === 12) {
  // ReadProperty-ACK
  const ack = decodeReadPropertyAck(new Uint8Array(apdu.payload));
  console.log("Property value bytes:", ack.value_bytes);
}
```

---

#### decodeReadPropertyAck

```typescript
decodeReadPropertyAck(data: Uint8Array): ReadPropertyAckResult
```

Decode a ReadProperty-ACK service payload into a structured JavaScript object.

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `data` | `Uint8Array` | ReadProperty-ACK service payload bytes (from `DecodedApdu.payload`). |

**Returns:** An object with the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `object_type` | `number` | BACnet object type of the responding object. |
| `instance` | `number` | Instance number of the responding object. |
| `property_id` | `number` | Property identifier that was read. |
| `array_index` | `number \| undefined` | Array index, if the request specified one. |
| `value_bytes` | `Uint8Array` | Raw application-tagged value bytes. |

**Throws:** Decoding error if the ACK payload is malformed.

**Example:**

```javascript
const ack = decodeReadPropertyAck(payload);
console.log(`Object: type=${ack.object_type}, instance=${ack.instance}`);
console.log(`Property ${ack.property_id} value:`, ack.value_bytes);
```

## Module-Level Functions

### version

```typescript
version(): string
```

Returns the crate version string (e.g., `"0.8.0"`).

**Example:**

```javascript
console.log("bacnet-wasm version:", version());
```

## Type Constants

### ObjectTypes

Static methods that return well-known BACnet object type codes. Use these instead of raw numeric values for readability.

```typescript
class ObjectTypes {
  static analog_input(): number;    // 0
  static analog_output(): number;   // 1
  static analog_value(): number;    // 2
  static binary_input(): number;    // 3
  static binary_output(): number;   // 4
  static binary_value(): number;    // 5
  static calendar(): number;        // 6
  static device(): number;          // 8
  static multi_state_input(): number;  // 13
  static multi_state_output(): number; // 14
  static notification_class(): number; // 15
  static schedule(): number;        // 17
  static multi_state_value(): number;  // 19
  static trend_log(): number;       // 20
}
```

**Example:**

```javascript
// These are equivalent:
await client.readProperty(ObjectTypes.analog_input(), 1, 85);
await client.readProperty(0, 1, 85);
```

For object types not listed here, use the raw numeric value from ASHRAE 135-2020 Clause 12.

### PropertyIds

Static methods that return well-known BACnet property identifier codes.

```typescript
class PropertyIds {
  static present_value(): number;       // 85
  static object_name(): number;         // 77
  static description(): number;         // 28
  static status_flags(): number;        // 111
  static out_of_service(): number;      // 81
  static units(): number;               // 117
  static object_list(): number;         // 76
  static object_type(): number;         // 79
  static object_identifier(): number;   // 75
  static priority_array(): number;      // 87
  static relinquish_default(): number;  // 104
}
```

**Example:**

```javascript
const name = await client.readProperty(8, 1234, PropertyIds.object_name());
const flags = await client.readProperty(0, 1, PropertyIds.status_flags());
```

For property identifiers not listed here, use the raw numeric value from ASHRAE 135-2020 Clause 12.

### JsObjectIdentifier

A JS-facing wrapper around a BACnet Object Identifier (object type + instance number).

```typescript
class JsObjectIdentifier {
  constructor(objectType: number, instance: number);

  readonly object_type: number;    // getter
  readonly instance_number: number; // getter

  toString(): string;
}
```

**Parameters:**

| Name | Type | Description |
|------|------|-------------|
| `objectType` | `number` | Object type (0-65535, typically 0-1023 per ASHRAE 135). |
| `instance` | `number` | Instance number (0 to 4194303 / 0x3FFFFF). |

**Throws:** Error if instance exceeds the maximum (4194303).

**Example:**

```javascript
const oid = new JsObjectIdentifier(0, 1);
console.log(oid.object_type);      // 0
console.log(oid.instance_number);  // 1
console.log(oid.toString());       // "analog-input:1"
```

## Event Callbacks

BACnet uses two patterns for responses: confirmed service replies (returned as Promise results) and unconfirmed broadcasts (delivered via callbacks).

### I-Am Pattern

When a device responds to a Who-Is request, it sends an I-Am broadcast. Register a callback before sending Who-Is:

```javascript
const devices = [];

client.onIAm((data) => {
  // data is a Uint8Array containing the raw I-Am service request bytes:
  //   - Object Identifier (4 bytes, context tag 0)
  //   - Max APDU Length Accepted (unsigned)
  //   - Segmentation Supported (enumerated)
  //   - Vendor ID (unsigned)
  devices.push(data);
});

client.whoIs();

// I-Am responses arrive asynchronously via the callback
```

### COV Notification Pattern

After subscribing to COV, value changes arrive as unconfirmed notifications:

```javascript
client.onCovNotification((data) => {
  // data is a Uint8Array containing the raw UnconfirmedCOVNotification
  // service request bytes:
  //   - Subscriber Process Identifier
  //   - Initiating Device Identifier
  //   - Monitored Object Identifier
  //   - List of Values (property-value pairs)
  console.log("Value changed:", data);
});

// Subscribe to Analog Input 1 with 5-minute lifetime
await client.subscribeCov(1, 0, 1, false, 300);
```

### Callback Notes

- Only one callback can be registered per event type. Setting a new callback replaces the previous one.
- Callbacks receive raw BACnet service request bytes as `Uint8Array`. Use `decodeApdu()` or manual parsing to extract structured data.
- Callbacks are invoked from the internal receive loop running via `wasm-bindgen-futures`. They execute on the browser's microtask queue.

## TypeScript Support

The package includes TypeScript declarations at `bacnet_wasm.d.ts`. These are generated by `wasm-bindgen` and provide full type information for all exported classes and functions.

Import types alongside values:

```typescript
import init, {
  BACnetScClient,
  ObjectTypes,
  PropertyIds,
  JsObjectIdentifier,
  encodeReal,
  encodeUnsigned,
  encodeBoolean,
  encodeEnumerated,
  encodeReadProperty,
  encodeWriteProperty,
  encodeWhoIs,
  encodeSubscribeCov,
  decodeApdu,
  decodeReadPropertyAck,
  version,
} from '@jscott3201/bacnet-wasm';
```

All async methods return standard `Promise` types. Optional parameters use `number | undefined` rather than `?` syntax due to wasm-bindgen conventions.

## Error Handling

Errors are surfaced as JavaScript exceptions (rejected Promises for async methods, thrown errors for sync methods).

### Connection Errors

```javascript
try {
  await client.connect("wss://invalid-host:1234");
} catch (e) {
  // "WebSocket connection failed" -- network/DNS/TLS error
  // "ConnectAccept not received or invalid" -- hub rejected connection
  console.error(e.message);
}
```

### BACnet Protocol Errors

When a remote device responds with an error, reject, or abort PDU:

```javascript
try {
  await client.readProperty(0, 99999, 85);
} catch (e) {
  // "BACnet error: class=2 code=31"  -- object/property not found
  // "BACnet reject: reason=4"        -- request too long, etc.
  // "BACnet abort: reason=0"         -- other abort reasons
  console.error(e.message);
}
```

Error class and code values are defined in ASHRAE 135-2020 Clause 18.

Common error class/code pairs:

| Error Class | Code | Meaning |
|-------------|------|---------|
| 1 (device) | 31 | Unknown object |
| 2 (object) | 32 | Unknown property |
| 2 (object) | 42 | Write access denied |
| 2 (object) | 47 | Value out of range |
| 1 (device) | 46 | Service not supported |

### Not Connected

Methods that require a connection throw immediately if the client is not connected:

```javascript
try {
  client.whoIs(); // throws synchronously
} catch (e) {
  console.error(e.message); // "not connected"
}
```

### Encoding Errors

Invalid object identifiers (instance > 4194303) throw during encoding:

```javascript
try {
  await client.readProperty(0, 5000000, 85);
} catch (e) {
  console.error(e.message); // instance number validation error
}
```

## Limitations

- **BACnet/SC only** -- This client supports BACnet Secure Connect (Annex AB) exclusively. It cannot communicate over BACnet/IP (UDP), MS/TP (serial), or Ethernet. A BACnet/SC hub must be available on the network.

- **Browser only** -- Requires browser WebSocket and `web-sys` APIs. Does not work in Node.js, Deno, or other server-side JavaScript runtimes (the `client` and `ws_transport` modules are gated with `#[cfg(target_arch = "wasm32")]`). The codec and type modules are available on native targets for testing.

- **Client only** -- This is a thin client. It cannot act as a BACnet server, router, or SC hub. Use the `bacnet-server` or `bacnet-transport` Rust crates for server-side functionality.

- **No segmentation** -- Requests are sent as unsegmented PDUs with a max APDU length of 1476 bytes. Very large property values that would require segmented transfers are not supported.

- **Raw value bytes** -- ReadProperty responses return raw application-tagged bytes in `value_bytes`. Higher-level value decoding (e.g., parsing a Real from 4 bytes, extracting strings) must be done in JavaScript.

- **Single callback per event** -- Only one `onIAm` and one `onCovNotification` callback can be active at a time. Setting a new callback replaces the previous one.

- **No automatic reconnection** -- If the WebSocket connection drops, the client moves to the Disconnected state. The application must detect this (via the `connected` getter) and call `connect()` again.

- **Confirmed COV only for subscribe** -- The `subscribeCov` method sends the confirmed SubscribeCOV service. The `onCovNotification` callback receives only unconfirmed COV notifications (UnconfirmedCOVNotification). Confirmed COV notifications from the server are resolved as SimpleAck responses to pending requests.

## Building from Source

```bash
# Prerequisites
rustup target add wasm32-unknown-unknown
cargo install wasm-pack

# Build the WASM package
wasm-pack build crates/bacnet-wasm --target web --out-dir ../../dist/wasm-npm

# Check compilation without building
cargo check -p bacnet-wasm --target wasm32-unknown-unknown

# Run native tests (codec and types only; client requires browser)
cargo test -p bacnet-wasm
```
