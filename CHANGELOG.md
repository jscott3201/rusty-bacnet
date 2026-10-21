# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Entries for the next release are fragment files in [`changelog.d/`](changelog.d/), assembled here when a version is released. Add one per change instead of editing this file: [`changelog.d/README.md`](changelog.d/README.md) has the format, and `python3 scripts/changelog.py preview` shows the section they make.

## [0.14.0] - 2026-10-21

### Release highlights

This pre-1.0 release ships the terminal UI and a performance and security
pass. It breaks Rust, Python and CLI APIs and changes some on-the-wire
behaviour, most visibly what the CLI does with no command, which commands
may write, and how a BACnet/SC node admits traffic. Before moving a v0.13.0
integration, read the
[upgrade guide](https://jscott3201.github.io/rusty-bacnet/project/upgrading/),
which groups what changes by area; the migration notes below give the exact
replacements. Conformance remains partial, and this release is not a
full-conformance or BTL certification claim.

- **The TUI is the CLI.** Plain `bacnet` on a terminal opens a full-screen
  terminal UI with saved B/IP, BACnet/SC and MS/TP profiles, a device
  browser, a watch list, guarded write sessions with a revert ledger, and
  Network, SC, MS/TP and Capture diagnostics with pcap export. The shell is
  gone, and one-shot commands that change a device need `--allow-write`
  (and `--yes` for DCC and reinitialize).
- **Faster requests.** A B/IP ReadProperty round trip costs less at every
  step from transport to handler: the last cut alone took 13% of the
  instructions (47,610 to 41,454) and half the bytes allocated, and one
  client gets about 36% more requests a second than on 0.13.0, with a third
  lower median latency, on the same machine.
- **Security.** One `ServerPolicy` makes every server authorization
  decision, and policies and a hardened router's control authorizer see the
  verified certificate principal of a direct BACnet/SC peer (#518, #524). SC
  nodes rate-limit network-layer messages per source, the CLI and TUI lint
  their BACnet/SC credentials before dialing, and `bacnet sc-check` inspects
  them offline.
- **NAT and multicast.** A BBMD can run behind a NAT router at a configured
  global address, and B/IP nodes and BBMDs can use an IPv4 multicast group,
  from Rust, Python and the CLI, with the matching Network Port rows (#940,
  #941).
- **MS/TP real time.** On Linux the MS/TP transport's dedicated thread can
  run under `SCHED_FIFO` pinned to chosen CPUs, timer lateness is kept in a
  histogram, and a receive-only monitor maps a trunk without transmitting
  (#501, #958, #1897).
- **Quieter logs.** Warnings a peer can set off once per packet or
  connection log at most once per 10 seconds with a count, and Python
  applications receive the stack's logs through `logging` (#1724, #1950).
- A new `no_std` crate, `bacnet-npdu`, holds the network-layer vocabulary
  that every crate now shares, and the codec crates build without an
  allocator, with errors that format no text until displayed.

### Migration notes

- **MS/TP execution mode (Rust API, #501):** `MstpExecutionMode::DedicatedThread`
  now carries an `MstpThreadConfig`, and the enum is `Clone` but no longer
  `Copy`. Write `MstpExecutionMode::dedicated_thread()` for the old behaviour,
  and clone a mode you use more than once. ([7260f31](https://github.com/jscott3201/rusty-bacnet/commit/7260f313ea013749cdcb7be40062d8ef3c83dc24))

- **SC admission (Rust API, #518):** `ScNpduAdmissionPolicy`, `ScNpduDropCounts`
  and the new `ScControlRateLimit` are `#[non_exhaustive]`; construct them via
  `Default` or the builders. An SC node now drops a source's network-layer
  messages beyond 256 at once or 32 a second; a site with a router fronting many
  networks raises it with `with_control_rate_limit()`. ([0476eb5](https://github.com/jscott3201/rusty-bacnet/commit/0476eb52a86d5b4e6e73618ce88c9b73cd605f23))

- **Server policy (Rust API, #524):** the four `*_authorizer` fields and setters,
  and the `LifeSafetyOperationAuthorizer`, `AuditNotificationAuthorizer` and
  `UnconfirmedAuditNotificationAuthorizer` aliases, are gone: implement
  `ServerPolicy` or chain `FnPolicy` closures. `.policy(..)` replaces any earlier
  policy, so merge them into one. `MutationAuthorizer` is now
  `mutation::DeviceWriteAuthorizer`. See
  [Server authorization policy](docs/mutation-policy.md). ([546f017](https://github.com/jscott3201/rusty-bacnet/commit/546f017e703382e00f2e7ec1d7b7597d591d4503))

- **Typed errors and units (Rust API, #915):** `Error::Protocol`,
  `Error::Structured` and `TsmResponse::Error` hold `ErrorClass`/`ErrorCode`,
  and `Error::protocol` takes them; `Reject`/`Abort` hold `RejectReason`/
  `AbortReason`. Match on the constants, or use `from_raw`/`to_raw`. Pass
  `EngineeringUnits` to the analog, accumulator, pulse converter and loop
  constructors and to `StagingConfig::units`. ([f729a45](https://github.com/jscott3201/rusty-bacnet/commit/f729a457dc9afc433d2b857ee2d6dd1284fe0de4))

- **BBMD options and NAT (Rust API, #940, #1668):** a `BbmdOptions` literal
  naming every field adds `nat: None` and `alone_on_subnet: false`, or ends
  with `..BbmdOptions::default()`; a `match` on `DataLinkSupport` adds a
  wildcard arm; build `Tables` with `Tables::new`;
  `BbmdState::local_address()` returns a `SocketAddrV4` (was
  `([u8; 4], u16)`), and the new `own_address()` gives the BDT's own address. ([7b81a3f](https://github.com/jscott3201/rusty-bacnet/commit/7b81a3f167931793827f40a337c9abe7ccc605d9))

- **B/IP multicast (Rust API, #941):** build a `BipPort` with
  `BipPort::new(endpoint, mode)` in place of a struct literal, and add a
  group view with `.with_multicast(...)`. ([305283f](https://github.com/jscott3201/rusty-bacnet/commit/305283f7b6a8aa7d896ef7243ba0ddb7c4f1c3e9))

- **BACnet/SC CLI (#968):** an expired or not-yet-valid operational
  certificate, or a key that does not match it, is now refused before the dial
  with a "SC credential check refused the dial" error instead of failing at the
  hub; renew or fix the files. In the TUI, a profile with check warnings
  connects only after `y` accepts them. See
  [Before a dial](docs/sc-check.md#before-a-dial). ([9f18cc3](https://github.com/jscott3201/rusty-bacnet/commit/9f18cc3b39cd5365183fa9f332ce2e6ad1e30061))

- **CLI write guard (#972):** add `--allow-write` to scripts that run `write`,
  `create-object`, `delete-object`, `file-write`, `ack-alarm`, `time-sync` or
  `unregister`, and `--allow-write --yes` for `control` (`dcc`) or `reinit`.
  A refusal exits with status 4. An indefinite `subscribe` needs
  `--lifetime 0 --allow-write`. See [the write guard](docs/CLI.md#write-guard). ([f0b530d](https://github.com/jscott3201/rusty-bacnet/commit/f0b530d4f7a18008d3227c032ec8d6d499f68abe))

- **CLI (#974):** `bacnet shell` is gone: open the TUI (`bacnet` or `bacnet
  tui`) or use the one-shot commands, with `--json` for scripts and screen
  readers. A script that piped commands into the shell calls one one-shot per
  command instead (default targets become explicit). For `devices`, use
  `discover` (or `discover --json`) or the TUI's device list.
  `--no-default-features` builds the CLI without the TUI. ([b98aac9](https://github.com/jscott3201/rusty-bacnet/commit/b98aac93d39b1313ff5492a91f29f7335293be41))

- **Typed single reads (Python API, #1499, #1888):** compare these reads by
  `.tag` and `.value` (a shed level is `{"kind", "value"}`, a factor
  `(format_type, format_class, value)`, Event_Algorithm_Inhibit_Ref an
  `object_property_reference` mapping of `object_identifier`,
  `property_identifier` and `property_array_index`); the new `.octets` gives
  the octets. See [typed constructed values](docs/python-api.md#typed-constructed-values). ([09c332c](https://github.com/jscott3201/rusty-bacnet/commit/09c332ce7fe817c0cadf44148dfb10f06eb3118c))

- **Decoding errors (Rust API, #1646, #1776):** match
  `Error::Decoding(DecodeError)`, not the `{ offset, kind, message }`
  variant; `Error::decode_fault()` reads its offset and kind. `decoding`,
  `decoding_kind`, `invalid_tag`, `missing`, `trailing`, `out_of_range`,
  `overflow` and `tagged::misplaced_tag` take a `&'static str`; for numbers
  or context, build a `DecodeError`. `decode_ctx_fixed` takes a `Fixed`, and
  the WPM cursor error holds `error: DecodeError`, not `message`. ([fca2389](https://github.com/jscott3201/rusty-bacnet/commit/fca238953b304bc79ffb0bec231939acd7eec465))

- **Router port numbers (Rust API, #1698):** give every `RouterPort` and
  core `PortConfig` a network number from 1 to 65534. For 0 or 0xFFFF,
  `BACnetRouter::start` now fails before starting any transport, and
  `Router::new` returns the new `ConfigError::ReservedNetwork`. ([71f238d](https://github.com/jscott3201/rusty-bacnet/commit/71f238da8f30878b07abdbaf36116c6c7564fcbd))

- **Network layer replies (Rust API, #1699):** a `ReceivedApdu` built by hand
  sets the new `router_response: None` (a router fills it in only for a
  Confirmed-Request it delivers locally); keep `response_route()` for router
  deliveries. The `*_on_issuance` callbacks must own their captures
  (`Send + 'static`), since a router queue may hold them until the send. ([b24829e](https://github.com/jscott3201/rusty-bacnet/commit/b24829eeea1c014465279febb9a90d71f3286cab))

- **NetworkLayer buffering (#1713):** the raw receiver has no 256-APDU queue
  of its own, so the transport's queue is the only buffer: 256 NPDUs on most
  links, 64 on MS/TP (before, 64 + 256). Read overload drops from the
  transport's counters. ([ed71a48](https://github.com/jscott3201/rusty-bacnet/commit/ed71a48addbc185cf68c3a02ce10b3bd18848ea5))

- **NetworkLayer (Rust API, #1713):** `NetworkLayer::start` returns
  `bacnet_network::layer::ApduReceiver` (`recv`, `poll_recv`, `try_recv`,
  `close`, `is_closed`) instead of `mpsc::Receiver<ReceivedApdu>`: `recv_many`,
  `poll_recv_many`, `blocking_recv`, `len`, `is_empty`, `capacity` and
  wrapping it in `ReceiverStream` are gone. Keep polling it while you need
  network controls, or use `start_with_admission`; `stop` ends its stream. ([ed71a48](https://github.com/jscott3201/rusty-bacnet/commit/ed71a48addbc185cf68c3a02ce10b3bd18848ea5))

- **Late send refusals (Rust API, #1716):** to return a length rejection for
  a send refused after queueing, a router-core adapter keeps each send's
  `reject_origin`, passes it to `Router::refused_later` and drains the
  resulting actions. A custom transport returns the new `Error::SendTooLong`
  for a length refusal only before it queues or writes any bytes. ([a47b0ae](https://github.com/jscott3201/rusty-bacnet/commit/a47b0aed2157e639d0083348e74ed427b8282f5b))

- **Device identity (#1720):** Use `DeviceIdentity`, `ServerConfig.device`,
  `DeviceConfig.identity` and `.device(...)` instead of separate identity fields
  or endpoint `.identity(...)`/`.with_identity(...)`. Without a database, full
  servers generate a Device from their configured/default identity; endpoints
  require explicit `.device(...)`. Supplied Devices must match the resolved
  identity. See [migration](docs/rust-api.md#local-receive-capacity-and-outgoing-limits). ([b146ed1](https://github.com/jscott3201/rusty-bacnet/commit/b146ed1e637e3975d891423547be2226e46713ac))

- **Router groups (Rust API, #1721):** use glue port configurations or
  `PortConfig::new(network, mac, limits, groups)` instead of struct literals.
  Move `Host::group` implementations to a `GroupClassifier` and pass it as
  the `G` parameter of `Storage` and `Router`, after `N`. The B/IP and MS/TP
  group helper functions and methods are removed; call
  `config.groups.classify(mac)`. ([5ce7592](https://github.com/jscott3201/rusty-bacnet/commit/5ce759270441155b6e784e2626419eb3503365bf))

- **AnyTransport (Rust API, #1725):** Add a `_` arm to matches outside
  `bacnet-transport`. Keep arms for the transports the caller supports and choose
  an appropriate fallback for other transports. Existing variants remain
  directly constructible. ([7e3450c](https://github.com/jscott3201/rusty-bacnet/commit/7e3450c9b04c2b9f70b5acbb836ca140c0ac2128))

- **BBMD port (Rust API, #1766):** `BbmdPort` runs the BBMD's `Link`, not a
  `Node`, so a BVLC-Result or table ACK reaching it is `Ingress::Handled`
  and no longer counts in `NodeCounters::unmatched_responses`; a BBMD that
  runs exchanges of its own drives a `Node` beside the port. ([d930e28](https://github.com/jscott3201/rusty-bacnet/commit/d930e28ee895bb5f7f98fa9e1a2e50004ad6fd98))

- **B/IP port driver (Rust API, #1766):** use `bacnet_bip_core::port`:
  `NodePort::new(node, held)`, `BbmdPort::new(link, tables, targets, rows)`,
  with `HeldBroadcast`, `Datagrams` (refusing with `SendRefusal::SocketFull`
  or `SocketError`), `Ingress` and `PortError` there; route through
  `bacnet_router_core::bip::BipPort` (`config`, `arrived`, `send`).
  `Ingress::Management` and `BbmdPort::on_deadline` are gone; management
  requests are `ForBbmd::Management(Request)`. ([d930e28](https://github.com/jscott3201/rusty-bacnet/commit/d930e28ee895bb5f7f98fa9e1a2e50004ad6fd98))

- **Router core (Rust API, #1767):** `router::Counters` fields are `u32`
  counts since the last `Router::take_counters()`; add them to your own
  totals. `Options::route_cap` is an `Option<u16>` (`None` fills the table),
  and `Options::default()` is `STANDARD`. `bacnet_network::router::Counters`
  is now its own `u64` type, not a re-export. ([d930e28](https://github.com/jscott3201/rusty-bacnet/commit/d930e28ee895bb5f7f98fa9e1a2e50004ad6fd98))

- **bacnet-time versioning (#1768):** a direct dependency on `bacnet-time`
  names that crate's own version, 0.13.1 in this release, not the release's:
  later releases may leave it where it is. The other crates already name the
  one they need. ([9a1ee25](https://github.com/jscott3201/rusty-bacnet/commit/9a1ee25c2be326718adb04da3d791adcc2e0fc4e))

- **MS/TP core (Rust API, #1769):** build the core with
  `MasterCore::new(config, &mut storage, now)` over a `StandardStorage` or
  `ExtendedStorage` (`from_parts` keeps the old arguments); rename
  `HeaplessQueues` to `FixedQueues`; the `on_*` line inputs,
  `on_transmit_complete` and `on_transmit_aborted` return `()`, so drop
  `?`/`unwrap` and watch `adapter_errors`; match `ConfigError`,
  `TransmitError` or `RequestError` where you matched `CoreError`. ([561a5a4](https://github.com/jscott3201/rusty-bacnet/commit/561a5a46aaf5dcaafaaac1c94bc7b072b0073859))

- **MS/TP transmit (Rust API, #1769):** match
  `Action::Transmit { id, header, length }`; right before enabling the line
  driver call `on_transmit_started(id, now)`, send the octets in its
  `Ok(frame)`, and send nothing on `Err` (a deferred frame comes back as a
  new `Transmit` action). Stop copying the frame at the poll. ([561a5a4](https://github.com/jscott3201/rusty-bacnet/commit/561a5a46aaf5dcaafaaac1c94bc7b072b0073859))

- **Router MS/TP port (Rust API, #1769):** build it with
  `MstpPort::new(port, core)`, take its `PortConfig` from its `port_config`
  method (`MstpPort::config` is now the free `mstp::port_config`), poll
  `poll_action()` (NPDUs arrive as `PortAction::Arrived`), call `release`
  and `send` on the port, and reach the core through `core_mut()`. ([561a5a4](https://github.com/jscott3201/rusty-bacnet/commit/561a5a46aaf5dcaafaaac1c94bc7b072b0073859))

- **NPDU decode errors (Rust API, #1770):** `NpduDecodeError` no longer
  converts from `bacnet_types::Error` or reports a `source()`; its
  `Malformed` case becomes `Error::Decoding`, and an empty or one-octet
  buffer is `Decoding` with kind `Missing` where it was `BufferTooShort`. ([62a5baf](https://github.com/jscott3201/rusty-bacnet/commit/62a5baf1dbe0fa014b1bc9948d76a86b4a9d9807))

- **More network-layer renames (Rust API, #1770):** `BACNET_PROTOCOL_VERSION`
  is `bacnet_npdu::VERSION`, `NpduAddressField` is
  `bacnet_npdu::AddressField`, router-core's `Limits::MIN` is
  `router::MIN_PORT_LIMIT`, and `bacnet_mstp_core::{NetworkPriority,
  PriorityError}` are under `bacnet_mstp_core::npdu`. bacnet-encoding
  re-exports the crate as `bacnet_encoding::npdu::view`. ([62a5baf](https://github.com/jscott3201/rusty-bacnet/commit/62a5baf1dbe0fa014b1bc9948d76a86b4a9d9807))

- **Network-layer types (Rust API, #1770):** import `NetworkMessageType`,
  `RejectMessageReason`, `NetworkPriority` (now `Normal`, `Urgent`,
  `CriticalEquipment`, `LifeSafety`), `NpduLimits` and `MAX_MAC_LEN` from
  `bacnet_npdu`. Router core: `MessageType`, `RejectReason`, `Limits`,
  `Npdu::parse` and `write` become `NetworkMessageType`,
  `RejectMessageReason`, `NpduLimits`, `decode` and `encode_into`. ([62a5baf](https://github.com/jscott3201/rusty-bacnet/commit/62a5baf1dbe0fa014b1bc9948d76a86b4a9d9807))

- **Owned NPDU (Rust API, #1770):** `bacnet_encoding::npdu::Npdu` drops
  `is_network_message` (call `is_network_message()`), its `message_type` is
  an `Option<NetworkMessageType>` and `NpduDecodeError::Malformed` carries a
  `bacnet_npdu::Malformed`; `ControlAuthContext::message_type` is a
  `NetworkMessageType`, and `router_table` no longer re-exports `Mac` or
  `RejectReason`. ([62a5baf](https://github.com/jscott3201/rusty-bacnet/commit/62a5baf1dbe0fa014b1bc9948d76a86b4a9d9807))

- **Reject payload (Rust API, #1794):** `RejectMessageToNetwork` and
  `decode_reject_message_to_network` leave `bacnet_encoding::npdu`, and
  router-core's `message::Reject` is gone: use
  `bacnet_npdu::RejectMessageToNetwork`, whose `decode` (was `parse`) returns
  a `RejectLengthError` and whose `dnet` field is now `network`. ([d89ce35](https://github.com/jscott3201/rusty-bacnet/commit/d89ce350037e2bae3ef3fd07add35a9135ec3748))

- **Encoding and range errors (Rust API, #1646, #1776, #1777, #1802):** match
  `Error::Encode(EncodeError)`, not `Error::Encoding`, for codec encoder
  refusals, and `Error::Range`, not `Error::Encoding` or `Error::OutOfRange`,
  for range checks: the APDU and NPDU encoders and validators,
  `ObjectIdentifier::new`, `DeviceInstanceRange`, an over-long NPDU address
  and oversized lighting, color and elevator command members.
  `Error::Segmentation` holds a `SegmentationError` enum. ([28262cb](https://github.com/jscott3201/rusty-bacnet/commit/28262cb7745816848f9efa447eb61c63ce3f06fe))

- **Log record failures (Rust API, #1814):** `LogDatum::Failure` and
  `LogValue::Failure` hold `error_class: ErrorClass` and
  `error_code: ErrorCode` instead of `u32`s. Build them from the constants
  (`ErrorClass::PROPERTY`) or `from_raw`, and read the numbers with
  `to_raw`. Decoding a record whose failure class or code is wider than 16
  bits now fails instead of keeping the value. ([a25f68d](https://github.com/jscott3201/rusty-bacnet/commit/a25f68dc99e14845174cc87708c41832cfc0553c))

- **BVLC management errors (Rust API, #1822):** `read_bdt`, `read_fdt`,
  `write_bdt`, `delete_fdt_entry` and `register_foreign_device_bvlc` return
  `Result<_, BvlcExchangeError>`. Match `Nak(code)` (or call `nak()`) instead
  of the `"BVLC-Result: …"` encoding-error text, and `NoReply` instead of
  `Error::Timeout`; other errors arrive as `Transport(Error)`. The last three
  return `Ok(())` on success instead of a result code, and a NAK as `Err`. ([0af6ec6](https://github.com/jscott3201/rusty-bacnet/commit/0af6ec681d2d6fc5de8b34b4a644af08559be116))

- **Network Port foreign mode (Rust API, #1825):** build
  `BipPortMode::Foreign { bbmd, subscription_lifetime }` as
  `BipPortMode::fixed_foreign(bbmd, subscription_lifetime)`, or lend your own
  `ForeignRegistration`; read the values with `registration.bbmd()` and
  `registration.subscription_lifetime()`. ([06a380c](https://github.com/jscott3201/rusty-bacnet/commit/06a380cdaf2faa524d4d4dd169c3715b2193df86))

- **SC dial timeouts (Rust API, #1839):** a hub dial that times out is now
  `Error::Transport` carrying `ScConnectError::DialTimeout { timeout, stage }`
  (read it with `ScConnectError::from_io_error`), not `Error::Timeout`; a
  missing Connect-Accept is still `Error::Timeout`. Python still raises
  `BacnetTimeoutError` for both. ([2135122](https://github.com/jscott3201/rusty-bacnet/commit/21351229af03c18476ec4cc7435043e5bb3b0709))

- **Lighting outputs (Rust API, #1866):** `write_property` refuses
  Present_Value and Value_Source writes on Lighting Output and Binary
  Lighting Output, and Lighting_Command writes on Lighting Output. Call
  `write_property_from` with a `CommandOrigin`, as for the other commandable
  objects; in a server, `BACnetServer::write_local` with a
  `LocalCommandSource`. `LightingOutputObject::set_lighting_command` still
  takes an application command, with source NONE. ([12a6109](https://github.com/jscott3201/rusty-bacnet/commit/12a610922ac914143630259eb61343e3e18c6d82))

- **Event Enrollment Event_Type (#1868):** set Event_Parameters to choose the
  algorithm; Event_Type follows them. The `event_type` argument of
  `EventEnrollmentObject::new` and Python's `add_event_enrollment(event_type=)`
  now only name the algorithm of legacy octet-string parameters. An
  enrollment whose parameters are never set reads Event_Type NONE. ([12a6109](https://github.com/jscott3201/rusty-bacnet/commit/12a610922ac914143630259eb61343e3e18c6d82))

- **MS/TP counters (Rust API, #1897):** read `MstpCounters::late_deadlines`
  where you read `deadline_misses`, in `bacnet_mstp_core` and
  `bacnet_transport::mstp`. `bacnet_mstp_core::LATE_DEADLINE` is gone: set
  `MstpCoreConfig::late_deadline` instead (`DEFAULT_LATE_DEADLINE`, 1 ms). The
  Tokio transport counts from 5 ms (`HOST_LATE_DEADLINE`). ([07ca68e](https://github.com/jscott3201/rusty-bacnet/commit/07ca68e8657471fbdb3b70103a6fa15d2e588fb3))

- **CLI output (#1916):** a script reading `discover --json` takes the routed
  station from `station` (was `remote_mac`), and one reading `subscribe --json`
  takes it from `routed_source` (was `source_address`). Both hold `NET:0xHEX`,
  network included. The `discover` (`whois`) table's `Remote MAC` column is
  now `Station`. ([d6c84ee](https://github.com/jscott3201/rusty-bacnet/commit/d6c84ee6ed992eb4d7cd759612bbe507640fe517))

- **DeviceConfig (Rust API):** `DeviceConfig` has two new fields, `backup_and_restore` and
  `backup_failure_timeout`. Code that builds a `DeviceConfig` with a struct
  literal adds `..DeviceConfig::default()`. ([488f7ec](https://github.com/jscott3201/rusty-bacnet/commit/488f7ecf41123bf8f1c30b24a368199d904c1dbc))

### Security

- **Dependencies:** `Cargo.lock` takes `rustls-webpki` 0.103.16, which fixes
  RUSTSEC-2026-0382: certificate verification could accept a distrusted
  intermediate. It affects BACnet/SC TLS in the release binaries and wheels.
  Library users should run `cargo update -p rustls-webpki` (#1987).

- **Breaking (Rust API):** A hardened router's control authorizer sees the
  sending direct BACnet/SC peer's certificate principal for unicast controls (SC
  broadcasts arrive hub-relayed, with none), and SC nodes rate-limit
  network-layer messages per certificate or VMAC (#518). ([0476eb5](https://github.com/jscott3201/rusty-bacnet/commit/0476eb52a86d5b4e6e73618ce88c9b73cd605f23))

- **Breaking (Rust API):** One `ServerPolicy` now makes the server's
  authorization decisions, with documented defaults, and each context carries
  the verified direct BACnet/SC certificate principal (`ScPrincipal`); hub-relayed
  and non-SC requests carry none (#524). ([546f017](https://github.com/jscott3201/rusty-bacnet/commit/546f017e703382e00f2e7ec1d7b7597d591d4503))

- Rust and Python BACnet/SC consumers now require Rustls 0.23.46 or later,
  incorporating upstream security fixes while preserving the existing TLS
  verification policy and session resumption (#1737). ([616196c](https://github.com/jscott3201/rusty-bacnet/commit/616196cba929df0356555b42d38c26d3482d5fb7))

### Added

- **Rust API:** On Linux the MS/TP transport's dedicated thread can
  run under `SCHED_FIFO` and be pinned to CPUs (`MstpThreadConfig`, CLI
  `--mstp-rt-priority` and `--mstp-cpus`); `start` fails if either can't be
  applied, and `execution_status()` reports the outcome (#501). ([7260f31](https://github.com/jscott3201/rusty-bacnet/commit/7260f313ea013749cdcb7be40062d8ef3c83dc24))

- **Python API:** `sc_npdu_drop_counts()` adds `control_rate_drops`, the
  network-layer messages an SC node dropped over their source's rate limit (#518). ([0476eb5](https://github.com/jscott3201/rusty-bacnet/commit/0476eb52a86d5b4e6e73618ce88c9b73cd605f23))

- The BACnet enumeration types have a `name()` method that returns a value's
  constant name, or `None` for a value without one (#915). ([f729a45](https://github.com/jscott3201/rusty-bacnet/commit/f729a457dc9afc433d2b857ee2d6dd1284fe0de4))

- A BBMD can run behind a NAT router that port-forwards its B/IP port, at a
  configured global address (`BbmdOptions::nat`, `NatTraversal`; in
  `bacnet-bip-core`, `Tables::set_nat` and `bbmd::Nat`), with two-hop BDT
  rules and the global origin in the Forwarded-NPDUs it builds (#940). ([7b81a3f](https://github.com/jscott3201/rusty-bacnet/commit/7b81a3f167931793827f40a337c9abe7ccc605d9))

- **Wire:** an IPv4 BBMD's Network Port serves BACnet_IP_NAT_Traversal and
  BACnet_IP_Global_Address, writable in any order and activated together
  with the BDT; an activated setting is saved with the persisted BDT (#940). ([7b81a3f](https://github.com/jscott3201/rusty-bacnet/commit/7b81a3f167931793827f40a337c9abe7ccc605d9))

- The PICS draft can claim the B/IP NAT traversal data-link option
  (`DataLinkSupport::BipV4NatTraversal`) (#940). ([7b81a3f](https://github.com/jscott3201/rusty-bacnet/commit/7b81a3f167931793827f40a337c9abe7ccc605d9))

- **Python API:** `Bbmd(nat_global_address="ip:port",
  nat_private_ranges=[...])` runs a BBMD behind a NAT router (#940). ([7b81a3f](https://github.com/jscott3201/rusty-bacnet/commit/7b81a3f167931793827f40a337c9abe7ccc605d9))

- A B/IP BBMD can serve a multicast group (Annex J.8.3): with `.multicast(...)`
  in BBMD mode it forwards the group's broadcasts to its BDT peers and
  foreign devices and theirs to the group; its own BDT row must be two-hop,
  and its Network Port lists the group read-only (#941). ([d7fa8bb](https://github.com/jscott3201/rusty-bacnet/commit/d7fa8bbdefc8b9ed54ab554b0e1f4cc283941ff8))

- A B/IP node can join an IPv4 multicast group on an explicit interface
  (Annex J.8): its broadcasts go to the group and it takes broadcasts only
  from the group (`BipTransport::set_multicast`, `.multicast(...)` on the
  client, server and endpoint builders, #941). ([305283f](https://github.com/jscott3201/rusty-bacnet/commit/305283f7b6a8aa7d896ef7243ba0ddb7c4f1c3e9))

- The CLI takes `--bip-multicast GROUP`, `--bip-multicast-ttl` and
  `--bip-multicast-port`; TUI profiles keep the group, TTL and port, and
  the profile editor and summary show the group (#941). ([305283f](https://github.com/jscott3201/rusty-bacnet/commit/305283f7b6a8aa7d896ef7243ba0ddb7c4f1c3e9))

- Every registered NORMAL B/IP Network Port on an explicit interface now
  lists BACnet_IP_Multicast_Address (0.0.0.0 while off), writable with
  ReinitializeDevice activation, which moves the running node to the new
  group, and read-only without a handler (#941). ([305283f](https://github.com/jscott3201/rusty-bacnet/commit/305283f7b6a8aa7d896ef7243ba0ddb7c4f1c3e9))

- **Python API:** `BACnetClient`, `BACnetServer` and `BipEndpoint` take
  `multicast=Multicast(group, ttl=1, port=None)` to join a B/IP multicast
  group, and with `bbmd=Bbmd(...)` serve it as its BBMD (#941). ([305283f](https://github.com/jscott3201/rusty-bacnet/commit/305283f7b6a8aa7d896ef7243ba0ddb7c4f1c3e9))

- A manual Docker check of two BBMD sites and two foreign devices behind
  iptables NAT routers, `examples/docker/nat/run.sh`, runnable from the
  `NAT topology` workflow (#942). ([7b81a3f](https://github.com/jscott3201/rusty-bacnet/commit/7b81a3f167931793827f40a337c9abe7ccc605d9))

- B/IP, B/IPv6, BACnet/SC and MS/TP transports have a bounded, lossy frame
  tap, `frame_tap()`, that shows every link-layer frame sent and received,
  control traffic included, with no copying while nobody subscribes
  ([docs](docs/rust-api.md#frame-tap), #957). ([1f6fa88](https://github.com/jscott3201/rusty-bacnet/commit/1f6fa884c444e912a2f200f4bac30c9b765b3b38))

- `EndpointSession::mstp_diagnostics` returns the MS/TP transport counters of
  a session that `MstpEndpointBuilder` built (#958). ([1e41deb](https://github.com/jscott3201/rusty-bacnet/commit/1e41debefdfd945e3f7666c7f44089c97caa693f))

- `mstp::monitor::MstpMonitor` listens to an MS/TP trunk without ever
  transmitting and publishes per-station statistics (frame counts, CRC
  errors, token rotation and hold times, lost tokens, inferred Max_Master)
  and a lossy frame stream; `mstp_serial::ReceiveOnlyPort` opens its port
  (#958, #970). ([1e41deb](https://github.com/jscott3201/rusty-bacnet/commit/1e41debefdfd945e3f7666c7f44089c97caa693f))

- BACnet/SC hub-connector observability: `connection_events()` reports
  connects, disconnect causes, reconnects, failover and primary restore
  attempts, `connection_status()` gives the active hub, hub VMAC and
  heartbeat timing, and `ScClientBuilder::failover_hub_url` sets a failover
  hub (#959, #1828). ([a3479d4](https://github.com/jscott3201/rusty-bacnet/commit/a3479d4fca1423d87d3fc5c54936f1c0d07bd387))

- **CLI:** `find` prints the objects that answer its Who-Has, and
  `whois-router` (now with `--dnet` and `--wait`) lists the routers that
  answer with the networks they reach; both print a JSON array with `--json`
  (#960). ([603c082](https://github.com/jscott3201/rusty-bacnet/commit/603c082ceb6f2edb1075178503bea014a4bf329c))

- `BACnetClient` hands received I-Haves to `i_have_events()` subscribers, and
  `who_has_replies` sends a Who-Has and collects its answers over a wait (#960). ([603c082](https://github.com/jscott3201/rusty-bacnet/commit/603c082ceb6f2edb1075178503bea014a4bf329c))

- `BACnetClient` sends Who-Is-Router-To-Network and What-Is-Network-Number,
  lists routers with `discover_routers`, and reports I-Am-Router-To-Network,
  Network-Number-Is and Reject-Message-To-Network through `network_reports()`
  (#960). ([603c082](https://github.com/jscott3201/rusty-bacnet/commit/603c082ceb6f2edb1075178503bea014a4bf329c))

- `NetworkLayer::send_network_message` sends Who-Is-Router-To-Network and
  What-Is-Network-Number, and `ReceivedNetworkReport::parse` reads the reports
  a non-routing node acts on; `bacnet_router_core::table::is_reserved` is
  public (#960). ([603c082](https://github.com/jscott3201/rusty-bacnet/commit/603c082ceb6f2edb1075178503bea014a4bf329c))

- **TUI:** `:find NAME` lists the devices whose I-Have answers a Who-Has
  below the device table, and the Network screen's new Networks tab maps the
  routers heard to the networks they reach, flagging a network number two
  routers claim (#960). ([603c082](https://github.com/jscott3201/rusty-bacnet/commit/603c082ceb6f2edb1075178503bea014a4bf329c))

- `bacnet tui` keeps named B/IP and BACnet/SC profiles (hub URLs, credential
  paths but never their contents, a UUID made once), with saved
  first-use authorization and connect, disconnect and retry; it refuses a
  missing credential file before sending anything. Flags-only runs save
  nothing (#962). ([fffd440](https://github.com/jscott3201/rusty-bacnet/commit/fffd440c430b089485ca45c4f9b0fdb9711eaa53))

- `bacnet` read, readm, write and subscribe reach routed devices as
  `dnet:instance` or `dnet:0xMAC`, and every target takes a BACnet/SC VMAC as
  hex octets, which device tables now print on SC (#963). ([70fc320](https://github.com/jscott3201/rusty-bacnet/commit/70fc320b96b9713bb14108f526923fe17562d5da))

- `bacnet tui` opens a read-only device browser on Enter: the device card,
  the Object_List grouped by type, an object's properties with text status
  badges, and its priority array with the slot in control marked, beside the
  properties on wide terminals (#963, #1787). ([70fc320](https://github.com/jscott3201/rusty-bacnet/commit/70fc320b96b9713bb14108f526923fe17562d5da))

- `bacnet tui` can write: time-boxed write sessions armed with `W` (profiles
  default to `writes = "session"`), a preview of every write with
  confirmation tiers, a ledger that reverts every change, array
  elements too, and survives a crash, an exit guard, an audit log, and
  `--passive` (#964, #1801). ([ab81faf](https://github.com/jscott3201/rusty-bacnet/commit/ab81fafb28d79484f410b15b6c39f86aba743d68))

- `BACnetClient::manage_cov_property_subscription` renews a SubscribeCOVProperty
  subscription before its lifetime ends, as `manage_cov_subscription` does for
  SubscribeCOV (#965). ([4b8fe77](https://github.com/jscott3201/rusty-bacnet/commit/4b8fe7752281f8188d541b6de46c9f352d4fa6c7))

- `bacnet tui` has a watch list: COV with finite, renewed lifetimes that are
  cancelled on remove and exit, rate-capped polling when a device refuses
  COV, routed devices as `NET:INSTANCE`, sparklines, a footprint counter and
  an optional CSV log (#965, #1891). ([4b8fe77](https://github.com/jscott3201/rusty-bacnet/commit/4b8fe7752281f8188d541b6de46c9f352d4fa6c7))

- `bacnet tui` has a Network screen for BACnet/IP: foreign-device status with
  a renewal countdown and banners for a refusal, a silent BBMD or a lapsed
  lease, the BBMD's BDT and FDT with our own entry marked, and a BDT
  consistency matrix that surveys four BBMDs at once (#966, #1823, #1889). ([dd5f68c](https://github.com/jscott3201/rusty-bacnet/commit/dd5f68cd01d6eeeae393bcf4f7707cdf385b6b3d))

- `bacnet tui` has an SC screen: hub-connector status with failover and
  heartbeat, the hub's certificate chain, VMAC and UUID, an event timeline
  from the first connect, and a connection doctor that names the failing
  dial phase with a fix (#967, #1890). ([a6dcb23](https://github.com/jscott3201/rusty-bacnet/commit/a6dcb234d11afe96feca6ce14f92bac9e6050545))

- `bacnet sc-check` inspects local SC credential files offline with complete
  human/JSON reports, explicit local diagnostic severities and exit statuses,
  reproducible inspection time, and fallible output. It requires `sc-tls` (#968). ([edeb409](https://github.com/jscott3201/rusty-bacnet/commit/edeb40922b9a727a5e3b225a8e94e29c169206e2))

- Rust and Python can inspect bounded local SC PEM inputs offline, reporting
  certificate fingerprints, dates, SAN/KU/EKU findings, key consistency and
  advisory Unix key-file mode bits. Existing SC adapters share PEM decoding
  while retaining TLS policy and startup error ordering (#968). ([1e2ca94](https://github.com/jscott3201/rusty-bacnet/commit/1e2ca949527a0e262981173cb9206f71a9b919d6))

- BACnet/SC dials lint this node's certificate, key and CA file first: hard
  failures stop the dial before any socket, and warnings (expiry within 30 days,
  no direct issuer in the CA file) need `y` in the TUI. The SC screen gains a
  My credentials view (#968). ([9f18cc3](https://github.com/jscott3201/rusty-bacnet/commit/9f18cc3b39cd5365183fa9f332ce2e6ad1e30061))

- **CLI:** MS/TP: `--mstp PORT` and `--mstp-mac` join an RS-485 trunk as a
  master station, `mstp:N` names a station, and `bacnet mstp-ports` lists the
  ports. The TUI asks before joining and adds MS/TP profiles and a station
  screen. In the Linux release binary; elsewhere build with `serial` (#969). ([366412a](https://github.com/jscott3201/rusty-bacnet/commit/366412a8aa757aa75fa77f38e4ef8b9236303ab4))

- **Rust API:** `MstpDiagnostics::node_state` reports an MS/TP station's
  master state, next station and sole-master flag (#969). ([366412a](https://github.com/jscott3201/rusty-bacnet/commit/366412a8aa757aa75fa77f38e4ef8b9236303ab4))

- `bacnet tui --mstp-monitor PORT` listens to an MS/TP trunk without
  transmitting: stations, the token ring, Max_Master findings and error
  counters, and a wizard that proposes a free MAC and joins after the same
  confirmation as `--mstp` (#970). ([23b6048](https://github.com/jscott3201/rusty-bacnet/commit/23b60488c8f870951a1408e80d6ffea46778e9d3))

- The TUI has a Capture screen (`7`): live frames from the stack's own
  traffic or the MS/TP monitor, a decode tree with a hex view, filters and
  pause over a bounded ring with an exact dropped count, and pcap export
  (`e`, `E`); `tui --capture-device` adds a libpcap capture (#971). ([89b4902](https://github.com/jscott3201/rusty-bacnet/commit/89b4902b0b061ea99584954a5e1997a6ecdc4652))

- A Terminal UI section on the website: a guide to every `bacnet tui` screen,
  from profiles and discovery to guarded writes and the B/IP, BACnet/SC and
  MS/TP diagnostics, with a key reference and Terminal screenshots that a
  script regenerates (#975). ([7e7a1e7](https://github.com/jscott3201/rusty-bacnet/commit/7e7a1e75e34b6356f19f3798127e25b43cb7d8c4))

- Client builders can return one shared type for B/IP, B/IPv6 and BACnet/SC,
  retaining their configured options and SC reconnect policy. Existing concrete
  builds remain available; SC rejects invalid segment windows before dialing
  (#1014). ([45f29c9](https://github.com/jscott3201/rusty-bacnet/commit/45f29c91fa7c690f5d8a5023216f24b63300d463))

- **Python API:** `add_trend_log` takes keyword settings as
  `add_trend_log_multiple` does, plus its reference, `"cov"` logging and the
  two COV properties (#1480). ([a25f68d](https://github.com/jscott3201/rusty-bacnet/commit/a25f68dc99e14845174cc87708c41832cfc0553c))

- **Wire:** A Trend Log logs by COV a property of its own device: Logging_Type
  takes COV, Log_Interval switches between polled and COV logging, and it
  serves Client_COV_Increment and COV_Resubscription_Interval (#1480). ([a25f68d](https://github.com/jscott3201/rusty-bacnet/commit/a25f68dc99e14845174cc87708c41832cfc0553c))

- **Python API:** `add_global_group(members=)`, `add_schedule(weekly_schedule=,
  exception_schedule=, effective_period=)` and `add_calendar(date_list=)` take
  the forms their typed reads give, and `PropertyValue.octets` gives a value's
  octets (#1499). ([09c332c](https://github.com/jscott3201/rusty-bacnet/commit/09c332ce7fe817c0cadf44148dfb10f06eb3118c))

- **Wire:** Event Enrollment serves Event_Message_Texts, Event_Message_Texts_Config
  and the Event_Algorithm_Inhibit pair. The inhibit pauses its event algorithm
  but not fault detection, and a configured text goes out, as written, in the
  enrollment's notifications (#1507). ([46d7ed0](https://github.com/jscott3201/rusty-bacnet/commit/46d7ed011c600c622263a7c2dabd6b95a1d397c3))

- **Wire:** Color and Color Temperature outputs are driven to their default at
  the server's start; a zero default keeps the stored Present_Value
  (`with_output_persistence`). Objects run without a server now read
  In_Progress NOT_CONTROLLED, not IDLE, until `start_output` or a write (#1526). ([d38cb02](https://github.com/jscott3201/rusty-bacnet/commit/d38cb02fd4fc0a2e1d7b6b377dd866ff0da2eef2))

- **Python API:** `add_color` and `add_color_temperature` take
  `default_color` or `default_color_temperature`, and a `storage_path` that
  keeps Present_Value across restarts (#1526). ([d38cb02](https://github.com/jscott3201/rusty-bacnet/commit/d38cb02fd4fc0a2e1d7b6b377dd866ff0da2eef2))

- Trend Log, Trend Log Multiple and Event Log can keep themselves in storage
  (`with_persistence`, `FileLogPersistence`): they commit every 100 records or
  a minute and at stop, and reopen with their records, numbering and
  BUFFER_READY counts plus a LOG_INTERRUPTED record (#1541). ([6b354c5](https://github.com/jscott3201/rusty-bacnet/commit/6b354c518ebf2342a30cd4ff6161b6969fcb66d0))

- **Python API:** `add_trend_log`, `add_trend_log_multiple` and
  `add_event_log` take a `storage_path` keyword that keeps the log across
  restarts, as `add_audit_log` does (#1541). ([6b354c5](https://github.com/jscott3201/rusty-bacnet/commit/6b354c518ebf2342a30cd4ff6161b6969fcb66d0))

- **Python API:** lighting outputs take trims and colour links, `add_color`
  and `add_color_temperature` add the colour objects, `lighting_color` reports
  an output's colour, and the point builders take `tags`, profile keywords
  and Value_Source tracking (#1554). ([09c332c](https://github.com/jscott3201/rusty-bacnet/commit/09c332ce7fe817c0cadf44148dfb10f06eb3118c))

- An accepted ReinitializeDevice request writes a DEVICE_RESET,
  DEVICE_BACKUP or DEVICE_RESTORE Audit record on the server and on endpoint
  sessions, and `BACnetServer::report_device_reset_local` records a reset the
  application makes for its own reasons (#1564). ([1e59e67](https://github.com/jscott3201/rusty-bacnet/commit/1e59e67db0afe5cef754f7fc51e86178ba8b544b))

- A ReinitializeDevice handler can register its restart with
  `ReinitializeContext::after_reply`, which the server and endpoint sessions
  run once the SimpleACK has been handed to the transport, and never for a
  request answered with an Error (#1565). ([1e59e67](https://github.com/jscott3201/rusty-bacnet/commit/1e59e67db0afe5cef754f7fc51e86178ba8b544b))

- **Python API:** `BACnetServer` and the endpoints take `on_reinitialize`
  and `after_reinitialize` to carry out ReinitializeDevice, which also makes a
  BBMD port's rows writable; endpoints take `reinit_password`, and
  `report_device_reset_local()` records an application reset (#1566). ([1e59e67](https://github.com/jscott3201/rusty-bacnet/commit/1e59e67db0afe5cef754f7fc51e86178ba8b544b))

- `BbmdTables` gains `check_broadcast_distribution_table` and `activate`, with
  `BbmdChanges`, for a transport to check and apply written BBMD settings;
  both refuse by default (#1578). ([0e84c56](https://github.com/jscott3201/rusty-bacnet/commit/0e84c564dd4fc9b6ce9d63841db1005350bb0ce3))

- **Wire:** A registered BBMD's Network Port takes writes to
  BBMD_Broadcast_Distribution_Table and BBMD_Accept_FD_Registrations: checked
  against the BDT policy, pending with Changes_Pending set, and put into effect
  by ReinitializeDevice ACTIVATE_CHANGES or WARMSTART (#1578). ([0e84c56](https://github.com/jscott3201/rusty-bacnet/commit/0e84c564dd4fc9b6ce9d63841db1005350bb0ce3))

- A B/IPv6 transport registers its Network Port as B/IP's does (`NetworkPortObject::new_bip6`, `TransportPort::bip6_port`): BACnet_IPv6_Mode and the IPv6 rows, writable foreign-device rows that move a running registration, and BBMD rows for a transport that lends them (#1579). ([9cc10a2](https://github.com/jscott3201/rusty-bacnet/commit/9cc10a2146d758faae84b213350a73bea8234e0c))

- **Python API:** `BACnetServer.add_bip6_network_port` adds a B/IPv6 Network Port, and `registered_network_port` now works with `transport="ipv6"` (#1579). ([9cc10a2](https://github.com/jscott3201/rusty-bacnet/commit/9cc10a2146d758faae84b213350a73bea8234e0c))

- `bacnet-types`, `bacnet-encoding` and `bacnet-services` build without an
  allocator: Who-Is, I-Am and ReadProperty encode into caller storage and
  decode in place, and tags, scalars, timestamps and references encode whole
  or not at all into slices, `heapless`, `Vec` or `BytesMut` sinks (#1614). ([fca2389](https://github.com/jscott3201/rusty-bacnet/commit/fca238953b304bc79ffb0bec231939acd7eec465))

- Test-only: the MS/TP bus simulator models buffered host adapters, delayed
  self-echo, inter-octet gaps, driver hold after the stop bit, receiver
  contention, each master's baud and clock error, and an application
  callback that answers delivered NPDUs (#1628, #1772, #1796). ([02313b1](https://github.com/jscott3201/rusty-bacnet/commit/02313b14b1a641a865eacae371cfda53ea841026))

- A BBMD alone on its subnet, as behind a NAT router, can skip the local
  rebroadcast of forwarded and distributed broadcasts:
  `BbmdOptions::alone_on_subnet`, `LocalSubnet::alone` in `bacnet-bip-core`,
  and **Python API:** `Bbmd(alone_on_subnet=True)`. Off by default (#1668). ([0e84c56](https://github.com/jscott3201/rusty-bacnet/commit/0e84c564dd4fc9b6ce9d63841db1005350bb0ce3))

- Routers can announce Router-Busy for a port whose egress queue stays full, reject what that queue then refuses with reason 2, and announce Router-Available once it drains: off by default, turned on with `RouterOptions::congestion` or the core's `Options::congestion` (#1697). ([72ab468](https://github.com/jscott3201/rusty-bacnet/commit/72ab46864f2846d09a5f3280a3148f3c24a7485d))

- Two opt-in router hardening options: rejects that change a learned route only from its next router (`next_hop_rejects`), and suppression of a global broadcast repeated within 1 s (`duplicates`). Ignored flow control and dropped repeats are counted (#1698). ([72ab468](https://github.com/jscott3201/rusty-bacnet/commit/72ab46864f2846d09a5f3280a3148f3c24a7485d))

- A router's own application can send requests to other networks through the routing engine, with an explicit source address: `BACnetRouter::originate`, and `Router::originate` in `bacnet-router-core` (#1699). ([72ab468](https://github.com/jscott3201/rusty-bacnet/commit/72ab46864f2846d09a5f3280a3148f3c24a7485d))

- `bacnet_types::map_hash`: a cheap multiply-and-rotate hasher for maps
  that remote input can't grow, which the client's TSM and the server's
  request admission now use (#1713). ([4a098fc](https://github.com/jscott3201/rusty-bacnet/commit/4a098fccbc4da1fb4dbba05efe609dc1a108b044))

- Python SC clients and servers can set the queued NPDU quota per origin and
  read snapshots of the drop counters; the Rust SC client builder also takes
  the quota (#1718). ([2e0e595](https://github.com/jscott3201/rusty-bacnet/commit/2e0e5955a484a9a12b81f080a97922e6456aa556))

- Python SC clients accept frozen ScReconnect settings and reuse their TLS
  policy and identity after connection loss. Client close drains active
  operations and retains joined shutdown when its waiter is cancelled (#1722). ([80710a3](https://github.com/jscott3201/rusty-bacnet/commit/80710a384d828c0c478af7f51b99e646da54d1fc))

- **Python API:** the stack's log events now reach Python's `logging` under
  `rusty_bacnet.<crate>` loggers (with a `NullHandler`, so nothing prints
  until the app configures logging); levels are read at import and at each
  start, and `refresh_logging()` rereads them (#1724). ([d30ad68](https://github.com/jscott3201/rusty-bacnet/commit/d30ad682d6e2e92abd10da546457db3927fe0bb5))

- Rust SC transports and Python clients expose owned active-Hub certificate
  snapshots with exact leaf fingerprints, connection provenance and bounded
  subject/issuer/validity metadata. Observation uses the existing connection
  without changing TLS acceptance (#1735). ([2c0bc07](https://github.com/jscott3201/rusty-bacnet/commit/2c0bc079ed695b00a5544e5597d3e64efb41a34d))

- Failed outbound SC TLS handshakes expose owned, explicitly untrusted peer
  certificates through Rust connect errors and Python transport exceptions.
  Capture uses the actual attempt and preserves normal certificate verification,
  rejection details and TLS resumption (#1735). ([ada647c](https://github.com/jscott3201/rusty-bacnet/commit/ada647c8009b0e4ab532dc0129e56df2802fb99b))

- `bacnet-time` 0.13.1 adds `earliest`, which merges optional deadlines
  without letting an unset one win as `Option::min` does, and a `std`
  feature with `HostClock`, which maps a host's `std::time::Instant` onto the
  cores' timeline and back. The hosted transports and router use both (#1768). ([9a1ee25](https://github.com/jscott3201/rusty-bacnet/commit/9a1ee25c2be326718adb04da3d791adcc2e0fc4e))

- The MS/TP core counts broken internal invariants in `internal_errors`,
  which the Tokio transport totals (#1769). ([561a5a4](https://github.com/jscott3201/rusty-bacnet/commit/561a5a46aaf5dcaafaaac1c94bc7b072b0073859))

- A new `no_std` crate, `bacnet-npdu`, holds the network-layer vocabulary
  once: the borrowed NPDU view and encoder, network message types, reject
  reasons, the network priority and NPDU limits (#1770). ([62a5baf](https://github.com/jscott3201/rusty-bacnet/commit/62a5baf1dbe0fa014b1bc9948d76a86b4a9d9807))

- The allocation-free `ApduHeader` also reads and writes an Error's header
  and a whole Reject or Abort, so firmware without a heap can refuse a
  request it doesn't serve (#1771). ([4ff79e2](https://github.com/jscott3201/rusty-bacnet/commit/4ff79e25f3166efd1ec398bffff6d6b292d8d9b6))

- Host-runnable examples for the portable cores: `bacnet-mstp-core`'s
  `node` (two master nodes on a simulated line) and `bacnet-router-core`'s
  `bip_mstp_router` (a B/IP-MS/TP router and BBMD in a firmware-style main
  loop) (#1771). ([4ff79e2](https://github.com/jscott3201/rusty-bacnet/commit/4ff79e25f3166efd1ec398bffff6d6b292d8d9b6))

- Test-only: CI links a `device` firmware image, an MS/TP device with no
  allocator that answers Who-Is and ReadProperty, for Cortex-M0/M3/M4F and
  RV32IMC within flash, RAM and stack budgets, and runs its scenario
  natively; the other images' stack budgets were measured again (#1771,
  #1774). ([4ff79e2](https://github.com/jscott3201/rusty-bacnet/commit/4ff79e25f3166efd1ec398bffff6d6b292d8d9b6))

- A "Run on a microcontroller" page on the website: the crates each embedded
  product needs, the shared clock, measured flash and RAM, adapter rules and
  what still needs an allocator; and Rust API notes for `bacnet-time`,
  `bacnet-npdu` and the two codecs (#1771). ([4ff79e2](https://github.com/jscott3201/rusty-bacnet/commit/4ff79e25f3166efd1ec398bffff6d6b292d8d9b6))

- `bacnet-router-core` gains `node::Node`, a non-routing node's receive
  rules, sharing the router engine's admission checks and adding
  `Discard::NotRouted`; `NetworkLayer` runs on it, with drop totals in
  `NetworkLayer::counters()`, `malformed` and `routed_drops` included (#1795). ([dbe342f](https://github.com/jscott3201/rusty-bacnet/commit/dbe342f949c1ff7840b80fe8c10967906cd42737))

- **CLI:** the TUI command bar takes quotes and backslash escapes, and a
  routed station as the device table shows it (`:watch 2:0x05 ai:1`); it
  says when no device is at that station or when `:watch NET:N` names the
  wrong network (#1800, #1914, #1917). ([d6c84ee](https://github.com/jscott3201/rusty-bacnet/commit/d6c84ee6ed992eb4d7cd759612bbe507640fe517))

- bip-core's `Node::retarget` and `NodePort::retarget` move a running foreign
  device to another BBMD or time-to-live, restarting its registration (#1825). ([06a380c](https://github.com/jscott3201/rusty-bacnet/commit/06a380cdaf2faa524d4d4dd169c3715b2193df86))

- `BbmdTables` and `ForeignRegistration` gain `can_change`, false by default:
  a Network Port lists its pending-activation rows writable, and takes writes
  to them, only when its transport's view can apply them; override it together
  with `activate` and the check (#1825). ([06a380c](https://github.com/jscott3201/rusty-bacnet/commit/06a380cdaf2faa524d4d4dd169c3715b2193df86))

- **Wire:** A registered foreign device's Network Port takes writes to
  FD_BBMD_Address and FD_Subscription_Lifetime, pending until ReinitializeDevice
  ACTIVATE_CHANGES or WARMSTART, which moves its next registration to the new
  BBMD and time-to-live (#1825). ([06a380c](https://github.com/jscott3201/rusty-bacnet/commit/06a380cdaf2faa524d4d4dd169c3715b2193df86))

- A BDT activated through the Network Port object is saved to the BBMD's
  persisted-BDT file, so a restart keeps it; a failed save is logged and
  counted (`BipTransport::bdt_persist_failures`) and leaves the new BDT in use
  (#1826). ([06a380c](https://github.com/jscott3201/rusty-bacnet/commit/06a380cdaf2faa524d4d4dd169c3715b2193df86))

- **Python API:** An SC `BACnetClient` takes `sc_failover_hub` and
  `sc_connect_timeout_ms`, and reports its hub link through
  `connection_events()`, an async iterator whose first call starts with the
  first `connected`, and `connection_status()` (#1829, #1839, #1908). ([b9364d2](https://github.com/jscott3201/rusty-bacnet/commit/b9364d28e9ffeb3a4a3c2c423930641b89edda17))

- TUI SC profiles can set the reconnect backoff, the connect timeout or
  `reconnect = false`; new `--sc-failover-url`, `--sc-reconnect` and
  `--sc-connect-timeout` flags give flags-only runs the same (#1831). ([c9c9033](https://github.com/jscott3201/rusty-bacnet/commit/c9c9033855d05a54459e7b93e073c79356c6285c))

- **Python API:** `BACnetClient` reads and writes a BBMD's tables, deletes an
  FDT entry and registers once as a foreign device; a NAK raises
  `BacnetBvlcNakError` with its result code, apart from `BacnetTimeoutError`
  for no reply (#1841). ([b9364d2](https://github.com/jscott3201/rusty-bacnet/commit/b9364d28e9ffeb3a4a3c2c423930641b89edda17))

- **Python API:** `ScPemInspection` reports `direct_issuer` and
  `direct_issuer_index`: whether a peer-trust CA signed the identity leaf
  directly, and which one (#1846). ([b9364d2](https://github.com/jscott3201/rusty-bacnet/commit/b9364d28e9ffeb3a4a3c2c423930641b89edda17))

- **Python API:** `BACnetClient` collects Who-Has answers with
  `who_has_replies()` and routers with `discover_routers()`, and follows
  received I-Haves and network reports through `i_have_events()` and
  `network_reports()`, async iterators that never hold up the client (#1850). ([b9364d2](https://github.com/jscott3201/rusty-bacnet/commit/b9364d28e9ffeb3a4a3c2c423930641b89edda17))

- On a TUI profile session, each confirmed MS/TP trunk join is now written to
  the profile's audit log with the port, station and baud rate (#1862). ([c9c9033](https://github.com/jscott3201/rusty-bacnet/commit/c9c9033855d05a54459e7b93e073c79356c6285c))

- **Wire:** Alert Enrollment serves Event_Message_Texts,
  Event_Message_Texts_Config, Event_Algorithm_Inhibit_Ref and
  Event_Algorithm_Inhibit, the optional rows its property table lists, through
  the same shared options as Event Enrollment (#1865). ([12a6109](https://github.com/jscott3201/rusty-bacnet/commit/12a610922ac914143630259eb61343e3e18c6d82))

- **Wire:** Lighting Output and Binary Lighting Output serve Value_Source,
  Value_Source_Array and Last_Command_Time, and a Value_Source COV
  subscriber on either hears the commandable report (#1866). ([12a6109](https://github.com/jscott3201/rusty-bacnet/commit/12a610922ac914143630259eb61343e3e18c6d82))

- **CLI:** `find` takes the object name as a plain word,
  `bacnet find "Zone Temp"`, as well as with `--name`; giving both is a usage
  error (#1872). ([d6c84ee](https://github.com/jscott3201/rusty-bacnet/commit/d6c84ee6ed992eb4d7cd759612bbe507640fe517))

- **Rust API:** `bacnet_encoding::constructed::property_element` names the
  constructed element each (object type, property) value is made of, and
  `split_elements` splits a value into those elements (#1888). ([1233dc1](https://github.com/jscott3201/rusty-bacnet/commit/1233dc1eeadc00360ff935ff96cac5ae37a9ab52))

- **CLI (TUI):** the browser, watch list and write previews decode
  timestamps, schedules and references, name states, use kebab case with
  unit symbols (`72.5 °F`) and show six-digit floats; previews and the watch
  CSV keep exact values (#1888). ([1233dc1](https://github.com/jscott3201/rusty-bacnet/commit/1233dc1eeadc00360ff935ff96cac5ae37a9ab52))

- **Rust API:** `ScClientBuilder::build_with_events` and
  `build_any_with_events` build a BACnet/SC client, concrete or
  transport-erased, with a hub-connector event receiver that already holds
  the first `Connected` (#1890, #1909). ([37d6db1](https://github.com/jscott3201/rusty-bacnet/commit/37d6db1593d91792d5b44539a1c94eb2910f31f4))

- `bacnet mstp-ports` and the TUI's join dialog list USB serial adapters
  first, once each, by their `/dev/serial/by-id` name on Linux and as
  call-out (`/dev/cu.*`) devices on macOS; `mstp_serial::available_port_infos`
  reports each port's kind (#1893, #1928). ([07ca68e](https://github.com/jscott3201/rusty-bacnet/commit/07ca68e8657471fbdb3b70103a6fa15d2e588fb3))

- The TUI's status bar numbers the screens that apply to the session, `1` and
  `2` switch between the devices and a kept browser, help lists the global and
  the screen's keys and scrolls, and the command bar shows the syntax being
  typed (#1895). ([3f3ab65](https://github.com/jscott3201/rusty-bacnet/commit/3f3ab65d7797389d4011a34292d5673b5d6bc278))

- `bacnet tui` shows device and object names, reading only the rows on
  screen within `--max-rate` in batches sized to the device's Max_APDU, and
  keeps up to 4,096 per device address; `/` matches names and `s` sorts the
  device table by them (#1896, #1929, #1930, #1931). ([2df806d](https://github.com/jscott3201/rusty-bacnet/commit/2df806d3333cfe8f1a336a7aadbf7c17da533410))

- `bacnet_transport::warn_throttled!` logs a warning through a `WarnThrottle`:
  at most once per interval with the count since the last one, and at debug
  level otherwise (#1950). ([53d4601](https://github.com/jscott3201/rusty-bacnet/commit/53d4601a5a17a75c9a02f7b2203ab7fae75cc46c))

- **Breaking (Rust API):** The Device gains setters for Object_List (through
  `DeviceAuthority`), System_Status and Database_Revision, and serves the
  backup and restore properties when `DeviceConfig::backup_and_restore` is set. ([488f7ec](https://github.com/jscott3201/rusty-bacnet/commit/488f7ecf41123bf8f1c30b24a368199d904c1dbc))

### Changed

- **Breaking (Rust API):** Protocol errors, rejects and aborts carry the typed
  error class, code and reason in `Error` and the client's `TsmResponse`, and
  the analog, accumulator, pulse converter, loop and staging objects take
  `EngineeringUnits`. The wire and the Python API are unchanged (#915). ([f729a45](https://github.com/jscott3201/rusty-bacnet/commit/f729a457dc9afc433d2b857ee2d6dd1284fe0de4))

- **Breaking (Rust API):** `BbmdOptions` gains `nat` and `alone_on_subnet`,
  `DataLinkSupport` is `#[non_exhaustive]` with `BipV4NatTraversal`,
  `bip_core::bbmd::Tables` gains a private field, and
  `BbmdState::local_address` returns the socket's `SocketAddrV4` even behind
  NAT (#940, #1668). ([7b81a3f](https://github.com/jscott3201/rusty-bacnet/commit/7b81a3f167931793827f40a337c9abe7ccc605d9))

- **Breaking (Rust API):** `bacnet_transport::port::BipPort` is
  `#[non_exhaustive]` with `BipPort::new`, and gains a `multicast` field,
  the group view a Network Port serves (#941). ([305283f](https://github.com/jscott3201/rusty-bacnet/commit/305283f7b6a8aa7d896ef7243ba0ddb7c4f1c3e9))

- **Wire:** `BACnetClient::who_has` sends its Who-Has as a global broadcast,
  as `who_is` does, so `find` and `:find` reach devices behind routers (#960). ([603c082](https://github.com/jscott3201/rusty-bacnet/commit/603c082ceb6f2edb1075178503bea014a4bf329c))

- **Python API:** an SC hub host that does not resolve now reads "WebSocket
  host lookup for HOST failed" rather than a TCP dial failure, in `--sc`
  output and `BacnetTransportError`; a name with no addresses gets errno
  ENOENT rather than EINVAL (#967). ([a6dcb23](https://github.com/jscott3201/rusty-bacnet/commit/a6dcb234d11afe96feca6ce14f92bac9e6050545))

- **Rust API:** BACnet/SC connect errors carry the TLS library's reason
  (`ScConnectError::tls_failure`), and a host that does not resolve reports
  `ScWebSocketErrorKind::HostLookup` (#967). ([a6dcb23](https://github.com/jscott3201/rusty-bacnet/commit/a6dcb234d11afe96feca6ce14f92bac9e6050545))

- **Breaking (CLI):** commands that change a device need `--allow-write`,
  and DCC and reinitialize also `--yes` or a typed confirmation; a refusal
  exits with status 4 and sends nothing. `subscribe` defaults to 300
  seconds, renewed and cancelled on exit (#972). ([f0b530d](https://github.com/jscott3201/rusty-bacnet/commit/f0b530d4f7a18008d3227c032ec8d6d499f68abe))

- The published `bacnet` CLIs and Python wheels are built with whole-program
  LTO in a new `dist` profile, so they are about a third smaller (the macOS
  arm64 CLI from 18.9 to 12.4 MB); panic backtraces keep their function names
  (#973). ([d1ddddd](https://github.com/jscott3201/rusty-bacnet/commit/d1ddddd655e511ac29db0014fd22d2a363078c1e))

- **Breaking (CLI):** Plain `bacnet` on a terminal opens the TUI, now a default
  feature; elsewhere, or with `--no-tui` or `BACNET_NO_TUI`, it prints the
  usage and exits 2 (#974). ([b98aac9](https://github.com/jscott3201/rusty-bacnet/commit/b98aac93d39b1313ff5492a91f29f7335293be41))

- **CI:** A merge to `dev` that could have been a fast-forward to a PR head
  whose runs passed `CI OK` and `Native OK` skips re-running the same jobs,
  unless it changes a cache-key input (#1466). ([f9025d9](https://github.com/jscott3201/rusty-bacnet/commit/f9025d9b12b6ca2cf6974d94d240fd1f6b2d171d))

- **Breaking (Python API):** shed levels, Event_Parameters, authentication
  factors, Access_Event_Time, Last_Restore_Time, Subordinate_List and
  Event_Algorithm_Inhibit_Ref read typed instead of as `application_data`
  (#1499, #1888). ([09c332c](https://github.com/jscott3201/rusty-bacnet/commit/09c332ce7fe817c0cadf44148dfb10f06eb3118c))

- A B/IP transport sharing its port by address hands up broadcasts and
  unicast in the order the kernel received them on Linux, macOS and the BSDs,
  so a unicast that depends on a broadcast just before it is handled after it
  (#1546). ([3f2da73](https://github.com/jscott3201/rusty-bacnet/commit/3f2da73be27ca8a25e5202a2101946fd511b1e37))

- Some codec error messages read differently (#1646, #1776, #1777, #1803):
  a context is followed by a colon, nested faults name their label, encoder
  range checks begin `value out of range:`, invalid UTF-8 says how many
  octets were valid, and an unknown BACnet/SC BVLC-Result code prints in
  upper case. ([fca2389](https://github.com/jscott3201/rusty-bacnet/commit/fca238953b304bc79ffb0bec231939acd7eec465))

- **Breaking (Rust API):** Codec errors carry static labels, formatted
  only on display: `Error::Decoding` holds a `DecodeError`, encoders refuse
  with `Error::Encode` or `Error::Range`, and `Error::Segmentation` holds a
  `SegmentationError`, so firmware links no formatting (#1646, #1776,
  #1777, #1802). ([fca2389](https://github.com/jscott3201/rusty-bacnet/commit/fca238953b304bc79ffb0bec231939acd7eec465))

- **Breaking (Rust API):** `BACnetRouter::start` and the router core's
  `Router::new` refuse a port on network 0 or 0xFFFF, which they used to run
  with no direct route; number each port from 1 to 65534 (#1698). ([71f238d](https://github.com/jscott3201/rusty-bacnet/commit/71f238da8f30878b07abdbaf36116c6c7564fcbd))

- `bacnet-router-core`'s `Storage`, `Router` and `RouterIn` take a trailing const parameter `B`, defaulting to 32: the global broadcasts kept for duplicate suppression, 0 for none (#1698). ([72ab468](https://github.com/jscott3201/rusty-bacnet/commit/72ab46864f2846d09a5f3280a3148f3c24a7485d))

- A BACnet/SC node queues up to 256 received NPDUs, 32 from any one peer
  (was 64 and 4), so concurrent requests to one device no longer stall until
  the APDU timeout, and dropped NPDUs log a throttled warning (#1712). For
  the old guard, call `with_npdu_per_origin_limit(4)`. ([b786e07](https://github.com/jscott3201/rusty-bacnet/commit/b786e071e67c72bed0098f4b6086884d9b7a139d))

- **Breaking (Rust API):** `NetworkLayer::start` returns an `ApduReceiver`
  that decodes the transport's NPDUs as it is polled, with no dispatch task
  or second queue in between; network controls flow only while it is
  polled, and the transport's queue bounds overload (#1713). ([ed71a48](https://github.com/jscott3201/rusty-bacnet/commit/ed71a48addbc185cf68c3a02ce10b3bd18848ea5))

- `LoopbackTransport` drops a frame that finds the peer's queue full, as a
  real data link does, and counts it in the peer's `receive_drops()`, instead
  of making the sender wait (#1713). ([ed71a48](https://github.com/jscott3201/rusty-bacnet/commit/ed71a48addbc185cf68c3a02ce10b3bd18848ea5))

- Requests cost less (#1713, #1727, #1728, #1729): the server runs and reaps
  request handlers more cheaply and encodes each reply once, and no task sits
  between transport and application. One B/IP ReadProperty client gets about
  36% more requests a second than on 0.13.0 ([Benchmarks](Benchmarks.md)). ([ed71a48](https://github.com/jscott3201/rusty-bacnet/commit/ed71a48addbc185cf68c3a02ce10b3bd18848ea5))

- The benchmark harness measures what its scenarios name: `stress-test`
  summarises every step, connects every client before the clock starts, adds
  B/IPv6 and SC clients and COV and BBMD capacity, and renames
  `whois` to `device-scan`; Docker gains Who-Is and a working foreign
  device (#1714, #1817). ([59d2089](https://github.com/jscott3201/rusty-bacnet/commit/59d2089cfd001ae82c45acc2bef2f503b72d78da))

- **Breaking (Rust API):** Servers and endpoints share one Device identity
  (#1720). Startup resolves receive capacity before generating a Device, so a
  default MS/TP server declares 480. Runtime announcements and served identity
  fields remain coherent after database replacement. ([b146ed1](https://github.com/jscott3201/rusty-bacnet/commit/b146ed1e637e3975d891423547be2226e46713ac))

- The types, encoding, services, transport, network and endpoint crates'
  docs.rs pages open with their READMEs, each with a runnable quickstart,
  its feature profiles and its allocator or host needs, and crate guides
  cover object set-up, a client's Who-Is and a server's ReadProperty (#1723). ([8c14ff5](https://github.com/jscott3201/rusty-bacnet/commit/8c14ff56c0f731b45cc5eac0ed78b19a977cc316))

- **Breaking (Rust API):** `AnyTransport` now requires a fallback in downstream
  matches, allowing callers to handle additional transport variants enabled by
  dependency features (#1725). ([7e3450c](https://github.com/jscott3201/rusty-bacnet/commit/7e3450c9b04c2b9f70b5acbb836ca140c0ac2128))

- **Breaking (Rust API):** The B/IP port driver moves from
  `bacnet_router_core::bip` into `bacnet_bip_core::port` (`NodePort`,
  `BbmdPort`), and `Tables::serve` answers BBMD management requests, so a
  BBMD port answers them itself, Delete-FDT-Entry included (#1766). ([d930e28](https://github.com/jscott3201/rusty-bacnet/commit/d930e28ee895bb5f7f98fa9e1a2e50004ad6fd98))

- **Breaking (Rust API):** `bacnet-router-core` drains its counters like the
  link cores (`take_counters()` of `u32`s, `counters()` to look), re-exports
  `bip_core`/`mstp_core`, and adds `RouterIn` and `Options::STANDARD`;
  `BACnetRouter` keeps its `u64` totals (#1767). ([d930e28](https://github.com/jscott3201/rusty-bacnet/commit/d930e28ee895bb5f7f98fa9e1a2e50004ad6fd98))

- `bacnet-time` versions on its own from this release: it takes a new
  version only when its own API changes, so its number can differ from the
  other crates' (#1768). ([9a1ee25](https://github.com/jscott3201/rusty-bacnet/commit/9a1ee25c2be326718adb04da3d791adcc2e0fc4e))

- **Breaking (Rust API):** `bacnet-mstp-core` takes its buffers as one
  `Storage` bundle (`StandardStorage`, `ExtendedStorage`), `HeaplessQueues`
  is now `FixedQueues`, line inputs return nothing, and `CoreError` splits
  into per-call errors (#1769). ([561a5a4](https://github.com/jscott3201/rusty-bacnet/commit/561a5a46aaf5dcaafaaac1c94bc7b072b0073859))

- **Breaking (Rust API):** an MS/TP core's frame octets now come only from a
  confirmed `on_transmit_started`, so a refused start has nothing to send;
  `Action::Transmit` names the frame by header and length (#1769). ([561a5a4](https://github.com/jscott3201/rusty-bacnet/commit/561a5a46aaf5dcaafaaac1c94bc7b072b0073859))

- **Breaking (Rust API):** `bacnet-router-core`'s `MstpPort` owns its master
  core, as the B/IP `NodePort` owns its node, and refuses one without
  extended frames (#1769). ([561a5a4](https://github.com/jscott3201/rusty-bacnet/commit/561a5a46aaf5dcaafaaac1c94bc7b072b0073859))

- **Breaking (Rust API):** every crate speaks `bacnet-npdu`'s types, with
  the duplicates gone, and `bacnet-encoding`'s owned `Npdu` decodes and
  encodes through its view; `NetworkPriority` is a closed enum and the
  router's control authorizer sees a typed message type (#1770, #915). ([62a5baf](https://github.com/jscott3201/rusty-bacnet/commit/62a5baf1dbe0fa014b1bc9948d76a86b4a9d9807))

- **Wire:** The hosted BBMD answers Register-Foreign-Device, Read-BDT,
  Read-FDT and Delete-FDT-Entry through `bacnet-bip-core`'s `Tables::serve`.
  A Delete-FDT-Entry naming a registration past its time-to-live and grace
  period now gets the NAK, since that entry no longer exists (#1773). ([0e84c56](https://github.com/jscott3201/rusty-bacnet/commit/0e84c564dd4fc9b6ce9d63841db1005350bb0ce3))

- `FdtCounters::registrations_expired` now also counts the lapsed
  registrations purged before a refused no-policy registration, a Read-FDT
  or a Delete-FDT-Entry (#1773). ([0e84c56](https://github.com/jscott3201/rusty-bacnet/commit/0e84c564dd4fc9b6ce9d63841db1005350bb0ce3))

- Two refusals keep the inner fault's kind (#1776): the weekly and exception
  schedule decoders no longer report every entry fault as `InvalidEncoding`,
  and a ReadRange log record's range error converts to `OutOfRange`. ([28e6316](https://github.com/jscott3201/rusty-bacnet/commit/28e63162c202a81740d051b5908df8f827652153))

- `CharacterStringRef::decode` refuses what the owned path refuses: a DBCS
  string without its code page and malformed UCS-2 or UCS-4. Both run one
  shared check, and the owned validator now reports its faults at offsets
  in the data with the view's wording (#1778). ([28e6316](https://github.com/jscott3201/rusty-bacnet/commit/28e63162c202a81740d051b5908df8f827652153))

- **Breaking (Rust API):** a Reject-Message-To-Network's reason and network
  are read and written in one place, `bacnet_npdu::RejectMessageToNetwork`,
  which the router core, network layer and client share (#1794). ([d89ce35](https://github.com/jscott3201/rusty-bacnet/commit/d89ce350037e2bae3ef3fd07add35a9135ec3748))

- **Breaking (Rust API):** A log record's failure holds `ErrorClass` and
  `ErrorCode`, and a decoded failure wider than 16 bits is refused, as an
  Error PDU's is (#1814). ([a25f68d](https://github.com/jscott3201/rusty-bacnet/commit/a25f68dc99e14845174cc87708c41832cfc0553c))

- The non-router network layer reads a Network-Number-Is through the router
  core's `message::NetworkNumberIs` view, the one parser of that payload;
  data of any length but three octets is still refused (#1818). ([dbe342f](https://github.com/jscott3201/rusty-bacnet/commit/dbe342f949c1ff7840b80fe8c10967906cd42737))

- **CLI:** `register` and `unregister` print "… accepted" on success, without
  the result code, in JSON mode too; a NAK fails the command with exit 1
  (#1822). ([0af6ec6](https://github.com/jscott3201/rusty-bacnet/commit/0af6ec681d2d6fc5de8b34b4a644af08559be116))

- **Breaking (Rust API):** The B/IP BBMD helpers on `BipTransport` and
  `BACnetClient` fail with a typed `BvlcExchangeError` that names a NAK, no
  reply and a local failure apart, and a refused registration, FDT delete
  or BDT write is now an error rather than an `Ok` code (#1822). ([0af6ec6](https://github.com/jscott3201/rusty-bacnet/commit/0af6ec681d2d6fc5de8b34b4a644af08559be116))

- **Breaking (Rust API):** `BipPortMode::Foreign` lends a live
  `ForeignRegistration` view instead of a fixed BBMD and lifetime, and
  `ConfigError` gains `NotForeign` (#1825). ([06a380c](https://github.com/jscott3201/rusty-bacnet/commit/06a380cdaf2faa524d4d4dd169c3715b2193df86))

- **Python API:** With `on_reinitialize` set, a Python foreign device's
  FD_BBMD_Address and FD_Subscription_Lifetime accept writes, applied by
  ACTIVATE_CHANGES or WARMSTART, where they answered WRITE_ACCESS_DENIED
  (#1825). ([06a380c](https://github.com/jscott3201/rusty-bacnet/commit/06a380cdaf2faa524d4d4dd169c3715b2193df86))

- **Breaking (Rust API):** A BACnet/SC dial that runs out of time fails with
  `ScConnectError::DialTimeout`, naming the stalled step, inside
  `Error::Transport` rather than `Error::Timeout`, and a refused WebSocket
  upgrade keeps its HTTP status (#1839). ([2135122](https://github.com/jscott3201/rusty-bacnet/commit/21351229af03c18476ec4cc7435043e5bb3b0709))

- **Python API:** `CovNotificationIterator` counts notifications lost to lag
  in a new `missed` instead of printing to stderr, and ends with "COV
  notifications ended: the client stopped" instead of "channel closed"
  (#1858). ([2135122](https://github.com/jscott3201/rusty-bacnet/commit/21351229af03c18476ec4cc7435043e5bb3b0709))

- **Python API:** event iterators complete concurrent reads one at a time.
  A read held up behind an item whose event loop stopped raises one
  `RuntimeWarning` after 5 seconds, and `missed` also counts items dropped on
  their way to a read, as when that loop is closed (#1858). ([2135122](https://github.com/jscott3201/rusty-bacnet/commit/21351229af03c18476ec4cc7435043e5bb3b0709))

- **Breaking (Rust API):** Lighting Output and Binary Lighting Output take
  Present_Value and Lighting_Command writes only through `write_property_from`,
  which names the writer their Value_Source records (#1866). ([12a6109](https://github.com/jscott3201/rusty-bacnet/commit/12a610922ac914143630259eb61343e3e18c6d82))

- **Breaking (wire):** Event Enrollment's Event_Type follows Event_Parameters;
  the type given to `EventEnrollmentObject::new` (and Python's
  `add_event_enrollment`) now applies only to legacy octet-string parameters
  (#1868). ([12a6109](https://github.com/jscott3201/rusty-bacnet/commit/12a610922ac914143630259eb61343e3e18c6d82))

- Every crate without unsafe code forbids it, bacnet-encoding's tests too;
  each remaining unsafe block needs a safety comment, and
  `bacnet-transport`'s OS calls sit in one private `sys` module behind safe
  wrappers. The Windows UDP control-message parser can no longer wrap
  (#1880, #1881, #1923). ([98fe664](https://github.com/jscott3201/rusty-bacnet/commit/98fe66455bd83ba2e7f3fc0863aea9786a0d142c))

- B/IP and B/IPv6 packet info, receive stamps and the B/IPv6 send go through
  nix's safe socket API on Linux, macOS and the BSDs; other Unix targets keep
  libc. nix trusts the kernel's message lengths (#1882). ([1e7f97a](https://github.com/jscott3201/rusty-bacnet/commit/1e7f97a27a7b0fc36026141eaa66888d12c8afc5))

- The Linux Ethernet transport's AF_PACKET socket goes through socket2 and
  nix with no unsafe code, is close-on-exec, and drops frames queued before
  its bind. A missing interface, or one without a 6-octet MAC (tun,
  WireGuard), fails start with a clear error (#1883). ([88ff4f2](https://github.com/jscott3201/rusty-bacnet/commit/88ff4f2c61ede647878c792be2252a319bdd5f32))

- The TUI's device table shows a routed device as its station in target
  syntax (`2:0x05`), with the router in a Via column when it fits, and sizes
  its columns to their content; the Who-Has panel names the object and sits
  beside the table from 160 columns (#1892, #1915). ([bada8d3](https://github.com/jscott3201/rusty-bacnet/commit/bada8d30a8a4d2273c0c481c961e42e96bb80300))

- The TUI's footer and status bar fit 80 columns by priority and always keep
  `? help` and `q quit`; from 160 columns a long notice shows in full. Help
  reads each screen's key table, so it always matches what the keys do
  (#1894, #1926, #1927). ([3f3ab65](https://github.com/jscott3201/rusty-bacnet/commit/3f3ab65d7797389d4011a34292d5673b5d6bc278))

- **Breaking (Rust API):** MS/TP sorts timer lateness into a fixed histogram
  with its maximum, and `deadline_misses` becomes `late_deadlines`, counted
  from the new `MstpCoreConfig::late_deadline` (1 ms by default, 5 ms in the
  Tokio transport). The TUI shows both (#1897). ([07ca68e](https://github.com/jscott3201/rusty-bacnet/commit/07ca68e8657471fbdb3b70103a6fa15d2e588fb3))

- **Breaking (CLI):** `discover` and `subscribe` print a routed device's
  station in the target syntax, `2:0x05`, so it can be pasted back as a
  target; the JSON keys are now `station` and `routed_source`, and the
  `Remote MAC` table column is now `Station` (#1916). ([d6c84ee](https://github.com/jscott3201/rusty-bacnet/commit/d6c84ee6ed992eb4d7cd759612bbe507640fe517))

- Warnings a peer can set off once per packet or connection (dropped inbound
  NPDUs, undecodable or refused requests and segments, bad SC direct and hub
  connections, a full Ethernet NPDU channel) warn at most once per 10
  seconds with a count; the rest log at debug (#1724, #1950). ([53d4601](https://github.com/jscott3201/rusty-bacnet/commit/53d4601a5a17a75c9a02f7b2203ab7fae75cc46c))

### Removed

- **Breaking (CLI):** `bacnet shell`, its rustyline editor and the
  `devices` command, whose device table only the shell filled, are removed.
  The shell's `writem` is a one-shot command (#974). ([b98aac9](https://github.com/jscott3201/rusty-bacnet/commit/b98aac93d39b1313ff5492a91f29f7335293be41))

### Fixed

- `bacnet capture` filters on the `-p` port rather than always 47808 (#971). ([390f18f](https://github.com/jscott3201/rusty-bacnet/commit/390f18fc5f251f2b7d617848ffe3695808cc406a))

- **Wire:** A Value_Source COV subscription on a Color or Color Temperature
  object that tracks its source now reports Present_Value changes too, and
  each notification carries Present_Value with Value_Source (#1582). ([46d7ed0](https://github.com/jscott3201/rusty-bacnet/commit/46d7ed011c600c622263a7c2dabd6b95a1d397c3))

- `BACnetRouter` follows a BACnet/SC port's NPDU limit through failover and
  reconnect, published by the new `TransportPort::max_npdu_changes()`, and
  counts a send the transport later refuses as too long in
  `egress_too_long` (#1696). ([71f238d](https://github.com/jscott3201/rusty-bacnet/commit/71f238da8f30878b07abdbaf36116c6c7564fcbd))

- Routers send queued traffic in network-priority order, keeping arrival order within each priority while retaining each port's existing queue bound (#1697). ([875aad9](https://github.com/jscott3201/rusty-bacnet/commit/875aad955fd9315787cec2a4d27ecb8a9e9f914f))

- At the 256-route cap, an Initialize-Routing-Table-Ack refreshes the
  networks the router's table holds, as an I-Am-Router-To-Network does; a
  network it doesn't hold is still refused and counted (#1698). ([71f238d](https://github.com/jscott3201/rusty-bacnet/commit/71f238da8f30878b07abdbaf36116c6c7564fcbd))

- Router-local replies now identify the addressed endpoint while returning through the captured ingress port and peer. Segmented responses, cached APDU replay, MS/TP handoff and direct BACnet/SC replies preserve that context and its owner lifetimes (#1699). ([b24829e](https://github.com/jscott3201/rusty-bacnet/commit/b24829eeea1c014465279febb9a90d71f3286cab))

- Routers return a length rejection to an eligible sender when BACnet/SC
  reports a definite length refusal for a queued transmission, including
  frame attributes and direct-peer limits. Rejected replies never trigger
  another reply (#1716). ([a47b0ae](https://github.com/jscott3201/rusty-bacnet/commit/a47b0aed2157e639d0083348e74ed427b8282f5b))

- **Breaking (Rust API):** portable router ports now supply their group-address
  rules, so B/IP and MS/TP group forwarding works with the default host.
  Custom links keep their own classification rules (#1721). ([5ce7592](https://github.com/jscott3201/rusty-bacnet/commit/5ce759270441155b6e784e2626419eb3503365bf))

- The Rust examples in `examples/rust` compile and run again, as Cargo
  examples that CI builds, and the Rust and Python API guides' database-lock
  and engineering-units snippets are corrected (#1723). ([a74e40a](https://github.com/jscott3201/rusty-bacnet/commit/a74e40a91c353d8620f85be92323196ff5c94622))

- **Wire:** the non-router `NetworkLayer` drops a network message whose DNET
  names one network instead of handing it to its control receiver, as it
  already did for APDUs (Clause 6.5.2.2, #1795). ([dbe342f](https://github.com/jscott3201/rusty-bacnet/commit/dbe342f949c1ff7840b80fe8c10967906cd42737))

- A B/IP foreign device whose broadcast address is its own address, as in
  loopback tests, now takes the Forwarded-NPDUs its BBMD sends it instead of
  dropping them as untrusted broadcasts (#1851). ([a9a0cb3](https://github.com/jscott3201/rusty-bacnet/commit/a9a0cb3038be6ac04e91a1be1861512d986ff501))

- **Python API:** SC event, COV, I-Have and network-report iterators no longer
  lose an item when a read is cancelled mid-delivery, as a timed-out
  `wait_for` does: it comes back on the next read, ahead of later items and
  of the end of the iteration (#1858). ([2135122](https://github.com/jscott3201/rusty-bacnet/commit/21351229af03c18476ec4cc7435043e5bb3b0709))

- **Wire:** An Event Enrollment whose Event_Parameters were never set serves
  the `none` alternative, which Python reads as `{"kind": "none"}`. Any
  enrollment with a monitored reference lists and acknowledges its faults,
  and `none` returns an offnormal one to NORMAL (#1868). ([12a6109](https://github.com/jscott3201/rusty-bacnet/commit/12a610922ac914143630259eb61343e3e18c6d82))

- The B/IP local-address lookup and the B/IPv6 interface lookup share one
  `getifaddrs` walk, which reads illumos's 64-bit interface flags, so
  `bacnet-transport` builds for illumos (#1884). ([9f8f41d](https://github.com/jscott3201/rusty-bacnet/commit/9f8f41dc87c6b50a72faf7e81f9a40d8f4ed1fc7))

- The Ethernet transport counts and logs XID and TEST replies that fail to
  go out, exposed as `EthernetTransport::reply_send_failures`, and a send
  that puts only part of a frame on the wire is now an error (#1918). ([5c07f8e](https://github.com/jscott3201/rusty-bacnet/commit/5c07f8ea376b99ae4c8fbf38de5b7b55fc025bc1))

## [0.13.0] - 2026-10-13

### Release highlights

This pre-1.0 release puts `no_std` first. It breaks Rust, Python and CLI
APIs and changes some on-the-wire behaviour, most visibly MS/TP timing,
BACnet/IPv6 addressing and what a router sends. Before moving a v0.12.0
integration, read the
[upgrade guide](https://jscott3201.github.io/rusty-bacnet/project/upgrading/),
which groups what changes by area; the migration notes below give the exact
replacements. Conformance remains partial, and this release is not a
full-conformance or BTL certification claim.

- Six crates publish for the first time, all `no_std` with no allocator:
  `bacnet-time`, `bacnet-mstp-codec`, `bacnet-bip-codec`,
  `bacnet-mstp-core`, `bacnet-bip-core` and `bacnet-router-core`; the three
  cores run on time the caller supplies. The Tokio MS/TP and B/IP transports
  and `BACnetRouter` now run on them, and CI links bare-metal images built
  on them for Cortex-M and RISC-V within flash and RAM budgets.
  `bacnet-types`, `bacnet-encoding` and `bacnet-services` build as `no_std`
  with an allocator through a new `alloc` feature.
- Before that first publish, an API pass gave the six crates common rules:
  `#[non_exhaustive]` errors, counters and snapshots, one naming scheme, wire
  codes as newtypes with named constants, `Debug` on every public type, and
  a README and docs.rs page for each.
- `BACnetRouter` runs on the router core and follows Clause 6 more closely:
  each port sends Network-Number-Is at startup, an NPDU too long for the port
  it would leave by draws reject reason 4, an NPDU for another router or a
  global broadcast that arrives with a hop count of 1 isn't passed on, an
  APDU for the router's own MAC on any of its networks is delivered locally,
  and a reject for a reason other than 1 or 2 no longer removes a learned
  route.
- MS/TP runs on the master core. Extended frames carry NPDUs of up to 1497
  octets, turned on for every MS/TP port a router routes, and host timing
  suits USB adapters: the longest usage and reply waits Clause 9 allows by
  default, a transmit pad for ports that can't drain, and echo handling.
- B/IP runs on the node engine. One `BipMode` picks normal, foreign-device
  or BBMD mode on every Rust builder, Python's B/IP owners take `bbmd=` and
  `foreign_device=`, and the CLI can run as a foreign device; a foreign
  device manages its own registration, a BBMD's tables and forwarding live
  in the core with its two-hop rebroadcast rule corrected, and a registered
  Network Port reports the mode.
- BACnet/IPv6 names a peer by its 3-octet VMAC instead of its address, in
  Rust, Python and the CLI, so stored addresses need rewriting and a router
  must be upgraded together with the nodes it serves.
- The terminal UI (`bacnet tui`) stays behind the opt-in `tui` feature until
  0.14.0.

### Migration notes

- **B/IP modes (Rust API, CLI, #938):** on `BipEndpointBuilder`,
  `register_as_foreign_device(config)` becomes
  `.mode(BipMode::Foreign(config))`, and `enable_bbmd` with the BBMD setters
  becomes `.mode(BipMode::Bbmd(BbmdOptions { .. }))`; the client and server
  builders take the same `mode`. A `ForeignDeviceConfig` literal adds
  `renewal_interval: None`. The CLI's `register` and `discover --bbmd` take
  the global `--ttl`. ([8042aef](https://github.com/jscott3201/rusty-bacnet/commit/8042aef046537d61cc4f46136b0d1d5747632cec))

- **B/IP registration (Rust API, #939):** a custom `TransportPort` that
  overrode `normal_bip_endpoint` overrides `bip_port` instead, returning a
  `BipPort` with the endpoint and a `BipPortMode`; a wrapper delegates it.
  Lock `bbmd_state()` with `.lock()` rather than `.lock().await`, never
  across an await; it returns a `LockResult`, so `unwrap()` it or take
  `into_inner` on poison. Drop `.await` from `fdt_counters()`.
  `ObjectDatabase::publish_bip_port_internal` takes the mode as a fifth
  argument. ([303ef1d](https://github.com/jscott3201/rusty-bacnet/commit/303ef1db1bd3b612594bde3f0a1213ddbc76741d))

- **Access Point (Rust API, #1132):** replace
  `set_access_event(event, tag, time, credential)` with
  `set_access_event(AccessEventReport { time: Some(time), credential,
  ..AccessEventReport::new(event, tag) })`; the report's
  `authentication_factor` sets Access_Event_Authentication_Factor. A wrapper
  that forwards `BACnetObject` methods forwards `report_access_input_internal`
  too. ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **ReinitializeDevice (Rust API, #1138):** exhaustive `ServerConfig` literals
  need `on_reinitialize: None`. Callers of `handle_reinitialize_device` get the
  decoded `ReinitializedState` back in place of `()`. ([1eb630f](https://github.com/jscott3201/rusty-bacnet/commit/1eb630fe74cbc21c1b2cad11e9366a2350cd812d))

- **Access Point (Rust API, #1325):**
  `set_number_of_authentication_policies` no longer refuses a count below
  Active_Authentication_Policy: it drops the active policy to 0, and the
  point reports CONFIGURATION_ERROR until a client writes a usable policy.
  Code that relied on the refusal should compare the count with the active
  policy first. ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Channel values (Rust API, #1474):** give `MemberDatatype::of` the
  member's object type first, and match its new `ColorCommand` and `XyColor`.
  Replace `is_lighting_command_channel_value(octets)` with
  `constructed_channel_value(octets) == Some(ConstructedChannelValue::LightingCommand)`. ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Color and Color Temperature rows (wire, #1474):** Status_Flags,
  Event_State, Reliability and Out_Of_Service now answer UNKNOWN_PROPERTY, so
  a client that read or wrote them should stop, and a whole-object COV report
  carries Present_Value alone. Default_Fade_Time starts at 100 ms and refuses
  a value outside 100 to 86,400,000 ms. ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Color objects (Rust API, #1474):** call
  `ColorObject::set_present_value(BACnetXyColor::new(x, y))`, and handle the
  `Result` it and `ColorTemperatureObject`'s `set_present_value` and
  `set_min_max` return: a value outside the object's range is refused. ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Endpoint ingress (Rust API, #1504):** `PolicyReason` has a new
  `GroupSource` variant; an exhaustive `match` on it needs an arm for it. ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **Life Safety rearm (Rust API, #1520):** await `set_life_safety_operation_expected_local` inside a
  Tokio runtime, like the other local writes (#1367). Polled by another executor, it now fails with
  `Error::Encoding` before anything changes. ([9872f34](https://github.com/jscott3201/rusty-bacnet/commit/9872f3408e534639604d0452de86b9f764acd91f))

- **Endpoint ReadRange (Rust API, #1531):** `EndpointReadRequest::Range`
  takes a `ReadRangeValidation`, and `EndpointReadAck::Range` and
  `into_range` carry a `ReadRangeReply`. ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **ReadRange (Rust API, #1531):** a ReadRange answer that breaks a rule fails
  with `Error::ReadRangeViolation(rule)`, not `Error::Decoding`; match it where
  you match `Error` exhaustively. Strict now also refuses pages it accepted,
  such as bacnet-stack 1.6.1's backward By-Position and By-Time pages (FIRST_ITEM
  with MORE_ITEMS); read such devices with `ReadRangeValidation::Lenient`. ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **ClientConfig and Error (Rust API, #1535, #1530):** a `ClientConfig`
  literal needs `min_request_interval_ms` (or `..ClientConfig::default()`),
  and an exhaustive match on `Error` needs an arm for `LogNotAdvancing`. ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **Log buffers (Rust API, #1536):** a custom `LogBufferRecords` implements
  `record_identity(index)`, the identity `log_record_identities_internal`
  lists at that index, and overrides `record_position` and `timestamp_order`
  when it can answer them without walking its records. A custom
  `AuditLogStorage` keeps its sequence numbers consecutive from the oldest
  record. ([f8ad451](https://github.com/jscott3201/rusty-bacnet/commit/f8ad4517a8e16be58536802b6fe3b192e7950800))

- **Share port by address (Rust API, #1538):** exhaustive `ServerConfig` and
  `ClientConfig` literals need `share_port_by_address: false`. ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **SessionConfig (Rust API, #1542):** a `SessionConfig` literal needs
  `min_request_interval_ms` (or `..SessionConfig::default()`). ([4cbe830](https://github.com/jscott3201/rusty-bacnet/commit/4cbe8305cf16e1ca0fbab21ea61e5276df5aef21))

- **Segmentation (Rust/Python API, #1593):** Add `DeviceConfig::apdu_segment_timeout` to full literals and match it and segmentation mode to `ServerConfig`. Narrow endpoints require `NONE`. Python full servers add `segmentation_supported` and `apdu_segment_timeout_ms`; defaults remain unsegmented. See [server timing](docs/server-segmentation.md). ([a321f74](https://github.com/jscott3201/rusty-bacnet/commit/a321f746cc5a5979a1779655d7e54e88b35dcb48))

- **BVLL (Rust API, #1604):** import `BvlcFunction` and `BvlcResultCode`
  from `bacnet_transport::bvll` or `bacnet_bip_codec`; `SECURE_BVLL` is gone.
  `encode_bvll` takes a `&Bvll` (`encode_bvll_forwarded` is
  `Bvll::ForwardedNpdu`). `BVLC_TYPE_BACNET_IP`, `BVLL_HEADER_LENGTH`,
  `FORWARDED_ADDR_LENGTH` and `bbmd::BDT_ENTRY_SIZE`/`FDT_ENTRY_SIZE` become
  `BVLC_TYPE_BIP`, `HEADER_LENGTH`, `BIP_ADDRESS_LENGTH` and
  `BDT_ENTRY_LENGTH`/`FDT_ENTRY_LENGTH` in `bvll`. ([bdac34b](https://github.com/jscott3201/rusty-bacnet/commit/bdac34b5d468c8a6e4fc9b287be5c4e2cc34bf1d))

- **BvllMessage (Rust API, #1604):** `payload` is now everything after the
  header, so a Forwarded-NPDU's starts with the 6-octet origin: code that
  read the NPDU from `.payload` still compiles but gets 6 extra octets; use
  `npdu()`. `originating_ip`/`originating_port` become `origin()`.
  `decode_bvll` refuses a datagram whose Length field isn't its length. ([bdac34b](https://github.com/jscott3201/rusty-bacnet/commit/bdac34b5d468c8a6e4fc9b287be5c4e2cc34bf1d))

- **no_std crates (Rust API, #1604):** a dependency on `bacnet-types`,
  `bacnet-encoding` or `bacnet-services` with `default-features = false` must
  now turn on `alloc` (no_std) or `std`; with neither, the crate refuses to
  build. `serde` on `bacnet-types` turns on `alloc`. `bacnet_types::bip_port`
  and `Error::Transport` need `std`, and a `match` on
  `bacnet_types::error::Error` needs a wildcard arm. ([5bee59d](https://github.com/jscott3201/rusty-bacnet/commit/5bee59dd215f623d5990deb343e528d9218ce6a9))

- **MS/TP diagnostics (Rust API, #1608):** for the removed snapshot fields
  read `mac.npdus_delivered` and `mac.frames_received` (`der_rx`, `dner_rx`,
  `reply_postponed_rx`), `mac.parser_errors` (`invalid_frame_discards`) and
  `mac.frame_aborts` (`stale_partial_resets`). `wait_for_reply_timeouts` no
  longer counts waits an invalid frame ended, and `serial_write_errors` also
  counts drain errors; either aborts the frame. ([3d76be6](https://github.com/jscott3201/rusty-bacnet/commit/3d76be6c22a41d6fee4c8328f1f5b7d34457139f))

- **MS/TP transport (Rust API, #1608, #1611, #1654, #1655):** a full
  `MstpConfig` literal adds `extended_frames`, `usage_timeout`,
  `reply_timeout` and `transmit_latency_pad`, or ends with
  `..MstpConfig::default()`. `start()` and the new `validate()` refuse
  `max_info_frames` 0, baud rates below 2667, timeouts outside 20-35 ms
  (usage) or 255-300 ms (reply), and a pad over 15 ms. 0.12 waited 20 ms
  after a token, 10 after a poll, 255 for a reply. Declare an echoing
  adapter: `with_echo(Echo::full())`. ([3d76be6](https://github.com/jscott3201/rusty-bacnet/commit/3d76be6c22a41d6fee4c8328f1f5b7d34457139f))

- **MS/TP routing (Rust API, #1611):** a `TransportPort` wrapper forwards
  the new `prepare_for_routing`, which `BACnetRouter` calls to turn on
  extended frames on each MS/TP port it routes, so a router's MS/TP ports
  now declare an APDU limit of 1476. ([30da0ea](https://github.com/jscott3201/rusty-bacnet/commit/30da0ea31da2ee73bc022bc425659a00aaa6610a))

- **B/IPv6 BVLL (Rust API, #1621):** import `Bvlc6Function` and
  `Bvlc6ResultCode` from `bacnet_transport::bip6` or `bacnet_bip_codec::ipv6`;
  the transport's enum variants become constants (`ForwardedNpdu` is
  `FORWARDED_NPDU`) and `from_byte`/`to_byte` become `from_raw`/`to_raw`.
  `Bip6Vmac` is a newtype: `Bip6Vmac::new([..])` and `octets()`. ([814d328](https://github.com/jscott3201/rusty-bacnet/commit/814d328fd605846042db6198c90b9cf6e082a7b8))

- **B/IPv6 frames (Rust API, #1621):** `Bvlc6Frame`, `decode_bvlc6`,
  `encode_bvlc6` and the per-message `encode_*` helpers become `Bvll6` values
  with `decode_bvll6`/`encode_bvll6`; a Forwarded-NPDU's address and NPDU
  come from `origin()`/`npdu()`, not `decode_forwarded_npdu_payload`.
  `BVLC6_TYPE` is `BVLC_TYPE_BIP6`, and the two header-length constants are
  gone. ([814d328](https://github.com/jscott3201/rusty-bacnet/commit/814d328fd605846042db6198c90b9cf6e082a7b8))

- **B/IP foreign devices (Rust API, #1622):** In foreign-device mode,
  `register_foreign_device_bvlc`, `delete_fdt_entry` and `write_bdt` to the
  transport's own BBMD fail, since the transport manages that registration.
  Send them from a transport in normal mode, as the CLI does. A foreign
  device's BBMD must be a unicast address with a nonzero port, other than the
  transport's own. ([8a5bd45](https://github.com/jscott3201/rusty-bacnet/commit/8a5bd4584deba17a50ee5efde0d6e1530dd6effa))

- **B/IP modes (Rust API, #1622):** A `BipTransport` can't call both
  `enable_bbmd` and `register_as_foreign_device`: `start()` fails. Run the
  BBMD and the foreign device on separate transports (another port or
  interface), or drop the one you don't need. ([8a5bd45](https://github.com/jscott3201/rusty-bacnet/commit/8a5bd4584deba17a50ee5efde0d6e1530dd6effa))

- **BbmdState (Rust API, #1623, #1669):** `bdt()` and `fdt()` return
  `Vec`s. Replace `encode_bdt(&mut buf)` with
  `bbmd::encode_bdt_entries(&state.bdt(), &mut buf)`, and `encode_fdt` by
  appending each `fdt_wire_entries()` row's `to_octets()`. In place of
  `forwarding_targets`, `fdt_forwarding_targets`, their `_at` forms and
  `is_registered_foreign_device`, build `bacnet_bip_core::bbmd` tables from
  the rows and ask those. `forwarded_npdu_needs_local_broadcast` goes by
  this BBMD's own BDT row. ([7ac93a6](https://github.com/jscott3201/rusty-bacnet/commit/7ac93a65a14b8f45e59bfac01652936f6b7129f1))

- **B/IPv6 MAC (Rust API, wire, #1637):** Pass a peer's 3-octet VMAC wherever
  an 18-octet B/IPv6 MAC went: sends, client requests, recipient lists,
  bindings and stored COV subscriptions. Find a VMAC from an address with
  `Bip6Transport::resolve_virtual_address`, and read this node's address with
  `local_address()`. Upgrade a router together with the B/IPv6 nodes it serves. ([b412923](https://github.com/jscott3201/rusty-bacnet/commit/b41292375d9432f7ab05296a3abb7cea2cb0b439))

- **MS/TP serial ports (Rust API, #1656):** a custom `SerialPort::read` must
  wait until at least one octet has arrived and return `Ok(0)` only at end of
  file, never to mean "nothing yet" (on a timeout, say). The transport now
  takes three `Ok(0)` reads in a row as the port gone and stops, as it does
  at a read error. ([c195838](https://github.com/jscott3201/rusty-bacnet/commit/c195838818232ab70aa312c0c2a6ef34194445fb))

- **Router egress limits (wire, #1665):** `BACnetRouter` now refuses, with
  reject reason 4, an NPDU longer than its egress port's
  `TransportPort::max_npdu()`, read when the port starts; the default is
  1,497 octets. A custom transport that carries more, or less, must override
  it, and a wrapper around an MS/TP or BACnet/SC transport must delegate it.
  A BACnet/SC port keeps the limit read at start through a failover (#1696). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **B/IPv6 VMACs from device instances (Python API, CLI, #1671):** An ipv6
  `BACnetServer`'s VMAC is its device instance, so `start()` fails when
  another node on its link and port uses that instance. A client without
  `ipv6_device_instance` (Python) or `--device-instance` (CLI) picks a random
  VMAC, which needs a multicast-capable link at start: give it an instance on
  Linux loopback or any link without multicast. ([f13f8ca](https://github.com/jscott3201/rusty-bacnet/commit/f13f8ca797dd45ec834a8fa40ae7aa0183198ccf))

- **B/IPv6 addresses (Python API, CLI, #1671):** Replace `'[ip]:port'` address
  strings with `bip6:<device instance>`, `bip6:0x<6 hex digits>` or the result
  of `await client.resolve_vmac('[ip]:port')`. An ipv6 server's
  `local_address()` now returns `bip6:0x…`. `DiscoveredDevice.mac_address`
  and COV `source_mac`/`source_address` are 3 octets on B/IPv6. Rewrite stored
  18-octet B/IPv6 recipients as the peer's VMAC, and replace
  `encode_bip6_mac`/`decode_bip6_mac` with `Bip6Address`. ([f13f8ca](https://github.com/jscott3201/rusty-bacnet/commit/f13f8ca797dd45ec834a8fa40ae7aa0183198ccf))

- **Group addresses (Rust API, #1677):** B/IP `send_unicast` to a group
  address fails: broadcast instead. `NetworkLayer::send_apdu` does so for an
  Unconfirmed-Request when the transport gives `group_destinations()`, which
  a custom transport implements. The router drops a network message for a
  group DADR. ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- **Router table (Rust API, #1679):** drop the `.lock().await` after
  `router.table()`, and don't hold the guard across an await or call
  `table()` again while it lives. `RouteEntry`, `ReachabilityStatus`,
  `ConvergenceMode` and `RoutingClaimSnapshot` are `Route`, `Reachability`,
  `Convergence` and `ClaimCounters` (`claim_counters()`). Calls that take a
  time take `router.now()`. `add_learned` and `add_direct` return `bool`,
  refusing a new network once the table holds 256 routes (`ROUTE_CAP`). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- **Router startup (wire, #1679):** Each `BACnetRouter` port now sends one
  more message at startup, a Network-Number-Is, ahead of its
  I-Am-Router-To-Network. A test or tool that reads a port's first message
  as the I-Am-Router should skip it, and devices on the port learn its
  network number from it. ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **BACnet/SC connection (Rust API, #1679):** Rename
  `ScConnection::hub_max_apdu_length` to `hub_max_npdu_length`; its value is
  unchanged. ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **MS/TP frames (Rust API, #1608, #1684):** `bacnet_transport::mstp_frame`
  is gone: use `bacnet_transport::mstp::codec`, the `bacnet-mstp-codec`
  crate. `FrameType`'s variants are constants (`Token` is `TOKEN`,
  `Unknown(raw)` is `from_raw(raw)`), so a `match` needs a `_` arm.
  `MstpFrame` and `encode_frame` become `Frame::new(..).encode_into`,
  `decode_frame_stream` a `StreamDecoder`, and the CRC functions move to
  `codec::crc`, without the `*_accumulate_all` functions and `*_RESIDUAL`
  constants. ([dc50aed](https://github.com/jscott3201/rusty-bacnet/commit/dc50aedc2a673eb835c0295d2adbfa9f564e0675))

- **Router routes (Rust API, #1684):** read a `Route` through `port()`,
  `is_direct()`, `next_hop()` and its other getters; `lookup` returns a
  copy, ports are `PortId`s and MACs `Mac`s. `lookup_mut` is gone: change a
  route with `add_learned`, `add_direct`, `mark_busy`, `mark_available`,
  `mark_unreachable` and `touch`. The `_at` and flap-detection helpers,
  `purge_stale`, `expire_pending_at` and the listings changed too: see
  [moving a router table](docs/rust-api.md#moving-a-router-table-from-012). ([6fa08e9](https://github.com/jscott3201/rusty-bacnet/commit/6fa08e926ceb3b3a5802df090a41efce1d0d7e0a))

### Added

- **Rust API:** `BipTransport::bvlc_client_snapshot()` reports the
  transport's own BVLC requests and its foreign registration: the lease
  its BBMD's successes prove, the next attempt, lapses, held broadcasts.
  `ForeignDeviceConfig::renewal_interval` sets the renewal, half the TTL by
  default (#1013). ([a2d84a9](https://github.com/jscott3201/rusty-bacnet/commit/a2d84a987fe1cfc35011ef2d61934532b561c105))

- **Rust and Python API, CLI:** the B/IP client and server builders take
  `.mode(BipMode::..)` and report the foreign registration; Python's B/IP
  owners take `bbmd=` and `foreign_device=`, and the CLI `--foreign-bbmd`,
  `--ttl` and `--renewal` (#938). ([8042aef](https://github.com/jscott3201/rusty-bacnet/commit/8042aef046537d61cc4f46136b0d1d5747632cec))

- **Wire:** A registered Network Port now follows its B/IP transport's mode:
  a BBMD serves its live BDT, FDT and Accept-FD-Registrations, and a foreign
  device its BBMD address and subscription lifetime, all read-only for now
  (#939). ([303ef1d](https://github.com/jscott3201/rusty-bacnet/commit/303ef1db1bd3b612594bde3f0a1213ddbc76741d))

- **Rust API:** `BACnetHostAddress`, `BACnetHostNPort`, `BACnetBDTEntry` and
  `BACnetFDTEntry` in `bacnet_types::constructed`, with strict codecs in
  `bacnet_encoding::constructed` (#939). ([303ef1d](https://github.com/jscott3201/rusty-bacnet/commit/303ef1db1bd3b612594bde3f0a1213ddbc76741d))

- **Rust API:** `BACnetServer` gains `report_access_event_local`,
  `report_credential_read_local` and `report_door_state_local`, which take an
  access event, a reader's read or a door's hardware state as one local write
  that COV and event reporting follow, with each object's out-of-service rule
  (#1132). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Python API:** `BACnetServer.report_access_event_local`,
  `report_credential_read_local` and `report_door_state_local` report an
  Access Point's events, a Credential Data Input's reads and an Access Door's
  hardware state to a running server (#1132). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- ReinitializeDevice, which restarts a device or steps it through backup and
  restore, can be carried out by an application handler on `BACnetServer` and
  endpoint sessions; without one it is still refused (#1138). ([1eb630f](https://github.com/jscott3201/rusty-bacnet/commit/1eb630fe74cbc21c1b2cad11e9366a2350cd812d))

- **Wire:** Access Point serves Authentication_Policy_List and
  Authentication_Policy_Names once the application sets them with
  `AccessPointObject::set_authentication_policies`, and bacnet-types gains
  `BACnetAuthenticationPolicy` with its codec (#1325). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Python API:** `add_access_point` takes `authentication_policies` as
  `(name, policy)` pairs, and a read of Authentication_Policy_List gives each
  policy in the form that keyword takes (#1325). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Rust API:** `bacnet_objects::access_control::evaluate_access_rights`
  checks an Access Credential against its assigned Access Rights at an
  Access Point and reports the grant, exemption or denial with its
  Access_Event, without writing anything (#1331). ([abf50e5](https://github.com/jscott3201/rusty-bacnet/commit/abf50e5fcb0d560842370e1443149bad277b9c54))

- **Wire:** Access Credential serves Authorization_Exemptions once the
  application sets it with
  `AccessCredentialObject::set_authorization_exemptions` (#1331). ([abf50e5](https://github.com/jscott3201/rusty-bacnet/commit/abf50e5fcb0d560842370e1443149bad277b9c54))

- **Wire:** A Channel's Present_Value and WriteGroup take the addendum's xy
  colour and colour command values, and a Channel passes each only to a
  member of its own datatype (#1474). ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Wire:** Color and Color Temperature carry out their Color_Command fades,
  ramps, steps and STOP, sampling Tracking_Value for COV as it moves, and a
  Present_Value write moves as their Transition says (#1474). ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Rust API:** `ColorOperationInProgress` and `ColorTransition` in
  bacnet-types, and `ConstructedChannelValue` with
  `constructed_channel_value` in `bacnet_encoding::constructed` (#1474). ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Python API:** `BACnetClient.write_group` sends an xy colour or colour
  command channel value, given as its framed octets (#1474). ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Rust API:** `bacnet_server::server::drop_database_off_runtime` lets go of
  a handle on the object database, dropping it on Tokio's blocking pool when
  it is the last, so durable objects' final saves don't hold a runtime worker
  (#1513). ([ba8245e](https://github.com/jscott3201/rusty-bacnet/commit/ba8245e1fc12b2c4141d05c9c3fd87a16ea93690))

- Color and Color Temperature serve the optional Audit_Level and
  Auditable_Operations rows once `set_audit_policy` provisions them, and
  their writes are audited by that policy; the other optional rows are
  documented as absent (#1525). ([d94bd2e](https://github.com/jscott3201/rusty-bacnet/commit/d94bd2ecefcacbc1fc801acb993b63ce40a78fb9))

- Both lighting output types take Color_Reference,
  Color_Override and Override_Color_Reference (Addendum 135-2020ca part 4),
  and `ObjectDatabase::lighting_color` follows them to the colour an output
  shows (#1527). ([d94bd2e](https://github.com/jscott3201/rusty-bacnet/commit/d94bd2ecefcacbc1fc801acb993b63ce40a78fb9))

- Lighting Output takes High_End_Trim, Low_End_Trim and Trim_Fade_Time
  (Addendum 135-2020ca part 5): Tracking_Value is held between the trims,
  In_Progress reads the new TRIM_ACTIVE meanwhile, and a trim change fades in
  over Trim_Fade_Time (#1528). ([d94bd2e](https://github.com/jscott3201/rusty-bacnet/commit/d94bd2ecefcacbc1fc801acb993b63ce40a78fb9))

- **Rust API:** `read_log_page` on the client and endpoint client pages a
  log's records from the oldest, a sequence number, a position or a time,
  across the sequence wrap, reporting gaps and returning a checkpoint cursor;
  a device whose pages stop advancing fails with `Error::LogNotAdvancing`
  (#1530). ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **Python API:** `read_log_page` pages a log by cursor, `decode_log_records`
  types records, `read_range(validation="lenient")` keeps a page that breaks a
  rule (strict raises `BacnetReadRangeViolationError`), and
  `BACnetClient(min_request_interval_ms=...)` paces requests (#1530, #1534,
  #1531, #1535). ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **Breaking (Rust API):** `read_range_with` and `ReadRangeValidation::Lenient`
  keep a ReadRange page that breaks a rule, listing the rules it broke; a
  strict refusal is now `Error::ReadRangeViolation` naming the rule, and also
  catches contradictory result flags and a count overrun (#1531). ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **CLI:** `read-range` takes `--position`, `--sequence` or `--time` with
  `--count`, shows result flags, first sequence number and broken rules
  (`--strict` refuses), and pages a log with `--all`, printing what it read
  before an error; `--min-interval-ms` paces requests (#1532). ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **Python API:** `read_range` takes `range_type="time"` with a
  `reference_time`, a naive `datetime` in the device's local time or a BACnet
  date and time pair, on the client and the endpoint (#1533). ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **Rust API:** `ReadRangeAck` decodes its item data as Trend Log, Event Log,
  Trend Log Multiple or Audit Log records, or names the offset of the first
  record that fails, keeping the ones before it (#1534). ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **Breaking (Rust API):** `BACnetClient` builders and `ClientConfig` take
  `min_request_interval_ms` (default 0, at most an hour): a request to a
  destination waits that long after the latest one sent there finished, so a
  slow device keeps room for its other clients (#1535). ([5eb9e32](https://github.com/jscott3201/rusty-bacnet/commit/5eb9e32cc9f5c09b7de733708310a5a47a245292))

- **Python API:** `add_trend_log`, `add_trend_log_multiple` and
  `add_event_log` take a keyword-only `total_record_count` that seeds the
  log's count, so its records can be numbered across the Unsigned32 wrap
  (#1537). ([f8ad451](https://github.com/jscott3201/rusty-bacnet/commit/f8ad4517a8e16be58536802b6fe3b192e7950800))

- Trend Log, Trend Log Multiple and Event Log objects restore their records
  and Total_Record_Count before start-up with a validated
  `restore_log_buffer`, which can also seed the count alone, and mark the
  restart with a LOG_INTERRUPTED record from `record_interruption` (#1537). ([f8ad451](https://github.com/jscott3201/rusty-bacnet/commit/f8ad4517a8e16be58536802b6fe3b192e7950800))

- **Rust API:** `BipTransport::check_bind` binds and releases the sockets
  `start()` would. A Python `BipEndpoint` sharing its port by address checks
  with it before starting (#1538). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **Rust and Python API:** a device sharing its port by address fails to start
  unless its broadcast address is its subnet broadcast or 255.255.255.255, and
  keeps only the broadcasts that arrived on its own interface (#1538). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **Rust and Python API:** `share_port_by_address` lets a B/IP device bind its
  interface address, so devices on one host share a port. It then sends from
  that address, gets broadcasts and unicast in no fixed order, and on Windows
  claims the address ([details](docs/rust-api.md#sharing-a-port-by-address), #1538). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **Breaking (Rust API):** the endpoint client paces confirmed requests to each
  device as `BACnetClient` does, through `SessionConfig::min_request_interval_ms`
  and the endpoint builders' `min_request_interval_ms` (default 0, at most an
  hour) (#1542). ([4cbe830](https://github.com/jscott3201/rusty-bacnet/commit/4cbe8305cf16e1ca0fbab21ea61e5276df5aef21))

- **Python API:** `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take a
  keyword-only `min_request_interval_ms`, validated and applied to their
  `EndpointClient` as `BACnetClient`'s keyword is (#1542). ([4cbe830](https://github.com/jscott3201/rusty-bacnet/commit/4cbe8305cf16e1ca0fbab21ea61e5276df5aef21))

- Color, Color Temperature and the noncommandable Analog, Binary and
  Multi-state Values serve Value_Source once `set_value_source_tracking`
  turns it on: the last writer of Present_Value. A Value's COV report of it
  carries Present_Value and Status_Flags and follows the value's criterion
  (#1552, #1582). ([ea578d5](https://github.com/jscott3201/rusty-bacnet/commit/ea578d5fd468637b19a0527238d5622aca8d158b))

- **Rust API:** `BACnetObject::set_present_value_from_internal` hands an
  application Present_Value update the local Device it comes from, which
  `set_present_value_local` now passes; object decorators should forward it
  (#1552). ([ea578d5](https://github.com/jscott3201/rusty-bacnet/commit/ea578d5fd468637b19a0527238d5622aca8d158b))

- **Rust API:** `BACnetNameValue` in `bacnet_types::constructed`, a tag's
  name and at most one primitive value, so a date and a time are two tags,
  with `encode_name_value` and `decode_name_value` in
  `bacnet_encoding::constructed` (#1553, #1583). ([ea578d5](https://github.com/jscott3201/rusty-bacnet/commit/ea578d5fd468637b19a0527238d5622aca8d158b))

- Color, Color Temperature, Lighting Output and Binary Lighting Output serve
  Tags, Profile_Location and Profile_Name once `set_profile` provisions them;
  Tags takes array writes and the profile rows are read-only (#1553). ([ea578d5](https://github.com/jscott3201/rusty-bacnet/commit/ea578d5fd468637b19a0527238d5622aca8d158b))

- Color, Color Temperature, Lighting Output and Binary Lighting Output support
  optional Tags persistence, including indexed writes and saved-empty overrides.
  Failed saves leave served Tags unchanged; clones retain data without storage
  ownership. Attached snapshots are limited to 1 MiB (#1583). ([b3e7e19](https://github.com/jscott3201/rusty-bacnet/commit/b3e7e191872a909d88acd76f3cee7b4b86e661c2))

- Device objects support optional Tags and profile text with opt-in Tags persistence.
  Full-server reads retain live Device views; endpoint responders keep their existing
  Device write allowlist and refuse Tags writes (#1584). ([6b4169d](https://github.com/jscott3201/rusty-bacnet/commit/6b4169de89d0fae7257814c64100b7ae84b92147))

- Analog, Binary and Multi-state Input objects support optional Tags and profile
  rows with shared opt-in Tags persistence. Profile writes remain independent of
  Out_Of_Service while Present_Value simulation ownership is preserved (#1584). ([0a16278](https://github.com/jscott3201/rusty-bacnet/commit/0a16278ad2d465f2fb8ccb96d1c8a7b8ddbd7be0))

- Analog, Binary and Multi-state Output objects support optional Tags and profile
  rows with shared opt-in Tags persistence. Profile writes preserve command
  priorities, source tracking, feedback and event behavior (#1584). ([a9295bd](https://github.com/jscott3201/rusty-bacnet/commit/a9295bdc3917b5abca2d34a3c5e4bb17372f0b2c))

- Rust clients can decode Tags reads as whole arrays, elements or counts.
  Python returns named typed values and the CLI displays names and values,
  preserving semantic tags, valued NULL and existing raw fallbacks (#1584). ([5e82605](https://github.com/jscott3201/rusty-bacnet/commit/5e826055ed65f994794036052e4cc0d541fe00af))

- Analog, Binary and Multi-state Value objects accept optional Tags and profile
  rows in every Present_Value mode, with shared opt-in Tags persistence across
  reconstruction and mode-independent metadata and PICS (#1584). ([0a08074](https://github.com/jscott3201/rusty-bacnet/commit/0a08074c738ef2a5784bd7943f6a63ec39490bce))

- New `bacnet-bip-codec` crate: an allocation-free `no_std` codec for every
  B/IP (Annex J) and B/IPv6 (Annex U) BVLL message, with typed addresses,
  VMACs and BDT/FDT views; `alloc`, `bytes` and `heapless` add encode sinks.
  The B/IP and B/IPv6 transports parse and encode BVLL with it (#1604,
  #1621). ([bdac34b](https://github.com/jscott3201/rusty-bacnet/commit/bdac34b5d468c8a6e4fc9b287be5c4e2cc34bf1d))

- New `bacnet-mstp-codec` crate: an allocation-free `no_std` codec for
  standard and extended MS/TP frames, with a stream decoder; `alloc`,
  `bytes` and `heapless` add sinks, `crc32k-table` a faster CRC-32K. The
  MS/TP transport re-exports it (`mstp::codec`). By Andrew Rodgers, ACE IoT
  Solutions (#1604). ([8d0a29f](https://github.com/jscott3201/rusty-bacnet/commit/8d0a29f10c667567155a800fd2260227aaaebac2))

- New `bacnet-mstp-core` crate: a sans-IO `no_std` MS/TP master node
  (Clause 9) on caller-supplied time, with bounded queues, typed actions for
  an adapter and optional extended frames; `alloc` adds heap-backed queues.
  The MS/TP transport runs on it. By Andrew Rodgers (ACE IoT Solutions)
  (#1605). ([4aeb3db](https://github.com/jscott3201/rusty-bacnet/commit/4aeb3dbb127713c142848c8adb9f5022e4e55d2c))

- Test-only: `bacnet-mstp-sim`, an unpublished crate that runs
  `bacnet-mstp-core` masters on a simulated RS-485 bus in virtual time, with
  Clause 9 token-ring scenarios, masters with and without extended frames
  on one bus, and randomized fault runs as tests (#1605, #1611). ([e746c89](https://github.com/jscott3201/rusty-bacnet/commit/e746c89ef947ce0df30a136ec5e3ce0fd79d326d))

- Test-only: fuzz targets for the MS/TP and B/IP codecs (every decoder and
  encode sink) and the MS/TP master core, checked against token custody,
  frame and poll limits and the adapter contract after every step, run by
  a workflow on codec PRs and twice a week on a pinned nightly (#1610,
  #1640). ([6faa8e3](https://github.com/jscott3201/rusty-bacnet/commit/6faa8e3de5ebe8932ad5fcf3a7be72debe710239))

- **Wire:** `MstpConfig::extended_frames` (off by default) lets the MS/TP
  transport send and take NPDUs of up to 1497 octets, APDUs of 1476, in
  extended frames, and `BACnetRouter` turns it on for every MS/TP port it
  routes, as Clause 9.3 requires of routers (#1611). ([30da0ea](https://github.com/jscott3201/rusty-bacnet/commit/30da0ea31da2ee73bc022bc425659a00aaa6610a))

- **Python API:** `BACnetClient` and `BACnetServer` take
  `mstp_extended_frames` (default False) to send and take MS/TP NPDUs of up
  to 1497 octets in extended frames (#1611). ([30da0ea](https://github.com/jscott3201/rusty-bacnet/commit/30da0ea31da2ee73bc022bc425659a00aaa6610a))

- Test-only: CI runs the `no_std` crates' tests and doctests with `alloc` and
  without `std`, checks those builds at the MSRV, and lints `bacnet-encoding`,
  `bacnet-services` and the codecs' `bytes` feature on ARMv6-M too (#1613). ([da229d0](https://github.com/jscott3201/rusty-bacnet/commit/da229d0505446a8056f254abee2baf62f5a03abb))

- New `bacnet-bip-core` crate (no features): B/IP (Annex J) as `no_std`
  decisions on caller-supplied time with no allocator: a node engine for
  normal, foreign-device and BBMD modes, registration included, and a BBMD's
  tables and forwarding in caller-sized arrays. The B/IP transport runs on
  it (#1623). ([7ac93a6](https://github.com/jscott3201/rusty-bacnet/commit/7ac93a65a14b8f45e59bfac01652936f6b7129f1))

- `BipTransport::receive_counters()` counts the datagrams the B/IP receive
  path drops, by reason: bad headers and payloads, unknown functions,
  destination mismatches, rate-limited and unanswerable requests, untrusted
  forwards, own echoes, a full or closed NPDU channel and socket errors
  (#1622, #1624). ([3bfe3ff](https://github.com/jscott3201/rusty-bacnet/commit/3bfe3ff5e7a7b802104f7cb2687350c0769374b8))

- Test-only: CI links bare-metal images (MS/TP, standard and extended; the
  B/IP codec, a foreign device and a BBMD; a B/IP-MS/TP router; the alloc
  crates) for Cortex-M0/M3/M4F and RV32IMC, checks flash and RAM against
  budgets, and runs each scenario natively (#1626, #1635, #1660, #1664,
  #1667, #1679). ([f0f6cef](https://github.com/jscott3201/rusty-bacnet/commit/f0f6cef0338807472522f888db9a28a3b2bd79a8))

- **Wire:** A B/IPv6 unicast to an unknown VMAC sends Address-Resolution
  (multicast, or to the BBMD as a foreign device) and waits up to 3 s, or queues
  on a router port; `Bip6Transport::resolve_virtual_address` finds a VMAC from
  an IPv6 address (#1637). ([b412923](https://github.com/jscott3201/rusty-bacnet/commit/b41292375d9432f7ab05296a3abb7cea2cb0b439))

- **Rust API:** `BACnetServer::transport()`,
  `bip6::vmac_from_device_instance`, and
  `Bip6Transport::resolution_counters()`, which counts address resolution
  as `Bip6ResolutionCounters` (#1637). ([b412923](https://github.com/jscott3201/rusty-bacnet/commit/b41292375d9432f7ab05296a3abb7cea2cb0b439))

- `Bip6Transport::receive_counters()` counts the datagrams the B/IPv6 receive
  path drops, by reason, frames from its own VMAC and from a source no reply
  could reach included, and the replies its quotas hold back, as
  `BipTransport::receive_counters()` does for B/IP (#1637, #1643). ([4a97626](https://github.com/jscott3201/rusty-bacnet/commit/4a97626191a7709438a70880372fb974e8cbebef))

- Test-only: a manual check, never run by CI, runs the firmware images
  under QEMU on Cortex-M0, M3 and M4F and RV32 boards, compares each
  scenario's summary with its host test's, and fails when an image's stack
  peak outgrows its budget (#1651, #1660). ([930b124](https://github.com/jscott3201/rusty-bacnet/commit/930b124e75c5d4d82764584e8e086a9761964b6e))

- **Python API:** `BACnetClient` and `BACnetServer` take `mstp_echo`
  (default False) and `mstp_echo_grace_ms` to declare an MS/TP adapter that
  returns what it sends, so the station drops its own echo (#1653). ([c195838](https://github.com/jscott3201/rusty-bacnet/commit/c195838818232ab70aa312c0c2a6ef34194445fb))

- **Python API:** `BACnetClient`, `BACnetServer` and `MstpEndpoint` take the
  keyword-only `mstp_transmit_latency_pad_ms`, `mstp_usage_timeout_ms` and
  `mstp_reply_timeout_ms`, and `MstpEndpointBuilder` the matching setters, for
  slower USB adapters and drivers (#1654, #1655). ([24ffae0](https://github.com/jscott3201/rusty-bacnet/commit/24ffae06a245bdf7dfdd4335c67eb9f3bb92df99))

- **MS/TP endpoint:** `MstpEndpointBuilder::extended_frames` turns on
  extended frames for an endpoint's link and lets its identity advertise a
  Max_APDU_Length_Accepted of up to 1476 instead of 480; the default is
  unchanged (#1663). ([24ffae0](https://github.com/jscott3201/rusty-bacnet/commit/24ffae06a245bdf7dfdd4335c67eb9f3bb92df99))

- **Python API:** `MstpEndpoint` takes the keyword-only
  `mstp_extended_frames`, which lifts its `max_apdu` bound to 1476, and
  `mstp_echo` and `mstp_echo_grace_ms`, with the same rules as on
  `BACnetClient` and `BACnetServer` (#1663). ([24ffae0](https://github.com/jscott3201/rusty-bacnet/commit/24ffae06a245bdf7dfdd4335c67eb9f3bb92df99))

- `TransportPort::max_npdu()` reports the longest NPDU a link sends, unicast
  and broadcast, as `NpduLimits`: 1,497 octets each way by default, 501 on
  MS/TP without extended frames, and on BACnet/SC what its hub connection
  carries with no data options (#1665). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **Rust API:** `bip6::parse_vmac_text` and `format_vmac_text` read and write
  a B/IPv6 VMAC as `bip6:1234` or `bip6:0x0004D2`, the text the CLI and the
  Python bindings take (#1671). ([f13f8ca](https://github.com/jscott3201/rusty-bacnet/commit/f13f8ca797dd45ec834a8fa40ae7aa0183198ccf))

- **Python API:** `BACnetClient(ipv6_device_instance=N)` gives an ipv6 client
  the VMAC of device instance N, a stable address peers can name as
  `bip6:N`, and lets it start on a link without multicast (#1671). ([f13f8ca](https://github.com/jscott3201/rusty-bacnet/commit/f13f8ca797dd45ec834a8fa40ae7aa0183198ccf))

- New `bacnet-router-core` crate: a `no_std` BACnet router (Clause 6) with
  no allocator, a sans-IO engine on injected time and caller storage with
  NPDU views and writers, network-layer messages, a routing table and route
  discovery; its `bip` and `mstp` features add ports over the other cores
  (#1679). ([f9d45aa](https://github.com/jscott3201/rusty-bacnet/commit/f9d45aa1cdaf07770b3a06f2341a084e5e6c0a42))

- `BACnetRouter::counters()` returns the router engine's counts, such as
  `write_failures`, and `bacnet_network::router` re-exports their `Counters`
  type. Reading them, like the four drop counts, takes no lock (#1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- New `bacnet-time` crate: the monotonic `Instant` and `Duration`, in
  nanoseconds, that the MS/TP, B/IP and router cores share, for one firmware
  clock. `no_std`, with no features, allocator or dependencies, saturating
  arithmetic and no 64-bit division. Like each new crate, it needs Rust 1.93
  (#1681). ([e842fa2](https://github.com/jscott3201/rusty-bacnet/commit/e842fa28dc6aabc54bb15bbcd305f3c8f49ef264))

### Changed

- **Breaking (Rust API):** `BipEndpointBuilder` takes one `BipMode` (normal,
  foreign device or BBMD) through `.mode(..)` in place of
  `register_as_foreign_device`, `enable_bbmd` and the BBMD setters (#938). ([8042aef](https://github.com/jscott3201/rusty-bacnet/commit/8042aef046537d61cc4f46136b0d1d5747632cec))

- **Breaking (Rust API):** `TransportPort::normal_bip_endpoint` is now
  `bip_port`, which reports the B/IP mode too; `BipTransport::bbmd_state`
  hands out a `std::sync::Mutex`, and `fdt_counters` no longer awaits (#939). ([303ef1d](https://github.com/jscott3201/rusty-bacnet/commit/303ef1db1bd3b612594bde3f0a1213ddbc76741d))

- **Breaking (wire):** Access Point serves Access_Event_Authentication_Factor,
  the UNDEFINED factor until an event carries one, so its COV report carries
  it too (#1132). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Breaking (Rust API):** `AccessPointObject::set_access_event` takes an
  `AccessEventReport`, which adds the authentication factor and lets the time
  default to the Device clock's, and `BACnetObject` gains
  `report_access_input_internal` (#1132). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Breaking (Rust API):** `ServerConfig` has a new `on_reinitialize` field,
  and `handle_reinitialize_device` returns the requested state instead of
  `()` (#1138). ([1eb630f](https://github.com/jscott3201/rusty-bacnet/commit/1eb630fe74cbc21c1b2cad11e9366a2350cd812d))

- **Breaking (wire):** An Access Point's Active_Authentication_Policy can
  read 0, with Reliability CONFIGURATION_ERROR, when no usable policy is in
  effect or its policy list holds an invalid one, and its Reliability takes
  simulated writes while out of service (#1325). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- `BACnetServer::stop()`'s docs say that a request already running when it
  begins can still make its write, unanswered, and that storage then keeps
  what the object serves (#1457). ([ba8245e](https://github.com/jscott3201/rusty-bacnet/commit/ba8245e1fc12b2c4141d05c9c3fd87a16ea93690))

- **Breaking (Rust API):** `MemberDatatype::of` takes the member's object
  type, `MemberDatatype` gains `ColorCommand` and `XyColor`, and
  `constructed_channel_value` replaces `is_lighting_command_channel_value`
  (#1474). ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Breaking (Rust API):** `ColorObject::set_present_value` takes a
  `BACnetXyColor`, and the colour objects' `set_present_value` and
  `set_min_max` check their values and return `Result` (#1474). ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- **Breaking (Rust API):** The endpoint ingress hands a confirmed request from
  a group source MAC to policy as the new `PolicyReason::GroupSource` (#1504). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **Wire:** While a Lighting Output or Color Temperature fade or ramp runs, a
  Tracking_Value subscriber whose COV increment is finer than the object's
  sample step now hears each of its own steps, at most one per 100 ms
  (#1510). ([d94bd2e](https://github.com/jscott3201/rusty-bacnet/commit/d94bd2ecefcacbc1fc801acb993b63ce40a78fb9))

- A running server shares one copy of its configuration with request dispatch, local writes, Command
  and Channel runs and its background tasks, rather than copying it for each local write (#1521). ([9872f34](https://github.com/jscott3201/rusty-bacnet/commit/9872f3408e534639604d0452de86b9f764acd91f))

- **Breaking (Rust API):** `LogBufferRecords` requires `record_identity` and
  provides `record_position` and `timestamp_order`, which ReadRange now
  selects a log's records by (#1536). ([f8ad451](https://github.com/jscott3201/rusty-bacnet/commit/f8ad4517a8e16be58536802b6fe3b192e7950800))

- ReadRange of a log builds identities only for the records it returns, and
  By Sequence Number and By Time find their window in a Trend Log, Event Log
  or Trend Log Multiple without walking it while its timestamps run in order
  (#1536). ([f8ad451](https://github.com/jscott3201/rusty-bacnet/commit/f8ad4517a8e16be58536802b6fe3b192e7950800))

- **Breaking (Rust API):** `ServerConfig` and `ClientConfig` have a new
  `share_port_by_address` field, read by the B/IP builders (#1538). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- Test-only: the BACnet/SC rejection-deadline, recovery, redial, failover,
  primary-restore and connect lifecycle tests run on tokio's paused clock,
  so a stalled runner can no longer fire a test's deadline before the
  transport's own timer (#1547, #1549). ([f8b1a94](https://github.com/jscott3201/rusty-bacnet/commit/f8b1a94b3c608dc5dfdf917e258502c7d177f8c0))

- The server's discovery and time-synchronization rate limits, discovery
  coalescing windows, binding freshness, COV subscription lifetimes, Life
  Safety operation replay and the confirmed-event repeat window use
  tokio's clock, so paused tests step them; production is unchanged
  (#1548, #1550, #1556). ([f8b1a94](https://github.com/jscott3201/rusty-bacnet/commit/f8b1a94b3c608dc5dfdf917e258502c7d177f8c0))

- **Wire:** a WriteProperty of Tags reaches the object as its raw elements, so
  an object without Tags now answers that it has no such property for any
  value, an empty array included (#1553). ([ea578d5](https://github.com/jscott3201/rusty-bacnet/commit/ea578d5fd468637b19a0527238d5622aca8d158b))

- The full server now answers matching Who-Has requests while message initiation is disabled, using the existing discovery rate limits and routing. Independent discovery announcements remain restricted (#1590). ([f344fa6](https://github.com/jscott3201/rusty-bacnet/commit/f344fa6f8daf5269a205f2282e390107a8aaafbb))

- **Breaking (Rust API):** `BvlcFunction` and `BvlcResultCode` move from
  `bacnet-types` to `bacnet-bip-codec`, and `bacnet_transport::bvll` builds
  and reads messages as the codec's typed `Bvll` values (#1604). ([bdac34b](https://github.com/jscott3201/rusty-bacnet/commit/bdac34b5d468c8a6e4fc9b287be5c4e2cc34bf1d))

- **CLI:** the packet decoder shows BVLC function 0x0C as `12` instead of
  `SECURE_BVLL`, and reports a capture whose BVLC Length disagrees with its
  UDP payload as a decode error (#1604). ([bdac34b](https://github.com/jscott3201/rusty-bacnet/commit/bdac34b5d468c8a6e4fc9b287be5c4e2cc34bf1d))

- **Wire:** the B/IP transport drops a datagram whose BVLC Length field is
  not its length instead of reading the shorter message, and a BBMD NAKs a
  Read-BDT or Read-FDT that carries a payload instead of answering it (#1604). ([bdac34b](https://github.com/jscott3201/rusty-bacnet/commit/bdac34b5d468c8a6e4fc9b287be5c4e2cc34bf1d))

- **Breaking (Rust API):** `bacnet_types::error::Error` is
  `#[non_exhaustive]`: its `std` feature adds a variant, and any crate in a
  build can turn that on (#1604). ([5bee59d](https://github.com/jscott3201/rusty-bacnet/commit/5bee59dd215f623d5990deb343e528d9218ce6a9))

- **Breaking (Rust API):** `bacnet-types`, `bacnet-encoding` and
  `bacnet-services` build as `no_std` with an allocator on bare-metal targets,
  through a new `alloc` feature; `std` stays the default. Based on work by
  Andrew Rodgers (ACE IoT Solutions) (#1604). ([5bee59d](https://github.com/jscott3201/rusty-bacnet/commit/5bee59dd215f623d5990deb343e528d9218ce6a9))

- **Breaking (Rust API):** `MasterNode`, `MasterState` and the `mstp_frame`
  codec are removed; MS/TP frames come from `bacnet_transport::mstp::codec`,
  `SerialPort` gains `echo()`, and `MstpDiagnosticsSnapshot` carries the
  master core's counts, totalled, as `mac` (#1608). ([3d76be6](https://github.com/jscott3201/rusty-bacnet/commit/3d76be6c22a41d6fee4c8328f1f5b7d34457139f))

- **Breaking (Rust API):** `MstpConfig` gains the `extended_frames` field,
  and `TransportPort` the `prepare_for_routing` hook that `BACnetRouter`
  calls before it starts each port (#1611). ([30da0ea](https://github.com/jscott3201/rusty-bacnet/commit/30da0ea31da2ee73bc022bc425659a00aaa6610a))

- **Breaking (Rust API):** `Bvlc6Function` and `Bvlc6ResultCode` move from
  `bacnet-types` to `bacnet-bip-codec`, and `bacnet_transport::bip6` builds
  and reads messages as the codec's typed `Bvll6` values (#1621). ([814d328](https://github.com/jscott3201/rusty-bacnet/commit/814d328fd605846042db6198c90b9cf6e082a7b8))

- **Breaking (wire):** A B/IP foreign device takes a Forwarded-NPDU only from
  its configured BBMD by unicast, as B/IPv6 does, and drops any other,
  counting it in `receive_counters()`; normal mode is unchanged (#1622). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- **Wire:** A B/IP foreign device holds up to 16 broadcasts until its first
  registration attempt after a start is answered or times out, then sends
  them in order, or drops them if the BBMD refuses it; a broadcast past that
  hold fails with an error (#1622). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- **Breaking (Rust API):** The B/IP transport runs on the `bacnet-bip-core`
  node engine. A foreign device now refuses manual Register, Delete-FDT-Entry
  and Write-BDT requests to its own BBMD, and `start()` refuses a BBMD
  address no device could register with (#1622). ([8a5bd45](https://github.com/jscott3201/rusty-bacnet/commit/8a5bd4584deba17a50ee5efde0d6e1530dd6effa))

- **Breaking (Rust API):** A `BipTransport` set up both as a BBMD and as a
  foreign device now fails `start()` instead of mixing the two modes (#1622). ([8a5bd45](https://github.com/jscott3201/rusty-bacnet/commit/8a5bd4584deba17a50ee5efde0d6e1530dd6effa))

- A B/IP transport's own management requests (`read_bdt` and the other BBMD
  helpers, and a foreign device's registration) run side by side, one per
  peer and operation, instead of one at a time; only the same request to the
  same peer, or a second one a success answers, is busy (#1622). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- **Wire:** A B/IP foreign device retries a lost registration sooner,
  backing off from the response wait up to the renewal interval, renews into
  the lease a success proved, registers again early when its BBMD NAKs a
  Distribute-Broadcast-To-Network, and logs each lapse once (#1622). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- **Breaking (wire):** A B/IP transport refuses to send an NPDU longer than
  1497 octets (Table 6-1) with an error, counted in the new
  `BipTransport::send_counters()`; it used to send any length. A BBMD still
  forwards what it receives as it came (#1622). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- **Breaking (Rust API):** `bbmd::BbmdState` keeps its tables in
  `bacnet-bip-core`: `bdt()` and `fdt()` return copies of the rows,
  `encode_bdt()`, `encode_fdt()` and the forwarding helpers are gone, and
  `forwarded_npdu_needs_local_broadcast` follows the corrected J.4.5 rule
  (#1623, #1669). ([7ac93a6](https://github.com/jscott3201/rusty-bacnet/commit/7ac93a65a14b8f45e59bfac01652936f6b7129f1))

- A B/IP BBMD asks the host for its own subnet's mask at start. Without one
  (a NAT global address, or an OS with no lookup) a forward from a peer on
  its own subnet is still rebroadcast (#1623). ([7ac93a6](https://github.com/jscott3201/rusty-bacnet/commit/7ac93a65a14b8f45e59bfac01652936f6b7129f1))

- B/IP logs an undecodable BVLL header or a malformed BVLC response at debug
  instead of warn, and warns about a full NPDU channel at most once every
  10 seconds with the number dropped; `receive_counters()` counts each
  (#1624). ([3bfe3ff](https://github.com/jscott3201/rusty-bacnet/commit/3bfe3ff5e7a7b802104f7cb2687350c0769374b8))

- Test-only: the BACnet/SC hub and TLS tests connect again when the OS hands
  an outbound loopback connect a source port that is still in use, as it did
  on Windows CI, instead of failing (#1629). ([ad52d69](https://github.com/jscott3201/rusty-bacnet/commit/ad52d696f28dbe9e5a5ba74c7e1fb76ac3a3ef79))

- **Breaking (Rust API, wire):** A B/IPv6 transport's MAC is now the node's
  3-octet VMAC (Annex U): `local_mac()`, received source MACs, `send_unicast`
  and routed SADR/DADR use it (#1637). ([b412923](https://github.com/jscott3201/rusty-bacnet/commit/b41292375d9432f7ab05296a3abb7cea2cb0b439))

- B/IPv6 logs an undecodable frame at debug; its full NPDU channel, VMAC
  collision warning and BVLC-Result NAK error, and a B/IP BBMD's full fan-out
  queue and failed forwarding sends, log at most once per 10 s with a count;
  a stopped fan-out worker logs at debug (#1643). ([4a97626](https://github.com/jscott3201/rusty-bacnet/commit/4a97626191a7709438a70880372fb974e8cbebef))

- **Rust API:** Service decoders no longer format and allocate a context label
  for each member of a decode that succeeds; a label is built only when an
  error names it, with the same text, and `decode_app_primitive` takes its
  label as any `impl Display` rather than a `&str` (#1645). ([ef42c6e](https://github.com/jscott3201/rusty-bacnet/commit/ef42c6eb07a65bc2d6d3c1861f59f31c3a2a0e44))

- **MS/TP transport:** every host port now waits the longest T_usage_timeout
  (35 ms) and T_reply_timeout (300 ms) Clause 9.5.3 allows, set by
  `MstpConfig::usage_timeout` and `reply_timeout`, so a successor, poll
  answer or reply that a USB adapter delivers late is not missed (#1655). ([24ffae0](https://github.com/jscott3201/rusty-bacnet/commit/24ffae06a245bdf7dfdd4335c67eb9f3bb92df99))

- **Breaking (Rust API):** The MS/TP transport takes three `Ok(0)` serial
  reads in a row as the port's end of file and stops as at a read error, so
  a custom `SerialPort` must wait for data instead of returning `Ok(0)`; an
  expired late-answer channel now goes at its expiry (#1656). ([c195838](https://github.com/jscott3201/rusty-bacnet/commit/c195838818232ab70aa312c0c2a6ef34194445fb))

- **Breaking (CLI):** On `--ipv6`, targets take `bip6:<device instance>` or
  `bip6:0x<6 hex digits>`; an `[ip]` or `[ip]:port` target is first resolved
  to the VMAC of the node there, and addresses print as `bip6:0x…`, which the
  CLI reads back (#1671). ([f13f8ca](https://github.com/jscott3201/rusty-bacnet/commit/f13f8ca797dd45ec834a8fa40ae7aa0183198ccf))

- **Breaking (Python API):** B/IPv6 peers are named by VMAC: addresses take
  `bip6:<device instance>` or `bip6:0x<6 hex digits>` and refuse `[ip]:port`,
  which `BACnetClient.resolve_vmac` turns into a VMAC. An ipv6 `BACnetServer`'s
  VMAC is its device instance, as `local_address()` shows (#1671). ([f13f8ca](https://github.com/jscott3201/rusty-bacnet/commit/f13f8ca797dd45ec834a8fa40ae7aa0183198ccf))

- `BACnetRouter` warns about a port's failed sends at most once every 10 s,
  with the count since the last warning, instead of once per frame;
  `bacnet_transport::warn_throttle::WarnThrottle` is public (#1672). ([b412923](https://github.com/jscott3201/rusty-bacnet/commit/b41292375d9432f7ab05296a3abb7cea2cb0b439))

- **Breaking (wire):** B/IP `send_unicast` to one of the link's group
  addresses fails instead of sending a datagram its receivers drop;
  `NetworkLayer::send_apdu` and `BACnetRouter` send an Unconfirmed-Request for
  one at the link's own UDP port as its local broadcast instead (#1677). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- **Wire:** `NetworkLayer::send_apdu` sends an Unconfirmed-Request for a group
  destination as the link's local broadcast on every transport that gives a
  group rule, so on Ethernet one for a multicast MAC now goes to the all-ones
  broadcast (#1677). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- `BACnetRouter` also delivers to its own application an Unconfirmed-Request it
  sends on as a group DADR's local broadcast, as it does a DLEN 0 broadcast
  for the network the request came from (#1677). ([2706c76](https://github.com/jscott3201/rusty-bacnet/commit/2706c76a3dc61afe27080ab4eb5cced7148d8084))

- A router's control authorizer now runs under the router's engine lock,
  one call at a time, where it could run concurrently before. One that
  keeps to its contract (no blocking, no calls into the router) sees no
  difference (#1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- **Breaking (Rust API):** `BACnetRouter` runs on the `bacnet-router-core`
  engine. `router_table` re-exports the core's table, with getters on
  `Route`, and `table()` returns a sync `RouterTableGuard` on the live
  table, on `now()`'s clock. A broadcast's relay now queues before its
  local copy (#1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- A router's table keeps at most 1,024 reject hold-down records, one per
  network and arrival port, where it kept any number before; a full pool
  gives up its oldest record and counts it in the claim counters'
  `hold_evicted` (#1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- **Wire:** `BACnetRouter` stores an I-Could-Be-Router offer only below its
  256-route cap, as it does learned routes, where it stored one past the
  cap before; one the cap turns away counts in `learned_cap_ignored`
  (#1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- **Wire:** `BACnetRouter` doesn't pass on a global broadcast whose rewrite
  is longer than 65,556 octets, the largest BVLC message plus the source a
  router adds; it is dropped, logged and counted in `write_failures`. Other
  NPDUs meet their port's limit first (#1665, #1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- **Breaking (Rust API):** `BACnetRouter::start` refuses a router without
  ports, with more than 255, or with a port whose own MAC is longer than 18
  octets, which it started before (#1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- `BACnetRouter`'s routing table, route search and aging run on Tokio time
  from the router's start instead of the wall clock, so paused-time tests
  drive them, and the aging sweep runs at the engine's own deadlines
  (#1679). ([9e6c4d4](https://github.com/jscott3201/rusty-bacnet/commit/9e6c4d415970921e671363a3c36129c8dae1afdf))

- **Breaking (Rust API):** `ScConnection::hub_max_apdu_length` is now
  `hub_max_npdu_length`, naming what it holds: the hub's Max-NPDU-Length from
  its Connect-Accept (#1679). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

### Removed

- **Breaking (Rust API):** `bacnet_transport::bip6::encode_bip6_mac` and
  `decode_bip6_mac`: a B/IPv6 address is no longer a MAC, so build and read
  one with `Bip6Address` (#1671). ([f13f8ca](https://github.com/jscott3201/rusty-bacnet/commit/f13f8ca797dd45ec834a8fa40ae7aa0183198ccf))

### Fixed

- Access Point refuses an access event that is neither a named
  BACnetAccessEvent nor a proprietary one from 512 to 65535, through
  `set_access_event`, `report_access_event_local` and Python (#1132). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Wire:** Access User serves Global_Identifier, the required row clients
  write to name a user across devices, and `AccessUserObject` gains
  `set_global_identifier` (#1463). ([56c6b0d](https://github.com/jscott3201/rusty-bacnet/commit/56c6b0de6f1687ce5e9e14c28c56837723984a3c))

- **Breaking (wire):** Color and Color Temperature serve the rows of the
  addendum's tables: Color's Present_Value is writable, Color Temperature
  clamps a written value, the defaults check their ranges, and Status_Flags,
  Event_State, Reliability and Out_Of_Service are gone (#1474). ([f2f3ac9](https://github.com/jscott3201/rusty-bacnet/commit/f2f3ac93137b712ecfc032ef06f50037db5f6b81))

- Python reads return typed single references for Trend Log, Averaging, Event Enrollment, Access Point, Lift and Escalator, preserving optional fields, unset identifiers, exact wire bytes and existing malformed-value fallbacks. ([74fca30](https://github.com/jscott3201/rusty-bacnet/commit/74fca307156ffa7e2583afcab8b0b7e8d1ec501b))

- **Wire:** B/IP drops a datagram whose UDP source is a group address before
  handling its BVLC function, so a BBMD neither registers nor forwards from
  one, and counts it in the new `BipTransport::group_source_drops()` (#1504). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **Wire:** The server, client and endpoint ignore a confirmed request whose
  link-layer source is a group address, so no reply or confirmed COV
  notification goes to a group. The server and client count it in the new
  `group_source_request_drops()` (#1504). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- **Wire:** `BACnetRouter` passes on to a group DADR on a directly
  connected port only an Unconfirmed-Request, and for a B/IP group only at
  the link's own UDP port; it drops anything else, network messages and
  relayed rejects included, counting it in the new `group_dadr_drops()`
  (#1504, #1677). ([248c82f](https://github.com/jscott3201/rusty-bacnet/commit/248c82fde8decd15a47fb5e90dd5bc14627a125c))

- The tasks a `stop()` leaves to settle staged writes and end runs once the
  application lets go of the database now drop it off the runtime when they
  hold its last handle, as a dropped server's own hand-off does (#1513). ([ba8245e](https://github.com/jscott3201/rusty-bacnet/commit/ba8245e1fc12b2c4141d05c9c3fd87a16ea93690))

- **Breaking (Rust API):** `set_life_safety_operation_expected_local` must run inside a Tokio runtime,
  failing and changing nothing outside one; once Operation_Expected changes, its COV notifications go
  out even if the caller is dropped (#1520). ([9872f34](https://github.com/jscott3201/rusty-bacnet/commit/9872f3408e534639604d0452de86b9f764acd91f))

- BACnet/SC: after a failed attempt to return to the primary hub, the next
  attempt waits a full restore interval, and a failover after a long spell on
  the primary no longer sets off a burst of attempts (#1555). ([c87f2c5](https://github.com/jscott3201/rusty-bacnet/commit/c87f2c55a1ef6c66aac6ee7ea92c622619c50ee0))

- A server dropped while a DeviceCommunicationControl request or the DCC
  timer's expiry held the timer's slot now cancels the timer before letting
  go of the database, so with Audit reporting the timer no longer keeps the
  objects alive until it runs out (#1560). ([ba8245e](https://github.com/jscott3201/rusty-bacnet/commit/ba8245e1fc12b2c4141d05c9c3fd87a16ea93690))

- Every endpoint session holder of the object database (the session, the
  server role's responder, the source Audit runtime and the Number task) now
  lets go of it off the runtime, so whichever goes last, durable objects'
  final saves don't hold a Tokio worker (#1561). ([4cbe830](https://github.com/jscott3201/rusty-bacnet/commit/4cbe8305cf16e1ca0fbab21ea61e5276df5aef21))

- **Wire:** Accepted WARMSTART and COLDSTART requests end DISABLE_INITIATION immediately, cancel its expiry timer and resume queued COV notifications, even if the reply later fails (#1567). ([7d4f2fb](https://github.com/jscott3201/rusty-bacnet/commit/7d4f2fb1a008b32c19a6f87abf07be6dbf72f171))

- BACnet/SC: primary restore dials and Connect handshakes no longer block
  traffic or heartbeat processing on the active failover hub. Stop, drop and
  loss of that connection cancel the pending restore; completed candidates
  cannot replace a connection that has started shutting down (#1575). ([e820fc7](https://github.com/jscott3201/rusty-bacnet/commit/e820fc7cbc10c20afc4f60e1daeac08330459ae0))

- Server reassembly and synchronized Device clocks share the runtime clock,
  making paused-time tests independent of runner stalls (#1576). ([b5474df](https://github.com/jscott3201/rusty-bacnet/commit/b5474df7658809a4788510a2c73bdf665f527101))

- Endpoint shutdown waits for queued durable saves and abandoned-write corrections before completing, including after cancellation and retry (#1581). Completion reports finished attempts, not storage success. ([047a681](https://github.com/jscott3201/rusty-bacnet/commit/047a681f0ffc805af0396c971111546dd3fd3761))

- **Wire:** Full servers share a configurable segment timeout with their Device declaration, reclaim quiet requests, and report local no-progress expiry. Narrow endpoints reject unsupported segmentation declarations (#1593). ([a321f74](https://github.com/jscott3201/rusty-bacnet/commit/a321f746cc5a5979a1779655d7e54e88b35dcb48))

- **Wire:** The MS/TP transport runs the Clause 9 master core: maintenance
  polls every 50 token passes, the wait states' handling of unexpected
  and invalid frames, turnaround before each frame, per-station generation
  slots, delivered broadcast requests, and dropped frames skipped whole
  (#1608, #1609). ([3d76be6](https://github.com/jscott3201/rusty-bacnet/commit/3d76be6c22a41d6fee4c8328f1f5b7d34457139f))

- **Wire:** A B/IP node no longer sends a NAK to a source no reply can
  reach (the unspecified address, port 0 or its own address), as B/IPv6
  already did (#1622). ([8a5bd45](https://github.com/jscott3201/rusty-bacnet/commit/8a5bd4584deba17a50ee5efde0d6e1530dd6effa))

- **Wire:** a BBMD rebroadcasts a peer's Forwarded-NPDU on its subnet when
  its own BDT row is two-hop and the datagram came by unicast, as Annex J.4.5
  says, not by the sender's row; never for a peer on its own subnet at its own
  port (#1623). ([7ac93a6](https://github.com/jscott3201/rusty-bacnet/commit/7ac93a65a14b8f45e59bfac01652936f6b7129f1))

- **Wire:** A B/IPv6 node now NAKs the three requests only a BBMD serves
  (foreign-device registration, FDT entry deletion and broadcast
  distribution), and answers a BBMD's Forwarded-Address-Resolution for its
  VMAC, so address resolution across BBMDs reaches it. Both are rate-limited
  (#1632). ([2f0772c](https://github.com/jscott3201/rusty-bacnet/commit/2f0772c3122f172b2c73e992ecac829d00b2d9a0))

- A full B/IPv6 VMAC table evicts the peer heard from least recently instead
  of ignoring new peers and never learns this node's own VMAC; frames from
  another node using that VMAC, and NPDUs from a source no reply could reach,
  are dropped and counted (#1637). ([b412923](https://github.com/jscott3201/rusty-bacnet/commit/b41292375d9432f7ab05296a3abb7cea2cb0b439))

- **Wire:** A B/IPv6 node now NAKs a malformed foreign-device registration,
  FDT entry deletion or broadcast distribution as it does a well-formed one,
  under the same rules and rate limit, as B/IP does (#1638). ([4a97626](https://github.com/jscott3201/rusty-bacnet/commit/4a97626191a7709438a70880372fb974e8cbebef))

- **Wire:** MS/TP: with a successor above Max_Master, a station's
  maintenance polls end before they reach its own address instead of
  polling itself, and when that successor fails, the search for a new one
  starts from the address after it, modulo Max_Master + 1, not from 0
  (#1640). ([f27b4de](https://github.com/jscott3201/rusty-bacnet/commit/f27b4de4d6d45ac857ebf66c332c1f261f9a1966))

- **MS/TP transport:** a port with no drain, such as `TokioSerialPort` on
  Windows, no longer reports a frame finished before its last octet can have
  left: its wire time now counts `MstpConfig::transmit_latency_pad` on top,
  4 ms by default on Windows for a USB adapter (#1654). ([24ffae0](https://github.com/jscott3201/rusty-bacnet/commit/24ffae06a245bdf7dfdd4335c67eb9f3bb92df99))

- The MS/TP examples in `docs/rust-api.md` build `MstpTransport` from an
  `MstpConfig`, and the `serial` rustdoc build on macOS and the `ethernet`
  clippy build on aarch64 Linux are warning-free (#1657). ([c195838](https://github.com/jscott3201/rusty-bacnet/commit/c195838818232ab70aa312c0c2a6ef34194445fb))

- **Wire:** `BACnetRouter` answers an NPDU too long for the port it would
  leave by with reject reason 4 to its source, counted in `egress_too_long`.
  That port's send used to fail it, with a warning, and the source heard
  nothing. A global broadcast skips such a port, with no reject (#1665). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **Wire:** `BACnetRouter` no longer passes on, with a hop count of 0, an
  NPDU for another router or a global broadcast that arrives with 1; it
  counts it in `hop_count_drops`, still taking the broadcast itself. Delivery
  onto an attached network is unchanged (#1679). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **Wire:** `BACnetRouter` delivers an APDU addressed to its own MAC on any
  of its networks to its local receiver. One for its address on another of
  its networks used to go out on that network's wire, to the router itself
  (#1679). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **Wire:** `BACnetRouter` answers an unscoped Who-Is-Router-To-Network
  without the networks it holds as unreachable, keeping busy ones, and lists
  the rest in ascending order (#1679). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **Wire:** `BACnetRouter` no longer removes a learned route when a reject
  with a reason other than 1 or 2 arrives for its network, so a reason-4
  reject can't erase a good route; the claim counters' new `reject_kept`
  counts it, and the reject is still passed on (#1679). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **Wire:** At startup `BACnetRouter` broadcasts a Network-Number-Is on each
  port, giving the port's configured network number, before its
  I-Am-Router-To-Network; a one-port router now sends that one message
  (#1679). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

- **Wire:** `BACnetRouter` sends nothing for any Who-Is-Router-To-Network
  naming a network it reaches through the port the request came in on, which
  the router on that network answers; it used to relay it to its other ports
  (#1679). ([050a027](https://github.com/jscott3201/rusty-bacnet/commit/050a02770f27079161901e5774cfd7730bf90499))

## [0.12.0] - 2026-10-05

### Release highlights

This pre-1.0 release breaks Rust, Python and CLI APIs and changes some
on-the-wire behaviour. Before moving a v0.11.0 integration, read the
[upgrade guide](https://jscott3201.github.io/rusty-bacnet/project/upgrading/),
which groups what changes by area; the migration notes below give the exact
replacements. Conformance remains partial, and this release is not a
full-conformance or BTL certification claim.

- Objects do more of their own work: Command and Channel run their actions
  and member writes, in other devices too; the server executes WriteGroup;
  Lighting Output carries out fades, ramps, steps and warnings; Trend Log
  Multiple polls its members; and the Notification Forwarder is new.
- Event notifications reach a Device recipient the server holds no binding
  for through a targeted Who-Is, and the Device serves the bindings it holds
  in Device_Address_Binding.
- Access control, life safety, Schedule and log objects serve many more of
  their standard properties. Notification Class, Notification Forwarder and
  Access Rights can keep what peers write across a restart, saved off the
  database lock.
- Python reads constructed values typed, and its value and enumeration
  classes copy and pickle.
- Releases are built natively for Linux (x86_64, aarch64), macOS (arm64,
  Intel) and Windows. The Linux CLI needs glibc 2.17 and no libpcap, wheels
  cover CPython 3.11 to 3.14, and `bacnet-endpoint` and `bacnet-cli` are new
  on crates.io; see the
  [installation page](https://jscott3201.github.io/rusty-bacnet/start/installation/).
- The terminal UI (`bacnet tui`) stays behind the opt-in `tui` feature until
  0.13.0.

### Migration notes

- **Audit queries (#345):** Rust callers pass
  `BACnetSuccessFilter::{ALL, SUCCESSES_ONLY, FAILURES_ONLY}` and
  `Option<u64>` cursors. Python callers pass `successful_actions_only` as 0, 1
  or 2 instead of a bool: 1 for the old `True` and 0 for the old `False`.

- **SC TLS (Rust API, #513):** build hub TLS with `ScHubTlsConfig::from_der`
  for `ScHub::start`, and node TLS with `ScNodeTlsConfig` for
  `TlsWebSocket::connect` and the builders' `tls_config`, from owned CA, chain
  and key DER. See the [hub](docs/rust-api.md#bacnetsc-hub) and
  [node](docs/rust-api.md#strict-local-node-tls-configuration) migration
  notes.

- **SC device UUID (#517):** pass a nonzero 16-byte UUID to the `ScHub`
  startup APIs, `ScServerBuilder::device_uuid` and
  `ScTransport::with_device_uuid`, or `sc_device_uuid` and `device_uuid` in
  Python. Generate it before deployment, store it durably and reuse it; see
  the [Python](docs/python-api.md#sc-device-uuid-migration) and
  [Rust](docs/rust-api.md#sc-device-uuid-migration) migration notes.

- **DCC policy (Rust API, #522):** exhaustive `ServerConfig` literals need
  `dcc_disable_rate_limit: None` and `dcc_source_restriction: None`.

- **Custom transports (Rust API, #693):** implement
  `local_receive_apdu_capacity()`, the largest APDU the link accepts, and
  rename `max_apdu_length()` to `egress_apdu_limit()`; a wrapping transport
  delegates both. A `ReceivedNpdu` literal sets
  `provenance: TransportProvenance::unverified()` and `direct_response: None`.
  See [local receive capacity](docs/rust-api.md#local-receive-capacity-and-outgoing-limits). ([df016bb](https://github.com/jscott3201/rusty-bacnet/commit/df016bb551d99c88bd42b96cb6bca8ec75651045))

- **ObjectDatabase (Rust API, #728):** `ObjectDatabase::remove` returns
  `Result`.

- **Intrinsic reporting (Rust API, #746):** custom objects implement the
  evaluate and tick proposal hooks and `commit_event_transition_internal`
  instead of using `impl_intrinsic_reporting!` and
  `intrinsic_reporting_requires_atomic_commit`.

- **LifeSafetyOperation (Rust API, #752):** custom objects return
  `LifeSafetyOperationOutcome` from `apply_life_safety_operation`, listing the
  properties they changed for COV, and drop the `_detailed` hook.

- **Command sources (#824):** Rust local writes take a `LocalCommandSource`,
  and Python local writes the keyword `source_object`. `write_property_from`
  carries a network source, and the unsourced Analog Value setter is gone.

- **ObjectIdentifier (#847):** replace `ObjectIdentifier::new_unchecked` with
  `new` or `new_addressable`. Python raises `ValueError` for an oversized
  object type.

- **Builders (Rust API, #873):** replace `BACnetClient::builder()` and
  `BACnetServer::builder()` with `bip_builder()`.

- **Colour property identifiers (#887, Rust and Python API):** the
  `PropertyIdentifier` constants `DEFAULT_COLOR`, `DEFAULT_COLOR_TEMPERATURE` and
  `COLOR_COMMAND` keep their names but change value, to 4194330, 4194331 and 4194334
  (in Python, `PropertyIdentifier.COLOR_COMMAND.to_raw()` was 508). Use the
  constants, not 508 to 510, which name Network Port properties. ([97baf49](https://github.com/jscott3201/rusty-bacnet/commit/97baf4921afafc2ae9018a0dd68706de4e879bcd))

- **Parameter structs (Rust API, #902):** build
  `AnyTransport::Bip(Box::new(..))`, pass a `CovPropertySubscription` to
  `subscribe_cov_property` and a `RoutedTarget` to
  `send_apdu_routed_with_data_attributes`.

- **VT, WriteGroup and device identity services (#912):** `VTOpenRequest`
  needs a local session ID and a `VTClass`, WriteGroup takes `u16` channels
  and a `NonZeroU32` group number, and `WhoAmIRequest` the vendor ID, model
  name and serial number. Python's `vt_open`, `write_group` and `who_am_i`
  take the same. ([5ffc8a1](https://github.com/jscott3201/rusty-bacnet/commit/5ffc8a1a00c718762246839cba088b8c813d7f6e))

- **Typed alarm values (#914, #930, #932):** replace raw integers with the
  `bacnet-types` enumerations (`EventState`, `Reliability`, `LifeSafetyState`
  and so on) and bit strings (`StatusFlags`, `EventTransitionBits`,
  `DaysOfWeek`), and the objects' `LimitEnable` with
  `bacnet_types::bitstring::LimitEnable`. Python's `acknowledge_alarm`,
  `add_event_enrollment` and `get_enrollment_summary` take enum values. ([ca35afc](https://github.com/jscott3201/rusty-bacnet/commit/ca35afcde79727fce0176fc562e280d93134904d))

- **Wildcard BBMD (#952, #937):** a B/IP BBMD bound to `0.0.0.0` fails
  `start()` unless exactly one BDT row names a local address at the bound port
  or the default-route address is usable, and a loaded persisted BDT decides
  alone. Bind the interface address to avoid this; see the
  [BBMD section](docs/rust-api.md#bbmd).

- **Transport accessors (Rust API, #956):** add an
  `Error::UnsupportedTransport` arm to exhaustive matches. Read SC link state
  with `connection_state_changes()` instead of `ScTransport::connection()`,
  and MS/TP counts with `diagnostics()` instead of `node_state()`. ([5e9e676](https://github.com/jscott3201/rusty-bacnet/commit/5e9e6763b3224ff7dfc2e19ad7a6c9a132587a3a))

- **Schedule codecs (Rust API, #996):** import the calendar and schedule
  codecs from `bacnet_encoding::constructed`; `bacnet_services::schedule` and
  `BACnetDateRange::encode` and `decode` are gone. ([107e9fa](https://github.com/jscott3201/rusty-bacnet/commit/107e9fa5fe54d0c990083ac264b833b06672833f))

- **Python panics (#1002):** a Rust panic in an async method now raises PyO3's
  `PanicException` instead of `pyo3_async_runtimes.RustPanic`. It derives from
  `BaseException`, so `except Exception` no longer catches it. ([52bfdeb](https://github.com/jscott3201/rusty-bacnet/commit/52bfdeb3b6a76c2b953ff8bbfe7a7155ceb76dad))

- **Calendar and Schedule (Rust API, #1029, #845):** Calendar's
  `set_present_value` is gone and `add_date_entry` returns `Result`.
  `BACnetTimeValue::value` is a primitive `PropertyValue`. `tick_schedule`
  takes the date, the time and a Calendar resolver, not the weekday, hour and
  minute, and returns `Option<ScheduleWrite>`, whose `references` keep their
  array index, not a value with `(ObjectIdentifier, u32)` pairs. The Schedule
  setters and the `bacnet-encoding` schedule encoders return `Result`. ([5dc2537](https://github.com/jscott3201/rusty-bacnet/commit/5dc2537d6cd70588f76f74e85bfd459c94cf1f55))

- **Structured errors (Rust API, #1047, #1048):** `Error` gains
  `Structured { class, code, detail }`, and `TsmResponse::Error` gains
  `detail`. Some list-write refusals now return `Error::Structured` where they
  returned `Error::Protocol`, so match both for the class and code. ([0f43456](https://github.com/jscott3201/rusty-bacnet/commit/0f43456afa2ae49d066aa2875d2db1524e0e4966))

- **Access Credential and Access Door (Rust API, #1073, #979):**
  Credential_Status is read-only now; raise disable reasons with
  `add_disable_reason`. Set Assigned_Access_Rights and Authentication_Factors
  with `set_assigned_access_rights` and `set_authentication_factors`, and
  build `BACnetAssignedAccessRights` with a `BACnetDeviceObjectReference`.
  `AccessDoorObject::set_relinquish_default` takes a `DoorValue`. ([42e4528](https://github.com/jscott3201/rusty-bacnet/commit/42e45288181d8fa383c5b41fce6de387bdb034a1))

- **Schedule references (Rust API, #1088):**
  `ScheduleObject::add_object_property_reference` returns `Result`. A wrapper
  that forwards every trait method needs the new `take_owed_schedule_writes`
  and `complete_schedule_write` hooks. ([7762130](https://github.com/jscott3201/rusty-bacnet/commit/77621307d247d88a91dcb0478a844ffd54bcf880))

- `RangeSpec` reference index and sequence, `ReadRangeAck::first_sequence_number`
  and `LogRecordIdentity::sequence_number` are now `u64`; widen any code that
  builds or matches them (#1092). ([e49a6f3](https://github.com/jscott3201/rusty-bacnet/commit/e49a6f35072009ee10dd2333a262039b41225c02))

- **Recipient_List (Rust API, #1098, #1125):** read the list with
  `recipient_list()`, since the field is private, and handle the `Result` from
  `add_destination`. A local write takes only the framed BACnetLIST in
  `PropertyValue::ApplicationData`; write empty application data to clear it. ([2caee31](https://github.com/jscott3201/rusty-bacnet/commit/2caee311979e4311fd7f87d4e09bee815dee50ac))

- **Global Group (Rust API, #1107):** `GlobalGroupObject::present_value` is a
  `Vec<AccessResult>` holding, by member position, the value or error each
  member's read produced. ([0980c10](https://github.com/jscott3201/rusty-bacnet/commit/0980c10774260915fc65845d99e8fa05b07fee5f))

- **`BACnetObject::cov_increment` returns `Option<f64>` (#1111):** an object
  that overrides it changes the signature and returns
  `Some(f64::from(increment))`; `CovSubscriptionTable::should_notify` takes
  an `Option<f64>` increment too. ([d81bcfa](https://github.com/jscott3201/rusty-bacnet/commit/d81bcfa04d6669bb474a8b0503e83ceecfcccee5))

- `BACnetShedLevel` percent and level hold `u64`, and
  `LoadControlObject::set_requested_shed_level` and `set_actual_shed_level`
  return `Result` (#1133). ([0debbca](https://github.com/jscott3201/rusty-bacnet/commit/0debbcadb03353f5b065c76b9207f97219248d33))

- **Group (Rust API, #1134):** `GroupObject::add_member` takes a
  `ReadAccessSpecification` and returns `Result`. `PropertyReference` and
  `ReadAccessSpecification` moved to `bacnet_types::constructed`, with their
  codecs in `bacnet_encoding::constructed`; update imports. ([9217089](https://github.com/jscott3201/rusty-bacnet/commit/9217089a8c6dd561c5dcb096738f3d58a8fb42c1))

- **Structured View and Command (Rust API, #1135):**
  `StructuredViewObject::subordinate_list` holds `BACnetDeviceObjectReference`
  values (an `ObjectIdentifier` still converts), and
  `CommandObject::set_action` takes `Vec<BACnetActionList>` and returns
  `Result`. ([b7f1209](https://github.com/jscott3201/rusty-bacnet/commit/b7f12093227a886113ca8d63eb6f488a65e185cb))

- **Reference setters (Rust API, #1182, #1234):** the Life Safety `add_member`
  and `add_zone_member` take a `BACnetDeviceObjectReference` (an
  `ObjectIdentifier` converts), and they, `set_log_device_object_property` and
  `add_property_reference` return `Result`; handle it. A single reference now
  reads as `PropertyValue::ApplicationData`, and the Trend Log Multiple array
  and Life Safety lists as a `List` of them. ([11c55eb](https://github.com/jscott3201/rusty-bacnet/commit/11c55eb65924f8de11ace8c20cf57f330dfbb451))

- `TrendLogMultipleObject::add_record` and `records()` use
  `BACnetLogMultipleRecord` (one `LogValue` per member) instead of
  `BACnetLogRecord`; object wrappers forward the new
  `BACnetObject::add_trend_multiple_record` hook (#1203). ([eb86197](https://github.com/jscott3201/rusty-bacnet/commit/eb8619781f2917baa381a61d65fd671dc76bf987))

- **Breaking:** `BACnetRouter::start` takes a `RouterOptions` and returns a
  `StartedRouter` (#1220). Replace `start(ports)` with
  `start(ports, RouterOptions::new())` and take `router` and `apdus` from the
  result. ([ff7ba6b](https://github.com/jscott3201/rusty-bacnet/commit/ff7ba6b9ad49359a00b46453334c83a8fd7dafb0))

- **ReceivedApdu (Rust API, #1225):** `bacnet_network::layer::ReceivedApdu` has a new
  `global_broadcast` field. Code that builds one with a struct literal adds
  `global_broadcast: false`, or `true` for an NPDU sent to DNET 65535. ([fafc1dd](https://github.com/jscott3201/rusty-bacnet/commit/fafc1ddf07b6f4171f219274d1e2b5ae5c08c529))

- `EventLogObject` stores `BACnetEventLogRecord`; log statuses are `LogStatus`, Trend Log
  `status_flags` is `Option<StatusFlags>`, and INTEGER/ENUMERATED log values are `i64`/`u64`.
  Read records through `records()` or ReadRange; wrappers forward `log_buffer_internal`, whose
  `encode_record` returns nothing (#1233, #1237). ([e1d1d0d](https://github.com/jscott3201/rusty-bacnet/commit/e1d1d0d35367438d4da43b1e6bf76e0240cf95af))

- 0.11.0 Audit Log snapshots are converted automatically on first load and saved as schema v3.
  The Python `log_status` int keeps 1 = log-disabled, 2 = buffer-purged, 4 = log-interrupted
  (#1233). ([e1d1d0d](https://github.com/jscott3201/rusty-bacnet/commit/e1d1d0d35367438d4da43b1e6bf76e0240cf95af))

- **Trend Log Multiple (Rust API, #1235):** `set_logging_type` takes a
  `LoggingType` and returns `Result`; handle the VALUE_OUT_OF_RANGE a COV
  value now gets. `TrendLogObject::set_logging_type` takes a `LoggingType`
  too. Object wrappers forward the new
  `BACnetObject::refresh_log_window_internal` hook. ([72c1770](https://github.com/jscott3201/rusty-bacnet/commit/72c17700434f51678eab44474bc41bbbe4e5f7e6))

- **Audit Log persistence (Rust API, #1270):** custom Audit Log persistence
  runs on a plain `std` thread with no Tokio context, and a panic there fails
  the save. ([1d3eb21](https://github.com/jscott3201/rusty-bacnet/commit/1d3eb211c7700a20b59320ddf968c4fe8c269744))

- **Server (wire, #1257):** the server no longer answers a confirmed request sent by broadcast or
  multicast. A client that relied on that must address the device directly, by unicast or a routed
  DNET/DADR. ([76b2b84](https://github.com/jscott3201/rusty-bacnet/commit/76b2b84f0d75ce1d0d2b7a17713c10dbfaf5a945))

- **Lighting Output Lighting_Command (#1263):** a read returns
  `PropertyValue::ApplicationData` (Python `application_data`) holding the encoded
  command, and a write takes that encoding instead of an octet string. Build it with
  `bacnet_encoding::constructed::encode_lighting_command`, or set it with
  `LightingOutputObject::set_lighting_command`. A new object reads operation NONE,
  which can't be written back. ([91ce62a](https://github.com/jscott3201/rusty-bacnet/commit/91ce62a13f533db07b8a6c5586c2facf75edeb03))

- **Event notification codecs (Rust API, #1276):** `EventNotificationRequest`,
  `NotificationParameters` and `BACnetPropertyValue` lose their `encode` and
  `decode` methods; call `encode_event_notification`,
  `decode_event_notification`, `encode_notification_parameters`,
  `decode_notification_parameters`, `encode_bacnet_property_value` or
  `decode_bacnet_property_value` in `bacnet_encoding::constructed`. Build an
  Event Log notification record from the typed request, not its bytes. ([ea05d58](https://github.com/jscott3201/rusty-bacnet/commit/ea05d5890885085fd54f7e3e3f6a0d186434fda2))

- **Structured View (Rust API, #1285):**
  `StructuredViewObject::add_subordinate` returns `Result`, and the Structured
  View subordinate fields are private: replace them with `set_subordinates`
  and read them with `subordinates()`. ([9cd82bc](https://github.com/jscott3201/rusty-bacnet/commit/9cd82bc9c4126d5fa0953099a9fa5fdd898e52c2))

- **Python reads and local writes (#1296, #1297):** a whole array or list (such
  as Object_List or Priority_Array) is a `list` even with one element, a
  constructed value is `application_data` bytes unless it is one of the typed
  reads listed under #1310, #1344 and #1345, and an empty value is
  `PropertyValue.list([])`. `BACnetServer.read_property` and
  `write_property_local` raise `BacnetProtocolError` as network requests do: a
  missing object is UNKNOWN_OBJECT, not `RuntimeError`. ([ce572cf](https://github.com/jscott3201/rusty-bacnet/commit/ce572cf812e35750d255bfc885eb5ebdfe364548))

- **Device reference setters (Rust API, #1308):**
  `EventEnrollmentObject::set_object_property_reference` returns `Result`;
  `GlobalGroupObject::group_members` is private, so set it with
  `set_group_members` or `add_group_member` (both `Result`) and read it with
  `group_members()`. ([7ade5e3](https://github.com/jscott3201/rusty-bacnet/commit/7ade5e36df847e9d7909ff6d4b9d7615824cd0f4))

- **Python typed reads (#1310):** Recipient_List, List_Of_Group_Members,
  Group Present_Value, Action, Door_Members, Access_Doors, Target_References,
  Supported_Formats and Stages read as typed elements in the typed write's
  form. A 0.11.0 local read gave the stored form (`application_data` octets
  for Recipient_List, a flat list for the others) and a client read only the
  first application-tagged value; compare against the typed form. Writing
  back is unchanged. ([e04ef28](https://github.com/jscott3201/rusty-bacnet/commit/e04ef2883cafe4f8b1a3958a3b5395928723a183))

- **Loop and Pulse Converter references (#1312):** a read returns
  `PropertyValue::ApplicationData` (Python `application_data`) holding the encoded
  reference, and a write takes that encoding; decode it with
  `bacnet_encoding::constructed::decode_object_property_reference`, or
  `decode_setpoint_reference` for Setpoint_Reference. Clear a reference by writing
  one to instance 4194303, or Setpoint_Reference with the empty value; Null changes
  nothing, and an empty [0] frame or unframed members are refused. ([808e0c2](https://github.com/jscott3201/rusty-bacnet/commit/808e0c2903b79898c17009d9f583c1986c248fe0))

- **Access Rights (Rust API, #1316):** `BACnetAccessRule` takes the Clause 21
  shape: the specifiers are `AccessRuleTimeRangeSpecifier` and
  `AccessRuleLocationSpecifier`, and the time range is a
  `BACnetDeviceObjectPropertyReference`. `BACnetAccessRule::new` builds one
  from optional references. ([2323bf7](https://github.com/jscott3201/rusty-bacnet/commit/2323bf7cfd07a888ebf64c1272cec84657c1e642))

- Code that matched `CovAckResult::Error` matches `CovAckResult::Error(_)`, or binds the `Refusal` to read what the peer answered (#1323). ([0863c0f](https://github.com/jscott3201/rusty-bacnet/commit/0863c0f590582684016b70b3ff1f89442509b2a5))

- **Event_Algorithm_Inhibit (Rust API, #1329):** `BACnetObject` gains the
  hidden hooks `event_algorithm_inhibit_reference_internal` and
  `follow_event_algorithm_inhibit_internal`, and `ObjectDatabase` gains
  `follow_event_algorithm_inhibit`, which the bundled server calls before it
  evaluates an object. A wrapper object that forwards its hooks forwards
  both; a custom intrinsic reporter can serve the rows by answering them. ([d63feb9](https://github.com/jscott3201/rusty-bacnet/commit/d63feb908613f0550d858a19bef70a25ebc67796))

- **Confirmed answers (Rust API, #1342):** `bacnet_server::server::CovAckResult`
  has a `Data(Bytes)` variant, the service data of a ComplexAck answering a
  read the server sent, and is no longer `Copy`; add an arm for it where you
  match the enum, and clone where you copied it. ([cbe62f3](https://github.com/jscott3201/rusty-bacnet/commit/cbe62f311b478671b7cafadb789affb2fbf238c9))

- **Python typed access reads (#1344):** compare reads of Entry_Points,
  Exit_Points and Credentials with `ObjectIdentifier` or `(device, object)`
  values tagged `device_object_reference`, not 0.11.0's plain object
  identifiers; writing a read value back is unchanged. ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Python typed constructed reads (#1345):** reads of the properties in
  [the typed constructed values table](docs/python-api.md#typed-constructed-values)
  return the forms it gives (mappings, tuples, `BACnetTimeStamp`) instead of
  0.11.0's octets or flat lists, so a read that gave
  `PropertyValue.application_data` no longer equals it; writing a read value
  back sends the same octets. ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Trend Log (Rust API, #1354):** `TrendLogObject::set_logging_type`
  returns `Result`; handle the OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED a COV
  value gets until COV acquisition lands (#1480). To stop a polled Trend Log,
  clear Enable or make it TRIGGERED: writing its Log_Interval from nonzero to
  zero is refused the same way. ([1be8248](https://github.com/jscott3201/rusty-bacnet/commit/1be824832f0d675a17f3f3cf9846eb230287c57b))

- **Python integer range errors (#1360):** any integer argument outside the
  type of the field it fills raises `OverflowError`, as a parameter, a tuple
  member or a mapping value; catch it where you caught `ValueError` or
  `BacnetProtocolError` for, say, `BACnetTimeStamp.sequence_number(65536)`, a
  Destination `process_identifier` past 2**32 - 1, a channel number past
  65535 or a mapping's negative array index. Values that fit but BACnet
  refuses still raise `ValueError` or `BacnetProtocolError`. ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Local writes (Rust API, #1367):** await `write_local`, `write_local_encoded` and the `*_local`
  setters inside a Tokio runtime. Polled by another executor, they now fail with `Error::Encoding`
  before anything is written. ([aef70c4](https://github.com/jscott3201/rusty-bacnet/commit/aef70c43149af99d5c3dcc3d36d5ccc3313f7eb1))

- **bacnet-encoding (Rust API, #1374):** `tags::decode_optional_context` is
  removed. Peek with `constructed::tagged::next_is_context`, then read the
  member with `decode_ctx_primitive` or a typed reader, or wrap one in
  `decode_optional_ctx`. ([d682790](https://github.com/jscott3201/rusty-bacnet/commit/d68279070d0a21da717d84f10db7854b59a2f10b))

- **Color and Color Temperature Color_Command (#1386):** a read returns
  `PropertyValue::ApplicationData` holding the encoded command, and a write takes
  that encoding instead of an octet string. Build it with
  `bacnet_encoding::constructed::encode_color_command`, or set it with
  `set_color_command` on `ColorObject` or `ColorTemperatureObject`. A new object
  reads operation NONE, which can't be written back. ([97baf49](https://github.com/jscott3201/rusty-bacnet/commit/97baf4921afafc2ae9018a0dd68706de4e879bcd))

- **I-Am announcements under DCC (Rust API, #1388):**
  `BACnetServer::broadcast_i_am()` and `IAmBroadcaster::broadcast_i_am()`
  return `Error::Protocol` with `SERVICES` / `COMMUNICATION_DISABLED` while a
  remote DeviceCommunicationControl restricts initiation. An announce loop
  that unwraps the result must treat that error as a skipped announcement. ([cd9c650](https://github.com/jscott3201/rusty-bacnet/commit/cd9c650d1042f0aeb9501c94785e821a545a1dc3))

- **DCC state (Rust API, #1399):** `BACnetServer::comm_state()` returns
  `bacnet_server::server::DccState`. Compare with `DccState::Enable` or
  `DccState::DisableInitiation`, call `initiation_restricted()`, or take
  `EnableDisable::from(state).to_raw()` where the old 0 or 2 is needed; that
  is a `u32`, where the old getter returned a `u8`. ([e9472a8](https://github.com/jscott3201/rusty-bacnet/commit/e9472a845f846dbb32285fdfbbc8e352d59dcac5))

- **Server drop (Rust API, #1270, #1409):** durable saves now finish on a writer thread, so storage
  may still change after a `BACnetServer` dropped without `stop()` is gone. Call `stop().await`
  before dropping a server and building another on the same storage. ([9a24975](https://github.com/jscott3201/rusty-bacnet/commit/9a24975c693bb99f132fedcf73fe8474cbbb8263))

- **Unset references (wire and Python API, #1417):** Controlled_Variable_Reference,
  Manipulated_Variable_Reference, Input_Reference, a Trend Log's
  Log_DeviceObjectProperty and Object_Property_Reference no longer read NULL
  while unset. Treat a reference whose object or Device instance is 4194303 as
  unset, and clear one by writing such a reference, not NULL; clear
  Fault_Parameters with its context-tagged `none` choice. ([5555911](https://github.com/jscott3201/rusty-bacnet/commit/55559112ff4daadbb7570f8c29d3839ab53750a2))

- **DCC handler (Rust API, #1430):**
  `bacnet_server::handlers::handle_device_communication_control` is removed.
  Let a `BACnetServer` answer DeviceCommunicationControl under its `DccPolicy`
  and `dcc_password`, and read the state with `BACnetServer::comm_state()`, a
  `DccState`. A custom dispatcher decodes the request with
  `bacnet_services::device_mgmt::DeviceCommunicationControlRequest::decode`
  and applies its own checks. ([eeb9dd9](https://github.com/jscott3201/rusty-bacnet/commit/eeb9dd9c7fe2f666423d82a378a42592352367f1))

- **DCC state (Python API, #1431):** compare `await server.comm_state()` with
  `EnableDisable.ENABLE` or `EnableDisable.DISABLE_INITIATION`. The result
  never equals 0 or 2 and is truthy in both states, so
  `if await server.comm_state():` no longer tells them apart. Call
  `.to_raw()` where the number is needed. ([eeb9dd9](https://github.com/jscott3201/rusty-bacnet/commit/eeb9dd9c7fe2f666423d82a378a42592352367f1))

- **Multi-state objects (Rust API, #1443):** Number_Of_States rows carry the
  new `PropertyWriteCapability::Through(STATE_TEXT)`, which doesn't count as
  writable; the PICS row carries `PropertySupport::written_through`. ([d63feb9](https://github.com/jscott3201/rusty-bacnet/commit/d63feb908613f0550d858a19bef70a25ebc67796))

- **Decoding errors (Rust API, #1446):** add `..` to patterns on
  `Error::Decoding { offset, message }`, or bind `kind`, and build one with
  `Error::decoding` or a kind's constructor (`invalid_tag`, `missing`,
  `trailing`, `out_of_range`, `overflow`) rather than a struct literal. Match
  `kind: DecodingKind::InvalidTag` where you matched `Error::InvalidTag`, and
  turn a request's decode error into its Reject with
  `Error::into_request_reject`. ([d54b143](https://github.com/jscott3201/rusty-bacnet/commit/d54b143114dbc71de28731df2362893a34824393))

- **Answer matching (Rust API, #1465):** `CanonicalPeer::from_source` takes the
  known local network number as a third argument (`None` keeps the old
  matching). ([520abe7](https://github.com/jscott3201/rusty-bacnet/commit/520abe781edc7613dfb2a38e0c8a20e2b31d6cdd))

- **Broadcast sends (Rust API, #1479):** send a remote network's broadcast
  with `NetworkLayer::broadcast_to_network`, not
  `send_apdu_routed_via_local_broadcast` with an empty `dest_mac`. Send a
  confirmed request, an acknowledgement, an Error, a Reject or an Abort to
  one device: the broadcast sends and a routed send with no DADR refuse it,
  and so do confirmed requests from `BACnetClient`, the endpoint and Python
  to any group address, all before anything is sent. ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- **Who-Is and Who-Has ranges (Rust API, #1483):** `WhoIsRequest::range(low,
  high)` is gone. Write `range: None` or
  `range: Some(DeviceInstanceRange::new(low, high)?)` in `WhoIsRequest` and
  `WhoHasRequest`, and pass that one value to `BACnetClient::who_is`,
  `who_is_directed`, `who_is_network` and `who_has`. `single(n)` and
  `from_limits` return a `Result`; `device(oid)` cannot fail. Python callers
  pass both limits or neither. ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- **Property metadata (Rust API, #1485):**
  `PropertyPresenceCondition::IntrinsicReporting` is split into
  `IntrinsicReportingRequired`, which `PropertyMetadata::is_required` counts,
  and `IntrinsicReportingOptional`; a custom object's rows pick the one its
  table's footnotes give. ([d63feb9](https://github.com/jscott3201/rusty-bacnet/commit/d63feb908613f0550d858a19bef70a25ebc67796))

- **Accumulator Scale and Prescale (#1487):** a client decoding the old
  application-tagged Scale or Prescale decodes the context-tagged forms
  instead. In Python, Scale's tag is `"scale"`, not `"real"`, and Prescale
  reads as `(5, 100)`, not `[5, 100]`. In Rust, `read_property` returns
  `PropertyValue::ApplicationData`, not a `List`; decode it with
  `bacnet_encoding::constructed::{decode_scale, decode_prescale}`. ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Device bindings (Rust and Python API, #1493):** a binding at a multicast
  address, 255.255.255.255 or the broadcast IP at another port, as the
  device's own address or its router's, now stops `build()` and `start()`.
  Bind each device, or its router, at its unicast address. ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Dates (Python API, #1501):** a `"date"` value's `.value` gives the full
  year, so drop any `+ 1900` applied to it; an unspecified year still reads
  as 255, `rusty_bacnet.UNSPECIFIED`. `PropertyValue.date` takes 1900 to 2154
  or 255. `time_synchronization` and `utc_time_synchronization` need a real
  date with its own weekday and a time with no field unspecified. ([17404bb](https://github.com/jscott3201/rusty-bacnet/commit/17404bb55178e00727f2431bc2ccbc44a87599ad))

- **DCC password (Rust and Python API):** a password alone no longer enables
  DeviceCommunicationControl. Select `RequirePassword` (`"require_password"`)
  with a password, or the **insecure** `LegacyPermissive` mode; exhaustive
  `ServerConfig` literals need `dcc_policy`. See
  [DCC policy](docs/dcc-policy.md).

- **Request encoders (Rust API, #771, #780, #793, #798, #805, #808):** the
  ReadRange, ReadPropertyMultiple, WriteProperty, WritePropertyMultiple,
  AddListElement, RemoveListElement, SubscribeCOV and
  SubscribeCOVPropertyMultiple request encoders return `Result`; handle the
  error where you call one directly.

- **Request admission (Rust and Python API):** exhaustive `ServerConfig`
  literals and patterns need the new fields; see
  [request admission](docs/request-admission.md).

### Security

- One event notification is forwarded to at most 64 destinations across all
  Notification Forwarders, and a retransmitted ConfirmedEventNotification is
  not forwarded again (#1259). ([76b2b84](https://github.com/jscott3201/rusty-bacnet/commit/76b2b84f0d75ce1d0d2b7a17713c10dbfaf5a945))

### Added

- **Rust and Python API:** an SC hub can bind verified leaf certificate
  SHA-256 identities to provisioned UUID and VMAC groups (#800).

- Standalone clients passively learn and answer local Network Number controls
  on transports that opt in (#879). ([a8194c1](https://github.com/jscott3201/rusty-bacnet/commit/a8194c1425cd192d59cee2ff010a296ff26ad6df))

- **Python and Rust API:** `rusty_bacnet.list_serial_ports()` and
  `bacnet_transport::mstp_serial::available_ports()` list the serial ports the
  OS reports, for MS/TP (#951).

- **Rust API:** `BACnetClient::transport()` borrows a built client's
  transport, so SC link state, B/IP counters and BBMD state, and MS/TP
  diagnostics are reachable after `build()` (#956). ([5e9e676](https://github.com/jscott3201/rusty-bacnet/commit/5e9e6763b3224ff7dfc2e19ad7a6c9a132587a3a))

- `bacnet tui` opens a read-only, full-screen terminal UI with a live device
  table and a Who-Is form, behind the opt-in `tui` feature (`--features tui`). Release builds leave it
  out until the terminal UI ships (#961, part of #975). ([6d866b6](https://github.com/jscott3201/rusty-bacnet/commit/6d866b683f92c23fd06a0ed0d2ab4e5e17f4ccc8))

- Staging objects support COV with a COV_Increment, reporting Present_Value,
  Status_Flags and Present_Stage, and the new `cov_reported_properties` hook
  lists what a report carries (#988). ([943b8fa](https://github.com/jscott3201/rusty-bacnet/commit/943b8fabc2cdc5d358a5ce4799cc797c5e214c5e))

- Python `add_elevator_group` takes an optional `machine_room_id`, which must name a Positive Integer Value object (#1023). ([9270394](https://github.com/jscott3201/rusty-bacnet/commit/9270394fed9218eaf5122140e3a624e1a633e1e6))

- **Wire and Rust API:** the application can set Energy_Meter_Ref on an
  Escalator and on a Lift, which gains the optional property. While a
  reference is set, Energy_Meter reads 0.0 (#1036). ([c88f61a](https://github.com/jscott3201/rusty-bacnet/commit/c88f61afece69424171fc35e970b7aefaf6703d8))

- Notification Forwarder Subscribed_Recipients: a `SubscribedRecipients` store with minute
  lifetimes for an application's forwarder, edited by AddListElement and RemoveListElement
  by recipient and process identifier (#1049). ([0bfcfd9](https://github.com/jscott3201/rusty-bacnet/commit/0bfcfd97fa2bb488959e03e9a28f28c0e3a4eda0))

- **Breaking (wire):** the Lift serves nine more optional Table 12-77 rows,
  among them its landing and car calls, door commands, car mode and drive
  status, set by the application and writable while out of service (#1052). ([bfeba23](https://github.com/jscott3201/rusty-bacnet/commit/bfeba23626e30614e99929c6e8a52f0e072286fc))

- **Wire:** a Schedule's Present_Value is writable while Out_Of_Service is
  TRUE, and each written value goes on to the references at
  Priority_For_Writing (#1055). ([122b629](https://github.com/jscott3201/rusty-bacnet/commit/122b6295263ed98daa8f360d5a3c81a1a8b3bcc5))

- **Wire:** a Schedule accepts writes of Weekly_Schedule, Exception_Schedule
  and Effective_Period and applies them at once, and reports
  CONFIGURATION_ERROR while its values mix datatypes (#1057, #1056). ([5e884c5](https://github.com/jscott3201/rusty-bacnet/commit/5e884c58d099ec9a910f1d8df23c985594217f88))

- A running server's application can update a Loop's Controlled_Variable_Value
  through `set_controlled_variable_value_local`, in Rust and Python, with COV
  for property subscribers (#1063). ([f04e5e1](https://github.com/jscott3201/rusty-bacnet/commit/f04e5e1434bcf0f4400ba35cf5b939816d9d986b))

- **Rust API:** the new `CovCounters::untimed_references_oversized`
  counts untimestamped COV-multiple references left out of a report for being
  too large for one notification (#1066). ([3b8fd74](https://github.com/jscott3201/rusty-bacnet/commit/3b8fd74240b9ab8219afe4a72016f211edb5d7d6))

- **Breaking (wire, Rust API):** a running server's application can feed an
  Averaging object samples with `add_averaging_sample_local`, and the object
  takes SubscribeCOVProperty. `AveragingObject::add_sample` returns `Result`
  (#1083). ([417f798](https://github.com/jscott3201/rusty-bacnet/commit/417f7985b8ca6b785cc7ae5a17aa61a88136c03a))

- **Python API:** `BACnetServer.cov_counters()` returns every Rust
  `CovCounters` field in a dict, typed by a new `CovCounters` TypedDict
  (#1084). ([97d084c](https://github.com/jscott3201/rusty-bacnet/commit/97d084cdb9406e0b78d83d8bfce486d95a3cfbe1))

- **Breaking (wire, Rust API):** a Schedule's
  List_Of_Object_Property_References and Priority_For_Writing are
  network-writable, and a target that refuses the schedule's datatype faults
  the Schedule (#1088, #1086). ([7762130](https://github.com/jscott3201/rusty-bacnet/commit/77621307d247d88a91dcb0478a844ffd54bcf880))

- **Breaking (wire, Rust and Python API):** the Averaging object serves
  Window_Interval and Window_Samples and computes its statistics over a
  sliding window instead of every sample since creation (#1092).

- **Breaking (wire, Rust API):** Life Safety Point and Zone gain
  Accepted_Modes, which bounds Mode writes, the Zone gains Tracking_Value, and
  Global Group gains Event_State and Member_Status_Flags (#1092).

- **Breaking (wire, Rust API):** Integer, Positive Integer and Large Analog
  Value gain Units, the Pulse Converter gains its count rows and derives
  Present_Value from Count, and the lighting objects gain their required rows
  (#1092).

- **Rust API:** Python servers take a `cov_policy` dict, and the new
  `CovPolicy::validate` refuses zero caps or budgets and unreachable reserved
  peers, so a Rust server configured with them doesn't start (#1100). ([4535b22](https://github.com/jscott3201/rusty-bacnet/commit/4535b222d513c705c3080ee57297e3cb3d014aa1))

- **Wire:** a Life Safety Point or Zone accepts Tracking_Value and Reliability
  writes while Out_Of_Service is TRUE, and both objects implement
  `set_reliability_internal` (#1108). ([e2ff2d0](https://github.com/jscott3201/rusty-bacnet/commit/e2ff2d0f10d34a79f4bae2804de7ba6696c15f06))

- **Breaking (wire):** The 12 commandable value types serve
  Current_Command_Priority, Integer, Positive Integer and Large Analog Value
  serve a writable COV_Increment that gates their COV notifications, and
  Lighting Output's Default_Fade_Time is writable from 100 to 86,400,000 ms
  (#1111). ([d81bcfa](https://github.com/jscott3201/rusty-bacnet/commit/d81bcfa04d6669bb474a8b0503e83ceecfcccee5))

- **Wire:** AddListElement and RemoveListElement edit a Schedule's
  List_Of_Object_Property_References, and a member naming this server's own
  Device is taken as a local reference (#1121, #1122). ([0c655f6](https://github.com/jscott3201/rusty-bacnet/commit/0c655f61bb445d67d2c99c5b941283a1c13f7878))

- **Breaking (Rust API):** a running server's application can set a Life
  Safety Point or Zone's Present_Value and Tracking_Value through
  `set_present_value_local` and the new `set_tracking_value_local` (#1123). ([27a7ada](https://github.com/jscott3201/rusty-bacnet/commit/27a7ada1b7b75906b0b531948ba24a6d56aa88d2))

- **Wire:** a Staging Target_References element naming this server's own
  Device is accepted as a local reference, as a Schedule's is (#1136). ([2edc489](https://github.com/jscott3201/rusty-bacnet/commit/2edc48980ef4108e590d6235026708d820b414c3))

- `event_notification_counters()` has a count per cause, including
  device_recipient_unbound, recipient_unroutable,
  confirmed_broadcast_recipient, unconfirmed_send_failed, apdu_too_large,
  received_not_forwarded, forwarding_cap_dropped, received_not_logged (#1142,
  #1160, #1196, #1225, #1259, #1346). ([9220fce](https://github.com/jscott3201/rusty-bacnet/commit/9220fce7d794debaa632a6f80ea35939d984015a))

- **Breaking (Rust API):** a running server samples an Averaging object's
  Object_Property_Reference itself, once per Window_Interval / Window_Samples
  seconds; application samples still count (#1144). ([37db5e0](https://github.com/jscott3201/rusty-bacnet/commit/37db5e075083766a4d738ecd200181a29dfb16d6))

- **Wire:** Access Door serves Alarm_Values, Fault_Values and Masked_Alarm_Values and
  reports CHANGE_OF_STATE on Door_Alarm_State, with fault values as
  MULTI_STATE_FAULT, to its Notification Class recipients (#1149). ([a81304f](https://github.com/jscott3201/rusty-bacnet/commit/a81304f82c9a7d51a0d8b8f5d325ba698d6f5e33))

- **Breaking (wire):** writing a Command object's Present_Value runs the
  Action list it selects through the local write path, tracking In_Process and
  All_Writes_Successful; a number past the Action size is refused (#1150). ([c6decbd](https://github.com/jscott3201/rusty-bacnet/commit/c6decbd5edd9034f52595a66754607e4ef730883))

- **Channel object (wire):** Channel (type 53) is back, with its three arrays,
  Last_Priority and Write_Status. A running server passes each Present_Value
  write to the local members, coerced per Table 12-63, at the written priority
  (#1151). ([36d8b72](https://github.com/jscott3201/rusty-bacnet/commit/36d8b7227baafbc1629420e642d16c248aeefe8c))

- **WriteGroup (wire):** the server executes inbound WriteGroup on its Channels
  and declares it, Channels serve Allow_Group_Delay_Inhibit, and
  `BACnetClient::write_group` (Python too) sends to a device or a broadcast
  (#1151). ([50273a5](https://github.com/jscott3201/rusty-bacnet/commit/50273a5236cb5ed52f9d4354ced13f80e94f6fbf))

- **Router control receiver (#1175):**
  `RouterOptions::network_control_receiver` adds a `ReceivedNetworkControl`
  receiver for rejects addressed to the router itself, which still update the
  routing table and are no longer relayed. ([5c1afb3](https://github.com/jscott3201/rusty-bacnet/commit/5c1afb3f1dc2e9a65b25866709c67799f0f34046))

- Python `add_command` takes keyword-only `action` (lists of `ActionCommand` mappings) and `action_text`, so a Python server's Command runs its lists (#1179). ([4c20f13](https://github.com/jscott3201/rusty-bacnet/commit/4c20f135e523b39e53e1dba60f6181545998a259))

- **Breaking (wire):** A Command action naming another device is written there as a confirmed WriteProperty, addressed from the server's device bindings (#1180). ([2a8495a](https://github.com/jscott3201/rusty-bacnet/commit/2a8495a32cb03363e7f8712697e215ee526d2f7e))

- **Breaking (wire):** Trend Log Multiple objects are polled, one value per
  member in each record, and Log_Buffer serves each record framed as
  BACnetLogMultipleRecord (#1203). ([eb86197](https://github.com/jscott3201/rusty-bacnet/commit/eb8619781f2917baa381a61d65fd671dc76bf987))

- `SessionConfig::read_work_limit` and the endpoint builders' `read_work_limit`
  set the shared endpoint's ReadProperty work limit, which a Group's
  Present_Value read is charged to (default 256) (#1215). ([7ad513c](https://github.com/jscott3201/rusty-bacnet/commit/7ad513c5e8ca1dac9df0c386bc7a6a4e1ba5d047))

- **Breaking (wire):** The Notification Forwarder (type 51) returns and forwards received and local event
  notifications to its Recipient_List and Subscribed_Recipients, which can persist across restarts; the
  server now executes ConfirmedEventNotification and UnconfirmedEventNotification (#1225). ([fafc1dd](https://github.com/jscott3201/rusty-bacnet/commit/fafc1ddf07b6f4171f219274d1e2b5ae5c08c529))

- **Breaking (wire):** Lighting Output serves a writable COV_Increment that gates its COV notifications (#1227). ([0d45e79](https://github.com/jscott3201/rusty-bacnet/commit/0d45e792eb1df42ea8bc5bdd71678ed7339c358c))

- **Python API:** `add_trend_log_multiple` takes the members, Log_Interval,
  Logging_Type, Start_Time / Stop_Time window and alignment as keyword
  arguments, so a Python server can run a polled or triggered log (#1235). ([72c1770](https://github.com/jscott3201/rusty-bacnet/commit/72c17700434f51678eab44474bc41bbbe4e5f7e6))

- **Wire:** Trend Log Multiple serves Start_Time, Stop_Time, Align_Intervals,
  Interval_Offset and Trigger, and the server logs inside that window, at
  clock-aligned times and once per Trigger (#1235). ([72c1770](https://github.com/jscott3201/rusty-bacnet/commit/72c17700434f51678eab44474bc41bbbe4e5f7e6))

- **Wire:** Audit Log Buffer_Size takes writes while logging is off, keeping the
  newest records that fit, and `BACnetServer::purge_audit_log` (also in Python)
  lets the application purge a log, leaving a BUFFER_PURGED record (#1238). ([3acf048](https://github.com/jscott3201/rusty-bacnet/commit/3acf0488d3bf760ad00eaefca008d73c857e4c43))

- **Tracked router control receiver (#1242):**
  `RouterOptions::network_control_receiver_with_admission` returns the
  router's network-control receiver as an `AdmissionReceiver`, whose counters
  report its queue depth, high-water mark and drops. ([85f3d2e](https://github.com/jscott3201/rusty-bacnet/commit/85f3d2e24ad1951ff442a8d3ff057956bea3e3a9))

- **Loopback unicast destinations (#1243):**
  `LoopbackTransport::record_unicast_destinations` reports the MAC each
  unicast was sent to, in the order the peer receives the frames, so tests
  can check where a unicast went. ([85f3d2e](https://github.com/jscott3201/rusty-bacnet/commit/85f3d2e24ad1951ff442a8d3ff057956bea3e3a9))

- Python `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take a keyword-only
  `read_work_limit` (default 256, zero refused) and gain `add_group` (#1250). ([53f39c1](https://github.com/jscott3201/rusty-bacnet/commit/53f39c1cc1d4ac511cd8227cc80ddc7687d67c48))

- **Rust API:** A Notification Forwarder built with persistence keeps a written
  Recipient_List across a restart too, and it wins over configured destinations (#1256). ([1d3eb21](https://github.com/jscott3201/rusty-bacnet/commit/1d3eb211c7700a20b59320ddf968c4fe8c269744))

- Python `add_notification_forwarder` takes keyword-only `recipients` and `port_filter`
  to seed Recipient_List and Port_Filter, and `BACnetServer.forwarder_save_counters()`
  reports failed Subscribed_Recipients saves (#1260). ([6ef5fe4](https://github.com/jscott3201/rusty-bacnet/commit/6ef5fe452a3a29dbee8759b69ddc7238cfdc9187))

- **Python API:** `BACnetServer.add_channel` registers a Channel with its
  members, execution delays, control groups and Allow_Group_Delay_Inhibit.
  Its members, and `add_trend_log_multiple`'s, are `(object, property)`
  tuples or mappings (#1262). ([c59f965](https://github.com/jscott3201/rusty-bacnet/commit/c59f9656afd3e21e794daa1eca040cfa9428a19d))

- **Wire:** A Channel member in another device is written there as a confirmed WriteProperty, and the Channel serves Reliability, naming why a FAILED distribution failed (#1264). ([0863c0f](https://github.com/jscott3201/rusty-bacnet/commit/0863c0f590582684016b70b3ff1f89442509b2a5))

- `bacnet read-range` decodes Trend Log, Event Log, Trend Log Multiple and
  Audit Log records instead of printing hex; its JSON for a log buffer lists
  them under `records` instead of `items` (#1274). ([ea05d58](https://github.com/jscott3201/rusty-bacnet/commit/ea05d5890885085fd54f7e3e3f6a0d186434fda2))

- A server with a valid Device clock records the device's own event notifications
  in each Event Log, except BUFFER_READY reports on an Event Log's buffer and
  reports from a local Event Enrollment that watches a local Event Log (#1275). ([019a20b](https://github.com/jscott3201/rusty-bacnet/commit/019a20b6688f7095c9526549da7dcbb8d07c6e44))

- **Wire and Rust API:** Access Point serves Authentication_Status, DISABLED
  while out of service, and Access_Event_Credential, which `set_access_event`
  takes with each event (#1284). ([9cd82bc](https://github.com/jscott3201/rusty-bacnet/commit/9cd82bc9c4126d5fa0953099a9fa5fdd898e52c2))

- **Wire:** Access Zone serves Occupancy_State, Event_State and its
  occupancy-counting properties, with Adjust_Value writable (#1284). ([9cd82bc](https://github.com/jscott3201/rusty-bacnet/commit/9cd82bc9c4126d5fa0953099a9fa5fdd898e52c2))

- Python `BACnetServer.add_group` takes `members`, checked as the endpoint `add_group`
  checks them, so a Python server serves a Group's Present_Value (#1286). ([6ef5fe4](https://github.com/jscott3201/rusty-bacnet/commit/6ef5fe452a3a29dbee8759b69ddc7238cfdc9187))

- **Loopback data attributes (#1289):** `LoopbackTransport::carry_data_attributes`
  hands the peer the data attributes each frame is sent with, so tests can
  check the attributes a router sends back. By default they are still dropped. ([a48c931](https://github.com/jscott3201/rusty-bacnet/commit/a48c931e7e209e30959d1ddb8ac88586a2db9a95))

- Python `BACnetServer` takes a keyword-only `time_sync_policy` dict for the time-sync
  allowlist, step cap, rate limits and coalescing, and
  [time sync policy](docs/time-sync-policy.md) documents the policy (#1292). ([6ef5fe4](https://github.com/jscott3201/rusty-bacnet/commit/6ef5fe452a3a29dbee8759b69ddc7238cfdc9187))

- **Rust API:** `BACnetServer::write_local_encoded` writes a local property from
  its encoded value through the WriteProperty handler's decoding (#1296). ([ce572cf](https://github.com/jscott3201/rusty-bacnet/commit/ce572cf812e35750d255bfc885eb5ebdfe364548))

- `NetworkLayer::local_network_number` is a lock-free handle to the local network number for
  stacks and adapters built directly on `NetworkLayer`; the full server and client fill their own
  layers internally (#1298). ([5e66d84](https://github.com/jscott3201/rusty-bacnet/commit/5e66d84a1278aa6fd8795cc2487653d77334fad7))

- **Wire:** Access Zone reports CHANGE_OF_STATE on Occupancy_State, with its
  event properties and Alarm_Values, to its Notification Class recipients
  (#1305). ([bbf4acb](https://github.com/jscott3201/rusty-bacnet/commit/bbf4acb25aa4100df6fd1c289ff56cc71473376a))

- **Wire:** Access Point serves Active_Authentication_Policy,
  Number_Of_Authentication_Policies, Authorization_Mode and
  Priority_For_Writing; clients can write the policy in effect and the mode
  (#1307). ([ecc1af4](https://github.com/jscott3201/rusty-bacnet/commit/ecc1af46d41a04c886f5909d7866b62e0592e7d8))

- **Python API:** an `ActionCommand` for `add_command` accepts
  `write_successful` and ignores it, so the mappings a read of Action returns
  can be given back (#1310). ([e04ef28](https://github.com/jscott3201/rusty-bacnet/commit/e04ef2883cafe4f8b1a3958a3b5395928723a183))

- A Notification Class can keep a written Recipient_List across a restart:
  `NotificationClass::with_persistence` with `FileNotificationClassPersistence`, or
  `storage_path` on Python's `add_notification_class` (#1315). ([aaba646](https://github.com/jscott3201/rusty-bacnet/commit/aaba646b05bd788928cf10603482511899d5ba4a))

- **Rust and Python API:** Access Rights serves positive and negative access
  rules, set with `set_positive_access_rules`, `set_negative_access_rules` or
  `add_access_rights`, read in Python as `AccessRule` mappings. **Breaking
  (Rust API):** `BACnetAccessRule` takes its Clause 21 shape (#1316, #1344). ([2323bf7](https://github.com/jscott3201/rusty-bacnet/commit/2323bf7cfd07a888ebf64c1272cec84657c1e642))

- **Wire:** Target Audit reporting records each Channel write of an inbound
  WriteGroup as a WRITE of its Present_Value from the requester. A Channel's
  Present_Value counts as commandable, so its WriteProperty records carry the
  priority and drop at priorities Audit_Priority_Filter disables (#1318). ([cbe62f3](https://github.com/jscott3201/rusty-bacnet/commit/cbe62f311b478671b7cafadb789affb2fbf238c9))

- **Wire:** A Command action or Channel member in a device with no fresh binding sends one Who-Is for it, globally or where its last I-Am came from, and waits the APDU timeout for the I-Am; at most one a minute per device, none while DCC restricts initiation (#1322). ([83cfadb](https://github.com/jscott3201/rusty-bacnet/commit/83cfadb9ff9e434be27946a29b590c2ebaa39c4b))

- **Wire, Python API:** every object that reports intrinsically serves
  Event_Message_Texts_Config, whose texts replace the server's Message Text,
  and Event_Algorithm_Inhibit with its _Ref, which suspends the event
  algorithm but not fault reporting and can follow a local property (#1329). ([d63feb9](https://github.com/jscott3201/rusty-bacnet/commit/d63feb908613f0550d858a19bef70a25ebc67796))

- **Wire and Rust API:** Access Rights takes WriteProperty and
  WritePropertyMultiple of Positive_Access_Rules and Negative_Access_Rules,
  whole, by index or resized at index 0, with the setters' checks; either
  array holds at most `MAX_ACCESS_RULES` (1024) rules (#1330). ([997733d](https://github.com/jscott3201/rusty-bacnet/commit/997733d52de1a6ef06d3698c179b2115044b9bc4))

- **Wire, Rust and Python API:** Access Rights serves its required Enable row
  (property 133), TRUE by default and writable, set with `set_enable` or the
  `add_access_rights` `enable` keyword in Python (#1332). ([997733d](https://github.com/jscott3201/rusty-bacnet/commit/997733d52de1a6ef06d3698c179b2115044b9bc4))

- **Wire, Rust and Python API:** an Event Log can record the event
  notifications the server receives, opted in with
  `EventLogObject::set_log_received_notifications` or
  `add_event_log(log_received_notifications=True)`; each source is held to
  5 records a second (#1346). ([d3529d5](https://github.com/jscott3201/rusty-bacnet/commit/d3529d5154f715f4d99a641555949bd17a0b6b58))

- **Wire:** Event Log, Trend Log and Trend Log Multiple serve the intrinsic
  reporting rows and send a BUFFER_READY notification to their Notification
  Class each time Notification_Threshold more records are collected; no Event
  Log records a report on an Event Log's buffer (#1347). ([d3529d5](https://github.com/jscott3201/rusty-bacnet/commit/d3529d5154f715f4d99a641555949bd17a0b6b58))

- **Wire:** Trend Log and Event Log serve writable Start_Time and Stop_Time
  and keep records only inside that window, logging each opening and
  closing; the server's poller now looks at Event Log windows too (#1353). ([1be8248](https://github.com/jscott3201/rusty-bacnet/commit/1be824832f0d675a17f3f3cf9846eb230287c57b))

- **Wire:** Trend Log serves Trigger, Align_Intervals and Interval_Offset and
  takes Logging_Type writes, so the server polls it at clock-aligned times or
  once per Trigger (#1354). ([1be8248](https://github.com/jscott3201/rusty-bacnet/commit/1be824832f0d675a17f3f3cf9846eb230287c57b))

- **Python API:** `BACnetClient.write_group` takes a `PropertyValue` as a
  change-list value and encodes it, beside encoded `bytes` or `bytearray`; a
  list of ints is no longer taken as the encoded octets (#1359). ([cbe62f3](https://github.com/jscott3201/rusty-bacnet/commit/cbe62f311b478671b7cafadb789affb2fbf238c9))

- **Python API:** `add_notification_class` takes keyword-only `recipients=`
  to seed a Notification Class's Recipient_List, with the checks and
  saved-list precedence of `add_notification_forwarder`'s (#1364). ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Wire:** an event notification to a Device recipient with no fresh binding sends one Who-Is and
  waits, behind the ones already waiting for that device, up to the APDU timeout (a minute at most)
  for the I-Am; at most 1,024 wait, none under DCC, a silent device counted once (#1368). ([aef70c4](https://github.com/jscott3201/rusty-bacnet/commit/aef70c43149af99d5c3dcc3d36d5ccc3313f7eb1))

- **Wire:** the Device's Device_Address_Binding lists the server's configured bindings and I-Am
  observations under ten minutes old, on every read path; a value the application stored is replaced
  by that list, and by an empty one on the endpoint (#1369). ([aef70c4](https://github.com/jscott3201/rusty-bacnet/commit/aef70c43149af99d5c3dcc3d36d5ccc3313f7eb1))

- **Python API:** a read of the Device's Device_Address_Binding comes back typed, one
  `"address_binding"` element per binding: a mapping of `device_identifier`, `network_number` and
  `mac_address` (#1369). ([aef70c4](https://github.com/jscott3201/rusty-bacnet/commit/aef70c43149af99d5c3dcc3d36d5ccc3313f7eb1))

- **Rust API:** `bacnet_encoding::constructed::tagged` adds `decode_app_fixed`,
  `decode_app_object_id` and `next_is_application` (#1374, #1375). ([d682790](https://github.com/jscott3201/rusty-bacnet/commit/d68279070d0a21da717d84f10db7854b59a2f10b))

- **Rust API:** `constructed::tagged` makes `expect_opening`, `expect_closing`,
  `decode_optional_ctx`, `decode_ctx_character_string`,
  `decode_ctx_bit_string`, `decode_ctx_octet_string`, `decode_app_bit_string`
  and `decode_app_character_string` public (#1374). ([d682790](https://github.com/jscott3201/rusty-bacnet/commit/d68279070d0a21da717d84f10db7854b59a2f10b))

- **Breaking (wire):** Lighting Output carries out its lighting commands:
  fades and ramps move Tracking_Value, steps and STOP act on the priority
  array, and the warn commands (also Present_Value -1.0 to -3.0) act at once,
  or blink and hold for Egress_Time when Blink_Warn_Enable is TRUE (#1384). ([b9ad662](https://github.com/jscott3201/rusty-bacnet/commit/b9ad6626f88014b09126ec57860110a6667fbbc9))

- **Rust API:** `BACnetColorCommand`, `BACnetXyColor` and `ColorOperation` in
  bacnet-types, their codecs in `bacnet_encoding::constructed`, and
  `set_color_command` and `color_command` on both colour objects (#1386). ([97baf49](https://github.com/jscott3201/rusty-bacnet/commit/97baf4921afafc2ae9018a0dd68706de4e879bcd))

- **Wire:** With target Audit configured, the server reports each DeviceCommunicationControl change
  it carries out, and a timed disable running out, as DEVICE_DISABLE_COMM or DEVICE_ENABLE_COMM
  records (#1387). ([b07dc35](https://github.com/jscott3201/rusty-bacnet/commit/b07dc359cf5d2d0c43317b31462910b7d14e225d))

- Access Rights can keep the rule arrays and Enable that peers write across a restart:
  `AccessRightsObject::with_persistence` with `FileAccessRightsPersistence`, or `storage_path`
  on Python's `add_access_rights` (#1392). ([7b7905b](https://github.com/jscott3201/rusty-bacnet/commit/7b7905ba09cf6b1bb469b0eb11af58b4d2b7a293))

- **Wire, Rust and Python API:** Access Rights serves its optional
  Accompaniment row once the application sets it (`set_accompaniment`, or
  `accompaniment` on `add_access_rights`); peers can write it,
  `with_persistence` keeps what they write, and Python reads it in the
  keyword's form (#1393, #1344). ([b32ee3b](https://github.com/jscott3201/rusty-bacnet/commit/b32ee3bbdf78c47c3cea545d4192261f1f734191))

- **Wire, Rust and Python API:** Access User serves Members and Member_Of;
  `set_credentials`, `set_members` and `set_member_of`, or the matching
  `add_access_user` keywords, fill its three lists, and Python reads them in
  the keywords' forms (#1394, #1344). ([7cd802c](https://github.com/jscott3201/rusty-bacnet/commit/7cd802cb427216243a764e67f1b136e533a931ac))

- **Rust API:** `bacnet_encoding::constructed` adds
  `decode_object_property_reference_at` and `decode_setpoint_reference_at`,
  which decode a reference at an offset and return the offset past it (#1414). ([d54b143](https://github.com/jscott3201/rusty-bacnet/commit/d54b143114dbc71de28731df2362893a34824393))

- **Python API:** `add_access_zone` takes `alarm_values`, the Access Zone's
  starting Alarm_Values, with the checks a write gets, so NORMAL is refused
  (#1421). ([b32ee3b](https://github.com/jscott3201/rusty-bacnet/commit/b32ee3bbdf78c47c3cea545d4192261f1f734191))

- **Wire and Rust API:** CreateObject sets Units on Analog Input and Output,
  which stay read-only to WriteProperty, and Number_Of_States and State_Text
  on the multi-state objects; `pics::ObjectTypeSupport` gains
  `creation_only_properties`, so struct literals need it (#1429). ([fe20bc5](https://github.com/jscott3201/rusty-bacnet/commit/fe20bc58092d5e07ad4d7e6b7796f60696472854))

- **Python API:** the enum classes (`ObjectType`, `EnableDisable`,
  `ErrorCode` and the rest) belong to the `rusty_bacnet` module and support
  `copy.copy`, `copy.deepcopy` and `pickle` (#1456). ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- `CONTRIBUTING.md`, issue forms for bug reports and feature requests, and a
  pull request template explain how to report a problem and propose a change
  on GitHub (#1472). ([78fd518](https://github.com/jscott3201/rusty-bacnet/commit/78fd51855e00cf82c1b14154acfc6014a23c67ce))

- `TransportPort::is_group_destination` and `group_destinations` say whether a
  MAC reaches a group of nodes, such as any B/IP broadcast or multicast
  address at any port or any IPv6 multicast group, beside the link's own
  broadcast MAC (#1479). ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- **Python API:** `ObjectIdentifier`, `PropertyValue` (typed constructed
  reads included) and `BACnetTimeStamp` support `copy.copy`,
  `copy.deepcopy` and `pickle`, and every exported class belongs to the
  `rusty_bacnet` module (#1500). ([17404bb](https://github.com/jscott3201/rusty-bacnet/commit/17404bb55178e00727f2431bc2ccbc44a87599ad))

### Changed

- **Breaking (Rust and Python API):** AuditLogQuery's success filter is the
  three-state `BACnetSuccessFilter`, so failures-only now filters, and the
  start sequence number is an Unsigned64 cursor (#345).

- **Rust and Python API:** a server can run 1 to 64 target Audit Reporters
  that send bounded notifications for inbound writes, list edits, file writes
  and object creation and deletion, plus summaries of dropped records; the
  lowest instance is elected and all share one admission budget (#345, #782).

- **Python API:** `BACnetServer.configure_audit_notification_sink` lets a
  standalone server take Audit notifications into a file-backed Audit Log
  under an explicit policy; see the
  [receiver configuration](docs/python-api.md#inbound-audit-notification-sink)
  (#345).

- **Python API:** a standalone B/IP server binds Device addresses with
  `add_device_binding` and forwards its Audit Log to a parent with
  `configure_audit_log_parent`; see
  [parent forwarding](docs/python-api.md#direct-audit-log-parent-forwarding)
  (#345).

- **Breaking (CLI):** `bacnet --sc` requires `--sc-ca` with explicit site CA
  certificates and no longer loads the system roots; see the
  [CLI migration](docs/CLI.md#transport-variants) (#513).

- **Breaking (Rust API):** `ScHub::start` and its variants take an
  `ScHubTlsConfig` instead of a `TlsAcceptor`, enforcing explicit CA trust,
  client verification and TLS 1.3, and the server-auth-only SC benchmarks are
  retired (#513).

- `ScHubTlsConfig::from_der` builds hub TLS from loaded CA, chain and key DER
  with mandatory client verification and TLS 1.3 only, and Python hub startup
  uses it (#513).

- **Breaking (Rust API):** `TlsWebSocket::connect` and the SC client and
  server builders' `tls_config` take an opaque `ScNodeTlsConfig` built from
  CA, chain and key DER instead of a rustls `ClientConfig` (#513).

- The mTLS benchmark hub launcher and the CLI and server SC test fixtures
  build their TLS through `ScHubTlsConfig` (#513).

- **Breaking (Python API):** `BACnetClient` and `BACnetServer` with
  `transport="sc"` require `sc_ca_cert`, `sc_client_cert` and `sc_client_key`,
  with no fallback to the system roots; see
  [SC configuration](docs/python-api.md#bacnetsc-secure-connect) (#513).

- **Breaking (Python API):** `ScHub` requires an explicit `ca_cert` and always
  verifies clients over TLS 1.3, with no one-way TLS default; see
  [ScHub](docs/python-api.md#schub) (#513).

- **Breaking (CLI):** `bacnet-sc-hub` requires `--ca`, `--cert` and `--key`,
  and `bacnet-device --transport=sc` its SC credentials and identity, for
  mutual TLS 1.3; see the [Docker recipe](examples/docker/README.md) (#513).

- **Breaking (Rust and Python API):** every SC hub startup API requires a
  nonzero hosting device UUID (`device_uuid` in Python, `--device-uuid` for
  `bacnet-sc-hub`) and refuses reserved hub VMACs (#517).

- **Breaking (Rust API):** `ScTransport::start` requires a nonzero UUID set
  with `with_device_uuid`, and refuses reserved local VMACs (#517).

- **Wire:** an initiating SC node silently discards a Connect-Accept with an
  all-zero peer UUID, so it can't complete the handshake (#517).

- **Wire:** an SC hub refuses a Connect-Request carrying an all-zero Device
  UUID before it registers the peer (#517).

- **Breaking (Rust and Python API):** `ScServerBuilder` requires
  `device_uuid`, and Python SC clients and servers the keyword
  `sc_device_uuid`; provision one UUID per device and keep it for life (#517).

- **Wire:** an accepting SC hub silently discards an unsolicited
  Connect-Accept or Disconnect-ACK without touching its state (#519).

- **Wire:** an SC Connect-Request or Connect-Accept that advertises a zero
  Max-BVLC or Max-NPDU is refused (#519).

- **Wire:** SC nodes and hubs discard an empty Encapsulated-NPDU, answering an
  eligible unicast with a NAK (#519).

- **Wire:** an SC hub relays registered unicast Address-Resolution requests
  and ACKs between nodes (#519).

- **Wire:** an SC hub relays registered, addressed messages of unknown
  function and returns their results (#519).

- **Wire:** an unsupported Must Understand option on a received SC NPDU no
  longer refreshes node activity or clears a pending heartbeat (#519).

- **Wire:** an established SC node rejects a BVLC function it doesn't know
  before refreshing activity, with a NAK for an eligible unicast (#519).

- An SC node's rejection NAKs must go out within the remaining heartbeat
  budget; a node that misses it retires the socket and recovers only through a
  fresh connection (#519).

- The server bounds request admission with configurable global and per-peer
  quotas, keeps a reserve for DCC ENABLE, and joins its request work on stop;
  see the
  [acceptance matrix](docs/request-admission.md#bounded-acceptance-and-evidence)
  (#521).

- **Rust and Python API:** an optional, default-off token bucket limits DCC
  DISABLE_INITIATION requests per server (`dcc_disable_rate_limit`); see
  [DCC policy](docs/dcc-policy.md) (#522).

- **Rust and Python API:** with `RequirePassword`, DCC can also be limited to
  an exact list of claimed source addresses, which remain spoofable; see
  [DCC policy](docs/dcc-policy.md) (#522).

- **Breaking (Rust API):** a custom `TransportPort` must provide
  `local_receive_apdu_capacity()`, `max_apdu_length()` is renamed
  `egress_apdu_limit()` with no alias, and `ReceivedNpdu` gains `provenance`
  and `direct_response` (#693). ([df016bb](https://github.com/jscott3201/rusty-bacnet/commit/df016bb551d99c88bd42b96cb6bca8ec75651045))

- `MstpTransport::diagnostics()` exposes redacted, counts-only MS/TP host
  diagnostics, and a [bench method](docs/mstp-qualification.md) describes how
  to qualify a host (#707, #502).

- Direct B/IP endpoint client sessions can report their ReadProperty requests
  as bounded source audit records to the Device's recipient; see
  [endpoint source READ](docs/rust-api.md#bounded-endpoint-source-read-reporting)
  (#727, #345).

- **Rust API:** endpoint source auditing sends to the built-in Device's
  recipient; see the [Device audit recipient](docs/device-audit-recipient.md)
  contract (#728).

- **Rust and Python API:** the Audit recipient lives on the built-in Device,
  and Python sets it with `configure_audit_recipient` (#728).

- Endpoint source READ audit records lost to resource exhaustion are
  summarized in one bounded AUDITING_FAILURE count (#732, #345).

- **Wire:** an SC node without a live matching direct listener answers
  Address-Resolution with a NAK instead of a URI ACK (#733).

- **Breaking (Rust API):** custom objects report intrinsic events through one
  proposal and commit contract, and
  `intrinsic_reporting_requires_atomic_commit` and the
  `impl_intrinsic_reporting!` macro are gone (#746).

- Tests run with cargo-nextest 0.9.145 or later in CI, the release gate and
  `scripts/ci/local-macos.sh`, plus `cargo test --doc` for doctests (#751). ([56249e6](https://github.com/jscott3201/rusty-bacnet/commit/56249e66f6c29e8e41031f7f2defc2db55827219))

- **Breaking (Rust API):** `BACnetObject::apply_life_safety_operation` returns
  a `LifeSafetyOperationOutcome` listing the exact property changes, replacing
  the `_detailed` hook (#752).

- **Rust and Python API:** SC hub admission sees the current UUID and VMAC
  registration, and Python can opt into
  `admission_policy="deny_uuid_replacement"` to keep an incumbent (#767,
  #476).

- **Python API:** `BACnetServer` takes `mutation_policy="deny_all"` to refuse
  the ten network mutation services while reads and trusted local writes still
  work; the default stays permissive (#768).

- **Rust and Python API:** `ScHubProbePolicy` configures the SC hub's probe
  timing on a monotonic clock, and Python exposes the hub's relay send budget
  and broadcast-rate policy (#769, #476).

- **Rust and Python API:** SC hub status adds per-start counters of its
  replacement and removal decisions (#770, #476).

- **Rust and Python API:** the endpoint client sends ReadRange through the
  shared request path and records one source Audit READ per attempt; ReadRange
  request encoding returns `Result` (#771, #345).

- **Rust and Python API:** one SC hub relay send budget, five seconds unless
  configured, bounds every unicast, broadcast and forwarded-result relay, so a
  blocked destination no longer holds up the source's reader (#762, #774,
  #476).

- **Wire:** an SC hub answers a peer's WebSocket Close with its own Close and
  reclaims the peer's slot (#776).

- Standalone and endpoint clients share ReadProperty ACK validation, so an ACK
  for another object, property or index is a decoding error (#784, #345).

- **Wire:** a ReadPropertyMultiple response includes a property array index
  only for declared arrays (#789).

- **Rust and Python API:** WritePropertyMultiple request encoding returns
  `Result` and validates every write first, refusing empty lists, special
  property selectors and invalid priorities (#793).

- **Rust and Python API:** AddListElement and RemoveListElement request
  encoding returns `Result` and refuses index zero, empty elements and
  malformed framing (#798).

- **Breaking (Rust API):** the `BACnetClient::builder()` and
  `BACnetServer::builder()` aliases are gone (#873).

- **Breaking (Rust API):** `bacnet_server::schedule::tick_schedules` drops its
  unused UTC-offset argument (#889).

- **Breaking (Rust API):** `AnyTransport::Bip` holds a `Box<BipTransport>`,
  like `Sc`, and `From<BipTransport>` still converts (#902).

- `clippy::print_stdout` and `clippy::print_stderr` are denied across the
  workspace, and library crates report through `tracing` (#902).

- Every public item is documented and `missing_docs` is denied. CI treats
  clippy and rustdoc warnings as errors for every feature set, the PyO3 crate
  and each published crate on its own (#902, #906).

- **Breaking (Rust API):** long parameter lists become structs, such as
  `CovPropertySubscription` for `subscribe_cov_property` and `RoutedTarget`
  for routed sends (#902).

- **Rust API:** `ClientRoleHandle::write_property` takes a
  `WritePropertyRequest`, and `NetworkLayer::send_response_apdu_on_issuance`
  an `IssuedApdu` (#902).

- CI builds the Python bindings with maturin and runs their test suite on
  every PR, and Cargo Deny checks their dependencies (#903). ([e1ca5a6](https://github.com/jscott3201/rusty-bacnet/commit/e1ca5a6096f132509ae9acde8d4e9dd2d43b5ad4))

- **Breaking (Rust and Python API):** alarm and event service types use the
  `bacnet-types` enumerations and bit strings instead of raw integers, with
  the wire encoding unchanged (#914). ([ca35afc](https://github.com/jscott3201/rusty-bacnet/commit/ca35afcde79727fce0176fc562e280d93134904d))

- **Breaking (Rust API):** the `bacnet-objects` event detectors and enrollment
  objects hold typed transitions, notify types and reliabilities, and the
  duplicate `bacnet_objects::event::LimitEnable` is gone (#914). ([ca35afc](https://github.com/jscott3201/rusty-bacnet/commit/ca35afcde79727fce0176fc562e280d93134904d))

- Optional dependencies are no longer published as implicit features; enable
  the documented `ipv6`, `sc-tls`, `serial`, `serial-gpio`, `ethernet`,
  `pcap`, `std` or `serde` feature instead (#917). ([85e402c](https://github.com/jscott3201/rusty-bacnet/commit/85e402ce205890a92963e104fa97e1c33a1026bd))

- The workspace no longer turns on pyo3's `extension-module` feature, so
  `cargo nextest run -p rusty-bacnet` links libpython and runs the crate's
  tests (#919). ([e40ea24](https://github.com/jscott3201/rusty-bacnet/commit/e40ea249a7b608478eab2a325c20f9a31a13be4e))

- **Breaking (Rust and Python API):** GetEnrollmentSummary's acknowledgment
  filter is an `AcknowledgmentFilter`, and every stored Event_State and
  Event_Type is typed (#930). ([e67cd5c](https://github.com/jscott3201/rusty-bacnet/commit/e67cd5cbd2c3dded0df9b19edf9de241e6d192c4))

- **Breaking (Rust API):** Notification Class recipients use typed bit strings
  (`DaysOfWeek`, `EventTransitionBits`), and `pack_octet` and `unpack_octet`
  are no longer public (#930). ([e67cd5c](https://github.com/jscott3201/rusty-bacnet/commit/e67cd5cbd2c3dded0df9b19edf9de241e6d192c4))

- **Breaking (Rust API):** objects store Reliability as a typed enumeration,
  and Life Safety Point and Zone, the access-control objects and Elevator
  Group store their enumerated fields typed too; the wire values and the
  Python API are unchanged (#932). ([339a02a](https://github.com/jscott3201/rusty-bacnet/commit/339a02a068df641867ccee41f6076fa6ce7fea7e))

- The pinned development, CI and release toolchain moves to Rust 1.99.0; the
  MSRV stays 1.93 (#936). ([d9b0f91](https://github.com/jscott3201/rusty-bacnet/commit/d9b0f9125b939c61ea4e0f6749ab7e17f51d19ab))

- Releases are built on GitHub's runners, each wheel and CLI binary natively
  on its own platform (Linux x86_64 and arm64, macOS Apple Silicon and Intel,
  Windows), and run there before anything is published (#943, #944, #1472). ([dfc317a](https://github.com/jscott3201/rusty-bacnet/commit/dfc317a071f8abfa8a0182874cc65fd830a8e211))

- `tokio-tungstenite` is built without TLS features, so nothing loads the
  system root certificates and the native-certs crates leave the lockfile
  (#944). ([dfc317a](https://github.com/jscott3201/rusty-bacnet/commit/dfc317a071f8abfa8a0182874cc65fd830a8e211))

- The test suites, clippy and rustdoc also run natively on macOS and Windows,
  in parallel test and lint jobs on GitHub-hosted runners (#950). ([7abb5e4](https://github.com/jscott3201/rusty-bacnet/commit/7abb5e451793bca30c923e2244e6bba93efea336))

- **Breaking (Rust API):** `Error` gains `UnsupportedTransport`, the client
  BBMD helpers work on any transport that implements `AsBip`, and
  `ScTransport::connection()` and `MstpTransport::node_state()` are private
  (#956). ([5e9e676](https://github.com/jscott3201/rusty-bacnet/commit/5e9e6763b3224ff7dfc2e19ad7a6c9a132587a3a))

- The workspace uses Cargo's `resolver = "3"`, so lock file updates prefer
  dependency versions that support the MSRV (#961). ([6d866b6](https://github.com/jscott3201/rusty-bacnet/commit/6d866b683f92c23fd06a0ed0d2ab4e5e17f4ccc8))

- CI's per-crate default-features check also covers the Windows and macOS
  targets, and the native Windows job stops the Compatibility Appraiser that
  upset test timing (#981, #1003). ([b28c16f](https://github.com/jscott3201/rusty-bacnet/commit/b28c16ffba4dc8a8672c1a31ccda0c8f15869c02))

- A COV-multiple context keeps up to about four notifications of timestamped
  changes, under a memory ceiling; overflow drops the oldest, never a
  reference's newest or partly sent one, counted in
  `CovCounters::timed_changes_dropped` and warned once per cause (#1039,
  #1163, #1197, #1287, #1357). ([a8450ee](https://github.com/jscott3201/rusty-bacnet/commit/a8450ee759a7ba9bcbec6826fff63596c9dc8cdf))

- Test-only: the endpoint Device-write, benchmark hub-restart and BBMD tests
  no longer fail when another socket takes their port (#1068). ([6bf9a0e](https://github.com/jscott3201/rusty-bacnet/commit/6bf9a0ecbcf51dd36d30fe8b39927c9903df83d3))

- Test-only: the BBMD restart and port-release tests rerun on fresh ports when
  another socket takes the port between the stop and the bind (#1070). ([499dd0c](https://github.com/jscott3201/rusty-bacnet/commit/499dd0c803eb2e075ad7c2ecf429153cf35297c8))

- Test-only: the SC hub shutdown and B/IP restart tests rerun on fresh ports
  the same way, through test-only `port_ownership` helpers shared in
  `bacnet-transport` (#1070, #1095). ([499dd0c](https://github.com/jscott3201/rusty-bacnet/commit/499dd0c803eb2e075ad7c2ecf429153cf35297c8))

- The Python and Rust API docs give Network Port snapshots, the SC heartbeat
  and identity settings and the Audit Reporter their own headings (#1071). ([499dd0c](https://github.com/jscott3201/rusty-bacnet/commit/499dd0c803eb2e075ad7c2ecf429153cf35297c8))

- **Breaking (wire, Rust API):** Access Credential and Access Door serve every
  required row of their tables. Credential_Status is derived from
  Reason_For_Disable, a door command outside the four BACnetDoorValue values
  is refused, and a pulse unlock relinquishes after its pulse time (#1073,
  #979). ([42e4528](https://github.com/jscott3201/rusty-bacnet/commit/42e45288181d8fa383c5b41fce6de387bdb034a1))

- The running server's Device read view forwards every read-only
  `BACnetObject` query to the object it wraps, and a test fails when one isn't
  forwarded (#1076). ([436dd01](https://github.com/jscott3201/rusty-bacnet/commit/436dd01dd5ff577d5cccf2a15e0ca91639969a94))

- **Breaking (wire):** Audit Log serves Log_Buffer through ReadRange by position, sequence or time, and
  ReadProperty answers it with READ_ACCESS_DENIED. ReadRange references and First Sequence Number are
  `u64` (#1092). ([e49a6f3](https://github.com/jscott3201/rusty-bacnet/commit/e49a6f35072009ee10dd2333a262039b41225c02))

- **Breaking (Python API):** a transport I/O failure raises
  `BacnetTransportError`, a subclass of both `BacnetError` and `OSError` that
  carries `errno`, and the message loses its `transport error:` prefix
  (#1120). ([b9ae0e2](https://github.com/jscott3201/rusty-bacnet/commit/b9ae0e2ef2b38111252b47cddaf550d520522de2))

- **Wire and Rust API:** Access Door keeps Door_Alarm_State to NORMAL and its
  alarm and fault values, outside its masked values, refusing other states from
  the application and from clients (#1149). ([a81304f](https://github.com/jscott3201/rusty-bacnet/commit/a81304f82c9a7d51a0d8b8f5d325ba698d6f5e33))

- **DCC source restriction entry length:** `DccSourceRestriction` and the
  Python `dcc_source_restriction` keyword refuse an entry longer than
  `BACnetAddress::MAX_MAC_LEN` (18 octets), which could never match a source
  (#1157). ([0696eb8](https://github.com/jscott3201/rusty-bacnet/commit/0696eb861cae65a1c4a855bdb0ddbcc4c48aa3fb))

- Reading a Group's Present_Value charges every member row to the request's
  ReadPropertyMultiple work limit, so a request that would pass it is aborted
  with OUT_OF_RESOURCES (#1172). ([15a787f](https://github.com/jscott3201/rusty-bacnet/commit/15a787fb2dc82d3811ce3d6a8070304a7f444d36))

- Changelog entries are one or two high-level sentences naming the issue,
  which `changelog.py check` enforces, and `assemble` links each entry to its
  GitHub commit (#1188). ([942c31f](https://github.com/jscott3201/rusty-bacnet/commit/942c31fe510373660b07bdf1b0f40ceb216ae4d4))

- **Rust and Python API:** `CovPolicy` and the Python `cov_policy` keyword
  refuse a reserved peer or recipient MAC longer than 18 octets, which could
  never match a subscriber (#1199). ([981024a](https://github.com/jscott3201/rusty-bacnet/commit/981024a31d3bcb15fcb5d84136a4fb4c2d16e546))

- With several Devices in the database, Audit Reporters, Audit Log receipt and
  forwarding, endpoint Device writes and the standalone PICS treat the lowest
  as this device, as wildcard reads already do, where they used to refuse or
  pick any one ([details](docs/rust-api.md#databases-with-several-devices), #1204). ([0897982](https://github.com/jscott3201/rusty-bacnet/commit/089798289ead647e370c92189dd2af85a1f05c18))

- **Breaking (wire, Rust API):** Trend Log Multiple refuses COV logging, over
  the wire and through `set_logging_type`, which returns `Result`; both trend
  objects' `set_logging_type` take a `LoggingType`, and a log with a
  proprietary Logging_Type is no longer polled (#1235). ([72c1770](https://github.com/jscott3201/rusty-bacnet/commit/72c17700434f51678eab44474bc41bbbe4e5f7e6))

- **Wire:** an Audit Log Log_Enable write whose commit fails is refused with a
  device error instead of a generic one, as Buffer_Size writes are (#1238). ([3acf048](https://github.com/jscott3201/rusty-bacnet/commit/3acf0488d3bf760ad00eaefca008d73c857e4c43))

- **Rust and Python API:** `AuditLogObject::new` and Python `add_audit_log` reopen
  a log with the Buffer_Size it stored, which a peer may have written, instead of
  failing when `buffer_size` differs; a warning names both sizes (#1238). ([3acf048](https://github.com/jscott3201/rusty-bacnet/commit/3acf0488d3bf760ad00eaefca008d73c857e4c43))

- **Rust API:** `GroupObject::add_member` returns a `GroupMemberRefusal`
  naming the rule a member breaks (#1250). ([53f39c1](https://github.com/jscott3201/rusty-bacnet/commit/53f39c1cc1d4ac511cd8227cc80ddc7687d67c48))

- **Breaking (Rust API):** `BACnetLightingCommand` takes a `LightingOperation` and a
  `u8` priority, gains a codec in `bacnet_encoding::constructed`, and Lighting Output
  gains `set_lighting_command` and `lighting_command` (#1263). ([91ce62a](https://github.com/jscott3201/rusty-bacnet/commit/91ce62a13f533db07b8a6c5586c2facf75edeb03))

- **Rust API:** `TimeSyncSourceRestriction` refuses an entry longer than 18
  octets, which could never match a source (#1266). ([cf0d55a](https://github.com/jscott3201/rusty-bacnet/commit/cf0d55ae8c5788195aaee9fe3dac00566c6c2aaa))

- **Breaking (Rust API):** routed confirmed requests and the routed-path limit
  methods refuse a router MAC, local source MAC or DADR longer than 18 octets
  before the path is reserved, instead of failing later (#1267). ([cf0d55a](https://github.com/jscott3201/rusty-bacnet/commit/cf0d55ae8c5788195aaee9fe3dac00566c6c2aaa))

- Durable saves run on a writer thread of their own, so a slow disk no longer holds the object
  database lock; a list that can't be saved is still refused, an Audit notification is acknowledged
  only once durable, and `stop()` waits for queued saves (#1270, #1363). ([1d3eb21](https://github.com/jscott3201/rusty-bacnet/commit/1d3eb211c7700a20b59320ddf968c4fe8c269744))

- **Breaking (Rust API):** `EventLogDatum::Notification` holds a typed
  `EventNotificationRequest`, now in bacnet-types with its codec in
  bacnet-encoding. `decode_event_log_record` refuses a notification that isn't
  a valid request, but drops an unreadable message text (#1276). ([ea05d58](https://github.com/jscott3201/rusty-bacnet/commit/ea05d5890885085fd54f7e3e3f6a0d186434fda2))

- **Breaking (Rust API):** routed confirmed requests, `add_routed_device` and
  the endpoint requester refuse DNET 0, DNET 65535 and an empty DADR before any
  path or transaction state is taken, instead of sending a request no single
  device can answer (#1278). ([0618115](https://github.com/jscott3201/rusty-bacnet/commit/06181155fd7b7ca7992651dc2b1495692d8b034b))

- B/IP and B/IPv6 also take an Original-Unicast-NPDU's group-delivery flag from the address it was
  sent to (#1301). ([ccec56f](https://github.com/jscott3201/rusty-bacnet/commit/ccec56fc3d243a73b286c2333ae11179171300c2))

- **Rust API:** the ReadAccessSpecification, BACnetChannelValue, audit
  notification and formal Error body decoders report contents cut short as
  `Error::BufferTooShort` and a fixed-size field of the wrong length as
  `Error::Decoding`; Python error messages change, peer replies don't (#1303). ([c8830e3](https://github.com/jscott3201/rusty-bacnet/commit/c8830e320963a18028d798f8a26f896f01567ac9))

- **Rust API:** `bacnet_encoding::constructed::tagged` is public, and the
  service decoders read their parameters with it in place of their own
  helpers, so a member read through it that is cut short is
  `Error::BufferTooShort`; Python error messages change, peer replies don't
  (#1304). ([c5ab51e](https://github.com/jscott3201/rusty-bacnet/commit/c5ab51eb677d00887d549c27e8975e36b91829aa))

- **Breaking (Python API):** Recipient_List, a Group's members and
  Present_Value, a Command's Action and the other constructed lists the
  binding writes read as typed values. In 0.11.0 a local read gave the stored
  octets or flat list, and a client read only the first application-tagged
  value (#1310). ([e04ef28](https://github.com/jscott3201/rusty-bacnet/commit/e04ef2883cafe4f8b1a3958a3b5395928723a183))

- **Breaking (Rust API):** the Loop and Pulse Converter references read as
  `PropertyValue::ApplicationData`, and `decode_setpoint_reference` takes an empty value
  as no reference and refuses an empty frame (#1312). ([808e0c2](https://github.com/jscott3201/rusty-bacnet/commit/808e0c2903b79898c17009d9f583c1986c248fe0))

- **Breaking (wire):** device reference properties share one decode: anything after a single reference, a Trend Log's included, is INVALID_DATA_ENCODING, and a Staging target of another datatype INVALID_DATA_TYPE (#1313). ([7ade5e3](https://github.com/jscott3201/rusty-bacnet/commit/7ade5e36df847e9d7909ff6d4b9d7615824cd0f4))

- A Schedule's written reference list is decoded whole and held to its cap before any member's Device check, so a malformed member or the cap wins over an earlier member's refusal (#1313). ([7ade5e3](https://github.com/jscott3201/rusty-bacnet/commit/7ade5e36df847e9d7909ff6d4b9d7615824cd0f4))

- `NetworkLayer` sends that name a destination network refuse network 0 with an error saying so,
  before encoding or sending anything (#1314). ([ccec56f](https://github.com/jscott3201/rusty-bacnet/commit/ccec56fc3d243a73b286c2333ae11179171300c2))

- **Rust API:** The mutation authorizer decides each Channel write of an
  inbound WriteGroup; `DenyAll` denies them, and the decisions are counted
  (#1319). ([cbe62f3](https://github.com/jscott3201/rusty-bacnet/commit/cbe62f311b478671b7cafadb789affb2fbf238c9))

- **Breaking (Rust API):** `CovAckResult::Error` carries the refusing answer as a `Refusal`: the Error class and code, or the Reject or Abort reason (#1323). ([0863c0f](https://github.com/jscott3201/rusty-bacnet/commit/0863c0f590582684016b70b3ff1f89442509b2a5))

- **Rust API:** decoders report contents cut short as `Error::BufferTooShort`
  inside constructed frames, Error PDUs and timestamps too, and check a
  fixed-size application value's length before its contents. Python error
  messages change; peer replies don't (#1333). ([d54b143](https://github.com/jscott3201/rusty-bacnet/commit/d54b143114dbc71de28731df2362893a34824393))

- `NetworkLayer` routed and on-issuance sends refuse destination network 0xFFFF paired with a
  device address, naming the error, before encoding or sending anything (#1340). ([6d08557](https://github.com/jscott3201/rusty-bacnet/commit/6d08557f5bcffb5da3b43d5b96fca9ea8e58cc73))

- **Wire, Breaking (Rust API):** A Channel reads a remote member's datatype and
  coerces its value to it, so a remote Binary Output takes REAL 1.0 as ACTIVE;
  an unanswered read fails the member unsent, a refused one sends the value as
  written. `CovAckResult` gains `Data` (#1342). ([cbe62f3](https://github.com/jscott3201/rusty-bacnet/commit/cbe62f311b478671b7cafadb789affb2fbf238c9))

- **Breaking (Python API):** Access Zone Entry_Points and Exit_Points and
  Access User Credentials read as `device_object_reference` values, an
  `ObjectIdentifier` or a `(device, object)` pair, where a 0.11.0 local read
  gave plain object identifiers (#1344). ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Breaking (Python API):** Date_List, schedules, timestamps, Value_Source
  and the other constructed properties in
  [the Python API guide](docs/python-api.md#typed-constructed-values) read as
  typed values. In 0.11.0 a local read gave the stored octets or flat list,
  and a client read only the first application-tagged value (#1345). ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Breaking (wire, Rust API):** Trend Log refuses COV logging until the
  stack can do it (#1480), whether asked through Logging_Type or by writing a
  polled log's Log_Interval to zero; `TrendLogObject::set_logging_type`
  returns `Result` (#1354). ([1be8248](https://github.com/jscott3201/rusty-bacnet/commit/1be824832f0d675a17f3f3cf9846eb230287c57b))

- **Wire:** Once the client knows its network's number, `confirmed_request_routed` to that number
  ignores `router_mac` and sends the request straight to the DADR (#1358). ([f0c8d74](https://github.com/jscott3201/rusty-bacnet/commit/f0c8d74ce75c5cb897a613e53de97e7af83f544d))

- **Breaking (Python API):** an integer argument outside the type of the field
  it fills raises `OverflowError` everywhere, as a mapping value or tuple
  member as well as a parameter, instead of `ValueError` or
  `BacnetProtocolError` on some paths (#1360). ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Breaking (Rust API):** the bacnet-services decoders that still read tags
  inline use `bacnet_encoding::constructed::tagged`, so a member they read
  that is cut short is `Error::BufferTooShort`; `tags::decode_optional_context`
  is removed. Peer replies don't change (#1374). ([d682790](https://github.com/jscott3201/rusty-bacnet/commit/d68279070d0a21da717d84f10db7854b59a2f10b))

- `NetworkLayer` routed and on-issuance sends refuse destination network 0xFFFF
  with no device address too, pointing to `broadcast_global_apdu`, since a
  unicast global broadcast reaches only one router (#1380). ([520abe7](https://github.com/jscott3201/rusty-bacnet/commit/520abe781edc7613dfb2a38e0c8a20e2b31d6cdd))

- **Breaking (Rust API):** A value written directly to a Loop or Pulse
  Converter reference is decoded like a device reference, so a malformed list
  gets the same refusal code and an empty list clears Setpoint_Reference;
  WriteProperty answers are unchanged (#1395). ([8174d11](https://github.com/jscott3201/rusty-bacnet/commit/8174d1172eaf150a7d5a187578f1ce5703b1aa77))

- **Breaking (Rust API):** `BACnetServer::comm_state()` returns the new
  `DccState` (`Enable` or `DisableInitiation`) instead of a raw `u8`, since the
  server refuses DISABLE; the unreachable DISABLE request drops are gone
  (#1399). ([e9472a8](https://github.com/jscott3201/rusty-bacnet/commit/e9472a845f846dbb32285fdfbbc8e352d59dcac5))

- **Breaking (Rust API):** durable saves finish on a writer thread, so a `BACnetServer` dropped
  without `stop()` returns before storage settles, and in async code it hands its object database
  to the blocking pool rather than block a Tokio worker (#1270, #1409). ([9a24975](https://github.com/jscott3201/rusty-bacnet/commit/9a24975c693bb99f132fedcf73fe8474cbbb8263))

- **Wire:** an Accumulator without a configured Prescale no longer reads it as
  NULL; the property is absent until the application sets one (#1417). ([5555911](https://github.com/jscott3201/rusty-bacnet/commit/55559112ff4daadbb7570f8c29d3839ab53750a2))

- **Breaking (wire):** an unset Loop, Pulse Converter, Trend Log, Averaging or
  Event Enrollment reference reads as a reference to instance 4194303, which
  clears one when written, and a NULL written to it, or to Fault_Parameters,
  succeeds and changes nothing (#1417). ([5555911](https://github.com/jscott3201/rusty-bacnet/commit/55559112ff4daadbb7570f8c29d3839ab53750a2))

- **Python API:** reading an unset reference property returns
  `application_data` naming instance 4194303 instead of null, and
  `write_property_local` of null on one leaves it as it is (#1417). ([5555911](https://github.com/jscott3201/rusty-bacnet/commit/55559112ff4daadbb7570f8c29d3839ab53750a2))

- **Breaking (Rust API):** `BACnetServer::write_local` returns `Err` for an
  array index on a property that isn't an array or that the object doesn't
  have, where it used to write the property (#1426). ([be29d40](https://github.com/jscott3201/rusty-bacnet/commit/be29d40796389a10b85e418044307490bf0c2948))

- **Rust API:** `NotificationClass` and `AccessRightsObject` stay `UnwindSafe` and
  `RefUnwindSafe` when built with persistence (#1428). ([3bb57d6](https://github.com/jscott3201/rusty-bacnet/commit/3bb57d6f1eb518ed6c2b8010c33934d909a7133a))

- **Wire:** Multi-state Input and Value refuse an Alarm_Values entry past
  Number_Of_States with VALUE_OUT_OF_RANGE naming the element, over
  WriteProperty, the list services and CreateObject, instead of storing it
  (#1429). ([fe20bc5](https://github.com/jscott3201/rusty-bacnet/commit/fe20bc58092d5e07ad4d7e6b7796f60696472854))

- **Breaking (Python API):** `BACnetServer.comm_state()` returns
  `EnableDisable.ENABLE` or `EnableDisable.DISABLE_INITIATION` instead of 0 or 2
  (#1431). ([eeb9dd9](https://github.com/jscott3201/rusty-bacnet/commit/eeb9dd9c7fe2f666423d82a378a42592352367f1))

- **Rust API:** `ScheduleWrite` has a public `retry` field, set
  when a Schedule offers its value again only to the references that refused
  it (#1436). ([c223214](https://github.com/jscott3201/rusty-bacnet/commit/c2232141aa2e42d2ec0d641a7542fde6d443ad46))

- A Schedule whose reference was refused because the object it names didn't
  exist offers that object its value as soon as it is created, by CreateObject
  or a local `ObjectDatabase::add`, instead of at the next 60-second pass
  (#1440). ([5555911](https://github.com/jscott3201/rusty-bacnet/commit/55559112ff4daadbb7570f8c29d3839ab53750a2))

- **Breaking (wire, Rust API):** State_Text written whole on a multi-state
  object, by WriteProperty, WritePropertyMultiple or a CreateObject without
  Number_Of_States, or its size at index 0, sets Number_Of_States, refusing
  a shrink that would strand a state the object holds (#1443). ([d63feb9](https://github.com/jscott3201/rusty-bacnet/commit/d63feb908613f0550d858a19bef70a25ebc67796))

- **Breaking (Rust API):** `Error::Decoding` carries a `DecodingKind`,
  `Error::InvalidTag` is gone, and server handlers return a request's decode
  error as its `Error::Reject`. AtomicReadFile-ACK count, stream and trailing
  faults are decoding errors, not `Error::Reject` (#1446). ([d54b143](https://github.com/jscott3201/rusty-bacnet/commit/d54b143114dbc71de28731df2362893a34824393))

- **Python API:** an AtomicReadFile-ACK with more or fewer records than its
  count, or octets after its data, raises `BacnetError`, where it raised
  `BacnetRejectError` as if the device had rejected the request (#1446). ([d54b143](https://github.com/jscott3201/rusty-bacnet/commit/d54b143114dbc71de28731df2362893a34824393))

- The samples in `examples/rust/samples` are workspace members outside the
  default build. They share the workspace's `Cargo.lock` instead of keeping
  their own, which could go stale, and CI lints and tests them with the rest
  of the workspace (#1406, #1450). ([78fd518](https://github.com/jscott3201/rusty-bacnet/commit/78fd51855e00cf82c1b14154acfc6014a23c67ce))

- **Wire:** A DCC or time-sync allowlist entry routed through the server's known network number
  also admits that station's direct requests, and time sync's per-source budget counts a station's
  direct and relayed requests as one source (#1458). ([b07dc35](https://github.com/jscott3201/rusty-bacnet/commit/b07dc359cf5d2d0c43317b31462910b7d14e225d))

- **Python API:** `configure_audit_recipient` accepts an Address on network 0 or on one numbered
  1 to 65534, so a server can report to a logger named by its own network's number (#1460). ([b07dc35](https://github.com/jscott3201/rusty-bacnet/commit/b07dc359cf5d2d0c43317b31462910b7d14e225d))

- Issues, pull requests and CI moved from a private Forgejo to GitHub, keeping
  every issue number (#1472). Linux CI runs on GitHub-hosted runners, a PR
  merges only when its `CI OK` and `Native OK` checks pass, and runs on dev
  prune superseded Actions caches (#1471). ([06ff246](https://github.com/jscott3201/rusty-bacnet/commit/06ff2463aa83328f31afdc0004db4360540a4bf6))

- **Breaking (Rust API):** `NetworkLayer` broadcasts only an
  Unconfirmed-Request, naming any other PDU type it refuses, as does a routed
  send with no DADR; `send_apdu_routed_via_local_broadcast` refuses an empty
  DADR. A confirmed call to a broadcast or group address fails in Rust and
  Python (#1479). ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- **Breaking (Rust API):** `WhoIsRequest` and `WhoHasRequest` hold one
  `range: Option<DeviceInstanceRange>`, which the client's `who_is`,
  `who_is_directed`, `who_is_network` and `who_has` take as one argument, so
  a request can't carry one limit, or send one past instance 4194303 (#1483). ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- The server drops a Who-Is or Who-Has that doesn't decode before spawning a
  task for it, and counts it in the new `DiscoveryCounters::malformed_dropped`
  (#1483). ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- **Rust API:** `DiscoveryCounters` is `#[non_exhaustive]`, so
  counters can be added without a break (#1493). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Breaking (Rust API):** Building a `BACnetServer` fails when a
  `DeviceBinding`, as the device's own MAC or its router's, is any group
  address of the link, such as a multicast address or the broadcast IP at
  another port, not only its broadcast; the error names the device (#1493). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Python API:** `BACnetServer.start()` raises `BacnetError`, naming the
  device and the address, when `add_device_binding` gave a multicast address,
  255.255.255.255 or the broadcast IP at another port (#1493). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Breaking (Python API):** a `PropertyValue` date reads with the full year
  (2026, not 126) and an unspecified year as 255 (`UNSPECIFIED`); and
  `time_synchronization` and `utc_time_synchronization` take only a
  specific date and time, raising `ValueError` otherwise (#1501). ([17404bb](https://github.com/jscott3201/rusty-bacnet/commit/17404bb55178e00727f2431bc2ccbc44a87599ad))

- RPM, GetAlarmSummary, GetEnrollmentSummary, AtomicReadFile, ReadRange,
  GetEventInformation and AtomicWriteFile get local payload and response
  budgets; see the
  [service budget index](docs/request-admission.md#delivered-service-budgets).

- **Breaking (Rust and Python API):** a configured server's DCC authorization
  defaults to deny all, even with the right password, until a `DccPolicy` mode
  (Python: `dcc_policy`) is chosen.

- **Wire:** B/IPv6 with `::` or no interface picks one usable link and
  address, keeps its traffic on that link, and fails startup when the choice
  is ambiguous; it used to ask the routing table for an address and fall back
  to `::1`. ([3bdf951](https://github.com/jscott3201/rusty-bacnet/commit/3bdf9510caa0dcdca4df8d163cbf0693da196ad4))

- **Rust and Python API:** multi-device RP, RPM and WP batch limits take
  `Option<NonZeroUsize>` in Rust, and Python refuses zero with `ValueError`;
  `None` still means 32.

- The tracked repository-local `.codex/` configuration and agent profiles are
  removed.

- **Rust API:** `BACnetObject::configure_audit_reporter_internal` takes all
  five Reporter settings at once.

- **Rust and Python API:** WriteProperty request encoding returns `Result` and
  refuses priorities outside 1 to 16 before anything is sent.

### Removed

- **Breaking (Rust API):** `bacnet_server::handlers::handle_device_communication_control`
  is gone; no server called it, and it stored the DCC state as a raw integer under
  the permissive rules (#1430). ([eeb9dd9](https://github.com/jscott3201/rusty-bacnet/commit/eeb9dd9c7fe2f666423d82a378a42592352367f1))

- The conformance ledger, its generated pages (the support summary and the
  draft PICS and BIBBs) and their tooling are removed while conformance
  reporting is reworked for the docs website, whose
  [support page](https://jscott3201.github.io/rusty-bacnet/project/support/#whats-supported)
  summarizes what is supported. ([4b8a17a](https://github.com/jscott3201/rusty-bacnet/commit/4b8a17a99c11dd6704eac73f5b764d7d6fbc0c16))

### Fixed

- **Rust and Python API:** endpoint clients send ReadPropertyMultiple through
  the shared read path, with ordered ACK correlation and one source READ
  record per reference; RPM request encoding returns `Result` (#780).

- Target Audit Reporters can carry an optional Maximum_Send_Delay and Send_Now
  pair that batches ordinary records; see
  [delayed target Audit reporting](docs/delayed-target-audit.md) (#783).

- **Python API:** RP, RPM and WP batch results carry a zero-based
  `request_index`, and their item errors are typed `BacnetError` instances
  instead of strings (#788).

- **Breaking (Rust API):** single-property COV subscribe methods take a
  `NonZeroU32` lifetime, and the server refuses a zero lifetime with
  VALUE_OUT_OF_RANGE (#802).

- **Wire and Rust API:** SubscribeCOV request encoding returns `Result` and
  refuses a lifetime without a confirmed-notification mode, and the server
  refuses that shape too (#805).

- **Rust and Python API:** `SubscribeCOVPropertyMultipleRequest::encode`
  returns `Result` and validates the whole request first, and Python validates
  every entry before it sends anything (#808).

- **Wire:** an Analog or Binary Value Audit policy change reserves its
  notification before committing, and is refused with SERVICE_REQUEST_DENIED
  when it can't (#809).

- **Breaking (Rust API):** COV compares a typed sample with validated
  Status_Flags, so a status-only change passes the threshold and a late
  unconfirmed send can't replace a newer one; `last_notified_observation`
  replaces `last_notified_value` and its setter (#810, #817, #826, #833,
  #840).

- **Rust API:** COV subscription identity distinguishes exact endpoints,
  subscription families, array indexes and confirmed forms, and table lookup
  and cancellation take typed keys (#812).

- **Breaking (Rust API):** Event Enrollment evaluation reports through one
  `evaluate_event_enrollments_report`, which now includes reliability results;
  the duplicate detailed API is gone (#815).

- **Wire:** a finite COV notification no longer rounds a remaining lifetime
  under one second down to zero, which means indefinite (#819).

- **Wire:** specialized commandable Value_Source COV for AO, AV, BO, BV, MSO
  and MSV: reports carry Value_Source, Last_Command_Time and
  Current_Command_Priority with Present_Value and Status_Flags (#823).

- **Rust API:** the typed ValueSource codec prerequisite for #824:
  `BACnetValueSource::Object` carries a `BACnetDeviceObjectReference`, encoded
  by `encode_value_source` and `decode_value_source` (#824).

- **Breaking (Rust and Python API):** a six-family command-source producer
  tracks the source of each command on Analog, Binary and Multi-state Outputs
  and Values, and local writes name their source (#824).

- With several Device objects in the database, wildcard Device reads, COV
  lists, discovery and notifications consistently pick the lowest instance
  (#832).

- **Wire:** COV-multiple contexts match the original client whichever router
  it came through (#833).

- The served Device's service bits, COV lists, Property_List and RPM rows come
  from the server that runs them, and writes to these fields are refused
  (#834).

- **Wire:** a ReadPropertyMultiple response to a wildcard Device request names
  the concrete Device in each result (#835).

- PICS property rows combine every configured instance of an object type and
  sort by property ID (#838).

- **Wire:** ordinary COV subscriptions match the original recipient across
  routers (#840).

- **Breaking (wire):** Priority_Array is read-only on every commandable
  object; command or relinquish through Present_Value with a priority (#842).

- **Breaking (Rust and Python API):** `ObjectIdentifier` validates its object
  type and instance at construction, and `new_unchecked` is gone (#847).

- **Rust and Python API:** direct B/IP endpoints send WriteProperty through
  the shared requester with source Audit WRITE records, and callers state
  `Commandability` (Python: `commandability=`) (#852).

- Present but empty Audit Target_Value and Current_Value fields stay distinct
  from absent values and from NULL (#853).

- **Wire:** timestamped COV-multiple reports carry each change's commit time
  and keep it until delivered, honouring Max_Notification_Delay. An oversized
  report splits in capture order, value by value if need be; a value no
  notification can carry is dropped (#856, #986, #1008, #1090).

- **Python API:** the remaining 104 native methods that return futures are
  typed `def -> Awaitable[T]` instead of as coroutines (#858).

- Python B/IP, SC and MS/TP endpoint startup and close go through one owner,
  so close joins startup and a second start can't open a second transport
  (#861).

- **Rust API:** an explicitly registered NORMAL B/IP Network Port follows its
  actual bind and resolves wildcard reads (#863).

- NORMAL B/IP full servers and endpoints discover and learn their local
  Network Number, taking configured provenance only from explicit registration
  (#875, #863).

- **Python API:** the remaining 43 native futures that produce nothing now
  return `None` instead of an empty tuple (#865).

- **Breaking (Rust and Python API):** raw Network Port construction is
  replaced by a configured, unbound IPV4/NORMAL snapshot; Python uses
  `add_bip_network_port` (#867).

- **Wire:** a write of a property a built-in object doesn't have returns
  UNKNOWN_PROPERTY instead of a write-access error (#870).

- A full server's shutdown retires I-Am admission, joins admitted sends and
  stops its own transport, so a retained broadcaster handle no longer keeps
  the socket alive (#872).

- Full servers and shared endpoints on every other built-in link answer and
  learn local Network Number controls, each starting UNKNOWN (#879). ([a8194c1](https://github.com/jscott3201/rusty-bacnet/commit/a8194c1425cd192d59cee2ff010a296ff26ad6df))

- **Breaking (wire, Rust and Python API):** Default_Color, Default_Color_Temperature
  and Color_Command use their standard property identifiers 4194330, 4194331 and
  4194334 instead of 508 to 510, which name Network Port properties (#887). ([97baf49](https://github.com/jscott3201/rusty-bacnet/commit/97baf4921afafc2ae9018a0dd68706de4e879bcd))

- **Breaking (Rust API):** An Event Enrollment reference may name a property
  identifier above 4194303, where ASHRAE assigns some, such as Default_Color
  (#887). ([97baf49](https://github.com/jscott3201/rusty-bacnet/commit/97baf4921afafc2ae9018a0dd68706de4e879bcd))

- Changes committed by background tasks, such as alarm confirmations,
  reliability changes and schedule writes, reach COV subscribers at once
  (#889).

- **Wire:** COV criteria report only an actual change, so binary, multi-state
  and zero-increment subscribers no longer get repeats of the same value
  (#889).

- B/IP and B/IPv6 transports bound to port 0 no longer set `SO_REUSEADDR`,
  which on Linux could let two sockets share an ephemeral port and lose
  replies (#892).

- **Wire:** a confirmed COV notification advances the subscriber's baseline
  only once acknowledged, and a failed one is reported again after a hold-off
  (#896, #923). ([3598b86](https://github.com/jscott3201/rusty-bacnet/commit/3598b8644648ccbdbf6c97453594acb5e0a1a771))

- BACnet/SC connections disable Nagle's algorithm, so a small message no
  longer waits on the peer's delayed ACK; the transport tests run more than
  twice as fast on Linux CI (#900). ([1b99550](https://github.com/jscott3201/rusty-bacnet/commit/1b9955044bfb2876af3e6994283555169d43b6b8))

- **Breaking (wire, Rust and Python API):** the Who-Am-I, You-Are, VT and
  WriteGroup codecs encode as the Clause 21 grammar does, so conformant peers
  can decode them, and their request types change to match (#912). ([5ffc8a1](https://github.com/jscott3201/rusty-bacnet/commit/5ffc8a1a00c718762246839cba088b8c813d7f6e))

- **Breaking (Rust API):** the PICS generator's `CharacterSet` offers exactly
  Annex A's six character sets with their Annex A labels: `DbcsMs` and `Ansi`
  are gone, and `DbcsIbm` and `Jisx0208` become `IbmMicrosoftDbcs` and
  `JisX0208` (#913). ([83b3639](https://github.com/jscott3201/rusty-bacnet/commit/83b3639eeb2af753950a9913c5a38838865994d8))

- **Wire:** a B/IP BBMD forwards its own broadcasts to its BDT peers and
  foreign devices, takes its own address from its BDT when bound to `0.0.0.0`,
  and no longer rebroadcasts a broadcast Forwarded-NPDU locally (#937, #952). ([873fd0c](https://github.com/jscott3201/rusty-bacnet/commit/873fd0cc97b25f0d137ffa30fb27698e39f3be8a))

- The `bacnet` CLI no longer overflows the main thread's stack on Windows
  (#950). ([7abb5e4](https://github.com/jscott3201/rusty-bacnet/commit/7abb5e451793bca30c923e2244e6bba93efea336))

- A BACnet/SC dial to a host name with several addresses races them, Happy
  Eyeballs style, instead of trying each in turn (#950). ([7abb5e4](https://github.com/jscott3201/rusty-bacnet/commit/7abb5e451793bca30c923e2244e6bba93efea336))

- A peer that the SC hub or a direct-connection listener refuses during the
  TLS handshake can now read the alert that says why, instead of seeing a
  connection reset (#950). ([7abb5e4](https://github.com/jscott3201/rusty-bacnet/commit/7abb5e451793bca30c923e2244e6bba93efea336))

- On Windows, a B/IP or B/IPv6 transport on an ephemeral port sets
  SO_EXCLUSIVEADDRUSE, so another socket can't take its unicast (#950). ([7abb5e4](https://github.com/jscott3201/rusty-bacnet/commit/7abb5e451793bca30c923e2244e6bba93efea336))

- On Windows, a B/IP transport bound to `0.0.0.0` lists the host's IPv4
  addresses and accepts only unicast addressed to one of them, as on Linux and
  macOS (#952).

- Starting a server, client or SC connection, and running the CLI, take much
  less stack in a debug build, and the native jobs now test with 1 MiB thread
  stacks (#953). ([943cc20](https://github.com/jscott3201/rusty-bacnet/commit/943cc2071b05b7942c63d4cb9849db564bf3a353))

- **Wire:** Loop, Schedule and both Trend Log object types compute
  Status_Flags from their state instead of always reading all FALSE, and a
  Loop's flag changes reach COV subscribers (#978). ([d5f0be7](https://github.com/jscott3201/rusty-bacnet/commit/d5f0be714354749fd13483501e553bbdd8c2498e))

- **Breaking (wire):** the Access Credential no longer serves Present_Value,
  which its table doesn't define; read Credential_Status instead (#979). ([badb67c](https://github.com/jscott3201/rusty-bacnet/commit/badb67cce780470d1b952503642ac2f1561bd288))

- **Breaking (wire, Rust API):** the Elevator Group's Landing_Call_Control and
  Landing_Calls carry BACnetLandingCallStatus values, and the unused
  `BACnetAssignedLandingCalls` is removed (#980). ([c3bce1f](https://github.com/jscott3201/rusty-bacnet/commit/c3bce1f0bef2887d82760f2c6d48b89fbed83042))

- **Breaking (wire):** Calendar no longer serves Status_Flags, Event_State or
  Out_Of_Service, which its table doesn't define (#984). ([cc2fc21](https://github.com/jscott3201/rusty-bacnet/commit/cc2fc21bb61138b998cf5ccf507dedf899148970))

- **Wire:** a Loop's SubscribeCOV report carries Setpoint and
  Controlled_Variable_Value, the Loop gains Controlled_Variable_Value and
  COV_Increment, and its Present_Value is writable while out of service
  (#985). ([943b8fa](https://github.com/jscott3201/rusty-bacnet/commit/943b8fabc2cdc5d358a5ce4799cc797c5e214c5e))

- **Breaking (wire):** neither Trend Log object type serves Out_Of_Service any
  longer, as their tables don't define it (#985). ([943b8fa](https://github.com/jscott3201/rusty-bacnet/commit/943b8fabc2cdc5d358a5ce4799cc797c5e214c5e))

- **Wire:** a field subscribed with timestamps no longer goes out without a
  Time_Of_Change in a timestamped COV-multiple report (#987). ([e31f7c3](https://github.com/jscott3201/rusty-bacnet/commit/e31f7c34597704b402dffa4e5044900280df93bd))

- The Python B/IP endpoint tests bind port 0 and read the bound port back
  instead of probing for a free one (#993). ([52bfdeb](https://github.com/jscott3201/rusty-bacnet/commit/52bfdeb3b6a76c2b953ff8bbfe7a7155ceb76dad))

- **Breaking (wire, Rust API):** Calendar's Date_List and the Schedule's
  calendar entries, special events, daily schedules and Effective_Period use
  their Clause 21 encodings, and Date_List is network-writable (#996). ([107e9fa](https://github.com/jscott3201/rusty-bacnet/commit/107e9fa5fe54d0c990083ac264b833b06672833f))

- **Breaking (wire):** the Elevator Group drops Status_Flags, Out_Of_Service
  and Reliability, gains the required Machine_Room_ID, and refuses a Group_ID
  above 255 (#997). ([63682c3](https://github.com/jscott3201/rusty-bacnet/commit/63682c3cdfb0f8a61dcc3cc46dc77a6589844551))

- **Wire:** the Lift's Car_Moving_Direction accepts every
  BACnetLiftCarDirection value, and a new Lift reads STOPPED (#998). ([63682c3](https://github.com/jscott3201/rusty-bacnet/commit/63682c3cdfb0f8a61dcc3cc46dc77a6589844551))

- **Wire:** AddListElement and RemoveListElement answer PROPERTY_IS_NOT_A_LIST
  when the target isn't a BACnetLIST, decided by the new
  `BACnetObject::is_list_property` (#999). ([22c6df8](https://github.com/jscott3201/rusty-bacnet/commit/22c6df893438fb5455e6726c408a436d2e48730a))

- **Breaking (Python API):** a Python process no longer crashes at exit while
  a Tokio thread completes an awaited future. The bindings bridge Tokio to
  asyncio themselves instead of through `pyo3-async-runtimes` (#1002). ([52bfdeb](https://github.com/jscott3201/rusty-bacnet/commit/52bfdeb3b6a76c2b953ff8bbfe7a7155ceb76dad))

- **Breaking (wire, Rust API):** the Lift object serves its Table 12-77 rows
  in the table's datatypes, gains the required rows it lacked, and drops
  Tracking_Value and Floor_Number (#1021). ([8b92c95](https://github.com/jscott3201/rusty-bacnet/commit/8b92c95831394b7d8e19bb683967a724c289875c))

- **Breaking (wire):** the Escalator gains the required Elevator_Group,
  Group_ID and Installation_ID rows, and its Energy_Meter_Ref becomes a
  BACnetDeviceObjectReference (#1022). ([8b92c95](https://github.com/jscott3201/rusty-bacnet/commit/8b92c95831394b7d8e19bb683967a724c289875c))

- **Breaking (wire):** ReadRange reads only BACnetLIST properties and answers
  PROPERTY_IS_NOT_A_LIST for anything else; Recipient_List and
  List_Of_Object_Property_References now page by element (#1025). ([77a0a81](https://github.com/jscott3201/rusty-bacnet/commit/77a0a8179e490b3ba49d7140ed8040cb48e0f667))

- **Breaking (wire, Rust API):** AddListElement and RemoveListElement answer
  every error with a ChangeList-Error carrying the first failed element
  number, which the client returns as `Error::Structured` and Python as
  `first_failed_element_number` (#1026). ([d697b35](https://github.com/jscott3201/rusty-bacnet/commit/d697b35ad304bb2c5eb3c0455054932a4d3119a1))

- **Breaking (wire):** AddListElement leaves an element that is already
  present alone, and RemoveListElement refuses the whole request when an
  element is missing or of another datatype (#1027). ([d697b35](https://github.com/jscott3201/rusty-bacnet/commit/d697b35ad304bb2c5eb3c0455054932a4d3119a1))

- **Breaking (wire, Rust API):** Calendar's Present_Value follows the device's
  local date, and a Schedule evaluates in Clause 12.24.4 order within its
  Effective_Period, writing typed values at Priority_For_Writing to each
  target's array index (#1029, #1028, #845). ([5dc2537](https://github.com/jscott3201/rusty-bacnet/commit/5dc2537d6cd70588f76f74e85bfd459c94cf1f55))

- **Breaking (wire):** an array-index read of the Elevator Group's
  Group_Members answers as a BACnetARRAY: index 0 is the member count, and an
  index past the end fails with INVALID_ARRAY_INDEX (#1034). ([c88f61a](https://github.com/jscott3201/rusty-bacnet/commit/c88f61afece69424171fc35e970b7aefaf6703d8))

- The Lift's Car_Door_Status and Landing_Door_Status accept writes while
  Out_Of_Service is TRUE, so a test tool can simulate the car; the door count
  stays the application's (#1035). ([c88f61a](https://github.com/jscott3201/rusty-bacnet/commit/c88f61afece69424171fc35e970b7aefaf6703d8))

- **Wire:** untimestamped COV-multiple values too large for one notification,
  such as the initial report of a large SubscribeCOVPropertyMultiple, now go
  out in several notifications that each fit (#1038). ([ccb3bad](https://github.com/jscott3201/rusty-bacnet/commit/ccb3bad0f31b7c630e080525d83b6517e023de01))

- The SC hub handshake-deadline tests run on Tokio's paused clock, and the
  B/IP BBMD and B/IPv6 VMAC-collision tests retry with a fresh port when
  another socket takes theirs (#1042, #1032). ([9d4baf4](https://github.com/jscott3201/rusty-bacnet/commit/9d4baf4b1146ec70f5958285cf1d1b7ddc64abc4))

- ReadRange on the Device's Active_COV_Subscriptions and
  Active_COV_Multiple_Subscriptions pages the live subscriptions instead of
  answering OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (#1046). ([ec9b5a3](https://github.com/jscott3201/rusty-bacnet/commit/ec9b5a304c97b11d43467976637c0a5138c8f4d2))

- **Breaking (Rust API):** the client decodes every Clause 21 structured error
  body, so a CreateObject, SubscribeCOVPropertyMultiple,
  ConfirmedPrivateTransfer or VTClose error returns its fields instead of
  timing out (#1047). ([0f43456](https://github.com/jscott3201/rusty-bacnet/commit/0f43456afa2ae49d066aa2875d2db1524e0e4966))

- **Python API:** `BacnetProtocolError` gains the structured error fields
  `first_failed_write_attempt`, `first_failed_subscription`, `vendor_id`,
  `service_number`, `error_parameters` and `vt_session_identifiers` (#1047). ([0f43456](https://github.com/jscott3201/rusty-bacnet/commit/0f43456afa2ae49d066aa2875d2db1524e0e4966))

- **Breaking (wire):** the server answers CreateObject and
  SubscribeCOVPropertyMultiple errors with their Clause 21 error bodies
  instead of a plain class and code (#1047). ([0f43456](https://github.com/jscott3201/rusty-bacnet/commit/0f43456afa2ae49d066aa2875d2db1524e0e4966))

- **Wire:** when an object refuses an AddListElement that adds several
  elements, the ChangeList-Error names the refused element (#1048). ([436dd01](https://github.com/jscott3201/rusty-bacnet/commit/436dd01dd5ff577d5cccf2a15e0ca91639969a94))

- **Breaking (wire, Rust API):** SubscribeCOVPropertyMultiple processes its
  references in order and stops at the first failure, keeping the ones before
  it; the subscription caps are checked per reference too (#1058, #1059). ([73a3b28](https://github.com/jscott3201/rusty-bacnet/commit/73a3b28ada3d2729431d0dcb7804a8ca3d417677))

- **Wire:** SubscribeCOV covers every Table 13-1 object type the stack builds,
  adding Access Point, Credential Data Input and Load Control, and each report
  carries the values its row names (#1061). ([9a2a7b7](https://github.com/jscott3201/rusty-bacnet/commit/9a2a7b72ef4945b3ed6593753f714585a0d75753))

- **Breaking (wire):** the Loop serves the required rows it left out:
  Controlled_Variable_Units, Action, Priority_For_Writing and the three
  gain-constant units. Python's `add_loop` takes them as keywords (#1062). ([f04e5e1](https://github.com/jscott3201/rusty-bacnet/commit/f04e5e1434bcf0f4400ba35cf5b939816d9d986b))

- **Breaking (wire):** fourteen object types drop properties their Clause 12
  tables don't define, such as Out_Of_Service on Command and Notification
  Class; reading one now fails with UNKNOWN_PROPERTY (#1064). ([8a748c2](https://github.com/jscott3201/rusty-bacnet/commit/8a748c258277400eff1029183fbffbff57cbd321))

- **Breaking (wire, Rust API):** a special event priority outside 1 to 16 now
  gets VALUE_OUT_OF_RANGE instead of INVALID_DATA_ENCODING, and
  `BACnetSpecialEvent::event_priority` is a `u64` (#1087). ([122b629](https://github.com/jscott3201/rusty-bacnet/commit/122b6295263ed98daa8f360d5a3c81a1a8b3bcc5))

- **Breaking (wire, Rust API):** a Notification Class Recipient_List holds at
  most 32 destinations, and `NotificationClass::add_destination` returns
  `Result` (#1098). ([2caee31](https://github.com/jscott3201/rusty-bacnet/commit/2caee311979e4311fd7f87d4e09bee815dee50ac))

- **Breaking (wire, Rust API):** a configured recipient's MAC is at most 18
  octets, routing holds every Notification Class to the 32-destination cap,
  and the flat Recipient_List form from before #152 is gone (#1098, #1124,
  #1125). ([2caee31](https://github.com/jscott3201/rusty-bacnet/commit/2caee311979e4311fd7f87d4e09bee815dee50ac))

- **Breaking (wire):** a SubscribeCOVPropertyMultiple reference that asks for
  timestamps while the Device has no valid clock is refused on its own, in
  request order, instead of refusing the whole request (#1102). ([6a8c79a](https://github.com/jscott3201/rusty-bacnet/commit/6a8c79a18934f40e828a40902e58eb3beaa1e810))

- **Breaking (Rust API):** every SC hub start method returns
  `Error::Transport` with the OS's `io::Error` when its bind fails, instead of
  `Error::Encoding` with the error's text (#1104). ([6a8c79a](https://github.com/jscott3201/rusty-bacnet/commit/6a8c79a18934f40e828a40902e58eb3beaa1e810))

- **Breaking (wire, Rust API):** the Global Group's Group_Members and
  Present_Value go out in their Clause 21 forms, and an indexed read of its
  arrays returns one element (#1107). ([0980c10](https://github.com/jscott3201/rusty-bacnet/commit/0980c10774260915fc65845d99e8fa05b07fee5f))

- **Breaking (wire):** an Access Door accepts Door_Status, Lock_Status and
  Door_Alarm_State writes while Out_Of_Service is TRUE, so a client can
  simulate the door (#1131). ([533aad0](https://github.com/jscott3201/rusty-bacnet/commit/533aad09c2831da43a0db8274cb43d3087989d15))

- **Breaking (wire, Rust API):** Load Control shed levels, Access Point
  Access_Event_Time and Credential Data Input Update_Time and Present_Value use
  their standard datatypes, and Requested_Shed_Level accepts the standard
  write form (#1133). ([0debbca](https://github.com/jscott3201/rusty-bacnet/commit/0debbcadb03353f5b065c76b9207f97219248d33))

- **Breaking (wire, Rust API):** the Group object serves List_Of_Group_Members
  and Present_Value in their Table 12-17 datatypes, and rebuilds Present_Value
  from the members on every read (#1134). ([9217089](https://github.com/jscott3201/rusty-bacnet/commit/9217089a8c6dd561c5dcb096738f3d58a8fb42c1))

- **Breaking (wire, Rust API):** an indexed read of Subordinate_List,
  Subordinate_Annotations or Action returns one element, and Subordinate_List
  and Action elements go out in their Clause 21 forms (#1135). ([b7f1209](https://github.com/jscott3201/rusty-bacnet/commit/b7f12093227a886113ca8d63eb6f488a65e185cb))

- **Breaking (wire, Rust API):** the network layer drops and counts an NPDU
  whose DLEN or SLEN exceeds 18 octets, a router rejects one naming a DNET
  with reason 6, and `decode_npdu` returns `NpduDecodeError` (#1141). ([fe9175b](https://github.com/jscott3201/rusty-bacnet/commit/fe9175b409b011af1c95e9b02f58d3e0f001b824))

- **Wire:** an Access Door works out Secured_Status from its command, alarm,
  door and lock state on every read instead of always reading SECURED (#1148). ([03161a6](https://github.com/jscott3201/rusty-bacnet/commit/03161a636b181815c61996ab717a176855bb1875))

- **Wire:** AddListElement and RemoveListElement report a COV change the edit makes,
  as WriteProperty does, even when no event transition follows (#1149). ([a81304f](https://github.com/jscott3201/rusty-bacnet/commit/a81304f82c9a7d51a0d8b8f5d325ba698d6f5e33))

- Command and Channel runs that write back into their own object, directly or
  through others, stop after one round (#1151). ([36d8b72](https://github.com/jscott3201/rusty-bacnet/commit/36d8b7227baafbc1629420e642d16c248aeefe8c))

- **Wire:** an Averaging Object_Property_Reference naming this device is
  accepted and stored as a local reference; one naming another device is
  refused with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (#1153). ([db640e5](https://github.com/jscott3201/rusty-bacnet/commit/db640e5887b2f89cf381af9522de5ffef8379473))

- **Wire:** a recipient's Abort, which carries the server flag, now ends a
  confirmed notification this device sent, and an Abort with the flag clear no
  longer does (#1155). ([b8309cb](https://github.com/jscott3201/rusty-bacnet/commit/b8309cb9216be951f8e5cadb6b7a7eec0deffd2c))

- **Breaking `BACnetAddress` MAC bound (wire, Rust API):** every recipient,
  ValueSource and AuditLogQuery address codec now refuses a MAC longer than
  `BACnetAddress::MAX_MAC_LEN` (18 octets) in both directions, and the
  recipient encoders return `Result` (#1156). ([0696eb8](https://github.com/jscott3201/rusty-bacnet/commit/0696eb861cae65a1c4a855bdb0ddbcc4c48aa3fb))

- **Breaking (wire):** router rejects go back to whoever first sent
  the refused NPDU, using its SNET/SADR, and a received reject is relayed by
  its DNET/DADR (#1158). ([83cb97e](https://github.com/jscott3201/rusty-bacnet/commit/83cb97e51059fa6961561e7cbd5832d0a8d4428d))

- **Breaking (wire):** Credential Data Input takes Present_Value and Reliability
  writes while out of service, checked against its declared formats, and still
  refuses them in service (#1168). ([3498938](https://github.com/jscott3201/rusty-bacnet/commit/349893897dee41017d160c9683e7593828754c1e))

- **Breaking (wire):** Credential Data Input Supported_Formats and
  Supported_Format_Classes, Access Door Door_Members and Access Point
  Access_Doors are served as arrays in their Clause 21 element forms, with new
  setters (#1169). ([3498938](https://github.com/jscott3201/rusty-bacnet/commit/349893897dee41017d160c9683e7593828754c1e))

- A Group member that names the Device's Active_COV_Subscriptions or
  Active_COV_Multiple_Subscriptions now reads the live subscription list
  instead of an empty one (#1171). ([15a787f](https://github.com/jscott3201/rusty-bacnet/commit/15a787fb2dc82d3811ce3d6a8070304a7f444d36))

- **Router rejects for a node on the arrival link (wire):** when a refused
  NPDU's SNET is the arrival port's own network, `BACnetRouter` now sends the
  reject as a local unicast to its SADR instead of addressing it with a DNET
  that a non-router discards (#1174). ([5c1afb3](https://github.com/jscott3201/rusty-bacnet/commit/5c1afb3f1dc2e9a65b25866709c67799f0f34046))

- Away from the bundled server, `tick_schedules` runs the Command lists it starts, and the bare write handlers end the runs they can't make as unsuccessful, so In_Process doesn't stay TRUE (#1178). ([4c20f13](https://github.com/jscott3201/rusty-bacnet/commit/4c20f135e523b39e53e1dba60f6181545998a259))

- **Breaking (wire):** Averaging and Event Enrollment Object_Property_Reference
  and Life Safety Member_Of and Zone_Members are served as context-tagged
  device references, Life Safety Zone gains Member_Of, and Averaging refuses
  the old flat form (#1182). ([11c55eb](https://github.com/jscott3201/rusty-bacnet/commit/11c55eb65924f8de11ace8c20cf57f330dfbb451))

- **Trend Log polling (wire):** a Log_DeviceObjectProperty naming another
  device logs a failure record instead of a same-numbered local object's
  value, and a failed local read logs its error instead of a null value
  (#1183). ([4cbc93b](https://github.com/jscott3201/rusty-bacnet/commit/4cbc93b72a153302b35407a7aea5a9e3d7af4a05))

- **Event Enrollment references:** a FLOATING_LIMIT setpoint reference naming
  this device is read and reported, and with several Devices every enrollment
  reference treats the lowest one as this device (#1184). ([4cbc93b](https://github.com/jscott3201/rusty-bacnet/commit/4cbc93b72a153302b35407a7aea5a9e3d7af4a05))

- **Breaking (custom transports):** `NetworkLayer` and `BACnetRouter` drop a
  frame whose link-layer source MAC is longer than 18 octets and count it in
  `address_length_drops()`; no built-in transport reports one (#1198). ([981024a](https://github.com/jscott3201/rusty-bacnet/commit/981024a31d3bcb15fcb5d84136a4fb4c2d16e546))

- **Breaking (wire, Rust API):** `YouAreRequest` refuses a device MAC address
  longer than 18 octets on decode and encode (#1200). ([981024a](https://github.com/jscott3201/rusty-bacnet/commit/981024a31d3bcb15fcb5d84136a4fb4c2d16e546))

- **Trend Log indexed references (wire):** the poller logs the array element a
  Log_DeviceObjectProperty index names, and a failure for an index past the end
  or on a property that is not an array (#1205). ([eb86197](https://github.com/jscott3201/rusty-bacnet/commit/eb8619781f2917baa381a61d65fd671dc76bf987))

- **Router rejects for a looped NPDU (wire):** when a refused NPDU's SNET is
  another of the router's networks, the reject goes out that port as a local
  unicast to the SADR; when its SNET/SADR is the router itself, none is sent
  (#1219). ([ff7ba6b](https://github.com/jscott3201/rusty-bacnet/commit/ff7ba6b9ad49359a00b46453334c83a8fd7dafb0))

- **Breaking (wire):** ReadRange serves Trend Log and Event Log records framed as BACnetLogRecord and
  BACnetEventLogRecord, with encoders and decoders in `bacnet_encoding::constructed` (#1233). ([e1d1d0d](https://github.com/jscott3201/rusty-bacnet/commit/e1d1d0d35367438d4da43b1e6bf76e0240cf95af))

- **Breaking:** Log objects refuse a record that would not encode when it is added, and the
  pollers log an any-value over 256 octets as PROPERTY / VALUE_TOO_LONG, so every stored record
  can be served (#1233, #1236). ([e1d1d0d](https://github.com/jscott3201/rusty-bacnet/commit/e1d1d0d35367438d4da43b1e6bf76e0240cf95af))

- **Breaking (wire):** Log records (Trend, Event, Trend Log Multiple and Audit Log) send and read
  BACnetLogStatus bit 0 first, so log-disabled goes out as `05 80`, not as log-interrupted (#1233). ([e1d1d0d](https://github.com/jscott3201/rusty-bacnet/commit/e1d1d0d35367438d4da43b1e6bf76e0240cf95af))

- **Breaking (wire):** Trend Log and Trend Log Multiple serve and accept
  Log_DeviceObjectProperty as a context-tagged reference, Trend Log Multiple's
  as an indexable array, and a written change purges the log (#1234). ([11c55eb](https://github.com/jscott3201/rusty-bacnet/commit/11c55eb65924f8de11ace8c20cf57f330dfbb451))

- **Breaking (wire):** The trend pollers (Trend Log Multiple too) log a CharacterString, Double,
  array or other unlisted datatype as an any-value carrying its encoding instead of NULL (#1236). ([e1d1d0d](https://github.com/jscott3201/rusty-bacnet/commit/e1d1d0d35367438d4da43b1e6bf76e0240cf95af))

- **Breaking (wire):** Trend Log, Trend Log Multiple and Event Log answer ReadProperty and RPM of
  Log_Buffer with READ_ACCESS_DENIED; ReadRange still reads it (#1237). ([e1d1d0d](https://github.com/jscott3201/rusty-bacnet/commit/e1d1d0d35367438d4da43b1e6bf76e0240cf95af))

- **Breaking (wire):** Access Zone takes Occupancy_Count and Reliability writes
  while out of service and serves its own values again on the return to service
  (#1247). ([af88659](https://github.com/jscott3201/rusty-bacnet/commit/af88659febed76ebbf8cd63ece71bf0d68e760c4))

- **Breaking (wire):** Access Point records an OUT_OF_SERVICE or
  OUT_OF_SERVICE_RELINQUISHED access event, with a new tag and time, each time
  Out_Of_Service changes (#1248). ([af88659](https://github.com/jscott3201/rusty-bacnet/commit/af88659febed76ebbf8cd63ece71bf0d68e760c4))

- **Rust API:** Credential Data Input keeps Present_Value to a
  declared format, and Python can now declare formats and set Door_Members and
  Access_Doors (#1249). ([af88659](https://github.com/jscott3201/rusty-bacnet/commit/af88659febed76ebbf8cd63ece71bf0d68e760c4))

- `BACnetServer::stop()` ends each Command or Channel run it cancels, or that never started, where it stood, with its unmade writes unsuccessful, so none stays In_Process or IN_PROGRESS (#1252). ([2a8495a](https://github.com/jscott3201/rusty-bacnet/commit/2a8495a32cb03363e7f8712697e215ee526d2f7e))

- **Breaking (wire):** The server ignores a confirmed request sent by broadcast or multicast,
  whatever the service, instead of executing and answering it, so the Notification Forwarder never
  forwards one (#1257). ([76b2b84](https://github.com/jscott3201/rusty-bacnet/commit/76b2b84f0d75ce1d0d2b7a17713c10dbfaf5a945))

- **Breaking (wire):** Lighting Output serves and takes Lighting_Command as a
  BACnetLightingCommand checked against its operation, refusing the old octet string,
  and takes the lighting commands a Channel or WriteGroup passes to it (#1263). ([91ce62a](https://github.com/jscott3201/rusty-bacnet/commit/91ce62a13f533db07b8a6c5586c2facf75edeb03))

- The Notification Forwarder's file backend synchronizes its directory after each rename, and the
  Audit Log's when it creates a slot, so a completed save survives a power loss (not on Windows)
  (#1270). ([1d3eb21](https://github.com/jscott3201/rusty-bacnet/commit/1d3eb211c7700a20b59320ddf968c4fe8c269744))

- **Breaking (Rust API):** Setters and write paths that store device object
  references, Channel members included, refuse a device identifier that isn't
  a Device object; Python raises `ValueError` (#1285). ([9cd82bc](https://github.com/jscott3201/rusty-bacnet/commit/9cd82bc9c4126d5fa0953099a9fa5fdd898e52c2))

- **Breaking (wire):** the array and list classification covers every array of
  the 2020 object tables, Lift and Network Port included. An indexed read of an
  array an object lacks answers UNKNOWN_PROPERTY, and Access Rights serves its
  rule arrays as empty arrays (#1296). ([ce572cf](https://github.com/jscott3201/rusty-bacnet/commit/ce572cf812e35750d255bfc885eb5ebdfe364548))

- **Breaking (Python API):** read results keep every element of a value. A whole
  array or list reads as a `list` at any length, several values as a `list`, and
  context-tagged or otherwise unrepresentable content as `application_data`
  octets; only broken framing raises (#1296). ([ce572cf](https://github.com/jscott3201/rusty-bacnet/commit/ce572cf812e35750d255bfc885eb5ebdfe364548))

- **Breaking (Python API):** `BACnetServer.read_property` reads through the
  server's ReadProperty evaluator and `write_property_local` decodes its value
  as a network WriteProperty does, so local reads and writes match network ones
  and what one reads writes back (#1297). ([ce572cf](https://github.com/jscott3201/rusty-bacnet/commit/ce572cf812e35750d255bfc885eb5ebdfe364548))

- **Breaking (wire):** Notification Class notifications and forwarded copies to a recipient
  naming the local network's number go as local traffic with no DNET, so non-routing nodes
  there receive them (#1299). ([5e66d84](https://github.com/jscott3201/rusty-bacnet/commit/5e66d84a1278aa6fd8795cc2487653d77334fad7))

- An AddListElement or RemoveListElement that moves an object into or out of
  alarm, such as an Alarm_Values edit, starts the transition at once, as
  WriteProperty does, instead of at the next periodic tick (#1305). ([bbf4acb](https://github.com/jscott3201/rusty-bacnet/commit/bbf4acb25aa4100df6fd1c289ff56cc71473376a))

- **Wire:** Access Zone serves Entry_Points and Exit_Points as lists of
  device object references to Access Points, set through new setters and
  Python keyword arguments (#1306). ([bbf4acb](https://github.com/jscott3201/rusty-bacnet/commit/bbf4acb25aa4100df6fd1c289ff56cc71473376a))

- **Breaking (wire, Rust API):** Schedule references, Global Group members, Command actions and the Event Enrollment reference refuse a non-Device device identifier with VALUE_OUT_OF_RANGE, and Python `add_command` raises ValueError (#1308). ([7ade5e3](https://github.com/jscott3201/rusty-bacnet/commit/7ade5e36df847e9d7909ff6d4b9d7615824cd0f4))

- **Breaking (wire):** the Loop's three references and Pulse Converter Input_Reference
  are served and written in their context-tagged forms, refusing the old flat list, and
  Averaging treats a reference that doesn't open with tag [0] as the wrong datatype (#1312). ([808e0c2](https://github.com/jscott3201/rusty-bacnet/commit/808e0c2903b79898c17009d9f583c1986c248fe0))

- A `write_local` dropped after its write committed, by a timeout or a
  `select!`, ends the Command or Channel run it started as if none of its
  writes were made (#1324). ([b7c351b](https://github.com/jscott3201/rusty-bacnet/commit/b7c351b2ec8fa5ddbb8fbb4d9d57a3f911b6714b))

- **Wire:** a confirmed COV or event notification outstanding when DeviceCommunicationControl
  disables initiation is no longer retried; it ends at its next retry and frees its invoke ID (#1327). ([74849ac](https://github.com/jscott3201/rusty-bacnet/commit/74849ac85e43e2ad193fd950f670f3858de135f5))

- **Wire:** WriteProperty and WritePropertyMultiple hand a list property to the
  object as a list at every length, so an empty value clears Alarm_Values; an
  empty write to a read-only or unserved list is now WRITE_ACCESS_DENIED or
  UNKNOWN_PROPERTY, not INVALID_DATA_ENCODING (#1328). ([cc8e7c0](https://github.com/jscott3201/rusty-bacnet/commit/cc8e7c01f528f3a8de45cbf205f8d4a8bea119d0))

- **Wire:** a Pulse Converter reports a configuration fault while its
  Input_Reference names a property it can't count from, re-checked as objects
  come and go, takes a simulated Reliability out of service, and a running
  server counts that property's increases into Count (#1341). ([5555911](https://github.com/jscott3201/rusty-bacnet/commit/55559112ff4daadbb7570f8c29d3839ab53750a2))

- A Channel writes each member when its own Execution_Delay is up, so a member
  in another device that waits for its answer doesn't delay the others; runs
  send each device one request at a time and at most 32 in all, and Reliability
  names the first failure to finish (#1343). ([cbe62f3](https://github.com/jscott3201/rusty-bacnet/commit/cbe62f311b478671b7cafadb789affb2fbf238c9))

- **Wire:** Command and Channel writes in another device, Audit notifications and forwarded
  Audit copies, and client requests and network broadcasts that name the local network's
  number go as local traffic with no DNET, so non-routing nodes there receive them (#1358). ([f0c8d74](https://github.com/jscott3201/rusty-bacnet/commit/f0c8d74ce75c5cb897a613e53de97e7af83f544d))

- **Wire:** A confirmed Audit notification whose Audit Log commit fails is refused with DEVICE /
  OPERATIONAL_PROBLEM instead of SERVICES / OTHER (#1366). ([b07dc35](https://github.com/jscott3201/rusty-bacnet/commit/b07dc359cf5d2d0c43317b31462910b7d14e225d))

- **Breaking (Rust API):** local writes (`write_local`, `set_present_value_local` and the like) must
  run inside a Tokio runtime, failing and writing nothing outside one; once committed, their COV, event,
  Schedule and Staging work finishes even if the caller is dropped (#1367). ([aef70c4](https://github.com/jscott3201/rusty-bacnet/commit/aef70c43149af99d5c3dcc3d36d5ccc3313f7eb1))

- **Python API:** a local write such as `write_property_local` or `set_present_value_local` whose
  asyncio task is cancelled after the write committed still sends its COV and event notifications
  and runs the Command or Channel it started (#1367). ([aef70c4](https://github.com/jscott3201/rusty-bacnet/commit/aef70c43149af99d5c3dcc3d36d5ccc3313f7eb1))

- **Wire:** audit notifications and Audit Log forwards keep going out while DCC disables initiation,
  as Clause 16.1 allows, and writes that owe an audit record get a SimpleACK there instead of
  SERVICE_REQUEST_DENIED (#1370). ([34165da](https://github.com/jscott3201/rusty-bacnet/commit/34165da9eeaa9dfa0d0d440122ef2f489e3904c2))

- **Wire:** a confirmed event notification whose Device recipient's observed binding expires before a
  retry now ends at that retry and frees its invoke ID, counted in `device_recipient_unbound` (#1371). ([aef70c4](https://github.com/jscott3201/rusty-bacnet/commit/aef70c43149af99d5c3dcc3d36d5ccc3313f7eb1))

- **Breaking (wire, Rust API):** a DeleteObject request whose object
  identifier is under any tag but its application tag is refused with
  SERVICES / OTHER instead of deleting the object; I-Have decoding refuses
  such members too (#1374). ([d682790](https://github.com/jscott3201/rusty-bacnet/commit/d68279070d0a21da717d84f10db7854b59a2f10b))

- **Breaking (wire, Rust API):** AtomicReadFile and AtomicWriteFile requests
  with a member under any tag but its application tag are refused with
  SERVICES / OTHER instead of being served (#1375). ([d682790](https://github.com/jscott3201/rusty-bacnet/commit/d68279070d0a21da717d84f10db7854b59a2f10b))

- **Wire:** `NetworkLayer` and `BACnetRouter` drop an inbound NPDU whose DNET
  0xFFFF also carries a DADR and count it in `global_broadcast_dadr_drops()`;
  the router no longer passes such an NPDU on to its other networks (#1379). ([520abe7](https://github.com/jscott3201/rusty-bacnet/commit/520abe781edc7613dfb2a38e0c8a20e2b31d6cdd))

- **Wire:** Lighting Output refuses a Lighting_Command_Default_Priority of 6,
  the priority kept for Minimum On/Off (#1384). ([b9ad662](https://github.com/jscott3201/rusty-bacnet/commit/b9ad6626f88014b09126ec57860110a6667fbbc9))

- **Wire:** Lighting Output stores a Present_Value or Relinquish_Default level
  above 0.0 and below 1.0 as 1.0, and -0.0 as 0.0. Tracking_Value, which
  always read 0.0, now follows Present_Value on every write (#1385). ([432a832](https://github.com/jscott3201/rusty-bacnet/commit/432a832dc68e81f6cb3f6395a0d1c7140fdd633b))

- **Breaking (wire):** Color and Color Temperature serve and take Color_Command as
  a BACnetColorCommand checked against the operations each object allows, refusing
  the old octet string (#1386). ([97baf49](https://github.com/jscott3201/rusty-bacnet/commit/97baf4921afafc2ae9018a0dd68706de4e879bcd))

- **Breaking (wire, Rust API):** While DeviceCommunicationControl's
  DISABLE_INITIATION is in force, the server no longer answers Who-Has with
  I-Have, and `broadcast_i_am()` sends nothing and returns an error; Who-Is
  still gets its I-Am (#1388). ([cd9c650](https://github.com/jscott3201/rusty-bacnet/commit/cd9c650d1042f0aeb9501c94785e821a545a1dc3))

- **Wire:** CreateObject decodes each initial value as WriteProperty does: a
  list such as Alarm_Values arrives as a list at any length, a scalar with a
  trailing element is INVALID_DATA_TYPE instead of keeping the first, and an
  index on a non-array is PROPERTY_IS_NOT_AN_ARRAY (#1389). ([a00da01](https://github.com/jscott3201/rusty-bacnet/commit/a00da01e6b4481e11f8b7b82b67822c5dda2bf05))

- **Wire:** Access User Credentials reads as a list of device
  object references instead of bare object identifiers, so it can name a
  credential held by another device (#1394). ([7cd802c](https://github.com/jscott3201/rusty-bacnet/commit/7cd802cb427216243a764e67f1b136e533a931ac))

- **Wire:** a NULL to a non-commandable property with no NULL in its datatype
  now succeeds unchanged over WP, WPM and local writes, not INVALID_DATA_TYPE.
  A Value_Source correction by a non-owner is WRITE_ACCESS_DENIED even when
  its value is malformed (#1396). ([32bc642](https://github.com/jscott3201/rusty-bacnet/commit/32bc64274fa26b0da548472b72694b4094e5db62))

- **Wire and Rust API:** Access Zone refuses NORMAL in Alarm_Values, as
  the Access Door does in its alarm lists, since alarming on NORMAL would put a
  zone in alarm while its count sits inside its limits (#1401). ([5b86027](https://github.com/jscott3201/rusty-bacnet/commit/5b86027bd48101bfac12038bf32dce517e6e8801))

- **Wire:** The shared endpoint publishes its network's number, so its routed reads naming that
  number, and source Audit records to an address on that network, go as local traffic with no DNET,
  which non-routing peers receive (#1403). ([8534bb1](https://github.com/jscott3201/rusty-bacnet/commit/8534bb121b08a748d140347cb0d02efa37db3e88))

- **Wire:** A request from a Device bound through the server's own network number, direct or
  relayed with that number as its SNET, is tied to that Device, so its Audit records and
  Value_Source name the Device instead of its address (#1404). ([8c06f00](https://github.com/jscott3201/rusty-bacnet/commit/8c06f00daab5ac67a34f8b11addf1bec71f7e37c))

- **Breaking (wire, Rust and Python API):** service decoders, `IHaveRequest`,
  `AtomicWriteFileAck` and `PrivateTransferAck` included, refuse trailing
  octets: the server refuses or drops such requests, the client ignores such an
  I-Am, and `confirmed_private_transfer` raises `BacnetError` (#1411). ([4ec1404](https://github.com/jscott3201/rusty-bacnet/commit/4ec14048e8ef00eed8506f3c9e745c1be46e776e))

- **Wire:** a NULL to a non-commandable property with no NULL in its datatype
  now succeeds unchanged as a CreateObject initial value and as a Schedule's
  target write, not INVALID_DATA_TYPE (#1416). ([a00da01](https://github.com/jscott3201/rusty-bacnet/commit/a00da01e6b4481e11f8b7b82b67822c5dda2bf05))

- **Wire:** `write_local`, Command and Channel local writes and Schedule
  target writes refuse an array index as WriteProperty does
  (PROPERTY_IS_NOT_AN_ARRAY, or UNKNOWN_PROPERTY) instead of writing the
  whole property; a Schedule's NULL to such a reference now fails (#1426). ([be29d40](https://github.com/jscott3201/rusty-bacnet/commit/be29d40796389a10b85e418044307490bf0c2948))

- **Breaking (wire):** a Schedule reference naming a missing object or property, or an
  array index the property can't take, sets the Schedule's Reliability to
  CONFIGURATION_ERROR until a write to it succeeds or it leaves the list
  (#1433). ([be29d40](https://github.com/jscott3201/rusty-bacnet/commit/be29d40796389a10b85e418044307490bf0c2948))

- **Wire:** an Object_Name already in use is refused with the PROPERTY error
  class instead of OBJECT, over WriteProperty, WritePropertyMultiple,
  CreateObject, local writes and `ObjectDatabase::add` (#1434). ([b687282](https://github.com/jscott3201/rusty-bacnet/commit/b687282bf2972e8ee875f54582cdd61f8b5259c1))

- **Wire:** a Schedule offers its value again, each pass, to the references that
  refused it, so CONFIGURATION_ERROR clears and the target gets the value once
  the cause is gone, such as a missing object being created (#1436). ([c223214](https://github.com/jscott3201/rusty-bacnet/commit/c2232141aa2e42d2ec0d641a7542fde6d443ad46))

- **Wire:** CreateObject no longer fails when another object holds the default
  name of the new one. That name is now the type and instance (`BINARY_VALUE-2`,
  not `ObjectType::BINARY_VALUE-2`), with the first free ` (n)` added when it
  is taken (#1437). ([fe20bc5](https://github.com/jscott3201/rusty-bacnet/commit/fe20bc58092d5e07ad4d7e6b7796f60696472854))

- **Breaking (wire):** a confirmed request the server can't decode now draws
  a Reject naming the fault, for every service, where some drew an Error, and
  some it already rejected get a different reason. The client rejects
  malformed notifications the same way (#1446). ([d54b143](https://github.com/jscott3201/rusty-bacnet/commit/d54b143114dbc71de28731df2362893a34824393))

- **Breaking (wire):** a Who-Is carrying only one of its two limits is
  malformed: `WhoIsRequest::decode` refuses it and the server drops it, where
  it used to answer as if the request named every device (#1447). ([d54b143](https://github.com/jscott3201/rusty-bacnet/commit/d54b143114dbc71de28731df2362893a34824393))

- **Wire:** The server's target Audit takes an Address recipient naming its own network number as
  local, sending to its MAC with no DNET; one whose network the number does not name, unknown or
  different, starts unresolved and resolves once it does (#1460). ([b07dc35](https://github.com/jscott3201/rusty-bacnet/commit/b07dc359cf5d2d0c43317b31462910b7d14e225d))

- **Wire:** A target Audit recipient change away from an Address the network number does not name
  goes ahead: the new recipient gets the record and an unconfirmed global broadcast replaces the old
  one's copy, and Reporter health follows each change of number (#1460). ([b07dc35](https://github.com/jscott3201/rusty-bacnet/commit/b07dc359cf5d2d0c43317b31462910b7d14e225d))

- **Wire:** An endpoint source Audit recipient Address whose network the session's number does not
  name, unknown or different, starts unresolved; a change away from it goes to the new recipient
  and by unconfirmed global broadcast (#1461). ([b07dc35](https://github.com/jscott3201/rusty-bacnet/commit/b07dc359cf5d2d0c43317b31462910b7d14e225d))

- **Wire:** Once the local network number is known, the client, the shared
  endpoint and the server complete a request whose answer a router relays back
  with that number as its SNET and the station as its SADR, instead of timing
  out (#1465). ([520abe7](https://github.com/jscott3201/rusty-bacnet/commit/520abe781edc7613dfb2a38e0c8a20e2b31d6cdd))

- **Wire:** a Trend Log classes Start_Time, Stop_Time, Log_Interval and
  Log_DeviceObjectProperty as required, so ReadPropertyMultiple with
  REQUIRED returns them and the PICS lists them as required (#1481). ([d3529d5](https://github.com/jscott3201/rusty-bacnet/commit/d3529d5154f715f4d99a641555949bd17a0b6b58))

- **Python API:** `who_is`, `discover`, `who_is_directed`, `who_has_by_id`
  and `who_has_by_name` raise `ValueError` for only one of `low_limit` and
  `high_limit`, a low limit above the high one, or a limit past 4194303, where
  one limit used to send a request for every device (#1483). ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- **Breaking (wire):** a Who-Has carrying only one of its two limits, or a
  low limit above the high one, is malformed: `WhoHasRequest::decode` refuses
  it and the server drops it unanswered, as it does such a Who-Is (#1483). ([5906270](https://github.com/jscott3201/rusty-bacnet/commit/5906270d083661dee9943b7c7302a5875533e1dd))

- **Breaking (wire, Rust API):** every object that reports intrinsically
  classes the event rows its table requires for that as required, so
  ReadPropertyMultiple with REQUIRED returns them and the PICS lists them as
  required; the rows the tables only permit stay optional (#1485). ([d63feb9](https://github.com/jscott3201/rusty-bacnet/commit/d63feb908613f0550d858a19bef70a25ebc67796))

- **Breaking (wire, Python API, Rust API):** an Accumulator serves Scale and
  Prescale in their context-tagged Clause 21 forms, Python reads them as a
  float or int and a `(multiplier, modulo_divide)` pair that
  `add_accumulator` now takes, and Rust reads them as `ApplicationData`
  (#1487). ([b352d84](https://github.com/jscott3201/rusty-bacnet/commit/b352d8488073935271ce2b62a9f21029b6c211f5))

- **Wire:** `BACnetRouter` neither forwards nor delivers a remote or global
  broadcast whose APDU isn't an Unconfirmed-Request, and `NetworkLayer` doesn't
  deliver one; both count it in the new `broadcast_pdu_type_drops()` (#1491). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Wire:** Linux Ethernet drops a frame whose source MAC is a group address
  before answering an XID or TEST command or decoding it, and counts it in the
  new `EthernetTransport::group_source_drops()` (#1492). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Wire:** Ethernet counts every multicast MAC, not only the all-ones
  broadcast, as a group destination, so a confirmed request to one is refused;
  `ethernet::is_group_mac` tells them apart (#1493). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Wire:** B/IP drops a Forwarded-NPDU whose originating address is a group
  address, so a forged I-Am can't bind a device to a group. B/IP and B/IPv6,
  which already dropped multicast origins, count such drops in the new
  `forwarded_group_origin_drops()` (#1493). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Wire:** The confirmed requests the server starts (events, Channel, Command,
  audit) never go to a group address such as a multicast one, and an I-Am from
  one binds nothing. A confirmed event recipient there counts in
  `confirmed_broadcast_recipient`; an unconfirmed one is still sent (#1493). ([df0f8b8](https://github.com/jscott3201/rusty-bacnet/commit/df0f8b878577ae71b68ce52ea860bf7be1deeb33))

- **Python API:** `PropertyValue.list` raises `ValueError` past 32 levels of
  nesting, pickles included, instead of crashing the process on a deep list,
  and `PropertyValue.real(0.0)` and `real(-0.0)`, equal already, now hash
  alike, as do `double` zeros (#1506). ([c4dc262](https://github.com/jscott3201/rusty-bacnet/commit/c4dc26269fdd21d184475bf3edb52e6df16b2be1))

- Analog and Binary Values carry independently optional Audit policy
  properties, set at creation in Rust and Python, which filter target Audit
  records.

## [0.11.0] - 2026-09-06

### Release highlights

This pre-1.0 release breaks Rust, Python and CLI APIs. Conformance remains
partial: the [conformance ledger](docs/conformance/bacnet-135-2020.json)
records the supported boundaries and the gaps, and this release is not a
full-conformance or BTL certification claim.

- Native BACnet/SC bounds its handshake, control sends, WebSocket resources
  and connection retirement, and validates heartbeats; WebSocket reader work
  continues in #525.
- Routed requests, peer APDU limits and segmented transactions follow the
  peer's identity and advertised capabilities.
- Intrinsic event history, exact BACnetTimeStamps, AcknowledgeAlarm
  correlation and ACK notifications keep committed transition state.
- Typed Rust and Python Audit clients join durable Audit Log storage, queries,
  notification receipt and duplicate detection; Audit Reporting BIBBs remain
  unclaimed.
- Python gains MS/TP configuration and local input Present_Value updates, File
  objects gain bounded resizing, logs gain exact ReadRange, and Staging gains
  explicit configuration.

### Migration notes

- Replace the stage-count `StagingObject::new` argument with a
  `StagingConfig`; see the [Rust example](docs/rust-api.md#building-control-7).
- Custom `AuditLogPersistence` implementations keep
  `AuditLogSnapshot::completed_receipts` in the same atomic commit as the
  records; the file backend writes schema v2 and reads v1 (see the
  [snapshot migration](docs/rust-api.md#audit-services)).
- CLI `ack-alarm` / `ack` need exact `--timestamp` and `--ack-time` values,
  and Python callers should use `acknowledge_alarm_request`, since the
  five-argument `acknowledge_alarm` is deprecated.
- GetEnrollmentSummary's `RecipientProcess::device` becomes
  `recipient: BACnetRecipient`, and the Audit request models and the raw
  Python `audit_log_query` signature changed; prefer the typed helpers.
- The Rust `AlertEnrollmentObject::new` and Python `add_alert_enrollment` take
  an explicit initial alert source (#291).
- A default `ServerConfig` advertises NO_SEGMENTATION and now refuses
  segmented traffic; set `segmentation_supported` to `BOTH` to accept it
  (#381).
- Drop uses of the withdrawn WASM crate, the Notification Forwarder and
  Channel constructors, and bundled-server WriteGroup execution.

### Added

- Application-owned `Present_Value` updates for in-service Analog, Binary and
  Multi-state Inputs through the server's `set_present_value_local`, in Rust
  and Python (#503, #510).
- Correlated AcknowledgeAlarm validation, lossless request construction
  (Python `acknowledge_alarm_request`), and ACK_NOTIFICATION delivery to the
  current Notification Class recipients (#132, #170, #175).
- Exact-delta Life Safety COV: Point and Zone subscriptions report only real
  Present_Value and Status_Flags changes (#177).
- Opt-in, application-owned execution of RESET, RESET_ALARM and RESET_FAULT on
  Life Safety Point and Zone objects (#177).
- Pre-start Python management of File objects: access method, payload copies
  and growth caps (#420).
- Bounded File_Size and Record_Count writes resize stream and record File
  objects (#417).
- The Event Log and both Trend Log object types give each record a stable
  sequence number and serve exact ReadRange By Position, By Sequence and By
  Time selection (#134, #339).
- BACnetLogStatus lifecycle records for the Event Log and both Trend Log
  object types (#189).
- Multi-state Input, Output and Value evaluate and recover their configuration
  Reliability, with a Rust `set_number_of_states` (#226).
- Optional `Reliability_Evaluation_Inhibit` on Analog, Binary and Multi-state
  Input, Output and Value objects (#232).
- Executable `PropertyMetadata` rows and the `BACnetObject::property_metadata`
  hook, starting with Time Value and Binary Input: the property-metadata core
  (#261).
- `bacnet_objects::file::FileStorage`, the channel AtomicReadFile and
  AtomicWriteFile use to reach a File object's contents, with growth caps for
  network writes (#397).
- `Time_Delay_Normal` on the nine intrinsic-reporting object types and on
  Event Enrollment (#225, #163).
- Enumeration parity with Clause 21: `LifeSafetyState` gains its tail, and new
  types such as `EscalatorOperationDirection`, `BinaryLightingPV`,
  `DoorValue`, `ProgramError` and `RestartReason` resolve by property (#253,
  #284).
- `BACnetClient::add_routed_device` registers a known routed peer, so requests
  follow its advertised limits (#372).
- Structured property values are writable: Multi-state Input Alarm_Values
  lists, DateTime Value Present_Value and Priority_Array entries, and
  Relinquish_Default on DateTime Value and DateTime Pattern Value (#182).
- Loop and Pulse Converter reference properties accept their Clause 21 wire
  forms through a strict `BACnetObjectPropertyReference` codec, and an empty
  setpoint reference clears the Loop's Setpoint_Reference (#182).
- Relinquish_Default is network-writable on the commandable object types, with
  a validated local `set_relinquish_default` (#270).
- Binary and Multi-state Input, Output and Value objects model
  Event_Time_Stamps and Event_Message_Texts (#235, #230, #258).
- `BackupAndRestoreState` and `Reliability` gain their missing named tail
  values (#246, #241).
- **Breaking (Rust API):** `FaultDetector` can no longer be built with a
  struct literal; use `FaultDetector::new(comm_timeout)`.
- **Rust API:** `BACnetObject::set_reliability_internal` is the trusted route
  for evaluated Reliability, separate from network writes; custom analog
  objects override it to keep fault detection working.
- Event_Detection_Enable on the seven other intrinsic-reporting types and
  Event Enrollment (default TRUE) and on Binary and Multi-state Output
  (default FALSE); writing FALSE holds the object at its initial event state,
  and the summary services skip it.
- `BACnetObject::is_writable_property`, `is_createable` and `is_deleteable`
  give PICS generation and dispatch one source of truth.
- `BACnetServer::write_local` is the server-owned local write, firing the same
  COV and event notifications as a network write.
- `BACnetObject::tick_intrinsic_reporting` and a 1-second server task advance
  Time_Delay countdowns.
- `BACnetObject::set_event_state_internal` persists an algorithmically derived
  Event_State, such as an Event Enrollment's.
- `ServerConfig::enable_event_enrollment` and `event_enrollment_interval_secs`
  configure Event Enrollment evaluation apart from fault detection.
- **Breaking (Rust API):** `evaluate_intrinsic_reporting` and
  `tick_intrinsic_reporting` return a `TransitionOutcome` that separates a
  transition from whether its notification may be sent.
- Binary Output and Multi-state Output gain Feedback_Value for the
  COMMAND_FAILURE algorithm.
- The event-property macros split into generic and analog halves, so Binary
  and Multi-state Outputs can be commissioned; Acked_Transitions stays
  read-only.
- `impl_intrinsic_reporting!` gains gated forms that take an
  Event_Detection_Enable field, and no ungated form remains.

### Changed

- **Breaking (CLI):** `ack-alarm` (and its `ack` alias) requires exact
  `--timestamp` and `--ack-time` values, and the five-argument Python
  `acknowledge_alarm` is deprecated with a warning.
- **Breaking (Rust API):** GetEnrollmentSummary's `RecipientProcess::device`
  becomes `recipient: BACnetRecipient`, and the service projects only
  object-owned event state with strict filters.
- **Breaking (wire):** WritePropertyMultiple follows ordered processing:
  writes before the first failure stay committed, and the error names the
  failed property (#242).
- `FileObject::set_data` and `set_records` update File_Size and Record_Count
  only for the selected access method (#397).
- The `bacnet-cli` argument definitions move into an `args` module to stay
  under the file-size cap (#213).
- The MSRV gate covers only the publishable crates, which lets `sysinfo` move
  to 0.39, and every other dependency is brought current; the MSRV stays 1.93.
- CI splits into a fast tier for PRs to dev and a heavy tier (cross-OS tests,
  MSRV, audit and deny) for main and tags.
- **Breaking (Python API):** `write_property_local` raises
  `BacnetProtocolError` for UNKNOWN_OBJECT and DUPLICATE_NAME instead of
  `RuntimeError`.
- The pinned development and CI toolchain moves to Rust 1.97.1; the MSRV stays
  1.93.

### Removed

- **Breaking:** the placeholder Notification Forwarder object and Python
  `add_notification_forwarder` are withdrawn (#188).
- **Breaking:** the placeholder Channel object, Python `add_channel` and
  inbound WriteGroup handling are withdrawn (#248).
- **Breaking (Rust API):** the uncalled `bacnet_server::intrinsic_reporting`
  module is removed (#237, #204).
- **Breaking:** the non-standard Alarm_Values and Fault_Values on Multi-state
  Output, and Fault_Values on Multi-state Input and Value with
  `set_fault_values`, are removed (#228).
- **Breaking (Rust API):** `bacnet_types::enums::LiftCarDoorStatus`, which
  matched no production, is removed (#245).
- **Breaking (Rust API):** the ungated three-ident arm of
  `impl_intrinsic_reporting!` is removed.
- The `bacnet-wasm` crate, its docs and its CI jobs are removed.
- The BACnet/SC WASM conformance evidence is withdrawn, narrowing those claims
  to native only.

### Fixed

- The deprecated GetAlarmSummary server selects only active ALARM candidates,
  and returns OPERATIONAL_PROBLEM instead of made-up defaults for unreadable
  fields (#172).
- GetEventInformation projects each event-initiating object as one strict
  snapshot, keeps all three BACnetTimeStamp choices, and takes priorities from
  one Notification Class (#128, #206).
- Binary Lighting Output treats WARN, WARN_OFF, WARN_RELINQUISH and STOP as
  operations rather than stored values, and its Relinquish_Default takes only
  OFF or ON (#283).
- In-service network writes to a Pulse Converter's Present_Value are refused;
  out-of-service simulation writes still work (#280).
- **Breaking (wire, Rust and Python API):** Alert Enrollment serves its
  bounded Table 12-61 property model, with a read-only Present_Value naming
  the latest alert source, and its constructors take an initial source (#291).
- Exact duplicate Confirmed-Request messages are discarded before decoding,
  under bounded, process-local tracking (#177).
- `bacnet file-read` decodes AtomicReadFile ACKs strictly and writes the
  payload instead of the raw ACK, and `atomic_read_file_decoded` validates the
  reply (#419).
- File payload changes stamp Modification_Date from the database clock and
  clear Archive (#416).
- Reliability evaluation belongs to each object through an opt-in hook; Analog
  Input and Value can opt into FAULT_OUT_OF_RANGE limits, and Min_Pres_Value
  and Max_Pres_Value no longer act as fault limits.
- **Breaking (Rust API):** NotificationParameters codecs are corrected for
  complex event values, AccessEvent credentials, the omitted choice and
  ChangeOfTimer fields, and `NoneParams` is gone (#351).
- Device `Active_COV_Subscriptions` reads return every field of each
  BACnetCOVSubscription (#183).
- Escalator Power_Mode, Operation_Direction, Escalator_Mode, Energy_Meter,
  Fault_Signals and Passenger_Alarm accept validated writes in and out of
  service (#401).
- `AtomicWriteFileRequest::decode` requires exactly Record Count records
  (#418).
- AtomicReadFile and AtomicWriteFile reach the File object's stored contents,
  with correct start, append and end-of-file handling (#397).
- AtomicReadFile and AtomicWriteFile refuse a non-File object with
  INCONSISTENT_OBJECT_TYPE, and AtomicWriteFile on a read-only File answers
  SERVICES / FILE_ACCESS_DENIED (#398, #399).
- Confirmed client requests stop the request timer once segment zero of a
  segmented ComplexACK arrives, and a receive segment timeout reclaims its
  state promptly (#379, #380).
- Devices configured for segmented transmit advertise Max_Segments_Accepted 1
  (#379).
- Server segmented transactions identify a routed peer by its SNET/SADR, so
  its segments may arrive through another router (#384).
- Segmented receivers use the corrected modulo-256 duplicate-in-window rule
  (#383).
- The client refuses locally to segment a request to a peer that advertises it
  can't receive segments (#371).
- DeviceTable no longer confuses a router with the routed peers behind it, and
  duplicate address matches pick the freshest row (#372).
- The Escalator stores Operation_Direction and Escalator_Mode as typed
  enumerations and accepts every named and proprietary value (#284, #400).
- The server enforces a File object's File_Access_Method for AtomicReadFile
  and AtomicWriteFile (#287).
- **Breaking (Rust and Python API):** Audit notification and AuditLogQuery
  wire models follow the Clause 21 productions, typed Python helpers decode
  queries, and Audit Log objects add durable storage, queries and authorized
  receipt (#345, #511, #458, #463, #466, #512, #535).
- **Breaking (Rust and Python API):** COV-multiple wire models match their
  productions, with mandatory booleans, cardinality checks and typed
  timestamps, and Python's `subscribe_cov_property_multiple` requires
  `issue_confirmed_notifications` (#342).
- **Breaking (Rust and Python API):** GetEnrollmentSummary uses its own Event
  State Filter values and full Unsigned Notification Class fields, and Python
  takes `EnrollmentSummaryEventStateFilter` (#358, #176).
- Test-only server dispatch constants compile only for tests (#359).
- **Rust API:** client-mode responders reject unhandled confirmed requests,
  B/IP and B/IPv6 read each datagram's real destination, and B/IPv6 VMAC
  handling follows Annex U; `ReceivedNpdu` and `ReceivedApdu` gain group
  fields (#374).
- Confirmed-Request `max-segments-accepted` rounds down to a valid rung
  instead of claiming more than 64 (#365).
- Confirmed event notifications reach remote-network recipients, correlating
  the routed acknowledgment and learning the router (#375).
- Server segmented request reassembly stops at 256 segments with a
  BUFFER_OVERFLOW Abort instead of wrapping (#364).
- A client reassembly session ends with its transaction (#367).
- A stale SegmentAck no longer kills a healthy segmented request, and negative
  acks continue after the last accepted segment (#368).
- **Breaking (wire):** the server honours its segmentation advertisement both
  ways, so a default server (NO_SEGMENTATION) refuses segmented traffic until
  `segmentation_supported` is set (#381, #377).
- Event notifications carry the network priority their event priority maps to
  (#187).
- Notifications to a remote-network unicast recipient go out as a routed local
  broadcast instead of being skipped (#186).
- A recipient spelled with the link's literal broadcast MAC resolves as a
  broadcast, through the new `TransportPort::is_broadcast_mac` (#360).
- Replies to a routed peer, such as SegmentACKs and Aborts, carry the
  request's source back as their destination (#366).
- Routed confirmed requests honour the routed peer's advertised Max APDU
  Length Accepted (#362).
- Alert Enrollment exposes Event_State and Acked_Transitions and resets them
  when Event_Detection_Enable goes FALSE (#205).
- Object snapshot hooks stay as an object-local surface, but
  WritePropertyMultiple now keeps successful prefix writes instead of
  restoring them (#209, #289).
- The Event Enrollment evaluator honours Time_Delay and Time_Delay_Normal
  (#163).
- Event Enrollment same-state transitions store the specific state and
  maintain Acked_Transitions, and CHANGE_OF_STATE re-indicates between alarm
  values (#166).
- Event Enrollment CHANGE_OF_VALUE compares against a detection baseline
  instead of the absolute value (#137).
- Event Enrollment algorithms recover from an Event_State the current
  algorithm can't reach, and CHANGE_OF_BITSTRING no longer reports OFFNORMAL
  on a prefix match.
- Event Enrollment alarms can be acknowledged, and a detection-disabled
  enrollment refuses with NO_ALARM_CONFIGURED.
- WriteProperty and WritePropertyMultiple decode the whole property value,
  refusing partial or empty payloads with INVALID_DATA_ENCODING (#182).
- Pulse Converter and Averaging PICS writability matches what their write
  paths accept (#182).
- Reliability write validation derives from `Reliability::ALL_NAMED`, so new
  named values are accepted without a second edit (#252).
- Indexed reads of the analog types' Event_Time_Stamps and Event_Message_Texts
  return one element (#235).
- The Device's Protocol_Services_Supported comes from the services the server
  actually executes (#192).
- **Breaking (wire):** bit strings of up to eight bits encode most significant
  bit first, which reverses Event_Enable, Acked_Transitions and the
  Recipient_List day and transition masks for peers still on the old bytes
  (#203).
- **Breaking (wire):** `FileAccessMethod`'s values were swapped; RECORD_ACCESS
  is now 0 and STREAM_ACCESS 1, so a stored raw File_Access_Method value needs
  remapping (#273).
- **Breaking (Rust API):** `DoorAlarmState::LOCK_FAULT` is renamed
  `LOCK_DOWN`, with its value unchanged (#274).
- **Breaking (Rust API):** the invented `StagingState` enumeration is removed;
  Present_Stage is an Unsigned index into Stages (#275).
- The Averaging Object_Property_Reference write is strict about its members
  and refuses references to other devices (#182).
- Loop and Schedule refuse in-service Reliability writes and validate
  out-of-service ones, and Trend Log refuses them entirely (#240).
- **Breaking (wire):** Notify_Type writes must name a defined value, and
  Event_Enable and Limit_Enable writes need their canonical encoding (#255).
- CHANGE_OF_STATE alarm parameters are reachable over BACnet: Binary Input and
  Value serve Alarm_Value, and Multi-state Alarm_Values is edited with the
  list services (#228).
- Binary Input, Binary Value, Multi-state Input and Multi-state Value expose
  writable Event_Enable, Notification_Class, Notify_Type and Time_Delay, so
  their notifications can be commissioned (#229).
- A refused Out_Of_Service write changes nothing.
- **Breaking (Rust API):** a change from one fault Reliability to another
  re-enters FAULT and notifies, and the detectors gain a public
  `fault_reliability` field (#217).
- The server's fault detector no longer overwrites the Reliability of an
  out-of-service analog object.
- Binary Output and Multi-state Output apply the COMMAND_FAILURE event
  algorithm, opt-in through Event_Detection_Enable (#222).
- **Breaking (Rust API):** a notification takes its Event Type from the
  algorithm the object runs, and `TransitionOutcome` gains `event_type`
  (#210).
- Every Event_State-to-transition-bit decision goes through one classifier,
  `EventTransition::for_target_state`, which fixes a latent inversion.
- **Breaking (Rust API):** Reliability drives Event_State, so FAULT and
  TO_FAULT transitions become reachable, and the detectors take a
  `reliability` argument (#167, #200).
- FAULT transitions report Event Type CHANGE_OF_RELIABILITY.
- PICS writable-property flags for the nine core input, output and value types
  mirror their write paths, and the createable and deleteable flags match what
  CreateObject and DeleteObject accept.
- Object_Name writes go through the name index, refusing duplicates with
  DUPLICATE_NAME and freeing the old name.
- Local writes fire the same COV and event notifications as network writes,
  through `BACnetServer::write_local`.
- A commandable Present_Value write in a WritePropertyMultiple prefix keeps
  its priority slot if a later write fails.
- The intrinsic detectors honour Time_Delay, splitting evaluation into a
  per-write `probe` and a periodic `tick`.
- Multi-state constructors refuse zero states.
- EventNotification takes each transition's Priority and Ack_Required from the
  referenced Notification Class (ASHRAE 135-2020 Clause 12.21).
- Notification Class recipient day bits and time windows follow one
  convention, with overnight windows and the device's local time (ASHRAE
  135-2020 Clause 12.21 / Clause 21.6).
- **Breaking (wire):** a Recipient_List address recipient keeps its network
  number (ASHRAE 135-2020 Clause 12.21 / Clause 21.6).
- **Breaking (Rust API):** Event Enrollment's Event_Parameters is a structured
  `BACnetEventParameter` (ASHRAE 135-2020 Clause 12.12 (Event_Parameters) /
  Clause 21.6 (BACnetEventParameter)) that the evaluator uses; older raw
  values still evaluate.
- Event Enrollment's Event_State is read-only over the network, with a
  separate internal lifecycle path.
- Event Enrollment evaluation has its own configuration, apart from the
  fault-detection switch.
- The release gate waits on every Tier 1 CI job, the file-size cap and MSRV
  included.
- Event_Enable gates only notification distribution, so a suppressed
  transition still updates Event_State.

## [0.10.1]

### Added

- `BACnetClient::device_events()` reports device discovery, updates and loss,
  alongside the `discovered_devices()` snapshot.
- Client builder setters for APDU retries, accepted segments,
  segmented-response acceptance and proposed window size.
- `BACnetClient` COV and COV-property subscribe and unsubscribe helpers that
  route through discovered devices, plus a managed finite subscription that
  renews before expiry.
- COV notifications carry source metadata and delivery kind, with a
  configurable confirmed-notification ACK policy.
- `ScTransport::connection_state_changes()` lets BACnet/SC consumers await
  link-state changes without polling.
- Typed BACnet/SC connect errors (`ScConnectError`) keep BVLC-Result NAK
  details and tell dial, TLS, handshake and subprotocol failures apart.
- `ScClientBuilder::device_uuid(...)` and the CLI `--sc-vmac` /
  `--sc-device-uuid` options provision a stable SC identity.
- `generate_random48_vmac()` and `is_valid_random48_vmac()` mint and validate
  Random-48 VMACs.

### Fixed

- `ScClientBuilder` and the SC CLI fail fast on missing or reserved SC
  identity instead of connecting with an all-zero VMAC.
- Negotiated SC hub Max-NPDU and BVLC limits reach
  `ScTransport::max_apdu_length()` and client segmentation, exposed by
  `transport_max_apdu_length()`.
- SC handshake timeouts return `Error::Timeout`, and malformed BVLC-Result
  failures typed `ScConnectError` values.
- SC reconnect, failover and primary restore re-dial the WebSocket instead of
  reusing a torn-down one.
- Dropping a client or SC transport aborts its receive tasks and closes its
  sockets, while graceful shutdown still sends Disconnect-Request.
- The device table purges stale entries on its own interval, without `Instant`
  underflow on fresh Windows runners.
- The server sends the initial COV notification after accepting a
  subscription.
- SubscribeCOVPropertyMultiple encodes its lifetime and max-delay fields in
  standard order, validates them, and expires server-side subscriptions.
- `bacnet-client` property method rustdoc describes the right methods.
- `ObjectIdentifier` documents wildcard instances and gains an
  addressable-object constructor that refuses the wildcard.
- The client's COV notification channel capacity is configurable, still 64 by
  default.

## [0.10.0]

### BACnet/SC - Connection Resilience (ASHRAE 135-2020 Annex AB.6.2, AB.6.3)

- A `NODE_DUPLICATE_VMAC` NAK reseeds a Random-48 VMAC and reconnects, and a
  hub lets a device with a known UUID reclaim its stale session.
- Failover follows mid-life reconnect exhaustion, and the primary hub is
  probed and restored afterwards.
- Heartbeats go out only on an idle link and must correlate to the outstanding
  request.
- Fatal NAKs, receive errors and reconnect exhaustion tear the transport down
  to `Disconnected`.

### BACnet/SC - Protocol Conformance (Annex AB.2, AB.3)

- BVLC-Result payloads are parsed, and malformed option chains rejected.
- SC data options are encoded, exposed as `ReceivedNpdu::data_attributes`, and
  unsupported must-understand options rejected.
- The hub checks the exact ConnectRequest length, enforces Max-BVLC-Length and
  stamps originating VMACs itself.
- TLS 1.3 is enforced, and `ErrorCode` gains constants 139 to 151,
  `NODE_DUPLICATE_VMAC` among them.

### BACnet/IP - Annex J

- The BIP socket binds `INADDR_ANY` so subnet and limited broadcasts arrive on
  Linux.
- Forwarded-NPDU rebroadcast loops and Original-Broadcast echoes are
  suppressed.
- BBMD forwarding failures NAK, FDT entries expire on a timer, the BDT
  persists across restart, and management ACKs and ACLs are corrected.

### WASM (browser bindings)

- BACnet/SC heartbeats, disconnect handling and data attributes in the WASM
  layer.

### Server

- `BACnetServer::i_am_broadcaster()` / `broadcast_i_am()` and
  `BipServerBuilder::vendor_id(u16)` for host-driven I-Am.

### Dependencies

- `pyo3` 0.29, resolving audit advisories.

## [0.9.0]

### Spec Compliance - Codec Strictness (ASHRAE 135-2020 Clauses 20.1.2.7, 20.1.2.8, 20.1.6.x)

- Codecs validate ConfirmedRequest max-APDU and SegmentACK window fields,
  refuse BVLL lengths past 16 bits, and reject wrong-length primitives and
  trailing bytes (Findings 4, 6, 7).

### Spec Compliance - Confirmed Notification TSM (ASHRAE 135-2020 Clause 5)

- Confirmed EventNotifications use the server TSM with timeouts and retries,
  keyed by peer and invoke ID (Finding 8).

### Spec Compliance - Segmentation (ASHRAE 135-2020 Clauses 5, 20.1.2.4, 20.1.2.5, 20.1.6.x)

- `split_payload` refuses more than 256 segments, and segmented ComplexACKs
  stay within the segment count the client accepts (Findings 1, 2).
- Server segmented receive validates the window, ACKs at window boundaries and
  NAKs gaps, and NAK retransmission resumes at the next segment (Findings 3,
  5).
- Routed confirmed requests segment when they exceed the local APDU limit,
  with responses matched by their routed key (Finding 9).

### Performance

- Notification sends freeze their payload buffers instead of copying them
  (Finding 10).

### Changed

- **API break:** APDU, BVLL and BVLC encoders and `split_payload` return
  `Result`.
- **Behavior change:** primitive decoders reject malformed encodings with
  trailing bytes.

### Engineering — CI guardrails

- Workspace lints are centralized (`unsafe_code` denied outside documented FFI
  sites), the toolchain is pinned, and CI adds `cargo audit`, a secret scan,
  the file-size cap and `--locked`.

### Engineering — Modularity (700 LOC cap)

- 34 source files split into modules so every file is under the 700-line cap,
  now enforced strictly, with import paths unchanged.

### Workspace reorganization

- **`bacnet-gateway`** moved to
  [`jscott3201/rusty-bacnet-mcp`](https://github.com/jscott3201/rusty-bacnet-mcp)
  and **`bacnet-btl`** to
  [`jscott3201/rusty-bacnet-btl-harness`](https://github.com/jscott3201/rusty-bacnet-btl-harness),
  leaving this workspace to the protocol stack.

### Removed (from this workspace)

- The `bacnet-btl` and `bacnet-gateway` crates, their docs and the BTL Docker
  assets.

### Notes

- Library APIs changed from 0.8.1 where noted above; the Python and WASM
  bindings and the CLI are unchanged.

## [0.8.1]

### Security

- `rustls-webpki` 0.103.13 and `rand` 0.10.1 and 0.9.4 address
  [RUSTSEC](https://rustsec.org/) advisories, including
  [RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097).

### Fixed

- **bacnet-gateway** uses the configured BIP port for its client transport
  instead of an ephemeral one.
- **bacnet-client** gates its `Ipv6Addr` import behind `ipv6`, and
  **bacnet-gateway** drops an unused import.

### Documentation

- **Benchmarks.md** is refreshed with a clean run of all nine Criterion
  suites.

## [0.8.0]

### Spec Compliance — BBMD & Router (ASHRAE 135-2020 Annex J, Clause 6)

A review of the BBMD and router found 22 spec compliance issues, all fixed.

#### Router — Congestion & Reachability (Clause 6.6.3)

- Routers refuse traffic to busy (reason 2) or unreachable (reason 1)
  networks, Router-Busy clears after 30 s, busy and available messages are
  re-broadcast, and unknown messages are rejected (reason 3).
- `RouterTable` gains busy, available and unreachable marking with timestamped
  auto-clear.

#### Router — Route Management (Clause 6.6.3.2/3)

- I-Am-Router-To-Network is always re-broadcast, route updates from other
  ports are accepted with flap detection, and active routes are refreshed on
  use.

#### Router — Network Messages (Clause 6.4)

- Initialize-Routing-Table-Ack and Network-Number-Is get handlers, and
  security messages are no longer rejected.

#### BBMD (Annex J)

- The BBMD includes itself in its BDT, non-BBMD Forwarded-NPDUs use the
  originating address, Distribute-Broadcast-To-Network and short registrations
  NAK, and the BDT can persist to a file.

#### TSM (Clause 5.4)

- A drop guard frees the invoke ID when a confirmed request's task is
  cancelled.

### Spec Compliance — Transport Layer (ASHRAE 135-2020 Clauses 7-9, Annexes J, U, AB)

A review of all five transports found 34 spec compliance issues: 31 fixed and 3 deferred.

#### MS/TP — State Machine (Clause 9.5)

- MS/TP uses the right token-loss timeout and per-station offset, finishes on
  a reply, validates the reply source, handles ReplyPostponed, and discards
  frames after an inter-byte gap.

#### BACnet/IPv6 — VMAC & Address Resolution (Annex U)

- Virtual-Address-Resolution frames have the right sizes, unicast uses a
  learned VMAC table, and B/IPv6 adds address resolution, a configurable
  multicast scope and foreign-device registration.

#### BACnet/SC — Client (Annex AB)

- Heartbeat-ACKs omit VMACs, BVLC-Results parse their result code,
  Connect-Accept message IDs are checked, `stop()` sends Disconnect-Request,
  and the hub's Device UUID is stored.

#### BACnet/SC — Hub (Annex AB)

- The hub requires a full ConnectRequest, NAKs early and unknown messages,
  relays broadcasts in parallel, enforces each client's max NPDU, and probes
  idle clients with heartbeats.

#### BACnet/SC — TLS (Annex AB.7.4)

- Every TLS client configuration requires TLS 1.3.

#### Ethernet — LLC Commands (Clause 7.1)

- XID and TEST commands are handled, the BPF filter admits them, and transient
  receive errors no longer stop the loop.

#### BIP — Foreign Device (Annex J)

- Foreign-device registration and broadcast NAKs are logged as errors.

#### Cross-Cutting Transport Improvements

- A second `start()` returns an error instead of leaking, "not started" is
  `Error::Transport(NotConnected)` everywhere, receive loops no longer stall
  on slow consumers, `stop()` clears state, and `bip6` sits behind the `ipv6`
  feature.

### Spec Compliance — Stack-Wide (ASHRAE 135-2020 Clauses 5, 6, 12, 13, 15, 16, 20)

A review of the other layers found 43 spec compliance issues; every critical, high and medium one is fixed.

#### Encoding & APDU (Clause 20)

- SegmentAck window sizes clamp to 1 to 127, and reserved max-APDU values log
  a warning.

#### Types & Enums (Clause 21)

- LifeSafetyOperation values are reordered, and LifeSafetyMode OEO values, a
  DaysOfWeek type and 11 BACnetPropertyStates variants are added.

#### Services (Clauses 13-16)

- TextMessage tags and the ReinitializeDevice password size are corrected,
  EventNotification gains `message_text`, and GetEnrollmentSummary gains its
  enrollment filter.

#### Objects (Clause 12)

- The nine event-capable types compute IN_ALARM, Value_Source tracking fields
  are added, and the trait gains `set_overridden()`.

#### Client (Clause 5.4)

- Segmented response reassembly acknowledges per window, handles duplicates
  and NAKs correctly and aborts when segmented responses aren't accepted, and
  the device table purges stale entries.

#### Server

- COV `ack_required`, DCC DISABLE, COV-property cancellation, RPM device
  wildcards, GetEnrollmentSummary priorities, Event_Enable reads and schedule
  times are corrected.

#### Network (Clause 6)

- Routers deliver remote broadcasts locally, pass network messages through,
  forward proprietary messages and report the real port in
  Init-Routing-Table-Ack.

### Python Bindings Improvements

- Rewritten type stubs, time synchronization, directed Who-Is, auto-routing
  reads and writes, `add_device()`, `discover()`, more `PropertyValue`
  constructors, structured error attributes, and DCC and reinitialize
  passwords.

### Added

- **New crate: `bacnet-gateway`**, an HTTP REST and MCP server for BACnet
  networks.
- `LoopbackTransport` for in-process composition, RS-485 GPIO direction
  control, concurrent multi-device client batches, client auto-routing and
  concurrent server dispatch.
- Architecture documentation and expanded API guides.

### Changed

- Dependencies are updated, resolving aws-lc-sys and rustls-webpki security
  advisories.

### Removed

- The Java/Kotlin bindings and their CI jobs.

## [0.7.2]

### Added

- **New crate: `bacnet-gateway`**, an HTTP REST API and MCP server with
  discovery, property access, local object management, a BACnet knowledge base
  and bearer-token authentication.
- `LoopbackTransport` and `AnyTransport::Loopback` for in-process composition.

## [0.7.1]

### Fixed

- The maturin wheel build works again without the invalid `python-source`
  setting.

## [0.7.0]

### Spec Compliance (ASHRAE 135-2020)

A seven-area compliance review brought more than 55 fixes across the stack.

#### BACnet/SC (Annex AB)

- SC control flags, Connect payloads (now with the Device UUID),
  control-message VMACs, BVLC-Result NAKs, header options and the broadcast
  VMAC follow Annex AB; the hub rewrites unicast VMACs, non-binary WebSocket
  frames close with 1003, and reconnects back off from 10 to 600 s.

#### BACnet/IPv6 (Annex U)

- B/IPv6 function and result codes, the Original-Unicast and Forwarded-NPDU
  headers and the FDT grace period are corrected, and the receive buffer grows
  to 2048 bytes.

#### Network Layer (Clause 6)

- I-Am-Router-To-Network is broadcast, final-hop delivery strips DNET, invalid
  SNETs and misaddressed messages are refused, and reject reasons are
  corrected.
- Routers re-broadcast I-Am-Router, forward Who-Is-Router, track reachability
  through Router-Busy and Router-Available, relay rejects and answer table
  queries.

#### Object Model (Clause 12)

- Property_List leaves out the four standard properties, Status_Flags is
  computed, and Object_Name is writable everywhere.
- Device gains Device_Address_Binding and Max_Segments_Accepted, commandable
  objects gain Current_Command_Priority and Value_Source tracking, and new
  detectors and event properties cover the binary, multi-state and analog
  objects.

#### Services (Clauses 13-16)

- SubscribeCOV lifetime 0 means indefinite and requires COV support,
  TextMessage, AcknowledgeAlarm, DCC, ReadRange and WriteGroup follow their
  clauses, RPM reports per-property encode errors, and COV subscriptions are
  keyed by property.

#### MS/TP (Clause 9)

- MS/TP fixes T_slot, initialization, token passing, Poll-For-Master, reply
  timeouts and NO_TOKEN arbitration, and adds EventCount and T_turnaround.

#### APDU Encoding (Clauses 5, 20)

- Window sizes clamp to 1 to 127, 256 segments are allowed, character set
  names are corrected, and the TSM gains an APDU_Segment_Timeout.

### BTL Compliance Test Harness

#### Test Harness

- **New crate `bacnet-btl`**: a BTL Test Plan 26.1 harness with 3808 tests
  across 13 sections, self-test, external-device and serve modes over BIP or
  SC, and Docker topologies.

#### Stack Compliance Fixes Found by BTL Tests (~40 fixes)

- Missing Device, Schedule, Lighting, Staging and event properties, wildcard
  Device reads, COV support on 11 more object types, commandable pattern
  values, and the new Color and Color Temperature objects.

### Code Review Fixes

#### Critical

- A segmented-send panic, silent u16 and u32 truncation in encoders, and a
  server dispatch `expect()` are fixed.

#### Security

- The I-Am-Router broadcast loop, routing-table bounds and reserved networks,
  BDT size panics, and SC hub pre-handshake limits and reserved VMACs are
  handled.

#### Concurrency

- TLS WebSocket lock ordering is fixed, SC hub broadcast relay is bounded, and
  COV delivery uses oneshot channels instead of polling.

#### Correctness

- COV subscription keys, DeleteObject COV cleanup, event invoke IDs,
  day-of-week numbering and SubscribeCOVProperty content are corrected, with
  added bounds checks and overflow guards.

### New Server Handlers

- GetAlarmSummary, GetEnrollmentSummary, Confirmed and Unconfirmed
  TextMessage, LifeSafetyOperation, WriteGroup and
  SubscribeCOVPropertyMultiple handlers, wired into dispatch.

### Python Bindings

- `rusty_bacnet.pyi` type stubs and a `py.typed` marker for IDEs and type
  checkers.

### Other

- Trend Log polling state moves into the server, LoopbackSerial buffers excess
  bytes, and Init-Routing-Table ACKs encode only `count` entries.

## [0.6.4]

### Changed

- BIP and confirmed-request internals are refactored, BBMD state is created at
  `start()` with the bound address, and `register_foreign_device` is renamed
  `register_foreign_device_bvlc`.

### Added

- `bvlc_request` refuses concurrent management requests, and the
  Forwarded-NPDU source MAC difference between BBMD and foreign-device modes
  is documented.

## [0.6.3]

### Added

- Interactive shell session state (`target`, `status`), BBMD auto-renewal, the
  remaining commands (`ack-alarm`, `time-sync`, `create-object`,
  `delete-object`, `read-range`), `discover --bbmd`, colored output and
  discovery progress.

### Changed

- Shell output helpers are deduplicated, dead code removed, and the BIP shell
  separated for BBMD commands.

### Fixed

- Repeated interactive `discover --target` finds devices again, and an unused
  import is gone.

## [0.6.2]

### Added

- CLI `discover --target`, `--bbmd` and `--dnet`, backed by
  `who_is_directed()`, `who_is_network()` and
  `NetworkLayer::broadcast_to_network()`, also in the shell.

### Fixed

- Backspace in the interactive shell deletes characters visually.

## [0.6.1]

### Added

- `bacnet capture` for live packet capture and pcap analysis with a BACnet
  frame decoder, behind the `pcap` feature; pre-built CLI binaries for Linux,
  macOS and Windows; and `docs/CLI.md`.

### Fixed

- Stale `nicegates` org references are replaced with `jscott3201`.

## [0.6.0]

### Added

- BBMD client API on `BipTransport` and `BACnetClient` (`read_bdt`,
  `write_bdt`, `read_fdt`, `delete_fdt_entry`, `register_foreign_device`),
  with working CLI `bdt`, `fdt`, `register` and `unregister` commands and
  table or JSON output.
- BACnet/IPv6 CLI support (`--ipv6`, `--ipv6-interface`, `--device-instance`,
  bracketed targets) and `Bip6ClientBuilder`.

### Changed

- BBMD management commands are limited to the BIP transport.

## [0.5.5]

### Fixed

- The Java/Kotlin Gradle publish URL points at the right GitHub org.

## [0.5.4]

### Fixed

- The WASM npm package has the `repository` field npm provenance needs.

## [0.5.3]

### Fixed

- Clippy's `io_other_error` in the Ethernet transport and `inherent_to_string`
  in the WASM bindings (Rust 1.93).

### Changed

- CHANGELOG entries for patch releases unblock the release pipeline.

## [0.5.2]

### Fixed

- Clippy's `io_other_error`: the Ethernet transport uses
  `std::io::Error::other()`.

## [0.5.1]

### Added

- Kotlin examples and a JMH benchmark suite, with results in Benchmarks.md.

### Fixed

- `Display` for `JsObjectIdentifier`, UniFFI's Tokio runtime on async blocks,
  and the benchmarks crate version.

### Changed

- CLAUDE.md covers the Java/Kotlin build.

## [0.5.0]

### Added

- **Java/Kotlin bindings:** the `bacnet-java` crate through UniFFI 0.31, with
  an async client, server object builders and COV streams, published as a
  multi-platform JAR to GitHub Packages.
- CI builds the native libraries on five platforms and publishes the JAR on
  release tags.

## [0.4.0]

### Added

- **WASM/JavaScript support:** the `bacnet-wasm` crate, a BACnet/SC thin
  client for browsers with service codecs and TypeScript definitions.
- CI checks the WASM build and publishes the npm package on release tags.

## [0.3.0]

### Fixed

- CI builds aarch64 Linux wheels on a native ARM64 runner instead of
  cross-compiling.

## [0.2.0]

### Changed

- Client-server integration tests move to `bacnet-integration-tests`, breaking
  the circular dev-dependency that blocked publishing.

### Fixed

- CI builds aarch64 Linux wheels under QEMU without sccache, and publishes
  crates in a simpler order.

## [0.1.5]

### Fixed

- CI publishes the server and client in the right order and builds aarch64
  Linux wheels.

## [0.1.4]

### Fixed

- Every workspace dependency declaration carries a version, so the crates
  publish.

## [0.1.3]

### Fixed

- Cargo Deny allows MPL-2.0 for `serialport`, and an Ethernet test imports
  `TransportPort`.

## [0.1.0]

Initial release of the Rusty BACnet protocol stack implementing ASHRAE 135-2020.

### Protocol Stack

- Application, network and transport layers with full BACnet encoding, two-way
  APDU segmentation and NPDU routing with hop counts.

### Transports

- BACnet/IP with BBMD, BACnet/IPv6, BACnet/SC with its hub, MS/TP and Ethernet
  (the last two Linux only).

### Services (24 modules)

- Property access, object management, discovery, COV, file access, alarm and
  event, device management and list operations, plus PrivateTransfer,
  ReadRange, TextMessage, VirtualTerminal, WriteGroup, LifeSafety and Audit.

### Object Types (62)

- The standard object types, from the analog, binary and multi-state families
  through access control, elevator, life safety, lighting, scheduling,
  trending and audit to the twelve value types.

### Client

- An async Tokio client with a transaction state machine, 31 service methods,
  segmentation, discovery with a device cache, retries, and `bip_builder()`,
  `sc_builder()` and `generic_builder()`.

### Server

- An async server dispatching 17 services over 62 object types, with COV,
  intrinsic reporting, event enrollment, fault detection, schedules, trend
  logging, DCC and PICS generation.

### Network Layer

- Routing with RouterTable, a multi-port BACnetRouter, priority queuing and
  hop-count loop prevention.

### Python Bindings (PyO3)

- An async API with `BACnetClient`, `BACnetServer` and `PyScHub`, typed enums,
  COV iteration and BIP, IPv6 and SC transports, published to PyPI as
  `rusty-bacnet`.

### Testing & Quality

- 1,682 tests, CI on Linux, macOS and Windows, zero-warning Clippy and
  `cargo-deny`.

### Benchmarks

- Nine Criterion suites, four Python mixed-mode benchmarks and a Docker
  benchmark environment.

### Examples

- Rust and Python examples for BIP, IPv6, SC, COV and device management, plus
  a Docker Compose deployment.
