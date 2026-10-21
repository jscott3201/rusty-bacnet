# bacnet-network

NPDU assembly, APDU dispatch and routing components for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
`layer::NetworkLayer` wraps one transport and handles NPDU headers around APDUs.
It can address remote devices through a local router, but does not forward
between networks. `router::BACnetRouter` is the separate multi-port router.
This hosted crate requires `std` and Tokio.

## Quickstart from development source

This dependency block uses a checkout named `rusty-bacnet` beside your application
directory. It describes development source, not the contents of a published
crate version. Adjust the relative paths for another layout. Each crate used
by the example is a direct dependency, including the Tokio runtime and macros.

```toml
[dependencies]
bacnet-network = { path = "../rusty-bacnet/crates/bacnet-network" }
bacnet-transport = { path = "../rusty-bacnet/crates/bacnet-transport" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
tokio = { version = "1", features = ["macros", "rt", "time"] }
```

Send an APDU through two network layers connected by in-process loopback
channels. The receiving layer removes the NPDU header and exposes the immediate
source MAC. The timeout bounds the wait; no socket or device is opened.

```rust
use bacnet_network::layer::NetworkLayer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_npdu::NetworkPriority;
use tokio::time::{timeout, Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (left_port, right_port) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut left = NetworkLayer::new(left_port);
    let mut right = NetworkLayer::new(right_port);
    let left_rx = left.start().await?;
    let mut right_rx = right.start().await?;

    let apdu = [0x10, 0x08];
    left.send_apdu(&apdu, &[2], false, NetworkPriority::Normal)
        .await?;
    let received = timeout(Duration::from_secs(2), right_rx.recv())
        .await?
        .ok_or("network receive channel closed")?;
    assert_eq!(received.apdu.as_ref(), &apdu);
    assert_eq!(received.source_mac.as_ref(), &[1]);
    assert!(received.source_network.is_none());
    assert!(!received.link_layer_group);

    left.stop().await?;
    right.stop().await?;
    drop((left_rx, right_rx));
    Ok(())
}
```

Both receivers returned by `start` stay owned until cleanup. Each network
layer's `stop` ends and joins its dispatch task before stopping its transport.
Loopback always hands a send to its paired peer, regardless of destination MAC;
the example does not establish destination filtering. These assertions prove
local APDU delivery without a routed source. They do not demonstrate a remote
response, a completed BACnet transaction or forwarding between networks.

## API boundaries and features

- `layer` owns single-transport NPDU processing, APDU delivery and optional
  network-control reception. `start_with_admission` exposes receive-queue
  counters and the documented per-source admission policy.
- `router` owns forwarding between configured ports and delivery to local
  endpoints. Its `BACnetRouter`, `RouterPort` and `RouterOptions` types describe
  that lifecycle separately from `NetworkLayer`.
- `network_number`, `router_table`, `response_route` and `priority_channel`
  provide the associated state, captured reply routes and queue primitives.

This crate declares no optional features. Select an optional transport through
a direct `bacnet-transport` dependency; Cargo unifies that crate's features.
Neither crate has a `no_std` or allocation-free hosted configuration. Physical
transport access, TLS configuration and hardware qualification are separate
from this channel-based quickstart.

## API documentation

This README is also the crate's Rust documentation landing page. The docs.rs
profile disables implicit defaults, selects no additional features, and renders
only `x86_64-unknown-linux-gnu`. Its own network layer and router APIs are present
without enabling every optional transport. The underlying transport crate has
its own documentation profile. Local rendering checks documentation; it does
not establish a docs.rs deployment or physical-network interoperability.
