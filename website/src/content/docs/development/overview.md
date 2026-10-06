---
title: "Build and integrate"
description: "Choose an integration path, find its API and evidence boundaries, and build from source when you need to."
---

**Integration guides for v0.12.0.** These pages cover the parts of the release that go beyond a first read: shared endpoints, Network Port and Number controls, transports and BACnet/SC. Install the release from [v0.12 installation](/rusty-bacnet/start/installation/), or try the [local lab](/rusty-bacnet/start/local-lab/) first.

## Choose the owner for your application

| Your task | Start here |
|---|---|
| Send requests to other devices | Standalone `BACnetClient`; choose a transport and use the API reference |
| Model a device with the full server's service set | Standalone `BACnetServer` and its object database |
| Request and respond as one device through one transport | [Shared endpoints](/rusty-bacnet/development/shared-endpoints/) for B/IP, SC or MS/TP |
| Associate a B/IP socket with a Network Port object | [Network Port registration](/rusty-bacnet/development/network-number/#register-a-bip-receiving-port) |
| Learn and answer local network-number queries | [Passive Number controls](/rusty-bacnet/development/network-number/); no discovery command or startup announcement required |
| Connect through an SC hub | [BACnet/SC setup](/rusty-bacnet/development/bacnet-sc/), with explicit trust, credentials and durable device identity |

Shared endpoints deliberately have a narrower responder than the full server. Transport implementation, language binding and actual runtime evidence are separate choices; compare the [transport matrix](/rusty-bacnet/development/transports/) before selecting a path.

## Build a source checkout

Build from source when you need a change that is not released yet, or a feature set the release builds leave out. Use a separate checkout and environment from a released installation:

```sh
git clone --branch dev https://github.com/jscott3201/rusty-bacnet.git
cd rusty-bacnet
git rev-parse HEAD
cargo build --locked
```

To build the release itself, clone with `--branch v0.12.0` instead. A `dev` checkout may already hold changes past the release while its version still reads 0.12.0, so record the commit.

The checked-in development toolchain is Rust 1.99.0; the declared minimum is 1.93. `cargo build` uses workspace default members, so select the CLI or Python binding explicitly when needed. Native prerequisites depend on the selected features and platform.

For the CLI, from this checkout:

```sh
cargo install --path crates/bacnet-cli --locked
# Use this variant when you need BACnet/SC:
cargo install --path crates/bacnet-cli --locked --features sc-tls
bacnet --version
bacnet --help
```

The executable is `bacnet`. IPv6 is enabled through CLI dependencies; `pcap` is a separate optional feature requiring native capture dependencies. A CLI version string alone cannot distinguish a source build from the release.

For Python, use a fresh virtual environment and build the native extension:

```sh
python3 -m venv .venv
# POSIX shells; on Windows use the virtual environment's Python directly.
. .venv/bin/activate
python -m pip install "maturin>=1,<2"
maturin develop --manifest-path crates/rusty-bacnet/Cargo.toml --locked
python -c "import rusty_bacnet; print(rusty_bacnet.__file__)"
```

This is a source-build workflow; the published wheels are described on the [installation page](/rusty-bacnet/start/installation/#install-the-python-package). Use the installed extension and interpreter you intend to run when validating Python behavior. Native compilation alone does not exercise the Python API.

## Use the reference for exact signatures

The [engineering documentation map](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/README.md) links the canonical [Rust API](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/rust-api.md), [Python API](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/python-api.md) and [CLI reference](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/CLI.md). These task guides summarize those contracts rather than duplicate every method.

Before 1.0, APIs may be removed or changed directly. Keep the source revision, feature selection and installed native artifact together in your integration record. Review [upgrade guidance](/rusty-bacnet/project/upgrading/) when moving an existing application.

## Next steps

- [Compose a shared endpoint](/rusty-bacnet/development/shared-endpoints/).
- [Choose a transport and its evidence scope](/rusty-bacnet/development/transports/).
- [Understand support and conformance](/rusty-bacnet/project/support/).
