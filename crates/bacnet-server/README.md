# bacnet-server

Serve an object database with [`server::BACnetServer`] in the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The server
owns a transport and dispatches BACnet service requests against registered
objects. [`server::ServerBuilder`] accepts a caller-selected transport, Device
identity and database.

This hosted crate requires `std`, Tokio and allocating state. It has no
alloc-only or no-heap profile. The standalone server's service dispatcher is
separate from the narrower server role in `bacnet-endpoint`; their service
contracts are not interchangeable.

## Read a property using development source

Place an application directory beside a checkout named `rusty-bacnet`, then
add this exact dependency block to the application's `Cargo.toml`. These paths
refer to development source; they do not imply the shown APIs are published.
Adjust paths for a different layout. Tokio uses its current-thread runtime,
macros and time support; the program needs no multi-thread runtime feature.

```toml
[dependencies]
bacnet-client = { path = "../rusty-bacnet/crates/bacnet-client", default-features = false }
bacnet-server = { path = "../rusty-bacnet/crates/bacnet-server", default-features = false }
bacnet-objects = { path = "../rusty-bacnet/crates/bacnet-objects" }
bacnet-transport = { path = "../rusty-bacnet/crates/bacnet-transport" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
tokio = { version = "1", features = ["macros", "rt", "time"] }
```

Save this complete program as `src/main.rs`. It initializes an Analog Input to
42.0 degrees Celsius, seeds a Device and its Object_List with
`build_database_with_extra`, and starts a real client and server over bounded
in-process loopback channels. The read has a two-second outer timeout, a
one-second APDU timeout and no retries. Its acknowledgement is checked against
independent application-tagged REAL bytes for 42.0.

```rust
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{build_database_with_extra, DeviceIdentity};
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::{timeout, Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identity = DeviceIdentity::new(1001, 42)?;
    let mut analog = AnalogInputObject::new(
        7,
        "supply-temperature",
        EngineeringUnits::DEGREES_CELSIUS,
    )?;
    analog.set_present_value(42.0);
    let database = build_database_with_extra(&identity, vec![Box::new(analog)])?;
    assert!(database.get(&identity.device_oid()).is_some());

    let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut server = BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(server_port)
        .build()
        .await?;
    let config = ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let mut client = BACnetClient::start(config, client_port).await?;
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7)?;
    let reply = timeout(
        Duration::from_secs(2),
        client.read_property(&[2], object, PropertyIdentifier::PRESENT_VALUE, None),
    )
    .await;

    let client_stopped = client.stop().await;
    let server_stopped = server.stop().await;
    let ack = reply??;
    client_stopped?;
    server_stopped?;
    assert_eq!(ack.object_identifier, object);
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(ack.property_array_index, None);
    assert_eq!(ack.property_value, [0x44, 0x42, 0x28, 0x00, 0x00]);
    Ok(())
}
```

Both owners are explicitly stopped before the read result is propagated. The
read timeout does not impose a shutdown deadline. Directly setting the analog
value is initialization before server ownership; live updates need the
server's local mutation APIs and their event/COV processing.

Loopback sends to its paired peer regardless of the requested destination.
This program establishes an in-process request/response exchange and cleanup,
not destination filtering, UDP, TLS, serial, physical equipment, timing,
firmware behavior or BACnet conformance. Enabling an optional feature while
running it does not exercise that feature's network transport.

## Features and transport selection

| Feature selection | Public entry points |
|---|---|
| Default or `--no-default-features` | `BACnetServer::generic_builder` and `BACnetServer::bip_builder`; hosted `std`/Tokio/allocation remain required. |
| `sc-tls` | Adds `ScServerBuilder` and `BACnetServer::sc_builder` for a constrained-TLS BACnet/SC hub connection. |

The server crate has no `ipv6` feature or dedicated IPv6 builder. Its generic
builder accepts a caller-selected transport; any transport feature selection
belongs to that transport dependency. For the displayed application,
`--features bacnet-server/sc-tls` enables the optional server API while the
program still uses loopback. An actual SC connection needs validated TLS
policy and a caller-provisioned nonzero device UUID. Authentication of an SC
peer does not automatically grant BACnet service or mutation authority;
install a `ServerPolicy` (see `bacnet_server::policy`), which sees the
verified certificate principal of a direct SC peer, and configure the other
service and source policies separately.

The docs.rs profile selects `sc-tls`, disables defaults and targets
`x86_64-unknown-linux-gnu`. The optional SC builder and factory carry feature
badges; the always-present `server` module and generic builder remain visible.
This README is the crate landing page and its program runs as a doctest. A
local render does not establish a docs.rs deployment.

## State, mutation and shutdown

[`server::BACnetServer::database`] exposes raw standalone object state. Use
[`server::BACnetServer::read_local`] for the executor's live view, including
served Device properties and live lists. Do not hold a database guard across
network operations or shutdown. Use the server's local mutation APIs, such as
[`server::BACnetServer::set_present_value_local`], to apply the documented
ownership checks and event/COV processing.

Call [`server::BACnetServer::stop`] before dropping the server or reopening its
storage. Stop seals egress, joins admitted work and stops the owned transport.
Durable state may require waiting for queued saves; stalled storage can delay
shutdown without a fixed deadline. Cancelling a stop waiter preserves cleanup
ownership for a later stop. Keeping a database handle can outlive the server;
follow the handle's documentation and [`server::drop_database_off_runtime`]
when releasing durable object state from async code.

Select service and write policies for the application. A successful read in
this example establishes neither write authorization nor support for every
service/object combination. See the relevant handler, object and policy APIs
for their limits.
