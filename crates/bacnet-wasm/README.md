# bacnet-wasm

BACnet/SC thin client for JavaScript/TypeScript via WebAssembly.

Provides a browser-compatible BACnet/SC (Secure Connect) client that connects to BACnet/SC hubs over WebSocket. Built on the [rusty-bacnet](https://github.com/jscott3201/rusty-bacnet) protocol stack.

## Installation

```bash
npm install @rusty-bacnet/bacnet-wasm
```

## Quick Start

```typescript
import { BACnetScClient, ObjectTypes, PropertyIds, encodeReal } from '@rusty-bacnet/bacnet-wasm';

// Create client and connect to SC hub
const client = new BACnetScClient();
await client.connect('wss://hub.example.com:1234');

// Read a property
const value = await client.readProperty(
  ObjectTypes.analog_input(), // object type
  1,                          // instance number
  PropertyIds.present_value() // property identifier
);
console.log('Present Value:', value);

// Write a property
await client.writeProperty(
  ObjectTypes.analog_value(),
  1,
  PropertyIds.present_value(),
  encodeReal(72.5),  // application-tagged value bytes
  8                   // priority (optional)
);

// Subscribe to COV notifications
client.onCovNotification((data) => {
  console.log('COV notification:', data);
});
await client.subscribeCov(1, ObjectTypes.analog_input(), 1, true, 300);

// Send Who-Is
client.onIAm((data) => {
  console.log('I-Am received:', data);
});
client.whoIs();

// Disconnect
await client.disconnect();
```

## API Reference

### `BACnetScClient`

| Method | Description |
|--------|-------------|
| `new()` | Create a new client (random VMAC) |
| `connect(url)` | Connect to SC hub via WebSocket |
| `readProperty(objectType, instance, propertyId, arrayIndex?)` | Read a property |
| `writeProperty(objectType, instance, propertyId, valueBytes, priority?)` | Write a property |
| `whoIs(low?, high?)` | Send Who-Is broadcast |
| `subscribeCov(processId, objectType, instance, confirmed, lifetime?)` | Subscribe to COV |
| `onIAm(callback)` | Register I-Am callback |
| `onCovNotification(callback)` | Register COV notification callback |
| `disconnect()` | Disconnect from hub |
| `connected` | Property: connection status |

### Value Encoding Helpers

| Function | Description |
|----------|-------------|
| `encodeReal(value)` | Encode a float as BACnet Real |
| `encodeUnsigned(value)` | Encode an unsigned integer |
| `encodeBoolean(value)` | Encode a boolean |
| `encodeEnumerated(value)` | Encode an enumerated value |

### Type Constants

- `ObjectTypes` — BACnet object type constants (e.g., `analog_input()`, `device()`)
- `PropertyIds` — Property identifier constants (e.g., `present_value()`, `object_name()`)

## Architecture

This crate compiles the core BACnet protocol logic (types, encoding, services) to WebAssembly and bridges to the browser's native WebSocket API. BACnet/SC already uses WebSocket as its transport, making browsers a natural fit.

```
┌─────────────────────────┐
│  JavaScript/TypeScript   │
├─────────────────────────┤
│  BACnetScClient (WASM)  │
├──────────┬──────────────┤
│ SC Frame │  APDU/NPDU   │
│  Codec   │   Codec      │
├──────────┴──────────────┤
│  Browser WebSocket API  │
└─────────────────────────┘
```

## License

MIT
