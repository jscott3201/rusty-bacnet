---
title: "Choose your starting point"
description: "CLI, Python, or Rust: choose the path that matches your work."
---

Rusty BACnet is a protocol stack with three user-facing entry points, not a desktop building-management application. Use the CLI to inspect an authorized network, Python to automate a task, or Rust to embed BACnet in your own software.

## Start without hardware

The [local client/server lab](/rusty-bacnet/start/local-lab/) is the shortest route to a complete read without a controller. Install the Python package, run the supplied script, and check the result before configuring a real network.

## Inspect a device with the CLI

Start here when your goal is to identify a device, read a point, or collect diagnostics without writing an application. You can use a downloaded executable or build `bacnet-cli` with Cargo. The executable is named `bacnet`.

[Install the CLI](/rusty-bacnet/start/installation/#install-a-cli-executable) and follow [Your first read](/rusty-bacnet/start/first-read/). The first-read path does not change a property value.

## Automate with Python

Choose Python for scripts, data collection, integration prototypes, or a small test device. The distribution is `rusty-bacnet`; the Python import is `rusty_bacnet`. The binding uses asynchronous I/O.

[Read a property with Python](/rusty-bacnet/start/python/) shows a complete script and explains the network assumptions. Python 3.11 or later is the declared minimum; an installable wheel for your exact interpreter and platform is a separate question.

## Embed with Rust

Choose Rust when BACnet belongs inside your application. The workspace has separate crates for clients, servers, types, encoding, services, transports, networking, and objects. You do not need every crate for a property read.

[Start with the Rust client](/rusty-bacnet/start/rust/). The reviewed release declares Rust 1.93 as its minimum; this documentation project does not raise the library's minimum Rust version.

## Pick the network path separately

Language choice is not a transport support guarantee. BACnet/IP is the simplest starting point for the included read examples. BACnet/SC requires certificate and identity configuration. MS/TP requires a serial interface and deployment-specific hardware qualification.

[Compare transport paths](/rusty-bacnet/reference/transports/) before selecting hardware or assuming a CLI flag exists for a Rust transport.

## Before contacting a real network

Obtain the network owner's authorization, choose a bounded target, and identify which interface and ports you are using. Use a lab device when learning. Discovery transmits traffic; a subscription creates remote subscription state even though it does not write a point value.

Keep writes, alarm acknowledgments, device communication control, time synchronization, and file operations out of a first-read exercise. Those operations have separate operational consequences.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[Release](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.11.0) · [Workspace manifest](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/Cargo.toml) · [Python packaging](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/rusty-bacnet/pyproject.toml).
