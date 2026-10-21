---
title: "Support status and evidence"
description: "What Rusty BACnet supports, and how to read release, development and platform evidence at the scope actually tested."
---

Rusty BACnet is pre-1.0, with changing APIs and partial conformance coverage. It makes no BTL certification or full BACnet conformance claim.

## Choose the version and owner first

The [installation and local tutorials](/rusty-bacnet/start/installation/), the [integration guides](/rusty-bacnet/development/overview/) (shared endpoints, SC requirements and local Network Number controls) and the operating guides describe **v0.14.0**. A `dev` checkout may hold later changes while it still reports the release's version; use its commit to identify behavior.

Standalone client, full server, shared endpoint and language binding are different surfaces. The shared endpoint's bounded responder does not acquire the full server's service set. Use the [transport matrix](/rusty-bacnet/development/transports/) to choose a starting point, then follow its evidence links.

## What's supported

This summary describes v0.14.0; its [release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.14.0) list what changed. It lists what the code implements, not what has been certified or tried against other vendors' devices.

**Objects.** `bacnet-objects` implements 64 of the 65 standard object types; Network Security is the one it leaves out. That covers the analog, binary and multi-state inputs, outputs and values; the integer, positive integer, large analog, character string, octet string, bit string, date, time, date-time and pattern values; Device, Network Port, File, Program, Loop, Accumulator, Pulse Converter, Averaging, Calendar, Schedule, Timer, Command, Channel, Group, Global Group, Structured View and Staging; Trend Log, Trend Log Multiple, Event Log, Audit Log and Audit Reporter; Notification Class, Notification Forwarder, and Event and Alert Enrollment; Load Control and the lighting and color objects; and the life safety, access control, elevator group, escalator and lift objects. A server keeps them in an `ObjectDatabase`, which also holds your own `BACnetObject` implementations.

**Full server.** `bacnet-server` executes ReadProperty, ReadPropertyMultiple, WriteProperty, WritePropertyMultiple, ReadRange, CreateObject, DeleteObject, AddListElement, RemoveListElement, AtomicReadFile, AtomicWriteFile, SubscribeCOV, SubscribeCOVProperty, SubscribeCOVPropertyMultiple, AcknowledgeAlarm, GetAlarmSummary, GetEnrollmentSummary, GetEventInformation, LifeSafetyOperation, DeviceCommunicationControl, ReinitializeDevice, AuditLogQuery, ConfirmedEventNotification, ConfirmedAuditNotification and ConfirmedTextMessage, and rejects any other confirmed service as unrecognized. It answers Who-Is and Who-Has, learns peers from I-Am, and accepts TimeSynchronization, UTCTimeSynchronization, WriteGroup and the unconfirmed event, audit and text-message notifications. It sends COV, event and audit notifications. One `ServerPolicy` makes every server authorization decision, and on BACnet/SC a policy sees the verified certificate principal of a direct peer (#524, #518).

**Client.** `bacnet-client` sends ReadProperty, ReadPropertyMultiple, WriteProperty, WritePropertyMultiple, ReadRange, CreateObject, DeleteObject, AddListElement, RemoveListElement, AtomicReadFile, AtomicWriteFile, SubscribeCOV, SubscribeCOVProperty, AcknowledgeAlarm, GetEventInformation, AuditLogQuery, audit notifications, DeviceCommunicationControl, ReinitializeDevice, TimeSynchronization, UTCTimeSynchronization, Who-Is, Who-Has and WriteGroup. It segments large requests and responses, renews COV subscriptions, delivers incoming COV and event notifications as streams, and manages BBMD tables and foreign-device registration. It also sends Who-Is-Router-To-Network and What-Is-Network-Number, lists the routers that answer, and collects the I-Have answers to a Who-Has, which goes out as a global broadcast so it reaches devices behind routers (#960). `confirmed_request` and `unconfirmed_request` send any other service from encoded bytes.

**Transports.** BACnet/IP is always built; the others are Cargo features of `bacnet-transport`:

- BACnet/IP: BBMD and foreign-device registration, and a BBMD behind a NAT router at a configured global address (Annex J.7.5), and B/IP multicast nodes (Annex J.8) on an explicit interface, with a BBMD that serves the group (J.8.3).
- BACnet/IPv6 (`ipv6`): selects one concrete local interface and address and keeps traffic on that link; the socket itself binds the wildcard address. Peers are named by their 3-octet VMAC, found by address resolution, and a server's VMAC is its device instance. A server registers a B/IPv6 Network Port as it does for B/IP (#1579).
- BACnet/SC (`sc-tls`): nodes, direct connections and a hub over TLS 1.3. Each device needs a site CA, its own certificate and key, and a provisioned device UUID.
- MS/TP (`serial`): standard frames, and extended (COBS-encoded) frames for NPDUs up to 1497 octets with `MstpConfig::extended_frames`, which a router turns on for its MS/TP ports. RS-485 kernel options and GPIO direction control (`serial-gpio`) are Linux-only, and on-wire timing is not qualified on any adapter. On Linux the transport's dedicated thread can run under `SCHED_FIFO`, pinned to chosen CPUs (#501), and a receive-only monitor maps a trunk's stations without transmitting (#958).
- Ethernet (`ethernet`): Linux only, through `AF_PACKET`, with `CAP_NET_RAW` or root.

Every transport but Ethernet has a bounded frame tap that shows the link-layer frames it sends and receives (#957). `bacnet-network` routes between networks and transports, and `bacnet-endpoint` lets one transport serve both client and server roles.

**Embedded (`no_std`).** `bacnet-time`, `bacnet-npdu`, `bacnet-mstp-codec`, `bacnet-mstp-core`, `bacnet-bip-codec`, `bacnet-bip-core` and `bacnet-router-core` build without the standard library or an allocator and take their time from the caller, so firmware can run MS/TP, BACnet/IP (BBMD included) and a router between them. The MS/TP and BACnet/IP transports and `BACnetRouter` run on these cores. `bacnet-types`, `bacnet-encoding` and `bacnet-services` build without the standard library when given an allocator.

**Python.** The `rusty-bacnet` package (CPython 3.11 or later) wraps the client as `BACnetClient`, the full server as `BACnetServer`, the SC hub as `ScHub` and the shared endpoints as `BipEndpoint`, `ScEndpoint` and `MstpEndpoint`, with asyncio methods. It includes BACnet/IP, BACnet/IPv6, BACnet/SC and MS/TP, but not Ethernet, and it does not wrap every Rust API.

**CLI and terminal UI.** The `bacnet` executable runs one-shot commands for scripts and, on a terminal, a full-screen terminal UI with saved B/IP, BACnet/SC and MS/TP profiles, a device browser, a watch list, guarded write sessions and network diagnostics. One-shot commands that change a device need `--allow-write`. The Linux release executables add MS/TP and packet capture.

**PICS for your device.** A running server can describe itself: `BACnetServer::generate_pics` builds a PICS (`bacnet_server::pics::Pics`, which renders as text or Markdown) from the objects actually in its database and the services the full server executes. `bacnet_server::pics::generate_pics` does the same for a database alone, taking the services from its Device object. Either one describes that configuration; it is not a conformance test result or a BTL listing.

## Understand what a result proves

| Result | Useful evidence | Still separate |
|---|---|---|
| Build or cross-compile passes | Selected code compiles on that target | Runtime behavior and native-library availability |
| Unit or simulated transport tests pass | Exercised state transitions and failure cases | Actual sockets, frames or physical timing |
| Loopback or isolated wire tests pass | Captured framing and the tested owner/lifecycle | Building-network interoperability and every platform |
| Installed Python tests pass | The tested interpreter and native artifact work together | Other wheels, interpreters and free-threaded builds |
| A generated PICS lists an object or service | What that configured server declares | Complete object/profile conformance or BTL listing |

A screenshot, test count or single-device demonstration does not establish all-device interoperability or fitness for an operating building. No green badge replaces the stated limits. External ignored fixtures, hardware tests and ordinary CI have different execution requirements.

## Report a problem

Start with [troubleshooting](/rusty-bacnet/help/troubleshooting/) or the relevant integration guide. Open a [GitHub issue](https://github.com/jscott3201/rusty-bacnet/issues/new) with revision/artifact, transport, feature set, platform, minimal reproduction and sanitized evidence. Include the exact conflicting documentation link.

Never attach private keys, customer identifiers or unredacted operational captures. Use the project's security-reporting guidance for sensitive findings.
