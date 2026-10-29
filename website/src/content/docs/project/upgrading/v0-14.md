---
title: "Upgrade from v0.13.0 to v0.14.0"
description: "Review the breaking changes by area before replacing a working integration."
---

Do not treat this pre-1.0 update as a drop-in replacement merely because the package name stayed the same. v0.14.0 makes the terminal UI the CLI's default entry and removes the shell, puts every server authorization decision behind one policy, gives codec errors static labels, moves the network-layer vocabulary into a new `bacnet-npdu` crate, and changes Rust, Python and CLI APIs and some on-the-wire behavior. This page groups what changes for code and deployments built on v0.13.0.

:::note[This guide and the changelog]
The [0.14.0 section of the changelog](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/CHANGELOG.md) lists every change by area and issue, and its migration notes give the exact replacements. This guide groups what changes for code written against v0.13.0.
:::

## Before updating

Record your current package/binary version, enabled features, platform, transport, and important application workflows. List the scripts that drive the `bacnet` CLI: the shell is gone, and every command that changes a device now needs a flag. Keep a known-working deployment or environment available according to your operational policy.

## Update the selected interface

In a Python virtual environment:

```sh
python -m pip install --upgrade rusty-bacnet==0.14.0
```

For Rust, move the `bacnet-*` dependencies to `0.14.0` together. The exception is `bacnet-time`, which now versions on its own (#1768): this release ships it as `0.13.1`, and later releases may leave it there, so a direct dependency on it names `0.13.1`. Eighteen crates publish, one of them, [`bacnet-npdu`](#new-crate-bacnet-npdu), for the first time. For the CLI, [download the v0.14.0 executable](/rusty-bacnet/start/installation/#install-a-cli-executable), check it against `SHA256SUMS`, and confirm its version and features.

The published CLIs and Python wheels are built with whole-program LTO, so they are about a third smaller; the platforms, glibc floor and CPython versions are those of v0.13.0 (#973).

## Rust API

- **Errors.** `Error::Decoding` holds a `DecodeError` instead of `{ offset, kind, message }`; read the offset and kind with `Error::decode_fault()`. `Error::decoding`, `decoding_kind`, `invalid_tag`, `missing`, `trailing`, `out_of_range`, `overflow` and `tagged::misplaced_tag` take a `&'static str`; build a `DecodeError` for a label with numbers in it. Codec encoders refuse with `Error::Encode(EncodeError)` in place of `Error::Encoding`, and range checks (the APDU and NPDU encoders, `ObjectIdentifier::new`, `DeviceInstanceRange`, oversized lighting, color and elevator command members) with `Error::Range` in place of `Error::Encoding` or `Error::OutOfRange`. `Error::Segmentation` holds a `SegmentationError` enum. Some messages read differently, so don't match on error text (#1646, #1776, #1777, #1802, #1803).
- **Typed protocol errors.** `Error::Protocol`, `Error::Structured` and `TsmResponse::Error` hold `ErrorClass` and `ErrorCode` instead of `u32`s, and `Reject` and `Abort` hold `RejectReason` and `AbortReason` instead of `u8`s; `from_raw` and `to_raw` convert. A log record's `LogDatum::Failure` and `LogValue::Failure` hold the same typed pair (#915, #1814).
- **Engineering units.** The analog, accumulator, pulse converter and loop constructors and `StagingConfig::units` take `EngineeringUnits` instead of a number (#915).
- **Device identity.** One `DeviceIdentity`, now in `bacnet-objects` (still re-exported by `bacnet-endpoint`), holds what the server, the endpoints and the Device share. `ServerConfig.device` replaces `ServerConfig`'s `vendor_id`, `max_apdu_length`, `segmentation_supported` and `apdu_segment_timeout_ms`, and `DeviceConfig.identity` replaces `DeviceConfig`'s `instance`, `name`, `vendor_id`, `max_apdu_length`, `segmentation_supported` and `apdu_segment_timeout`. On the endpoint builders, `.device(...)` replaces `.identity(...)` and `.with_identity(...)`; an endpoint now needs an explicit Device, and a Device you supply must match the resolved identity. `DeviceConfig` also gains `backup_and_restore` and `backup_failure_timeout`: end a literal with `..DeviceConfig::default()` (#1720).
- **Exhaustive matches.** `AnyTransport` and `DataLinkSupport` are `#[non_exhaustive]`: add a `_` arm to a `match` on either (#940, #1725).
- **Network layer.** `NetworkLayer::start` returns an `ApduReceiver` instead of an `mpsc::Receiver` of `ReceivedApdu`. It has `recv`, `poll_recv`, `try_recv`, `close` and `is_closed`; `recv_many`, `blocking_recv`, `len`, `capacity` and wrapping it in `ReceiverStream` are gone. It decodes NPDUs as it is polled, so network controls flow only while you poll it. The transport's queue is the only buffer now (256 NPDUs on most links, 64 on MS/TP). A `ReceivedApdu` built by hand sets `router_response: None`, and the `*_on_issuance` callbacks must own their captures (`Send + 'static`) (#1699, #1713).
- **NPDU types.** `NetworkMessageType`, `RejectMessageReason`, `NetworkPriority`, `NpduLimits` and `MAX_MAC_LEN` come from `bacnet_npdu`. `NetworkPriority` is a closed enum (`Normal`, `Urgent`, `CriticalEquipment`, `LifeSafety`) instead of a `u8` newtype. `BACNET_PROTOCOL_VERSION` is `bacnet_npdu::VERSION` and `NpduAddressField` is `bacnet_npdu::AddressField`. The owned `bacnet_encoding::npdu::Npdu` drops its `is_network_message` field for the `is_network_message()` method, and its `message_type` is an optional `NetworkMessageType`. `NpduDecodeError` no longer converts from `bacnet_types::Error`. `RejectMessageToNetwork` and `decode_reject_message_to_network` leave `bacnet_encoding::npdu`: use `bacnet_npdu::RejectMessageToNetwork`, whose `decode` returns a `RejectLengthError` and whose `dnet` field is `network` (#1770, #1794).
- **BBMD helpers.** `read_bdt`, `read_fdt`, `write_bdt`, `delete_fdt_entry` and `register_foreign_device_bvlc` on `BipTransport` and `BACnetClient` return `Result<_, BvlcExchangeError>`: match `Nak(code)` instead of the `"BVLC-Result: …"` error text and `NoReply` instead of `Error::Timeout`. The last three return `Ok(())` on success, and a NAK is an `Err` (#1822).
- **Lighting outputs.** `write_property` refuses Present_Value and Value_Source on Binary Lighting Output and Lighting Output, and Lighting_Command on Lighting Output: call `write_property_from` with a `CommandOrigin`, or `BACnetServer::write_local` with a `LocalCommandSource` in a server (#1866).
- **Event Enrollment.** Event_Type follows Event_Parameters. The `event_type` argument of `EventEnrollmentObject::new` now only names the algorithm of legacy octet-string parameters, and an enrollment whose parameters are never set reads Event_Type NONE (#1868).
- The new `bacnet_types::map_hash`, `bacnet_transport::warn_throttled!` and the enumerations' `name()` need no change to existing code (#915, #1713, #1950).

## Python API

- Shed levels, Event_Parameters, authentication factors, Access_Event_Time, Last_Restore_Time, Subordinate_List and Event_Algorithm_Inhibit_Ref read as typed values instead of `application_data`: compare them by `.tag` and `.value`, and read the octets with the new `.octets` (#1499, #1888). See [typed constructed values](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md#typed-constructed-values).
- `add_event_enrollment(event_type=)` only names the algorithm of legacy octet-string parameters; set Event_Parameters to choose it (#1868).
- `CovNotificationIterator` counts notifications lost to lag in `missed` instead of printing to stderr, and ends with "COV notifications ended: the client stopped" instead of "channel closed" (#1858).
- An SC hub host that does not resolve raises `BacnetTransportError` reading "WebSocket host lookup for HOST failed" rather than a TCP dial failure, with errno ENOENT for a name with no addresses (#967).
- An SC node queues up to 256 received NPDUs, 32 from one peer (was 64 and 4), and drops a source's network-layer messages beyond 256 at once or 32 a second; `sc_npdu_drop_counts()` reports those drops in `control_rate_drops` (#518, #1712, #1718).
- The stack's log events reach Python's `logging` under `rusty_bacnet.<crate>` loggers, with a `NullHandler`, so nothing prints until the application configures logging (#1724).

The new keywords and methods need no change to existing code: `multicast=`, `Bbmd(nat_global_address=, nat_private_ranges=, alone_on_subnet=)`, `on_reinitialize` and `after_reinitialize`, `storage_path` on the logs and colour objects, the BBMD table helpers, `who_has_replies()`, `discover_routers()`, the SC `connection_events()` and `connection_status()`, and `add_bip6_network_port` (#940, #941, #1541, #1566, #1579, #1668, #1829, #1841, #1850).

## CLI and TUI

- **The TUI is the default.** Plain `bacnet` on a terminal opens the full-screen TUI, now a default feature and in the release binaries. Off a terminal, or with `--no-tui` or `BACNET_NO_TUI`, `bacnet` with no command prints its usage and exits with status 2. `--no-default-features` builds the CLI without the TUI (#974).
- **The shell is gone.** `bacnet shell`, its line editor and the `devices` command are removed. Open the TUI or call one one-shot command per line, with explicit targets and `--json` for scripts and screen readers; use `discover` (or `discover --json`) for the device list. `writem` is a one-shot command (#974).
- **Write guard.** `write`, `writem`, `create-object`, `delete-object`, `file-write`, `ack-alarm`, `time-sync` and `unregister` need `--allow-write`; `control` (`dcc`) and `reinit` need `--allow-write --yes` or a typed confirmation. A refused command sends nothing and exits with status 4. `subscribe` defaults to a 300-second lifetime, renewed while it runs and cancelled on exit; an indefinite one needs `--lifetime 0 --allow-write` (#972). See [the write guard](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/CLI.md#write-guard).
- **Routed stations.** `discover` and `subscribe` print a routed device's station as a target, `2:0x05`. In JSON, `discover` names it `station` (was `remote_mac`) and `subscribe` names it `routed_source` (was `source_address`); the `discover` table's `Remote MAC` column is `Station` (#1916).
- `register` and `unregister` print "… accepted" without the result code, in JSON mode too, and a NAK fails the command with exit status 1 (#1822).
- On BACnet/SC, an expired or not-yet-valid certificate, or a key that doesn't match it, is refused before the dial with "SC credential check refused the dial"; renew or fix the files. `bacnet sc-check` inspects the files offline (#968).
- `find` and `:find` send Who-Has as a global broadcast and print the objects that answer, and `find` also takes the name as a plain word; `whois-router` takes `--dnet` and `--wait` and lists the routers that answer (#960, #1872).
- `bacnet capture` filters on the `-p` port instead of always 47808 (#971).

New in the CLI: `--mstp PORT` joins an MS/TP trunk, in the Linux release binary (elsewhere build with `serial`); `bacnet mstp-ports` lists serial ports; `--bip-multicast` joins a B/IP multicast group; and `--sc-failover-url`, `--sc-reconnect` and `--sc-connect-timeout` set the SC hub link (#941, #969, #1831). The TUI keeps B/IP, BACnet/SC and MS/TP profiles, and adds a device browser, guarded write sessions, a watch list, and Network, SC, MS/TP and Capture screens; the website's Terminal UI section covers each one (#962–#971, #975).

## Wire behavior

- `BACnetClient::who_has` sends Who-Has as a global broadcast, as `who_is` does, so it reaches devices behind routers (#960).
- A non-router `NetworkLayer` drops a network message whose DNET names a single network instead of handing it to its control receiver, as it already did for APDUs (#1795).
- An Event Enrollment's Event_Type follows its Event_Parameters, and one whose parameters were never set serves the `none` alternative; any enrollment with a monitored reference reports and acknowledges its faults (#1868). Event Enrollment and Alert Enrollment serve Event_Message_Texts, Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1507, #1865).
- Binary Lighting Output and Lighting Output serve Value_Source, Value_Source_Array and Last_Command_Time (#1866). Color and Color Temperature outputs go to their default at server start, and objects run without a server read In_Progress NOT_CONTROLLED until started or written; a Value_Source COV subscription on them reports Present_Value changes too (#1526, #1582).
- A Trend Log can log by COV a property of its own device (#1480).
- A server or endpoint resolves its link's receive capacity before it builds a Device, so the Device declares what the link can take: 480 octets on a default MS/TP port (#1720).

## MS/TP

- `MstpExecutionMode::DedicatedThread` carries an `MstpThreadConfig`, and the enum is `Clone` but no longer `Copy`: write `MstpExecutionMode::dedicated_thread()` for the v0.13.0 behaviour. On Linux the thread can run under `SCHED_FIFO` and be pinned to CPUs; `start` fails if either can't be applied, and `execution_status()` reports the outcome (#501).
- `MstpCounters::deadline_misses` is `late_deadlines`, in `bacnet_mstp_core` and `bacnet_transport::mstp`, with a lateness histogram beside it. `bacnet_mstp_core::LATE_DEADLINE` is gone: set `MstpCoreConfig::late_deadline` (1 ms by default; the Tokio transport counts from 5 ms) (#1897).
- `bacnet-mstp-core`: build `MasterCore::new(config, &mut storage, now)` over a `StandardStorage` or `ExtendedStorage` (`from_parts` keeps the v0.13.0 arguments); `HeaplessQueues` is `FixedQueues`; the line inputs, `on_transmit_complete` and `on_transmit_aborted` return `()`; `CoreError` splits into `ConfigError`, `TransmitError` and `RequestError`. `Action::Transmit` carries `{ id, header, length }`: call `on_transmit_started(id, now)` right before driving the line and send the octets in its `Ok(frame)` (#1769).
- `mstp::monitor::MstpMonitor` listens to a trunk without transmitting, and `EndpointSession::mstp_diagnostics` and `MstpDiagnostics::node_state` report a station's counters and state; none of them changes existing code (#958, #969, #970).

## B/IP: NAT, multicast and BBMDs

- A `BbmdOptions` literal naming every field adds `nat: None` and `alone_on_subnet: false`, or ends with `..BbmdOptions::default()`. `BbmdState::local_address()` returns the socket's `SocketAddrV4` (was `([u8; 4], u16)`), even behind NAT; the new `own_address()` gives the BDT's own address. Build `bacnet_bip_core::bbmd::Tables` with `Tables::new` (#940, #1668).
- `bacnet_transport::port::BipPort` is `#[non_exhaustive]`: build it with `BipPort::new(endpoint, mode)` and add a multicast group view with `.with_multicast(...)` (#941).
- `BipPortMode::Foreign` holds a live `ForeignRegistration` instead of a fixed BBMD and lifetime: build it with `BipPortMode::fixed_foreign(bbmd, subscription_lifetime)` and read the values through the registration (#1825).
- On the wire: a registered BBMD's Network Port takes writes to its BDT and its foreign-registration setting, and a foreign device's to FD_BBMD_Address and FD_Subscription_Lifetime, applied by ReinitializeDevice ACTIVATE_CHANGES or WARMSTART when a handler is set; an activated BDT is saved to the persisted-BDT file (#1578, #1825, #1826). Every registered normal-mode port on an explicit interface lists BACnet_IP_Multicast_Address (#941). A Delete-FDT-Entry for a registration past its time-to-live and grace period gets a NAK (#1773).
- A port shared by address hands up broadcasts and unicast in the order the kernel received them on Linux, macOS and the BSDs (#1546).
- New, needing no change: a BBMD behind a NAT router at a configured global address (`BbmdOptions::nat`), a BBMD alone on its subnet (`alone_on_subnet`), and nodes and BBMDs on an IPv4 multicast group (`.multicast(...)`) (#940, #941, #1668).
- `bacnet-bip-core`: the port driver moves from `bacnet_router_core::bip` into `bacnet_bip_core::port` (`NodePort::new(node, held)`, `BbmdPort::new(link, tables, targets, rows)`), and `BbmdPort` runs the BBMD's `Link` instead of a `Node`, answering management requests itself through `Tables::serve`. `Ingress::Management` and `BbmdPort::on_deadline` are gone (#1766).

## BACnet/SC: security, ServerPolicy and principals

- **Server policy.** One `ServerPolicy` makes every authorization decision. The `mutation_authorizer`, `life_safety_operation_authorizer`, `audit_notification_authorizer` and `unconfirmed_audit_notification_authorizer` fields and setters, and the `LifeSafetyOperationAuthorizer`, `AuditNotificationAuthorizer` and `UnconfirmedAuditNotificationAuthorizer` aliases, are gone: implement `ServerPolicy` or chain `FnPolicy` closures. `.policy(..)` replaces an earlier policy, so combine yours into one. `MutationAuthorizer` is `mutation::DeviceWriteAuthorizer` (#524). See [Server authorization policy](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/mutation-policy.md).
- **Principals.** Each policy context, and a hardened router's control authorizer for unicast controls, carries the verified certificate principal (`ScPrincipal`) of a direct BACnet/SC peer; hub-relayed and non-SC traffic carries none (#518, #524).
- **Admission.** `ScNpduAdmissionPolicy`, `ScNpduDropCounts` and the new `ScControlRateLimit` are `#[non_exhaustive]`: start from `Default` or the builders. A node queues up to 256 received NPDUs, 32 from one peer (was 64 and 4; `with_npdu_per_origin_limit(4)` restores the old guard), and drops a source's network-layer messages beyond 256 at once or 32 a second; a site whose router fronts many networks raises that with `with_control_rate_limit()` (#518, #1712).
- **Dial timeouts.** A hub dial that runs out of time fails with `Error::Transport` carrying `ScConnectError::DialTimeout { timeout, stage }` instead of `Error::Timeout`; read it with `ScConnectError::from_io_error`. A missing Connect-Accept is still `Error::Timeout`, and Python raises `BacnetTimeoutError` for both (#1839).
- BACnet/SC needs Rustls 0.23.46 or later (#1737). Before a dial, the CLI and TUI lint this node's certificate, key and CA file, and hard failures stop the dial before any socket opens (#968).
- New, needing no change: hub-connector events and status (`connection_events()`, `build_with_events`), a failover hub URL, the TLS failure reason on connect errors, and offline credential inspection (#959, #967, #968, #1735, #1890).

## The router

- `BACnetRouter::start` and the core's `Router::new` refuse a port on network 0 or 0xFFFF: number every `RouterPort` and core `PortConfig` from 1 to 65534. `Router::new` returns `ConfigError::ReservedNetwork` (#1698).
- `bacnet-router-core`: `PortConfig` is built with `PortConfig::new(network, mac, limits, groups)` or the glue configurations instead of a literal; `Host::group` implementations move to a `GroupClassifier`, passed as the `G` parameter of `Storage` and `Router`, and the B/IP and MS/TP group helpers are gone (#1721). `router::Counters` holds `u32` counts since the last `Router::take_counters()`, `Options::route_cap` is an `Option<u16>`, and `bacnet_network::router::Counters` is its own `u64` type (#1767). `MstpPort::new(port, core)` owns its master core and refuses one without extended frames (#1769). `MessageType`, `RejectReason`, `Limits`, `Npdu::parse` and `write` become `NetworkMessageType`, `RejectMessageReason`, `NpduLimits`, `decode` and `encode_into`, `Limits::MIN` is `router::MIN_PORT_LIMIT`, and `message::Reject` is gone (#1770, #1794).
- To return a length rejection for a send refused after queueing, a core adapter keeps each send's `reject_origin` and passes it to `Router::refused_later`; a custom transport returns `Error::SendTooLong` only before it queues or writes any bytes (#1716).
- On the wire: queued traffic leaves in network-priority order; at the 256-route cap an Initialize-Routing-Table-Ack refreshes the networks the table holds; a BACnet/SC port's NPDU limit is followed through failover; router-local replies return through the port and peer they came from; and B/IP and MS/TP group forwarding works with the default host (#1696, #1697, #1698, #1699, #1721).
- New and off by default: Router-Busy flow control for a congested port, next-hop-only route rejects, duplicate global-broadcast suppression, and `BACnetRouter::originate` for the router's own requests (#1697, #1698, #1699).

## Server

- See [Device identity](#rust-api) for `ServerConfig.device` and [Server policy](#bacnetsc-security-serverpolicy-and-principals) for `ServerPolicy`.
- The Device gains setters for Object_List (through `DeviceAuthority`), System_Status and Database_Revision, and serves the backup and restore properties when `DeviceConfig::backup_and_restore` is set.
- A ReinitializeDevice handler can run its restart after the SimpleACK leaves (`ReinitializeContext::after_reply`), and an accepted request writes an Audit record (#1564, #1565).
- Trend Log, Trend Log Multiple and Event Log can keep themselves in storage across restarts (`with_persistence`, `FileLogPersistence`) (#1541).
- Requests cost less: one B/IP ReadProperty client gets about 36% more requests a second than on v0.13.0 on the same machine. Warnings a peer can set off once per packet or connection log at most once per 10 seconds with a count (#1713, #1724, #1950).

## New crate: bacnet-npdu

`bacnet-npdu` publishes for the first time. Like the other portable crates it is `no_std` and needs no allocator; it has no features. It holds the network-layer vocabulary once: the borrowed NPDU view and encoder, network message types, reject reasons, the network priority and NPDU limits. Every other crate speaks its types, and `bacnet-encoding` re-exports it as `bacnet_encoding::npdu::view` (#1770).

`bacnet-types`, `bacnet-encoding` and `bacnet-services` also build without an allocator now: Who-Is, I-Am and ReadProperty encode into caller storage and decode in place (#1614). `bacnet-time` 0.13.1 adds `earliest` and, behind a new `std` feature, `HostClock` (#1768).

## Verify more than import success

Exercise a known read, error handling, any routing or segmentation paths you depend on, subscription lifecycle, relevant server objects, and controlled cleanup. Rerun every script that drives the CLI: the shell is gone, writes need `--allow-write`, and routed stations print in a new form. Check any code that matched error text, since codec messages changed. On BACnet/SC, check that your certificates pass the CLI's pre-dial lint (`bacnet sc-check`) and that a router fronting many networks stays under the control-message rate limit.

Retest your application's output parsing where it depends on CLI JSON keys, typed Python values or Rust models. Separate regressions from corrected behavior that an older integration may have depended on accidentally. The [integration guides](/rusty-bacnet/integrate/overview/) explain shared endpoints, Network Port registration and SC setup.

## Sources and release scope

This guide covers the move from **v0.13.0** to **v0.14.0**. Source review is not hardware qualification.

[v0.14.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.14.0) · [Changelog](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/CHANGELOG.md) · [Rust API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/rust-api.md) · [Python API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md).
