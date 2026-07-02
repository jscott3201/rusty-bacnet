# Rust Examples

These examples demonstrate using the `bacnet-*` crates directly in Rust.

## Examples

| Example | Description |
|---------|-------------|
| [`bip_client_server.rs`](bip_client_server.rs) | BACnet/IP client + server — read, write, RPM, WhoIs |
| [`cov_subscriptions.rs`](cov_subscriptions.rs) | COV subscription with broadcast channel receiver |
| [`multi_object_server.rs`](multi_object_server.rs) | Server with 14 object types, bulk RPM queries |

## Standalone sample binaries

For full client/server programs (discovery, point enumeration, priority arrays), see [`samples/`](samples/):

| Sample | Description |
|--------|-------------|
| [`mini-device-revisited`](samples/mini-device-revisited/) | Minimal BACnet/IP server with 4 points |
| [`whois-scan`](samples/whois-scan/) | Who-Is scanner |
| [`point-discover`](samples/point-discover/) | Object-list + present-value + priority-array discovery |
| [`bacnet-write`](samples/bacnet-write/) | WriteProperty with priority + relinquish |
| [`rpm-read`](samples/rpm-read/) | ReadPropertyMultiple bulk read |

## Running

These are standalone `.rs` files meant to be compiled as examples. To run them, add them to a `Cargo.toml` `[[example]]` section or compile directly:

```bash
# From the workspace root, these examples reference workspace crates
# They serve as documentation — adapt them for your own project
```

For working server/client binaries, see `benchmarks/src/bin/`:
- `bacnet-device` — BIP/SC server
- `bacnet-router` — Multi-port router
- `bacnet-bbmd` — Broadcast management device
- `bacnet-sc-hub` — SC hub relay
