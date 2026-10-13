# Rusty BACnet

A BACnet protocol stack written in Rust, with Python bindings and a command-line
tool. Use it to build BACnet clients, model devices and serve their objects, or
explore protocol behavior in a local lab. It targets ASHRAE Standard 135-2020.
It isn't BTL certified; [Conformance](#conformance) explains what is covered.

[![crates.io](https://img.shields.io/crates/v/bacnet-client.svg)](https://crates.io/crates/bacnet-client)
[![PyPI](https://img.shields.io/pypi/v/rusty-bacnet.svg)](https://pypi.org/project/rusty-bacnet/)
[![docs.rs](https://img.shields.io/docsrs/bacnet-client)](https://docs.rs/bacnet-client)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**[Documentation](https://jscott3201.github.io/rusty-bacnet/)** ·
[Install](#install) · [Quickstart](#quickstart) · [Transports](#transports) ·
[Crates](#crates) · [Find it in the docs](#find-it-in-the-docs) ·
[Roadmap](#roadmap) · [Contributing](#contributing)

> [!NOTE]
> **Release and branch.** The latest release is **0.13.0**. The published
> packages, the hosted guides and docs.rs all describe that release. This README
> and the reference docs in [`docs/`](docs/) follow the `dev` branch, which may
> hold changes merged since. Those wait in [`changelog.d/`](changelog.d/) until
> the next release adds them to the [changelog](CHANGELOG.md).
>
> **Pre-1.0.** Public APIs can change in any minor release while obsolete APIs
> are removed. They freeze at 1.0.0.

## Features

- **Async Rust client and server.** Built on Tokio. Transaction handling,
  segmentation, discovery, COV subscriptions, and alarm and event services, with
  the protocol layers split into separate crates you can depend on individually.
- **Object models.** A server-side object database with standard object types,
  property metadata, commandable outputs, intrinsic reporting, and draft PICS
  generation from your server's configuration.
- **Five data links.** BACnet/IP, BACnet/IPv6, BACnet/SC (secure WebSocket/TLS,
  including a hub), MS/TP over serial, and Ethernet on Linux. See
  [Transports](#transports).
- **Routing.** A network layer with router tables, routed requests and BBMD and
  foreign-device support for BACnet/IP.
- **Embedded cores.** Portable `no_std` cores for MS/TP, BACnet/IP (BBMD
  included) and routing between them, which run without an allocator on
  microcontrollers and take time from the caller. The Tokio MS/TP and
  BACnet/IP transports and the router run on these same cores. See
  [Roadmap](#roadmap).
- **Python bindings.** `BACnetClient`, `BACnetServer`, `ScHub` and the shared
  endpoints with asyncio support, typed enums and values, and async COV notification streams. Python and
  Rust expose different configuration surfaces, so check the Python API before
  assuming a Rust option exists there.
- **CLI.** The `bacnet` tool does discovery, reads and writes, COV
  subscriptions, alarms, file transfer and BBMD management over BACnet/IP,
  BACnet/IPv6 and BACnet/SC, with an interactive shell and optional packet
  capture.
- **Shared endpoints.** One device can send requests and
  answer a limited set of them (ReadProperty by default) through a single
  BACnet/IP, BACnet/SC or MS/TP transport, from Rust or Python. Use the
  standalone server when you need its full service set.

> [!IMPORTANT]
> Only use this on networks and devices you are authorized to access.
> Discovery generates network traffic. Writes, device management, time
> synchronization and file transfers can change real equipment. Start with the
> loopback examples below. Rusty BACnet makes no physical-safety guarantee.

## Install

CI runs the test suites on Linux, and natively on macOS (Apple Silicon) and
Windows (x86_64).

### Python

```bash
python -m pip install rusty-bacnet
```

This needs Python 3.11 or newer. The import name is `rusty_bacnet`. Wheels are
published for CPython 3.11–3.14 on Linux (glibc 2.17 or newer; x86_64,
aarch64), macOS (x86_64 on 10.12 or later, arm64 on 11.0 or later) and Windows
(x64). Every wheel includes BACnet/IPv6, BACnet/SC and MS/TP. On any other
Python version, platform or musl-based Linux, pip builds from source, which
needs Rust 1.93 or newer and a C compiler. Add `--only-binary=:all:` to fail
fast instead.

### Rust

Add only the crates you need. Most applications start with the client or the
server crate:

```toml
[dependencies]
bacnet-client = "0.13"
bacnet-types = "0.13"
bacnet-encoding = "0.13"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The minimum supported Rust version is **1.93**.

### CLI

Download the `bacnet-<os>-<arch>` file for your platform from the
[latest release](https://github.com/jscott3201/rusty-bacnet/releases/latest),
check it against the release's `SHA256SUMS`, rename it to `bacnet`
(`bacnet.exe` on Windows), make it executable and put it on your `PATH`. The
[installation guide](https://jscott3201.github.io/rusty-bacnet/start/installation/)
has the checksum command for each OS.
- There are builds for Linux (amd64, arm64), macOS (amd64, arm64) and Windows
  (amd64), all with BACnet/SC. The Linux builds also include packet capture.
- The Linux builds need glibc 2.17 or newer (RHEL/CentOS 7, Debian 8, Ubuntu
  14.04 and later) and no libpcap package, because they link it statically.
- The macOS builds need macOS 10.12 (amd64) or 11.0 (arm64) or later. They
  aren't notarized, so macOS may block one downloaded through a browser.
- The Windows build links the C runtime statically, so it needs no Visual C++
  Redistributable.

`bacnet-cli` is also on crates.io, so Cargo can build it on any platform with
Rust 1.93 or later. Add `,pcap` to the features for capture, which needs the
libpcap headers:

```bash
cargo install bacnet-cli --locked --features sc-tls
# or, from a checkout:
cargo install --path crates/bacnet-cli --locked --features sc-tls
```

### Build from source

To try changes merged after the release, build from a `dev` checkout. Its
version may still read 0.13.0, so record the commit you built.

- **Python:** in a virtual environment, run `python -m pip install "maturin>=1,<2"`,
  then `maturin develop --release --manifest-path crates/rusty-bacnet/Cargo.toml --locked`.
- **Rust:** depend on the git branch, for example
  `bacnet-client = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }`.

## Quickstart

These examples run entirely on loopback. Start the server in one terminal and
leave it running. Then read from it in a second terminal with Python, Rust or
the CLI.

### 1. Run a local server (Python)

Save this as `local_server.py` and run `python local_server.py`. It serves one
simulated temperature sensor on `127.0.0.1:47808`. Stop it with **Ctrl+C**.

```python
import asyncio
from rusty_bacnet import BACnetServer


async def main():
    server = BACnetServer(
        device_instance=1234,
        device_name="Local BACnet lab",
        interface="127.0.0.1",
        port=47808,
        broadcast_address="127.0.0.1",
    )
    # Units 62 = degrees Celsius. Add objects before starting the server.
    server.add_analog_input(1, "Zone temperature", units=62, present_value=22.5)
    try:
        await server.start()
        print(f"Listening at {await server.local_address()}", flush=True)
        await asyncio.Event().wait()
    finally:
        await server.stop()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
```

Before you leave loopback, pick a device instance that is unique on your
network.

### 2. Read a property

**Python:**

```python
import asyncio
from rusty_bacnet import BACnetClient, ObjectIdentifier, ObjectType, PropertyIdentifier


async def main():
    # Port 0 picks a free local port, so it doesn't clash with the server.
    async with BACnetClient(
        interface="127.0.0.1", port=0, broadcast_address="127.0.0.1"
    ) as client:
        value = await client.read_property(
            "127.0.0.1:47808",
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(value.value)  # 22.5


asyncio.run(main())
```

**Rust:** create a project with `cargo new`, add the dependencies from
[Install](#rust), replace `src/main.rs` with the following, then `cargo run`:

```rust
use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await?;

    // A BACnet/IP address: four IPv4 octets, then the UDP port (0xBAC0 = 47808).
    let address = [127, 0, 0, 1, 0xBA, 0xC0];
    let response = client
        .read_property(&address, oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    client.stop().await?;

    let ack = response?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("{value:?}");
    Ok(())
}
```

**CLI:**

```bash
bacnet --interface 127.0.0.1 --port 0 read 127.0.0.1:47808 ai:1 pv
bacnet --interface 127.0.0.1 --port 0 --json readm 127.0.0.1:47808 ai:1 pv,object-name
```

`ai:1` is Analog Input 1 and `pv` is Present_Value. Next, try discovery, COV
subscriptions and multi-property reads in the
[Python guide](https://jscott3201.github.io/rusty-bacnet/start/python/),
[Rust guide](https://jscott3201.github.io/rusty-bacnet/start/rust/) or
[CLI reference](docs/CLI.md). The [`examples/`](examples/) directory has
complete Rust, Python and Docker setups.

## Transports

BACnet/IP is always available. The other transports are opt-in Cargo features
of `bacnet-transport`:
- `bacnet-client` also has `ipv6` and `sc-tls` features.
- `bacnet-server` and `bacnet-endpoint` have `sc-tls`.

These turn on each crate's builders for those transports. The Python package
includes BACnet/IPv6, BACnet/SC and MS/TP, but not Ethernet.

| Transport | Feature | Notes |
|---|---|---|
| BACnet/IP (UDP/IPv4) | none | Includes BBMD and foreign-device registration. NAT traversal and B/IP multicast are not implemented. |
| BACnet/IPv6 | `ipv6` | Selects one concrete interface and address and keeps traffic on that link. With `::`, startup fails if the host has more than one candidate, so pass a concrete address. |
| BACnet/SC | `sc-tls` | Nodes, direct connections and a hub over TLS 1.3. Requires a site CA, a certificate and key for each device, and a provisioned device UUID. |
| MS/TP | `serial` (`serial-gpio` for GPIO direction control) | Standard frames, plus extended (COBS-encoded) frames for NPDUs up to 1497 octets with `MstpConfig::extended_frames`, which a router turns on for its MS/TP ports. RS-485 kernel options and GPIO are Linux-only. Evidence comes from a simulator and loopback; on-wire timing isn't qualified on any adapter or OS. |
| Ethernet (802.3 LLC) | `ethernet` | Linux only (`AF_PACKET`). Needs `CAP_NET_RAW` or root. |

The [BACnet/SC guide](https://jscott3201.github.io/rusty-bacnet/guides/bacnet-sc/)
covers CLI trust and identity. Before 0.12.0, BACnet/IPv6 could fall back to `::1`
when it found no address, and the CLI loaded SC trust from the system roots; the
migration notes in the [0.12.0 changelog](CHANGELOG.md#0120---2026-10-05) list
what to change.

How to configure each one:
- [Transport configuration](docs/rust-api.md#transport-configuration-examples) (Rust)
- [Python transport examples](docs/python-api.md#transport-configuration-examples)
- [MS/TP guide](https://jscott3201.github.io/rusty-bacnet/guides/mstp/)

## Crates

| Crate | Purpose |
|---|---|
| [`bacnet-types`](https://crates.io/crates/bacnet-types) | Enums, primitives, bit strings and errors (`no_std` with `alloc`) |
| [`bacnet-encoding`](https://crates.io/crates/bacnet-encoding) | ASN.1 tags, APDU/NPDU codecs, segmentation (`no_std` with `alloc`) |
| [`bacnet-services`](https://crates.io/crates/bacnet-services) | Service request and response types (`no_std` with `alloc`) |
| [`bacnet-time`](https://crates.io/crates/bacnet-time) | The monotonic `Instant` and `Duration`, in nanoseconds, that the portable cores share, so firmware feeds them one clock (`no_std`; no allocator, no dependencies) |
| [`bacnet-mstp-codec`](https://crates.io/crates/bacnet-mstp-codec) | MS/TP frame codec: CRCs, encode, decode and a stream decoder, for standard and extended (COBS-encoded) frames (`no_std`; no allocator or dependencies by default) |
| [`bacnet-mstp-core`](https://crates.io/crates/bacnet-mstp-core) | MS/TP master state machine driven by caller-supplied time and events (`no_std`; no allocator by default) |
| [`bacnet-bip-codec`](https://crates.io/crates/bacnet-bip-codec) | BACnet/IP and BACnet/IPv6 BVLL codec: every Annex J and Annex U message, B/IP and B/IPv6 addresses, VMACs and BDT/FDT tables (`no_std`; no allocator or dependencies by default) |
| [`bacnet-bip-core`](https://crates.io/crates/bacnet-bip-core) | Portable BACnet/IP behaviour: a node engine for normal, foreign-device and BBMD modes, a BBMD's BDT and FDT in caller-sized arrays, foreign-device registration and broadcast forwarding, on caller-supplied time (`no_std`; no allocator) |
| [`bacnet-router-core`](https://crates.io/crates/bacnet-router-core) | Portable router behaviour (Clause 6): a sans-IO routing engine with its routing table and route discovery, on caller-supplied time, which `bacnet-network`'s router runs on, and B/IP and MS/TP port glue behind features (`no_std`; no allocator) |
| [`bacnet-transport`](https://crates.io/crates/bacnet-transport) | BACnet/IP, BACnet/IPv6, BACnet/SC, MS/TP and Ethernet data links |
| [`bacnet-network`](https://crates.io/crates/bacnet-network) | Network layer and routing |
| [`bacnet-client`](https://crates.io/crates/bacnet-client) | Async client |
| [`bacnet-objects`](https://crates.io/crates/bacnet-objects) | `BACnetObject` trait, object database and object types |
| [`bacnet-server`](https://crates.io/crates/bacnet-server) | Async server: dispatch, COV, events, scheduling, PICS |
| [`bacnet-endpoint-core`](https://crates.io/crates/bacnet-endpoint-core) | Shared ownership and transaction coordination for endpoints |
| [`bacnet-endpoint`](https://crates.io/crates/bacnet-endpoint) | One transport owner for both client and server roles |
| [`bacnet-cli`](https://crates.io/crates/bacnet-cli) | The `bacnet` command-line tool (also as release binaries) |

All seventeen are on crates.io at 0.13.0. The
Python package is built from [`crates/rusty-bacnet`](crates/rusty-bacnet) with
[maturin](https://www.maturin.rs/). The [architecture guide](docs/architecture.md)
shows how the layers fit together.

## Find it in the docs

The [hosted guides](https://jscott3201.github.io/rusty-bacnet/) cover the
0.13.0 release. The [`docs/`](docs/) references track `dev`; the release's copies
are at the `v0.13.0` tag.

| Topic | Where to look |
|---|---|
| Installing and first steps | [Installation](https://jscott3201.github.io/rusty-bacnet/start/installation/), [choose your path](https://jscott3201.github.io/rusty-bacnet/start/choose-your-path/), [local lab](https://jscott3201.github.io/rusty-bacnet/start/local-lab/) |
| Rust API | [docs.rs](https://docs.rs/bacnet-client) (release), [`docs/rust-api.md`](docs/rust-api.md) (dev) |
| Python API | [v0.13.0](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/python-api.md) (release), [`docs/python-api.md`](docs/python-api.md) (dev) |
| CLI | [v0.13.0](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/CLI.md) (release), [`docs/CLI.md`](docs/CLI.md) (dev) |
| Discovery and COV | [Discovery](https://jscott3201.github.io/rusty-bacnet/guides/discovery/), [observing changes](https://jscott3201.github.io/rusty-bacnet/guides/observe-changes/) |
| BACnet/SC: credentials, device identity, hub policy | [Rust node](docs/rust-api.md#bacnetsc-client-transport), [Rust hub](docs/rust-api.md#bacnetsc-hub), [Python](docs/python-api.md#bacnetsc-secure-connect), [hub certificate bindings](docs/python-api.md#hub-certificate-bindings) |
| Shared endpoints | [Rust](docs/rust-api.md#bacnet-endpoint), [Python](docs/python-api.md#endpoint-one-transport-both-roles) |
| Writes and authorization | [Safe writes](https://jscott3201.github.io/rusty-bacnet/guides/safe-writes/), [mutation policy](docs/mutation-policy.md), [Device Communication Control](docs/dcc-policy.md) |
| Audit reporting | [Rust](docs/rust-api.md#audit-services), [Python](docs/python-api.md#audit-services), [target reporters](docs/target-audit-reporters.md) |
| Policy and resource limits | [Engineering docs index](docs/README.md#policy-and-resource-contracts) |
| Upgrading between releases | [Upgrade guide](https://jscott3201.github.io/rusty-bacnet/project/upgrading/), [changelog](CHANGELOG.md) |
| Troubleshooting | [Troubleshooting guide](https://jscott3201.github.io/rusty-bacnet/help/troubleshooting/) |

## Conformance

Rusty BACnet is **not BTL certified** and does not claim full BACnet
conformance. The website's
[What's supported](https://jscott3201.github.io/rusty-bacnet/project/support/#whats-supported)
section lists the object types, services, transports and Python bindings that
0.13.0 implements. A running server can also generate a PICS for its
own objects and services with `bacnet_server::pics`.

## Roadmap

Planned work is public on the
**[Rusty BACnet Roadmap](https://github.com/users/jscott3201/projects/4)**
project board. Every open issue is there with its track (MS/TP, BACnet/IP and
BBMD, routing, Python, server and objects, and so on), a priority from P0 to
P3, and the release it's aimed at:

- **0.13.0**, the current release, put `no_std` first. It brought the
  portable MS/TP and BACnet/IP cores, with BBMD and routing available to
  firmware, the Tokio transports on those same cores, and the first
  crates.io release of the new crates.
- **0.14.0** brings the interactive terminal UI for the `bacnet` CLI.
- An issue with no milestone is in the backlog.

Releases have no dates. Each ships when its milestone's work is done. The
board's views show the same issues by track, by release and by parent issue.
**Needs decision** lists open design questions, where comments help most, and
**Good first issues** lists self-contained places to start. To suggest
something, [open an issue](https://github.com/jscott3201/rusty-bacnet/issues/new/choose);
new issues join the board automatically.

## Contributing

Bug reports, test cases, documentation fixes and focused patches are welcome.
See [CONTRIBUTING.md](CONTRIBUTING.md) for the pull request flow, and the
[documentation guide](https://jscott3201.github.io/rusty-bacnet/project/contributing/)
for docs changes.

```bash
git clone https://github.com/jscott3201/rusty-bacnet.git
cd rusty-bacnet
cargo install cargo-nextest --locked   # 0.9.145 or newer
cargo build --locked
cargo nextest run --workspace --exclude rusty-bacnet --locked
cargo test --doc --workspace --exclude rusty-bacnet --locked
```

nextest skips doctests, which is why `cargo test --doc` is a separate step. The
repository pins Rust 1.99.0 in `rust-toolchain.toml`. For Python binding
development (on Windows, activate with `.venv\Scripts\activate`; the BACnet/SC
tests also need the `openssl` command):

```bash
python -m venv .venv && source .venv/bin/activate
python -m pip install "maturin>=1,<2"
maturin develop --manifest-path crates/rusty-bacnet/Cargo.toml --locked
python -m unittest discover -s crates/rusty-bacnet/tests
```

[`docs/ci.md`](docs/ci.md) lists the full set of checks CI runs, including
clippy, rustdoc and the feature matrix.

When you [open an issue](https://github.com/jscott3201/rusty-bacnet/issues),
include:
- the version or commit;
- your OS and the transport you use;
- a minimal reproduction that leaves out credentials and captures from real
  networks.

Report security vulnerabilities privately as described in the
[security policy](.github/SECURITY.md).

## License

[MIT](LICENSE)
