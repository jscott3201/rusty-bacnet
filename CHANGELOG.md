# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.2]

### Added
- **New crate: `bacnet-gateway`** — HTTP REST API and MCP (Model Context Protocol) server for BACnet networks
  - REST API at `/api/v1/` with endpoints for device discovery, property read/write, local object CRUD, and health check
  - MCP server at `/mcp` with 10 tools for LLM-driven BACnet interaction (discover_devices, read/write_property, list/read/write/create/delete local objects)
  - MCP reference knowledge base — 9 static resources teaching BACnet concepts (object types, properties, units, errors, reliability, priority array, networking, services, troubleshooting) plus per-object-type drill-down templates
  - 3 live state MCP resources (devices, local-objects, config)
  - Pluggable authentication with bearer token default, applied to both REST and MCP endpoints
  - TOML configuration with CLI overrides, config validation (reserved network numbers, mutual exclusivity checks)
  - Feature-gated binary (`--features bin`) with graceful shutdown, tracing, `--no-mcp`/`--no-api` flags
  - 13 supported object types for local creation (analog/binary/multi-state I/O/V, integer, large-analog, positive-integer, characterstring values)
- **`LoopbackTransport`** in `bacnet-transport` — in-process transport backed by mpsc channels for gateway client/server composition
- **`AnyTransport::Loopback`** variant for mixed-transport routing with loopback ports

## [0.7.1]

### Fixed
- **Fixed** maturin wheel build — removed invalid `python-source` setting from pyproject.toml that broke wheel builds for pure Rust extension module

## [0.7.0]

### Spec Compliance (ASHRAE 135-2020)

Comprehensive 7-area compliance review and 55+ fixes across the entire protocol stack.

#### BACnet/SC (Annex AB)
- **Fixed** control flag bit positions — was using bits 7-4 instead of spec's bits 3-0
- **Fixed** ConnectRequest/ConnectAccept payload — added 16-byte Device UUID (now 26 bytes per AB.2.10.1)
- **Fixed** removed VMACs from ConnectRequest, ConnectAccept, DisconnectRequest, DisconnectAck, HeartbeatRequest, HeartbeatAck (spec says 0-octets)
- **Fixed** BVLC-Result NAK format — added result_code byte and error header marker (7+ bytes per AB.2.4.1)
- **Fixed** hub relay — now rewrites Originating Virtual Address and strips Destination Virtual Address for unicast (AB.5.3.2/3)
- **Fixed** header option encoding — proper Must Understand (bit 6) and Header Data Flag (bit 5) handling per AB.2.3
- **Fixed** broadcast VMAC — removed all-zeros as broadcast (X'000000000000' is reserved/unknown per AB.1.5.2)
- **Fixed** non-binary WebSocket frames — now closed with status 1003 per AB.7.5.3
- **Fixed** reconnect minimum delay — 10s min, 600s max per AB.6.1

#### BACnet/IPv6 (Annex U)
- **Fixed** Bvlc6Function codes — 0x0B removed per Table U-1, 0x0C = Distribute-Broadcast-To-Network
- **Fixed** Bvlc6ResultCode values — corrected from sequential 0x10 increments to spec values (0x0060, 0x0090, 0x00A0, 0x00C0)
- **Fixed** Original-Unicast-NPDU — added 3-byte Destination-Virtual-Address (10-byte header per U.2.2.1)
- **Fixed** Forwarded-NPDU — added 18-byte Original-Source-B/IPv6-Address (25-byte header per U.2.9.1)
- **Fixed** FDT seconds_remaining — now includes 30-second grace period per J.5.2.3
- Increased BIP6 recv buffer from 1536 to 2048 bytes

#### Network Layer (Clause 6)
- **Fixed** I-Am-Router-To-Network — now sent as broadcast per Clause 6.4.2 (was unicast)
- **Fixed** router final-hop delivery — strips DNET/DADR/HopCount per Clause 6.5.4
- **Fixed** SNET=0xFFFF rejected on decode per Clause 6.2.2.1
- **Fixed** non-router now discards DNET-addressed messages per Clause 6.5.2.1
- **Fixed** reject reason — uses NOT_DIRECTLY_CONNECTED (1) instead of OTHER (0) per Clause 6.6.3.5
- **Fixed** What-Is-Network-Number ignores routed messages per Clause 6.4.19
- **Added** I-Am-Router re-broadcast to other ports per Clause 6.6.3.3 (with loop prevention)
- **Added** Who-Is-Router forwarding for unknown networks per Clause 6.6.3.2
- **Added** SNET/DNET validation at encode time
- **Added** reserved network numbers (0, 0xFFFF) rejected in routing table
- **Added** reachability status (Reachable/Busy/Unreachable) to RouteEntry per Clause 6.6.1
- **Added** Router-Busy/Router-Available messages update reachability status per Clause 6.6.4
- **Added** Reject-Message-To-Network relay to originating node per Clause 6.6.3.5
- **Added** Init-Routing-Table count=0 query returns full table without updating per Clause 6.4.7

#### Object Model (Clause 12)
- **Fixed** Property_List — excludes Object_Identifier, Object_Name, Object_Type, Property_List per Clause 12.1.1.4.1
- **Fixed** StatusFlags — now dynamically computed from event_state, reliability, out_of_service
- **Fixed** Object_Name — now writable on all object types per Clause 12.1.1.2
- **Added** Device_Address_Binding to Device object (required per Table 12-13)
- **Added** Max_Segments_Accepted to Device object (required when segmentation supported)
- **Added** Current_Command_Priority to all commandable objects (AO, BO, MSO, AV, BV, MSV)
- **Added** ChangeOfStateDetector for binary and multi-state objects (Clause 13.3.1)
- **Added** CommandFailureDetector for commandable output objects (Clause 13.3.3)
- **Added** Event_Time_Stamps and Event_Message_Texts to analog objects
- **Added** Alarm_Values and Fault_Values to multi-state objects
- **Added** ValueSourceTracking (Value_Source, Last_Command_Time) to commandable objects

#### Services (Clauses 13-16)
- **Fixed** SubscribeCOV lifetime=0 — now means indefinite per Clause 13.14.1.1.4 (was immediate expiry)
- **Fixed** TextMessage messageClass — uses constructed encoding (opening/closing tag) per Clause 16.5
- **Fixed** AcknowledgeAlarm — added time_of_acknowledgment parameter (tag [5]) per Table 13-9
- **Fixed** DCC DISABLE (value 1) — rejected per 2020 spec Clause 16.1.1.3.1 (deprecated)
- **Fixed** DCC password length — validated ≤ 20 characters per Clause 16.1.1.1.3
- **Fixed** SubscribeCOV — verifies object supports COV per Clause 13.14.1.3.1
- **Fixed** ReadRange count=0 — rejected per Clause 15.8.1.1.4.1.2
- **Fixed** ReadRange ByPosition — returns empty result for out-of-range indices per Clause 15.8.1.1.4.1.1
- **Fixed** WriteGroup — group_number=0 rejected per Clause 15.11.1.1.1
- **Fixed** RPM — encode failure produces per-property error instead of aborting response
- **Fixed** GetEventInformation — reads actual event timestamps when available
- **Fixed** COV subscription key — includes monitored_property (per-property and whole-object subs coexist)

#### MS/TP (Clause 9)
- **Fixed** T_slot — fixed to 10ms per Clause 9.5.3 (was incorrectly computed from baud rate)
- **Fixed** INITIALIZE state — NS=TS, PS=TS, TokenCount=N_poll per Clause 9.5.6.1
- **Fixed** ReceivedToken — clears SoleMaster per Clause 9.5.6.2
- **Added** PassToken state with retry/FindNewSuccessor per Clause 9.5.6.6
- **Added** DONE_WITH_TOKEN proper logic (sole master, maintenance PFM, NextStationUnknown)
- **Fixed** WaitForReply timeout — transitions to DoneWithToken per Clause 9.5.6.4
- **Added** NO_TOKEN T_slot*TS priority arbitration per Clause 9.5.6.7
- **Fixed** PollForMaster ReceivedReplyToPFM — sends Token to NS, enters PassToken per Clause 9.5.6.8
- **Added** EventCount tracking per Clause 9.5.2
- **Added** T_turnaround enforcement per Clause 9.5.5.1

#### APDU Encoding (Clauses 5, 20)
- **Fixed** window size — clamped to 1-127 on encode per Clauses 20.1.2.8, 20.1.5.5, 20.1.6.5
- **Fixed** 256-segment edge case — now allows 256 segments (sequence 0-255) per Clause 20.1.2.7
- **Fixed** character set names — IBM_MICROSOFT_DBCS (was JIS_X0201), JIS_X_0208 (was JIS_C6226) per Clause 20.2.9
- **Added** separate APDU_Segment_Timeout field in TSM config per Clause 5.4.1

### BTL Compliance Test Harness

New `bacnet-btl` crate — a full BTL Test Plan 26.1 compliance test harness with 3808 tests across all 13 BTL sections, 100% coverage of all BTL test references.

#### Test Harness
- **New crate** `bacnet-btl` with `bacnet-test` binary — self-test, external IUT testing, interactive shell
- **3808 tests** organized across 13 BTL sections (s02–s14), one directory per section
- **`self-test` command** — in-process server with all 64 object types, runs full suite in <1s
- **`run` command** — tests against external BACnet device over BIP or BACnet/SC
- **`serve` command** — runs the full BTL object database as a standalone server (BIP or SC)
- **SC client/server support** — feature-gated behind `sc-tls`, includes self-signed cert generation
- **Docker support** — `Dockerfile.btl` and `docker-compose.btl.yml` with SC hub + BIP + routing topologies
- **RPM/WPM test helpers** — `read_property_multiple`, `rpm_all`, `rpm_required`, `rpm_optional`, `write_property_multiple`, `wpm_single` on TestContext

#### Stack Compliance Fixes Found by BTL Tests (~40 fixes)
- **Added** EVENT_STATE to AccessDoor, LoadControl, Timer, AlertEnrollment objects
- **Added** Device properties: LOCAL_DATE, LOCAL_TIME, UTC_OFFSET, LAST_RESTART_REASON, DEVICE_UUID
- **Added** Schedule PRIORITY_FOR_WRITING property
- **Added** Device wildcard instance (4194303) support in ReadProperty/ReadPropertyMultiple handlers
- **Added** PROPERTY_IS_NOT_AN_ARRAY error in ReadProperty handler
- **Added** AccessDoor full command prioritization (priority array write, NULL relinquish)
- **Added** `supports_cov() = true` on 11 additional object types (LifeSafetyPoint, LifeSafetyZone, AccessDoor, Loop, Accumulator, PulseConverter, LightingOutput, BinaryLightingOutput, Staging, Color, ColorTemperature)
- **Fixed** AccumulatorObject `supports_cov()` was on wrong impl block (PulseConverterObject)
- **Added** EVENT_ENABLE, ACKED_TRANSITIONS, NOTIFICATION_CLASS, EVENT_TIME_STAMPS to Binary and Multistate objects
- **Added** EVENT_ENABLE, NOTIFICATION_CLASS to Multistate Input/Output/Value objects
- **Changed** DatePatternValue, TimePatternValue, DateTimePatternValue from `define_value_object_simple!` to `define_value_object_commandable!` (per BTL spec, all value types are commandable)
- **Added** LightingOutput DEFAULT_FADE_TIME property
- **Added** Staging PRESENT_STAGE, STAGES properties
- **Added** NotificationForwarder RECIPIENT_LIST, PROCESS_IDENTIFIER_FILTER properties
- **Added** Lift FLOOR_NUMBER property
- **New** Color object (type 63) — full implementation with CIE 1931 xy coordinates
- **New** ColorTemperature object (type 64) — full implementation with Kelvin value
- **Added** Device dynamic Protocol_Object_Types_Supported bitstring calculation (auto-detects all object types in database)

### Code Review Fixes

#### Critical
- **Fixed** client segmented send panic — validates SegmentAck sequence_number bounds (was unchecked index)
- **Fixed** silent u16 truncation in BVLL, BVLC6, and SC option encode functions (added overflow checks)
- **Fixed** silent u32 truncation in primitives encode functions (octet_string, bit_string)
- **Fixed** server dispatch `expect()` — replaced with graceful error handling (prevented server crash)

#### Security
- **Fixed** I-Am-Router broadcast loop — only re-broadcasts newly learned routes
- **Fixed** Init-Routing-Table — enforces MAX_LEARNED_ROUTES cap, validates info_len bounds
- **Fixed** routing table — rejects reserved network numbers (0, 0xFFFF), add_learned won't overwrite direct routes
- **Added** SC Hub pre-handshake connection limit (512 max) to prevent DoS
- **Added** SC Hub rejects reserved VMACs (unknown/broadcast) on ConnectRequest
- **Fixed** BDT size validation — returns error instead of panicking

#### Concurrency
- **Fixed** TLS WebSocket lock ordering — drops read lock before acquiring write lock in recv()
- **Fixed** SC Hub broadcast relay — sequential sends with per-client timeout (was unbounded task spawning)
- **Fixed** COV polling — replaced 50ms polling loop with oneshot channels for instant delivery

#### Correctness
- **Fixed** COV subscription key — includes monitored_property (per-property subs no longer overwrite whole-object subs)
- **Fixed** DeleteObject — now cleans up COV subscriptions for deleted objects
- **Fixed** event notification invoke_id — uses ServerTsm allocation (was hardcoded 0)
- **Fixed** day-of-week calculation — consistent 0=Monday convention across schedule.rs and server.rs
- **Fixed** COV notification content — sends only monitored property for SubscribeCOVProperty subscriptions
- **Added** route.port_index bounds check before indexing send_txs
- **Added** duplicate port network number detection at router startup
- **Added** checked_add in decode_error and decode_timestamp offset arithmetic
- **Added** ObjectIdentifier debug_assert on encode for type/instance overflow
- **Added** is_finite debug_assert in analog set_present_value
- **Added** transition_bit mask (& 0x07) in acknowledge_alarm
- **Added** messageText skip loop iteration limit

### New Server Handlers

- **Added** GetAlarmSummary handler — iterates objects, returns those with event_state != NORMAL
- **Added** GetEnrollmentSummary handler — with filtering by acknowledgment, event state, priority, notification class
- **Added** ConfirmedTextMessage handler
- **Added** UnconfirmedTextMessage handler
- **Added** LifeSafetyOperation handler
- **Added** WriteGroup handler
- **Added** SubscribeCOVPropertyMultiple handler — creates per-property COV subscriptions
- **Wired** all new handlers into server dispatch (GetAlarmSummary, GetEnrollmentSummary, TextMessage, LifeSafetyOperation, SubscribeCOVPropertyMultiple, WriteGroup)

### Python Bindings

- **Added** `rusty_bacnet.pyi` type stub file — full type introspection for IDEs (VS Code, PyCharm)
- **Added** `py.typed` marker (PEP 561) for mypy/pyright support
- Type stubs cover: 10 enum classes, 4 core types, 3 exception classes, BACnetClient (35+ async methods), BACnetServer (50+ methods), ScHub

### Other

- Moved trend_log polling state out of global static into server struct
- Cleaned up QUIC research branch and artifacts
- LoopbackSerial now buffers excess bytes instead of silently truncating
- Init-Routing-Table ACK only encodes up to count entries (prevents payload/count mismatch)

## [0.6.4]

### Changed
- **BIP transport refactor**: `RecvContext` struct replaces 9-parameter `handle_bvll_message` signature (removes `clippy::too_many_arguments`)
- **Confirmed request refactor**: `ConfirmedTarget` enum deduplicates `confirmed_request` and `confirmed_request_routed` into shared `confirmed_request_inner`
- **BBMD deferred initialization**: `enable_bbmd()` stores `BbmdConfig` instead of creating `BbmdState` with dummy address; real state created at `start()` with actual bound address
- **Foreign device registration clarity**: `register_foreign_device` renamed to `register_foreign_device_bvlc` to distinguish BVLC-only post-start registration from pre-start `register_as_foreign_device` (which enables Distribute-Broadcast-To-Network)
- Extracted `require_socket()` helper to deduplicate socket-not-started error in `send_unicast`/`send_broadcast`
- Updated `NetworkLayer` doc comments to clarify non-router role with DNET/DADR addressing capability

### Added
- **BVLC concurrency guard**: `bvlc_request` rejects concurrent management requests (returns error instead of silently overwriting pending sender)
- Documented Forwarded-NPDU source_mac asymmetry (BBMD mode vs foreign device mode) in BIP transport

## [0.6.3]

### Added
- **Interactive shell session state**: `target` command to set/show/clear default target address; `status` command shows transport, local address, BBMD registration, and discovered device count
- **BBMD auto-renewal** in interactive shell: `register` stores registration and spawns background task to renew at 80% TTL; `unregister` cancels renewal; shown in `status` output
- **Missing shell commands** now available interactively: `ack-alarm`/`ack`, `time-sync`/`ts`, `create-object`, `delete-object`, `read-range`/`rr`
- **Shell `discover --bbmd`**: register as foreign device and discover in one step (BIP shell only)
- **Colored terminal output** via `owo-colors`: green for success/values, red for errors, yellow for warnings, cyan for addresses, dimmed for metadata
- Default target auto-prepend: commands like `read`, `write`, `subscribe` use the session default target when no target argument is given
- Discovery progress feedback: "Waiting Ns for responses..." status line during WhoIs

### Changed
- Deduplicated `format_mac()` and `device_info()` into `output.rs` (removed from `discover.rs` and `router.rs`)
- Removed dead code: `is_tty()`, `print_ok()`, `print_value()` from output module
- BIP shell separated from generic shell for type-safe BBMD command dispatch

### Fixed
- Interactive `discover --target` returning "No devices found" on subsequent calls (removed stale HashSet filter)
- Unused import `use bacnet_encoding::npdu::Npdu` in `bacnet-network/src/layer.rs`

## [0.6.2]

### Added
- **CLI `discover` options** for cross-subnet and directed device discovery:
  - `--target <addr>` — send directed (unicast) WhoIs to a specific device or router
  - `--bbmd <addr>` — register as foreign device with a BBMD before discovering (with `--ttl`)
  - `--dnet <network>` — target a specific remote network number through a router
- `BACnetClient::who_is_directed()` — unicast WhoIs to a specific MAC address
- `BACnetClient::who_is_network()` — WhoIs broadcast to a specific remote network
- `NetworkLayer::broadcast_to_network()` — broadcast APDU to a specific DNET (not global 0xFFFF)
- Shell `discover` command supports `--target` and `--dnet` flags

### Fixed
- Interactive shell backspace not visually deleting characters (set `Behavior::PreferTerm` in rustyline)

## [0.6.1]

### Added
- **`bacnet capture` command** for live packet capture and offline pcap file analysis
- BACnet frame decoder (BVLC/NPDU/APDU) with summary and full decode modes (`--decode`)
- `--save` / `--read` flags for pcap file I/O, `--quiet` for headless recording
- `--filter` flag for additional BPF filter expressions (appended to default BACnet filter)
- `pcap` feature flag on `bacnet-cli` (off by default, requires libpcap)
- Pre-built CLI binaries for Linux (amd64/arm64, with pcap), macOS (amd64/arm64), Windows (amd64)
- CLI reference documentation (`docs/CLI.md`)

### Fixed
- Replaced stale `nicegates` org references with `jscott3201` across README, Java packaging, and npm config

## [0.6.0]

### Added
- **BBMD client API** (Annex J): `read_bdt()`, `write_bdt()`, `read_fdt()`, `delete_fdt_entry()`, `register_foreign_device()` on `BipTransport` with oneshot channel response correlation
- **BACnet/IPv6 CLI support** (Annex U): `--ipv6`, `--ipv6-interface`, `--device-instance` flags; `Bip6ClientBuilder` and `bip6_builder()` on `BACnetClient`
- IPv6 bracket-notation target parsing (`[::1]:47808`, `[fe80::1]`) in CLI
- `FdtEntryWire` struct and `decode_fdt()` / `encode_bdt_entries()` helpers in `bacnet-transport`
- `BACnetClient<BipTransport>` delegate methods for BBMD management
- CLI `bdt`, `fdt`, `register`, `unregister` commands now fully functional (were stubs)
- Table and JSON output for BDT/FDT query results via `comfy-table`

### Changed
- BBMD management commands restricted to BIP transport (clear error on SC/IPv6)
- BIP transport dispatch in CLI separated into `execute_bip_command()` for type-safe BIP-only operations

## [0.5.5]

### Fixed
- Java/Kotlin Gradle publish URL pointed to wrong GitHub org

## [0.5.4]

### Fixed
- WASM npm package missing `repository` field — npm provenance verification requires it

## [0.5.3]

### Fixed
- Clippy `io_other_error` lint in ethernet transport (Rust 1.93)
- Clippy `inherent_to_string` lint in WASM bindings — use `Display` trait instead

### Changed
- Added CHANGELOG entries for patch releases to unblock CI release pipeline

## [0.5.2]

### Fixed
- Clippy `io_other_error`: use `std::io::Error::other()` in ethernet transport

## [0.5.1]

### Added
- **Kotlin examples:** 4 examples (BIP client/server, COV subscriptions, device management, IPv6)
- **JMH benchmarks:** full Kotlin/JVM benchmark suite (BIP ops, concurrency, JNA overhead, object creation)
- Benchmarks.md: added Kotlin/JVM benchmark results section

### Fixed
- Clippy `inherent_to_string` in `bacnet-wasm` — implemented `Display` for `JsObjectIdentifier`
- UniFFI `async_runtime = "tokio"` on all async impl blocks (fixes "no reactor running" in JMH)
- Stale benchmarks crate version (0.4.0 → 0.5.x)

### Changed
- Updated CLAUDE.md with Java/Kotlin build commands, UniFFI conventions, workspace layout

## [0.5.0]

### Added
- **Java/Kotlin bindings:** new `bacnet-java` crate via UniFFI 0.31
  - `BacnetClient` with 23 async methods (ReadProperty, WriteProperty, RPM, WPM, COV, WhoIs, IAm, etc.)
  - `BacnetServer` with 60+ object type builders (all ASHRAE 135-2020 standard object types)
  - `CovNotificationStream` async iterator for real-time COV notifications
  - Transport factory supporting BIP/IPv4, BIP6/IPv6, and MS/TP configurations
  - Error mapping from internal BACnet errors to typed Kotlin exceptions
  - 36 unit tests covering all bindings
- **Java/Kotlin distribution:** multi-platform JAR with JNA native libraries
  - Platforms: Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64
  - Published to GitHub Packages as `io.github.jscott3201:bacnet-java`
  - Kotlin suspend functions for async operations via kotlinx-coroutines
  - Gradle build with `build-local.sh` for development workflow
- CI: Java native library builds across 5 platform matrix
- CI: Automated JAR packaging and GitHub Packages publishing on release tags
- CI: JAR attached to GitHub Releases

## [0.4.0]

### Added
- **WASM/JavaScript support:** new `bacnet-wasm` crate for BACnet/SC thin client in browsers
  - `BACnetScClient` class with `connect`, `disconnect`, `readProperty`, `writeProperty`, `whoIs`, `subscribeCov`
  - SC frame codec and connection state machine ported from bacnet-transport (pure computation, no tokio)
  - Browser WebSocket adapter via `web-sys` with async recv/send
  - Service codec functions: `encodeReadProperty`, `encodeWriteProperty`, `encodeWhoIs`, `encodeSubscribeCov`
  - Value encoding helpers: `encodeReal`, `encodeUnsigned`, `encodeBoolean`, `encodeEnumerated`
  - APDU decoder returning JS objects: `decodeApdu`, `decodeReadPropertyAck`
  - JS-facing type wrappers: `JsObjectIdentifier`, `ObjectTypes`, `PropertyIds` constants
  - TypeScript definitions auto-generated by wasm-bindgen
  - 40 native unit tests for SC frame codec, connection state machine, and service codecs
- CI: WASM compilation check (`wasm32-unknown-unknown`) on every push
- CI: wasm-pack build (bundler + web targets) and npm publish on release tags

## [0.3.0]

### Fixed
- CI: use native ARM64 runner (`ubuntu-24.04-arm`) for aarch64 Linux wheels instead of cross-compilation

## [0.2.0]

### Changed
- Moved client ↔ server integration tests into `bacnet-integration-tests` crate, breaking the circular dev-dependency that prevented publishing to crates.io

### Fixed
- CI: added QEMU emulation for aarch64 Linux wheel builds (fixes aws-lc-sys cross-compilation failure)
- CI: disabled sccache for emulated aarch64 builds
- CI: simplified crates.io publish ordering (server before client, no retry hack)

## [0.1.5]

### Fixed
- CI: fixed circular dev-dependency publish ordering (bacnet-server ↔ bacnet-client)
- CI: fixed aarch64 Linux wheel builds (install cmake/perl for aws-lc-sys cross-compilation)

## [0.1.4]

### Fixed
- Crate publishing: added version to all workspace dependency declarations

## [0.1.3]

### Fixed
- Cargo Deny: added MPL-2.0 to allowed licenses for `serialport` dependency
- Ethernet transport: added missing `TransportPort` trait import in test module

## [0.1.0]

Initial release of the Rusty BACnet protocol stack implementing ASHRAE 135-2020.

### Protocol Stack

- Complete BACnet protocol stack: application, network, and transport layers
- ASN.1/BER encoding and decoding for all BACnet data types
- APDU segmentation (both send and receive) with configurable segment sizes
- NPDU routing with hop-count loop prevention

### Transports

- **BACnet/IP (BIP)** — UDP transport with broadcast support (Annex J)
- **BACnet/IPv6 (BIP6)** — Multicast transport with 3-byte VMAC and 3 multicast scopes (Annex U)
- **BACnet/SC** — Secure Connect over WebSocket + TLS with mutual authentication (Annex AB)
- **BACnet/SC Hub** — Hub-and-spoke relay for SC topology with VMAC collision detection
- **MS/TP** — Master-Slave Token Passing over RS-485 serial (Annex G, Linux only)
- **Ethernet** — 802.3 with BPF filtering (Annex K, Linux only)
- **BBMD** — BACnet Broadcast Management Device with Foreign Device Table and management ACL

### Services (24 modules)

- **Property Access:** ReadProperty, WriteProperty, ReadPropertyMultiple, WritePropertyMultiple
- **Object Management:** CreateObject, DeleteObject
- **Discovery:** WhoIs/IAm, WhoHas/IHave, WhoAmI
- **COV:** SubscribeCOV, SubscribeCOVProperty, SubscribeCOVPropertyMultiple, COV notifications (confirmed and unconfirmed)
- **File Access:** AtomicReadFile, AtomicWriteFile
- **Alarm & Event:** AcknowledgeAlarm, GetAlarmSummary, GetEnrollmentSummary, GetEventInformation
- **Device Management:** DeviceCommunicationControl, ReinitializeDevice, TimeSynchronization, UTCTimeSynchronization
- **List Operations:** AddListElement, RemoveListElement
- **Other:** PrivateTransfer, ReadRange, TextMessage, VirtualTerminal, WriteGroup, LifeSafety, Audit

### Object Types (62)

- **Analog:** AnalogInput, AnalogOutput, AnalogValue
- **Binary:** BinaryInput, BinaryOutput, BinaryValue
- **Multistate:** MultistateInput, MultistateOutput, MultistateValue
- **Access Control:** AccessDoor, AccessCredential, AccessPoint, AccessRights, AccessUser, AccessZone, CredentialDataInput
- **Accumulator/Pulse:** Accumulator, PulseConverter
- **Averaging:** AveragingObject
- **Command:** Command
- **Device & File:** Device, File, Program, NetworkPort
- **Elevator:** ElevatorGroup, Escalator, Lift
- **Event & Alarm:** EventEnrollment, AlertEnrollment, EventLog, NotificationClass, NotificationForwarder
- **Group:** Group, GlobalGroup, StructuredView
- **Life Safety:** LifeSafetyPoint, LifeSafetyZone
- **Lighting:** LightingOutput, BinaryLightingOutput, Channel
- **Load Control:** LoadControl, StagingObject
- **Loop:** LoopObject
- **Schedule & Timer:** Calendar, Schedule, Timer
- **Trend:** TrendLog, TrendLogMultiple
- **Audit:** AuditLog, AuditReporter
- **Value Types (12):** BitStringValue, CharacterStringValue, DateValue, DatePatternValue, DateTimeValue, DateTimePatternValue, IntegerValue, LargeAnalogValue, OctetStringValue, PositiveIntegerValue, TimeValue, TimePatternValue

### Client

- Async client built on Tokio with Transaction State Machine (TSM)
- 31 async service methods covering all standard BACnet operations
- APDU segmentation receive with configurable window size
- Automatic device discovery via WhoIs with device cache
- Invoke ID management with retry logic (configurable retries and timeout)
- Support for BIP, BIP6, and SC transports via generic `TransportPort` trait
- Builder pattern: `bip_builder()`, `sc_builder()`, `generic_builder()`

### Server

- Async server with automatic APDU dispatch for 17 service handlers
- 62 object type factories via `add_*` methods (DeviceObject auto-created at start)
- Change of Value (COV) subscription engine with confirmed and unconfirmed notifications
- COV-in-flight semaphore (max 255 concurrent confirmed notifications)
- Intrinsic reporting engine with 5 algorithms: ChangeOfState, ChangeOfBitstring, ChangeOfValue, FloatingLimit, CommandFailure
- Event enrollment with algorithmic evaluation
- Fault detection and reliability evaluation (Clause 12)
- Schedule execution engine with weekly schedules and exception schedules
- Automatic trend logging with configurable polling intervals
- DeviceCommunicationControl state machine (Enable/Disable/DisableInitiation)
- PICS (Protocol Implementation Conformance Statement) generation per Annex A
- Segmentation receiver pool capped at 128 (DoS prevention)

### Network Layer

- Network layer routing with RouterTable and DNET/DADR lookup
- Multi-port BACnetRouter for inter-network routing
- PriorityChannel for APDU queuing (critical/urgent/normal/background)
- Hop-count management and loop prevention

### Python Bindings (PyO3)

- Full async API via `pyo3-async-runtimes` with Tokio backend
- `BACnetClient` class with 42 async methods mirroring the Rust client API
- `BACnetServer` class with 61 `add_*` methods for all object types plus 6 runtime methods
- `PyScHub` class for BACnet/SC hub management
- 11 enum types with named constants via `py_bacnet_enum!` macro
- COV async iterator (`async for event in client.cov_notifications()`)
- Support for BIP, IPv6, and SC transports with TLS/mTLS configuration
- Published to PyPI as `rusty-bacnet` (Python ≥ 3.11)

### Testing & Quality

- 1,682 tests across all crates
- CI pipeline on Linux, macOS, and Windows via GitHub Actions
- Clippy with `-Dwarnings` (zero warnings policy)
- `cargo-deny` for license/advisory/source auditing
- Integration tests for client-server round trips on localhost

### Benchmarks

- 9 Criterion benchmark suites: BIP latency/throughput, BIP6 latency/throughput, SC latency/throughput, SC mTLS latency/throughput, encoding performance
- 4 Python mixed-mode benchmarks: Python↔Rust client/server cross-language performance
- Docker Compose environment for isolated benchmark runs

### Examples

- 3 Rust examples: BIP client/server, COV subscriptions, multi-object server
- 5 Python examples: BIP client/server, IPv6 multicast, SC secure connect, COV subscriptions, device management
- Docker Compose example for containerized deployment
