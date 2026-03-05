# Rust Examples

These examples demonstrate using the `bacnet-*` crates directly in Rust.

## Examples

| Example | Description |
|---------|-------------|
| [`bip_client_server.rs`](bip_client_server.rs) | BACnet/IP client + server — read, write, RPM, WhoIs |
| [`cov_subscriptions.rs`](cov_subscriptions.rs) | COV subscription with broadcast channel receiver |
| [`multi_object_server.rs`](multi_object_server.rs) | Server with 14 object types, bulk RPM queries |

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
