# Rust Examples

These examples demonstrate using the `bacnet-*` crates directly in Rust. Each
starts a server and a client on 127.0.0.1 over BACnet/IP and exits when done.

## Examples

| Example | Description |
|---------|-------------|
| [`bip_client_server.rs`](bip_client_server.rs) | BACnet/IP client + server — read, write, ReadPropertyMultiple, directed Who-Is |
| [`cov_subscriptions.rs`](cov_subscriptions.rs) | COV subscription with broadcast channel receiver; the server changes the value with `set_present_value_local` |
| [`multi_object_server.rs`](multi_object_server.rs) | Server with 15 objects of 12 types, bulk ReadPropertyMultiple query |
| [`point_list_server.rs`](point_list_server.rs) | Server whose database is built from a `PointList` with `bacnet_objects::points::build_database`; a first reading set before it serves, the points read back |
| [`point_list_endpoint.rs`](point_list_endpoint.rs) | Two endpoints, each built from its own `PointList`, each serving its points and reading the other's through one socket |
| [`points.rs`](../../firmware/link/src/points.rs) | The firmware example: a `no_std` + `alloc` image's database built from a `PointList` of all nine analog, binary and multi-state types, then answered by `bacnet-device-core`'s `Responder`; `cargo nextest run -p firmware-link --test points` runs it natively |
| [`sc_peer_certificates.rs`](../../crates/bacnet-client/tests/support/sc_peer_certificates.rs) | Owned accepted-Hub snapshots and untrusted rejected-attempt certificates; exercised by the SC client integration test |
| [`sc_inspect_pem.rs`](../../crates/bacnet-transport/examples/sc_inspect_pem.rs) | Inspect supplied SC credential files offline, without a hub or client |

## Sample programs

For full client/server programs (discovery, point enumeration, priority arrays), see [`samples/`](samples/). Each is a workspace crate outside the default build; run one with `cargo run -p <name> -- --help`:

| Sample | Description |
|--------|-------------|
| [`mini-device-revisited`](samples/mini-device-revisited/) | Minimal BACnet/IP server with 4 points |
| [`whois-scan`](samples/whois-scan/) | Who-Is scanner |
| [`point-discover`](samples/point-discover/) | Object-list + present-value + priority-array discovery |
| [`bacnet-write`](samples/bacnet-write/) | WriteProperty with priority + relinquish |
| [`rpm-read`](samples/rpm-read/) | ReadPropertyMultiple bulk read |

## Running

The examples in this directory are Cargo examples of the
`bacnet-integration-tests` crate (see its `Cargo.toml`), so they build against
the workspace crates. From the workspace root:

```bash
cargo run -p bacnet-integration-tests --example bip_client_server
cargo run -p bacnet-integration-tests --example cov_subscriptions
cargo run -p bacnet-integration-tests --example multi_object_server
cargo run -p bacnet-integration-tests --example point_list_server
cargo run -p bacnet-integration-tests --example point_list_endpoint
```

CI compiles them in its `cargo clippy --workspace --all-targets` and
`cargo nextest run --workspace` steps. In your own project, copy an example
into `src/main.rs` and depend on the crates it imports.

For working server/client binaries, see `benchmarks/src/bin/`:
- `bacnet-device` — BIP/SC server
- `bacnet-router` — Multi-port router
- `bacnet-bbmd` — Broadcast management device
- `bacnet-sc-hub` — SC hub relay
