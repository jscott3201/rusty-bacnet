---
title: "Troubleshoot by symptom"
description: "Start with the layer that failed, not with a larger scan or a longer timeout."
---

Capture the exact program version, command or API call, transport, host platform, and failure symptom. Keep a sanitized record of the local address/port, target address, object/property, and whether a smaller operation works.

:::caution[Check the version before applying a fix]
These guides describe **v0.14.0**. Commands and APIs from earlier releases differ in places, notably the CLI's interactive front end and write guard, server authorization policy and BACnet/SC admission. If you are moving an existing setup, read the [upgrade guide](/rusty-bacnet/project/upgrading/) rather than combining commands from different versions.
:::

## The executable or module does not start

Check installation before the network. Check that the executable matches the machine's operating system and processor; the Linux release executables need glibc 2.17 or newer and link libpcap in. A source build with the `pcap` feature links the system's libpcap instead, so a missing libpcap shared library can prevent even `--version` from starting; install the platform's libpcap runtime, or build without `pcap`. A Python wheel/import problem is a different path: use the correct interpreter and run the [native import check](/rusty-bacnet/start/installation/#install-the-python-package).

A local fixture can separate installation and protocol basics from network configuration. [Try the bounded loopback lab](/rusty-bacnet/start/local-lab/).

## Nothing is discovered

Try a directed Who-Is to one known device, then a read of a known property. Verify the local interface, intended broadcast destination, BACnet UDP port, remote port, routing, and firewall. Consider VPNs, containers, virtual machines, and subnet boundaries.

An empty discovery result does not prove that a building has no devices. A successful directed read does not prove that broadcasts can traverse the same path.

## A device instance works in the terminal UI but not in a script

The terminal UI can retain discovered devices in its session. A separate process does not inherit that session's cache. Use an explicit target address for a one-shot operation, or keep discovery and device-instance lookup within the same supported session.

Only the one-shot `read`, `readm`, `write`, `writem` and `subscribe` commands take a `DNET:instance` target; the other one-shot commands refuse it. A routed network requires more than finding a router's IP address.

## The socket is already in use

Identify other local BACnet tools and services using the intended address/port. Close or reconfigure the conflicting lab application deliberately. An ephemeral port can suit a directed-read example, but changing ports is not a universal discovery fix.

On a multi-interface machine, verify actual transport behavior rather than assuming the configured interface input guarantees receive isolation.

## Unknown object or property

Confirm the selected device identity and object instance. Read the name/type before expanding to bulk requests. A service can be implemented while a particular object property is absent or read-only. Treat a per-property RPM error as a per-property result, not a complete call success or total network outage.

## Small reads work; large reads time out

Reduce the request to one object/property. Investigate the peer's APDU limits, routing path, response size, and segmentation behavior. Do not make an unsupported peer work by endlessly increasing concurrency or retry counts. Record the smallest working and smallest failing requests.

## BACnet/SC connection fails

Check whether the binary includes SC support, then distinguish URL/connectivity, certificate trust, hostname/validity, client identity, hub acceptance, and BACnet target errors. The CLI trusts only the CA named by `--sc-ca`, and Python only `sc_ca_cert`; neither falls back to the system's root certificates. Follow the stage table in [BACnet/SC setup](/rusty-bacnet/development/bacnet-sc/#diagnose-the-stage-that-failed). Fix the trust deployment using approved procedures; do not disable verification.

## The serial port opens but MS/TP is unstable

Check duplicate MACs, baud, `Max_Master`, adapter/driver behavior, transmit-direction timing, and the physical trunk. For a USB adapter, check the transmit-latency pad and the usage and reply timeouts (`mstp_*_ms`); for one that hears its own frames, `mstp_echo`. Test representative host load and observe the wire. Opening a serial file and completing one read do not establish deterministic token timing or full Clause 9 behavior.

Use the [MS/TP guide](/rusty-bacnet/guides/mstp/) for the qualification boundary. A real-time kernel recommendation must be tied to evidence from the actual host and adapter, not merely the language used by the stack.

## A write succeeded but the present value did not change

Check commandability, priority arbitration, datatype, and the active higher-priority slots. Verify the physical process through the approved site procedure. Do not escalate to a stronger priority just to override an unexplained result.

## What to include in an issue

Describe the task, version/build features, platform and transport, minimal reproduction, expected result, actual result, and sanitized logs. For serial issues, include the adapter and driver as well as baud and direction-control mode. For SC issues, identify the failing stage without attaching private keys.

[Open a project issue](https://github.com/jscott3201/rusty-bacnet/issues/new). Use the repository's current security-reporting instructions for sensitive findings rather than publishing operational credentials or sensitive site data.

## Sources and release scope

These instructions target **v0.14.0**. Source review is not a claim of hardware qualification.

[CLI behavior](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/crates/bacnet-cli/src/main.rs) · [SC error and trust implementation](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/crates/bacnet-cli/src/transport.rs) · [Versioned API](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md) · [MS/TP timing work](https://github.com/jscott3201/rusty-bacnet/issues/502).
