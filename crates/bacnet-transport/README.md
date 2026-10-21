# bacnet-transport

Data-link transports and framing for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace. The
`port::TransportPort` trait sends and receives NPDUs; transport-native MAC bytes
identify peers. This hosted crate requires `std` and Tokio. Disabling optional
features does not provide a `no_std` or allocation-free configuration.

## Quickstart from development source

This dependency block uses a checkout named `rusty-bacnet` beside your application
directory. It describes development source, not the contents of a published
crate version. Adjust the relative path for another layout.

```toml
[dependencies]
bacnet-transport = { path = "../rusty-bacnet/crates/bacnet-transport" }
tokio = { version = "1", features = ["macros", "rt", "time"] }
```

Send one NPDU through a pair of in-process channels and inspect what the peer
receives. The example uses a single-thread Tokio runtime, opens no socket or
device, and bounds its receive wait.

```rust
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use tokio::time::{timeout, Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut left, mut right) = LoopbackTransport::pair(vec![1], vec![2]);
    let left_rx = left.start().await?;
    let mut right_rx = right.start().await?;

    let npdu = [0x01, 0x00, 0x10, 0x08];
    left.send_unicast(&npdu, &[2]).await?;
    let received = timeout(Duration::from_secs(2), right_rx.recv())
        .await?
        .ok_or("loopback receive channel closed")?;
    assert_eq!(received.npdu.as_ref(), &npdu);
    assert_eq!(received.source_mac.as_ref(), &[1]);
    assert!(!received.link_layer_group);

    left.stop().await?;
    right.stop().await?;
    drop((left_rx, right_rx));
    Ok(())
}
```

Both receivers returned by `start` remain owned until cleanup. Loopback delivers
every send to its paired peer, regardless of the destination MAC supplied; this
example does not test destination filtering. Its `stop` has no background task
to join, and dropping the owners and receivers releases the channels. Receiving
these bytes demonstrates local delivery, not a remote response or a completed
BACnet transaction.

## Transports and features

The default feature set is empty. It includes BACnet/IP UDP (`bip`), BBMD state
and forwarding (`bbmd`), generic MS/TP (`mstp`), loopback, BVLL framing (`bvll`),
BACnet/SC framing (`sc_frame`) and the generic WebSocket-based SC transport
(`sc`). The generic MS/TP and SC transports take a serial or WebSocket adapter;
the concrete hosted adapters below are optional.

| Feature | Additional API and platform conditions |
| --- | --- |
| `ipv6` | `bip6::Bip6Transport` for BACnet/IPv6 over UDP. |
| `sc-tls` | `sc_tls::TlsWebSocket` and `sc_hub::ScHub`, using rustls and AWS-LC for TLS. |
| `serial` | `mstp_serial::TokioSerialPort`, and `mstp_serial::ReceiveOnlyPort` for the receive-only `mstp::monitor::MstpMonitor`; Linux also exposes kernel RS-485 direction control. |
| `serial-gpio` | Linux GPIO character-device direction control through `mstp_serial::GpioDirectionPort`; also enables `serial`. |
| `ethernet` | Ethernet frame codecs; `ethernet::EthernetTransport` additionally requires Linux and uses raw packet sockets. |
| `test-support` | `mstp::monitor::script`: the scripted MS/TP line and token ring the monitor's own tests use, for tools built on the monitor. |

Selecting a feature makes its APIs available; running a concrete adapter also
requires suitable operating-system access, configuration and, where applicable,
hardware or TLS credentials. Ethernet codecs and the Linux transport are distinct.
GPIO and kernel RS-485 control depend on Linux facilities. The current locked
serial dependency graph does not include `libudev`.

`any::AnyTransport` combines supported concrete transports for mixed links.
Its optional variants follow their feature and platform gates, and downstream
matches need a wildcard because the enum is non-exhaustive. Use
`port::TransportPort` for a generic transport boundary. NPDU construction,
APDU delivery and routing between networks belong to `bacnet-network`.

## API documentation

This README is also the crate's Rust documentation landing page. The docs.rs
profile disables implicit defaults, enables `ipv6`, `sc-tls`, `serial`,
`serial-gpio` and `ethernet`, and renders only `x86_64-unknown-linux-gnu`.
Gated public modules carry feature badges. Nested types and enum variants retain
their actual feature and platform gates; no `docsrs` condition bypasses them.
Rendering documentation does not qualify hardware operation or deploy docs.rs.
