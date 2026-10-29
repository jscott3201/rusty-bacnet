---
title: "Build from source"
description: "Build the development checkout, select CLI or Python features, and record its revision."
---

Use this path to work on the repository or test an unreleased change. For application integration against the release, start with [Build and integrate](/rusty-bacnet/integrate/overview/).

## Build a source checkout

Build from source when you need a change that is not released yet, or a feature set the release builds leave out. Use a separate checkout and environment from a released installation:

```sh
git clone --branch dev https://github.com/jscott3201/rusty-bacnet.git
cd rusty-bacnet
git rev-parse HEAD
cargo build --locked
```

To build the release itself, clone with `--branch v0.14.0` instead. A `dev` checkout may already hold changes past the release while its version still reads 0.14.0, so record the commit.

The checked-in development toolchain is Rust 1.99.0. The 0.15 development workspace uses edition 2024 and requires Rust 1.95; the released v0.14.0 workspace requires Rust 1.93. The minimum increases only in minor releases, to a stable Rust release at least about six months old. `cargo build` uses workspace default members, so select the CLI or Python binding explicitly when needed. Native prerequisites depend on the selected features and platform.

For the CLI, from this checkout:

```sh
cargo install --path crates/bacnet-cli --locked
# Use this variant when you need BACnet/SC:
cargo install --path crates/bacnet-cli --locked --features sc-tls
bacnet --version
bacnet --help
```

The executable is `bacnet`. IPv6 is enabled through CLI dependencies; `pcap` is a separate optional feature requiring native capture dependencies. The terminal UI (`tui`) is a default feature (`--no-default-features` leaves it out), and MS/TP needs `--features serial`. On `dev`, the MCP server (`bacnet mcp serve`) needs `--features mcp`, which includes the terminal UI. A CLI version string alone cannot distinguish a source build from the release.

A Rust project can depend on the `dev` branch instead of a release, for example `bacnet-client = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }`. Its `Cargo.lock` records the commit.

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

## Contribute to the project

Read the repository's [agent and contributor guidance](https://github.com/jscott3201/rusty-bacnet/blob/dev/AGENTS.md), [architecture map](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/README.md) and [CI and merge evidence](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/ci.md) before proposing a change. These links follow the development branch; release API references remain pinned to the release.

## Next steps

[Improve the documentation](/rusty-bacnet/development/documentation/) · [Application integration](/rusty-bacnet/integrate/overview/)
