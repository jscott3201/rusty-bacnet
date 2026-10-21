# bacnet-client

Run a standalone asynchronous BACnet client with [`client::BACnetClient`] in
the [Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. It owns
a transport and request lifecycle, with property/service APIs, discovery,
transaction management and segmentation. Use [`client::ClientConfig`] for
request settings or a transport-specific builder for connection setup.

This hosted crate requires `std`, Tokio and allocating state. It has no
alloc-only or no-heap profile. For one device combining client and server roles
under one transport owner, see the separate `bacnet-endpoint` crate and its
narrower service contract.

## Send Who-Is using development source

Place an application directory beside a checkout named `rusty-bacnet`, then
add this exact dependency block to the application's `Cargo.toml`. These paths
refer to development source; they do not imply the shown APIs are published.
Adjust paths for a different layout. Tokio uses its current-thread runtime,
macros and time support; the program needs no multi-thread runtime feature.

```toml
[dependencies]
bacnet-client = { path = "../rusty-bacnet/crates/bacnet-client", default-features = false }
bacnet-transport = { path = "../rusty-bacnet/crates/bacnet-transport" }
tokio = { version = "1", features = ["macros", "rt", "time"] }
```

Save this complete program as `src/main.rs`. It starts a real client over
bounded in-process loopback channels and sends Who-Is without device-instance
limits. The paired raw transport receives the emitted frame. Sending and
receiving share a two-second timeout; the complete NPDU is checked against
independent expected bytes for this global Who-Is request.

```rust
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use tokio::time::{timeout, Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client_port, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut frames = peer.start().await?;
    let mut client = BACnetClient::start(ClientConfig::default(), client_port).await?;
    let received = timeout(Duration::from_secs(2), async {
        client.who_is(None).await?;
        let frame = frames.recv().await.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "loopback peer closed")
        })?;
        Ok::<_, Box<dyn std::error::Error>>(frame)
    })
    .await;

    let client_stopped = client.stop().await;
    let peer_stopped = peer.stop().await;
    let frame = received??;
    client_stopped?;
    peer_stopped?;
    assert_eq!(
        &frame.npdu[..],
        &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08]
    );
    assert_eq!(&frame.source_mac[..], &[1]);
    assert!(frame.link_layer_group);
    Ok(())
}
```

Both owners are explicitly stopped before the operation result is propagated.
The operation timeout does not impose a shutdown deadline. This program
observes an emitted Who-Is frame; it neither receives I-Am nor establishes
that a device was discovered. For a complete client/server ReadProperty
exchange, see the [server guide](https://github.com/jscott3201/rusty-bacnet/blob/dev/crates/bacnet-server/README.md).

Loopback sends to its paired peer regardless of the requested destination.
This program establishes in-process transmission, receipt and cleanup,
not destination filtering, UDP, TLS, serial, physical equipment, timing,
firmware behavior or BACnet conformance. Enabling an optional feature while
running it does not exercise that feature's network transport.

## Features and builders

| Feature selection | Public entry points |
|---|---|
| Default or `--no-default-features` | `BACnetClient::start`, `BACnetClient::generic_builder` and `BACnetClient::bip_builder`; hosted `std`/Tokio/allocation remain required. |
| `ipv6` | Adds `Bip6ClientBuilder` and `BACnetClient::bip6_builder` for BACnet/IPv6. |
| `sc-tls` | Adds `ScClientBuilder` and `BACnetClient::sc_builder` for a constrained-TLS BACnet/SC hub connection. |

For the displayed application, select `--features bacnet-client/ipv6`,
`--features bacnet-client/sc-tls`, or
`--features bacnet-client/ipv6,bacnet-client/sc-tls` when running Cargo. Each
selection still runs the loopback program above. Real connections need the
appropriate transport addresses and, for SC, validated TLS policy and a
caller-provisioned nonzero device UUID. TLS peer authentication does not by
itself authorize BACnet services or mutations on a server.

The docs.rs profile enables `ipv6` and `sc-tls`, disables defaults and targets
`x86_64-unknown-linux-gnu`. Optional builders and factories carry feature badges;
the always-present `client` module and generic builder stay available under
each profile. This README is the crate landing page and its program runs as a
doctest. A local render does not establish a docs.rs deployment.

## Request and lifecycle ownership

[`client::BACnetClient::read_property`] returns encoded property data and checks
that the acknowledgement matches the requested object, property and array
index. Decode it for your property's expected type; the Who-Is byte literal
above is not a property-value decoder.

Keep the client owner until requests finish, then call
[`client::BACnetClient::stop`]. Stop aborts and joins its dispatch and local
network-number work, cancels transactions and stops the owned transport. A
cancelled stop waiter retains joins for a later stop; dropping the client is
not a substitute for awaiting cleanup. The API does not promise a universal
fixed shutdown deadline.

When using a server, its database exposes raw object state; use its `read_local`
method for executor-owned live properties. Release database guards before
network requests or shutdown. Server service/mutation/source policies remain
application decisions, including when a transport authenticates its peers.
