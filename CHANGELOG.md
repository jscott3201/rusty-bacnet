# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Add `BACnetObject::is_writable_property`, `BACnetObject::is_createable`, and `BACnetObject::is_deleteable` trait methods (with conservative defaults that preserve current behavior for unmigrated object types) so PICS generation and runtime dispatch share one truth source for property writability and object createability. The `bacnet-objects` crate is a public dependency; downstream implementors of `BACnetObject` now have these methods available to override (defaults preserve existing PICS output).
- Add `BACnetServer::write_local` as the server-owned local-mutation entry point. It writes a property under the database lock (routing `OBJECT_NAME` through the name-uniqueness check and index refresh, like the network handler) and then fires the same post-write COV and event notifications as a network `WriteProperty`. The Python `write_property_local` binding now delegates to it.
- Add `BACnetObject::tick_intrinsic_reporting` as the periodic (1-second) counterpart to `evaluate_intrinsic_reporting`. The production server now runs a 1-second `intrinsic_reporting_task` that calls `tick_intrinsic_reporting` on every object to advance pending Time_Delay countdowns and fire confirmed transitions, mirroring the per-write `evaluate_intrinsic_reporting` (probe) path. Object implementors overriding intrinsic reporting should implement both methods together.
- Add `BACnetObject::set_event_state_internal` as the internal lifecycle path for the algorithmically-derived `Event_State` on objects such as Event Enrollment (ASHRAE 135-2020 Clause 12.12). It is distinct from the network `write_property` route (`Event_State` is read-only over the network); the server evaluator calls it to persist a detected transition. Objects without an algorithmic `Event_State` keep the default (`OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`). The `bacnet-objects` crate is a public dependency; downstream implementors of `BACnetObject` now have this method available to override.

### Fixed

- Correct PICS writable-property flags for the 9 core I/O/V object types (AnalogInput/Output/Value, BinaryInput/Output/Value, MultiStateInput/Output/Value) so they mirror the objects' real `write_property` arms. The previous static heuristic omitted 33 writable routes that the objects actually accept — `LIMIT_ENABLE`, `EVENT_ENABLE`, `NOTIFY_TYPE`, `TIME_DELAY`, `NOTIFICATION_CLASS`, `HIGH_LIMIT`, `LOW_LIMIT`, `DEADBAND` (event-capable analog types), `PRIORITY_ARRAY` (commandable types), `STATE_TEXT` (multistate types), and `PRESENT_VALUE` on input types (writable when out-of-service) — producing false-negatives in PICS. Writability is now derived from the object's own `is_writable_property` capability method, so PICS and runtime dispatch cannot drift apart.
- Stop over-reporting `AnalogValue` as createable in PICS. The static heuristic declared all types createable except Device and NetworkPort, but `handle_create_object` has no branch for `AnalogValue` (it falls through to `UNSUPPORTED_OBJECT_TYPE`). `is_createable` now returns `true` only for the 8 types the runtime factory actually constructs (AI, AO, BI, BO, BV, MSI, MSO, MSV); `AnalogValue` is `false` until/unless a later PR adds the factory branch.
- Stop over-reporting `Device` as deleteable via the static heuristic; `Device` and `NetworkPort` now override `is_deleteable` to `false` on the objects themselves.
- Reconcile `handle_delete_object` with `is_deleteable`: the runtime DeleteObject handler now rejects `NetworkPort` (matching its `is_deleteable` override) in addition to `Device`, so PICS and runtime dispatch share one truth source for deleteability with no remaining drift.
- Route `WriteProperty`, `WritePropertyMultiple`, `CreateObject` initial values, and the local `write_property_local` Python-binding writes of `OBJECT_NAME` through the `ObjectDatabase` name index. A duplicate name is now rejected up front with `DUPLICATE_NAME` (the database's `check_name_available` helper, previously dead code), and a successful rename refreshes the index via `update_name_index` so `find_by_name` resolves to the new name and the old name is freed for reuse. `WritePropertyMultiple` rollback re-syncs the index for any rolled-back `OBJECT_NAME` write, restoring the pre-transaction name mappings; `CreateObject` rolls back (removes the created object) when an `OBJECT_NAME` initial value collides. Previously, `write_property(OBJECT_NAME, …)` and `CreateObject` initial-value writes mutated the object's name field in place without touching the database secondary index, leaving stale lookups and allowing duplicates.
- Fire COV (and event) notifications on local/internal property writes. The Python `write_property_local` path and direct object setters mutated objects without entering the server's post-write trigger path, so a subscription could observe a network mutation but not an equivalent local mutation. Local writes now go through `BACnetServer::write_local`, which applies the same post-write COV/event processing as the network dispatch loop (and respects DCC: notifications are skipped while communication is disabled). Low-level object setters (`set_present_value` and the like) remain notification-bypassing building blocks below the high-level server surface.
- Restore the priority-array slot — not the effective present value — when a `WritePropertyMultiple` rolls back a commandable `PRESENT_VALUE` write. The previous rollback snapshotted `PRESENT_VALUE` (which reads the resolved highest-priority value) and wrote it back with no priority, selecting priority 16; a failed multi-write could therefore leave the originally-changed slot un-restored and add a spurious priority-16 command, so the object's `PRIORITY_ARRAY`, `CURRENT_COMMAND_PRIORITY`, and effective `PRESENT_VALUE` could differ from their pre-request state despite the operation reporting failure. Rollback now snapshots `PRIORITY_ARRAY[priority]` (the exact slot the write targets, or `Null` if it was relinquished) and restores that slot directly; a `Null` snapshot relinquishes the slot again. Non-commandable objects (where `PRIORITY_ARRAY` is not readable) fall back to the prior value snapshot.
- Honor the intrinsic `Time_Delay` (ASHRAE 135-2020 §13.2.4) in the production `EventNotification` path. The production detectors (`OutOfRangeDetector`, `ChangeOfStateDetector`, `CommandFailureDetector`) mutated `event_state` eagerly and left their `time_delay` fields unused, so any nonzero `Time_Delay` was silently ignored — a transition fired on the first qualifying write instead of after the configured delay. The detectors now split evaluation into `probe` (per-write: seed a pending transition / cancel on revert / fire only when `Time_Delay == 0`; never decrement and never re-seed an existing transition to the same target) and `tick` (periodic: decrement the remaining delay / fire on expiry / cancel on revert). The server runs a 1-second `intrinsic_reporting_task` (with `MissedTickBehavior::Delay` so a delayed wake cannot burst-compress the countdown) that ticks every object, so the countdown advances per elapsed wall-clock second rather than per `evaluate()` call — repeated writes can neither shorten the delay nor pin it indefinitely (a `Time_Delay` is a debounce timer: a redundant write of the same qualifying value leaves the in-flight countdown untouched), and a condition that clears before the delay elapses cancels the pending transition with no notification and leaves `Event_State` at its confirmed (old) value. The notification bytes are unchanged; only their timing changes.
- Reject `number_of_states == 0` at construction for the three multi-state object types (`MultiStateInput`, `MultiStateOutput`, `MultiStateValue`). The constructors previously accepted zero, initialized `PRESENT_VALUE` (and, for Output/Value, the relinquish default) to `1`, and built an empty `STATE_TEXT` list — leaving the initial value outside the accepted `1..=number_of_states` write range with no attainable value. `new(.., 0)` now returns `Error::OutOfRange`, enforced through a single shared `require_nonzero_states` guard so every caller (direct instantiation, the `CreateObject` factory, and tests) hits one boundary; the factory already passes a nonzero count and is unaffected. Boundary tests cover zero (rejected) and one state (initial and relinquish-default values stay in range).
- Resolve the per-transition `Priority` and `Ack_Required` for an `EventNotification` from the referenced `NotificationClass` instead of a hardcoded branch (ASHRAE 135-2020 §13.2.1). The production sender chose `Priority` from a normal/non-normal branch (200 for `TO_NORMAL`, 100 otherwise) and derived `Ack_Required` purely from `Notify_Type == ALARM`, so the class's configured `PRIORITY[TO_OFFNORMAL/TO_FAULT/TO_NORMAL]` and `ACK_REQUIRED[TO_OFFNORMAL/TO_FAULT/TO_NORMAL]` were never projected into outbound notifications and an `EVENT`-type notification always cleared `Ack_Required`. The notification now selects the array element for the current transition coordinate (via `resolve_transition_priority_ack`), with a shared `find_notification_class` lookup; when no matching class is configured it falls back to the BACnet defaults (`Priority = 255`, no ack) so the notification is still delivered rather than dropped. `Ack_Required` is still suppressed on the wire for `ACK_NOTIFICATION` (the field is only valid for `ALARM`/`EVENT`). Coverage spans offnormal, fault, and normal transitions plus the missing-class fallback.
- Make NotificationClass recipient-list weekday and time-window semantics use one convention (ASHRAE 135-2020 §12.15.5 / Clause 21). The `BACnetDestination.valid_days` field is a `BACnetDaysOfWeek` bit string defined as `BIT STRING { monday(0), …, sunday(6) }`; the in-memory value (`bit 0 = Monday`) already followed this, and the wire encoding packs the 7-bit day mask as `value << 1` with one unused bit (round-tripping within this codebase), but the unit tests used the opposite convention (`monday_bit = 0x02` "bit 1 = Monday", "Sunday = bit 0", "Saturday = bit 6") and the Mon–Fri fixture was actually Tue–Sat, so a caller following the tests would select different recipients than one following the docs/sender. The tests now use the spec convention (`bit 0 = Monday`), and the `get_notification_recipients` `today_bit` doc states it explicitly. A `to_time` earlier than `from_time` (e.g. 22:00–02:00) now denotes an overnight window active from `from` to midnight and again from midnight to `to` (previously such a window could never match because the filter required `from ≤ current ≤ to`). The sender also evaluates the day/time filters in the device's local time: it reads the Device object's `UTC_Offset` (signed minutes, Clause 12.32) and shifts the wall clock before deriving the day-of-week and time-of-day (via the new `local_day_and_time` helper), so `valid_days`/`from_time`/`to_time` are interpreted in the same frame the device's schedule uses; with the default `UTC_Offset` of 0 this is a no-op (UTC). New tests cover Sunday/Monday/weekend day bits, inclusive window boundaries, and midnight-crossing windows; the overnight coverage is proven to fail under the prior non-wrapping comparison.
- Preserve the `BACnetAddress` network number in a NotificationClass `Recipient_List` address recipient (ASHRAE 135-2020 Clause 21 / §12.15.5). An `address` recipient was encoded as only the `mac_address` (a bare `OctetString`) and decoded back with `network_number = 0`, so a recipient configured for a nonzero BACnet network could not round-trip through the object property representation. The `Address` variant now encodes as `List[Unsigned(network_number), OctetString(mac_address)]` and decodes both back into the full `BACnetAddress`, so local (network 0), remote (nonzero network), and broadcast (network 65535, empty MAC) forms all survive. **This is a representation change** for `Recipient_List` address entries: values previously written by older versions (MAC-only, no network number) are not recognized by the new decoder and must be reconfigured — the MAC-only form was a non-conformant internal-only encoding (the spec's `BACnetAddress` is a `SEQUENCE { network-number, mac-address }`, never a bare octet string). Malformed address entries (wrong element count or field type) are now rejected on write rather than silently dropping the network number. The full ASN.1 `CHOICE`/`SEQUENCE` wire-spec-correctness of the whole `Recipient_List` (context-tagged `[0]`/`[1]` recipient choice and framed `BACnetDestination` SEQUENCE) is a larger, separate encoding-model change tracked as a follow-up; this fix is scoped to the property-value round-trip the issue names. Tests for the nonzero-network round-trip, all three address forms, and malformed-input rejection are split into `notification_class/tests/recipient_list.rs` (following the `analog/tests/` directory precedent) to keep the test files under the 700-LOC cap; the nonzero-network case is proven to fail under the prior MAC-only encoding.
- Model the `Event_Parameters` of an EventEnrollment object as a structured `BACnetEventParameter` CHOICE (ASHRAE 135-2020 Clause 13.5) instead of raw opaque octets, and make the server evaluator consume the structured alternatives. The `Event_Parameters` property was stored as a `Vec<u8>` and the evaluator re-parsed it with private little-endian layouts (no `time-delay`, wrong field order, `f32::from_le_bytes` instead of ASN.1 tags); `Fault_Parameters` reads reduced the structured value to its variant number, discarding every field; and the monitored `Object_Property_Reference` was flattened to a `List` of four `PropertyValue`s then re-parsed, ignoring the array index and device identifier. `BACnetEventParameter` now carries the five evaluated algorithms (`ChangeOfBitstring`, `ChangeOfState`, `ChangeOfValue`, `FloatingLimit`, `OutOfRange`) with their full fields, an `Extended` alternative (vendor_id / extended_event_type / raw parameters), and an `Opaque` catch-all that preserves any unknown tag verbatim; `FaultParameters` gains `encode_property_value`/`decode_property_value` so it round-trips with all fields intact (the prior lossy `Unsigned(variant_tag)` read is replaced by the structured value). The server evaluator decodes the structured parameter, dispatches each alternative to a typed evaluator (`read_setpoint` resolves a `FloatingLimit` setpoint reference against the local database rather than reading an embedded setpoint from the byte payload), and falls back to the original little-endian byte evaluators — keyed on the enrollment's `Event_Type` — for `Opaque` values, so existing raw-octet parameters still evaluate correctly. **This is a representation change** for `Event_Parameters` (and `Fault_Parameters`): the property is now a flat `PropertyValue::List` whose first element is the algorithm tag as `Unsigned`, not the prior raw octets. Values written by older versions are wrapped as `Opaque { tag: 0xFF, data }` on write and routed through the legacy evaluator, so they keep working; clients that want the structured form must write the new `List` encoding. The full ASN.1 context-tagged `BACnetEventParameter`/`BACnetFaultParameter` `CHOICE` wire encoding is a larger, separate encoding-model change tracked as a follow-up; this fix is scoped to the property-value round-trip and evaluator the issue names. Unit tests for every alternative's encode/decode round-trip, `Opaque`/`Extended` preservation, and malformed-input rejection live in `bacnet-types/src/constructed/event_parameter/tests.rs`; the structured evaluation, legacy fallback, and wrong-type-monitored-value skip are covered in `bacnet-server` (the wrong-type skip is proven to spuriously transition under the prior NORMAL-returning behavior).
- Separate the internal `Event_State` lifecycle on an EventEnrollment from the network-writable `write_property` route (ASHRAE 135-2020 Clause 12.12). The Event Enrollment evaluator persisted a detected transition by calling the same `write_property(EVENT_STATE, …)` branch exposed to network `WriteProperty`, and that branch also accepted direct `EVENT_STATE` writes, so internal lifecycle state and public property mutation shared one access path with no authorization boundary; `Event_State` is algorithmically derived and read-only over the network, so accepting network writes was also a route/PICS drift (PICS already reported it read-only via the default `is_writable_property`, but the route accepted it). A network `WriteProperty` of `EVENT_STATE` is now rejected with `WRITE_ACCESS_DENIED` (the route falls through to the existing access-denied tail), and the evaluator persists transitions through a new internal `BACnetObject::set_event_state_internal` trait method (default `OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`; `EventEnrollmentObject` overrides it), so the internal and network paths are distinct. The `read_property(EVENT_STATE)` output is unchanged (no wire-format change); only a previously-accepted non-conformant network write is now refused. Tests cover the network rejection, the internal setter, and that periodic evaluation persists `Event_State` while the network route on the same object is refused.


### Changed

- Split CI into a lean and a heavy tier so feature→dev PRs run only the fast gate (`rustfmt`, `clippy`, file-size cap, no-secret scan, and the Linux test matrix) while the cross-OS test matrix (macOS, Windows), MSRV, `cargo audit`, and `cargo deny` run on dev→main PRs, pushes to `main`, and `v*` tags. The feature→dev loop is no longer held up by macOS/Windows runners or the advisory/license/MSRV jobs; the full matrix still gates every merge into `main` and every release (the `validate` job waits on `test-cross` as well as `test`).
- The Python `BACnetServer.write_property_local` now raises `BacnetProtocolError` (with `error_class`/`error_code`) for `UNKNOWN_OBJECT` and `DUPLICATE_NAME` instead of a generic `RuntimeError`, giving it structured parity with the network `WriteProperty` path. Python callers that caught `RuntimeError` specifically should catch `BacnetProtocolError` (or `Exception`) instead. The server lock is also held across the whole call (including post-write COV/event sends), so concurrent Python calls on the same `BACnetServer` serialize behind a local write; a confirmed-COV send to an unresponsive subscriber can stall other Python calls for up to the COV retry timeout. This prevents a `stop()` from racing the notification sends mid-flight.

- Bring every dependency current. `cargo update` moves 94 in-range packages, including tokio 1.52.3 → 1.53.1, bytes 1.11.1 → 1.12.1, rustls 0.23.40 → 0.23.42, serde 1.0.228 → 1.0.229, serde_json 1.0.149 → 1.0.151, clap 4.6.1 → 4.6.4, libc 0.2.186 → 0.2.189, aws-lc-rs 1.16.3 → 1.17.3, socket2 0.6.3 → 0.6.5, getrandom 0.4.2 → 0.4.3, and thiserror 2.0.18 → 2.0.19. One semver-major bump follows, and it needed no source change: `rustyline` 17 → 18 in `bacnet-cli`. `sysinfo` stays at 0.38 — see below. The lock shrinks from 362 entries to 330, largely because the `wit-bindgen`/`wit-component`/`wasm-metadata` build chain leaves, which also clears the single allowed `cargo audit` warning this workspace had been carrying; `cargo audit` now reports no findings at all. Duplicate-version warnings from `cargo deny` drop from 10 to 3. All ten on `dev` were Windows-family crates — `windows-sys` plus `windows-targets` and eight `windows_*` target shims — and every one is resolved: locked `windows-sys` goes from five versions to three, `windows_x86_64_msvc` from three to two, `windows-targets` from two to one, and `fd-lock` leaves entirely with rustyline 17. Of the three that remain, `syn` is new and not ours to resolve — `serde_derive`, `clap_derive`, and `thiserror-impl` have moved to `syn` 3 while `tokio-macros`, `tracing-attributes`, `futures-macro`, and `zerocopy-derive` still require `syn` 2, so both majors stay locked until those crates migrate. `r-efi` is newly reported with an unchanged locked version set. `cargo deny` reports advisories, bans, licenses, and sources all ok, and MSRV 1.93 still builds.
- `sysinfo` is deliberately held at 0.38.4. Version 0.39 requires rustc 1.95, which would break the 1.93 MSRV that the published crates promise: `RUSTUP_TOOLCHAIN=1.93 cargo check --workspace --exclude rusty-bacnet --locked` exits 101 against it. Holding it costs nothing measurable — the duplicate-warning count and every other gate are identical either way.
- Bump the pinned development and CI toolchain from 1.95.0 to 1.97.1 — the `rust-toolchain.toml` channel and the six `toolchain:` values in `.github/workflows/ci.yml`. **MSRV is unchanged at 1.93.** The workspace `rust-version`, the `MSRV (1.93)` CI job, and the `rust:1.93-alpine` Docker example are untouched, so the published crates still build on 1.93; `cargo check --workspace` under 1.93 was verified against this change. 1.97.1 surfaced no new deny-level lint, rustfmt requested no reformatting, and the suite is unchanged at 2,227 tests across 34 suites.

### Removed

- Remove the `bacnet-wasm` crate and `docs/wasm-api.md`. The stack now ships as Rust with Python bindings through PyO3, and there is no browser or JavaScript client. Removed with it: the `wasm-check`, `build-wasm`, and `publish-npm` CI jobs, their entries in the `validate` and `github-release` `needs:` lists, the four `--exclude bacnet-wasm` flags on the clippy/MSRV/test/test-cross jobs, the README JavaScript quick-start, and the `node_modules/`, `pkg/`, and `*.tgz` ignore rules. The `@jscott3201/bacnet-wasm` npm package was never published, so no released artifact is affected; the README instructions for installing it described something that did not exist.
- Withdraw the BACnet/SC WASM conformance evidence, narrowing those claims to native only. Seven rows in `docs/conformance/` lose their WASM code anchors, tests, public claims, and prose — `BACNET-AB-SC-FRAME`, `BACNET-AB-SC-BVLC-RESULT`, `BACNET-AB-SC-DATA-ATTRIBUTES`, `BACNET-AB-SC-CONNECTION-STATE`, `BACNET-AB-SC-WEBSOCKET-TLS`, `BACNET-AB-SC-HEARTBEAT`, and the `docs/wasm-api.md` claim on `BACNET-5-SEGMENTATION-WINDOW`. WASM was corroborating parity evidence in every case, never the sole basis, so each row keeps its native `bacnet-transport` anchors and tests and no row's status changed. Browser-specific remaining gaps (live browser/WebSocket smoke coverage, pending-Promise cleanup) are dropped rather than carried forward, because the capability they described no longer exists. Negative-test counts fall on two rows as a result — `BACNET-AB-SC-FRAME` from 10 to 5 and `BACNET-AB-SC-DATA-ATTRIBUTES` from 18 to 5 — and both already carried a `needs-…-tests` status that still holds.

## [0.10.1]

### Added

- Add `BACnetClient::device_events()` with discovery/update/loss notifications while keeping `discovered_devices()` as the device-table snapshot API.
- Add BACnet client builder setters for APDU retries, accepted segment count, segmented-response acceptance, and proposed window size.
- Add `BACnetClient` COV subscribe/unsubscribe helpers that resolve discovered-device routing.
- Add `BACnetClient` SubscribeCOVProperty subscribe/unsubscribe helpers for property-level COV subscriptions.
- Add source metadata, delivery kind, and configurable confirmed-notification ACK policy to `BACnetClient` COV notification delivery.
- Add a managed finite `BACnetClient` COV subscription helper that renews before expiry and emits lifetime/renewal events.
- Add `ScTransport::connection_state_changes()` returning a `tokio::sync::watch::Receiver<ScConnectionState>` so BACnet/SC consumers can await latest link-state changes without polling the inspection-only connection handle; rapid transitions may coalesce under watch semantics.
- Add typed BACnet/SC connect-path errors via `bacnet_transport::sc::ScConnectError`, preserving BVLC-Result NAK class/code/details and distinguishing WebSocket dial, TLS, handshake, and subprotocol failures without string parsing.
- Add `ScClientBuilder::device_uuid(...)` and BACnet/SC CLI `--sc-vmac` / `--sc-device-uuid` options so convenience clients can provision stable SC identity without hand-assembling `ScTransport`.
- Export `bacnet_transport::sc::generate_random48_vmac()` and add `bacnet_transport::sc_frame::is_valid_random48_vmac()` so downstream composition roots can mint and validate Clause H.7.3 Random-48 VMACs without duplicating bit-shape logic.

### Fixed

- Fail fast in `ScClientBuilder` and the BACnet/SC CLI path when required SC identity is missing or uses reserved VMAC values, preventing all-zero Unknown VMAC connects from reaching the wire.
- Propagate negotiated BACnet/SC hub Max-NPDU/BVLC limits into `ScTransport::max_apdu_length()` and `BACnetClient` request segmentation, with `BACnetClient::transport_max_apdu_length()` exposing the live post-connect transport budget.
- Return BACnet/SC handshake timeouts as `Error::Timeout` and malformed BVLC-Result failures as typed `ScConnectError` values instead of collapsing them into `Error::Encoding`; downstream code should match `Error::Timeout` directly and use `ScConnectError::from_error` for SC transport details.
- Re-dial BACnet/SC WebSocket connections during reconnect, failover, and primary restore instead of reusing a torn-down socket object.
- Abort BACnet/SC receive tasks and close WebSocket ownership on ungraceful `Drop`, including the `BACnetClient`/`NetworkLayer` drop cascade, while preserving graceful Disconnect-Request shutdown.
- Abort `BACnetClient` dispatch on drop and cascade B/IP network cleanup so ungraceful teardown releases reader tasks and the UDP socket.
- Run `BACnetClient` device-table stale-entry purging on an independent interval so fully silent datalinks can evict dead devices.
- Avoid `Instant` underflow in `BACnetClient` device-table purging on fresh Windows runners.
- Send the required initial COV notification after successful server-side COV subscription acceptance.
- Encode `SubscribeCOVPropertyMultiple` lifetime/max-delay fields in standard order, validate their subscription/cancellation semantics, and expire server-side multiple-property COV subscriptions.
- Correct `bacnet-client` property method rustdoc comments so read/write, auto-routing, and batch helpers describe the method they document.
- Clarify `ObjectIdentifier` wildcard-instance semantics and add an addressable-object constructor that rejects the reserved wildcard value while preserving wire-level round trips.
- Make the `bacnet-client` COV notification broadcast channel capacity configurable while preserving the default capacity of 64.

## [0.10.0]

### BACnet/SC - Connection Resilience (ASHRAE 135-2020 Annex AB.6.2, AB.6.3)

- Duplicate-VMAC recovery per AB.6.2: a `NODE_DUPLICATE_VMAC` NAK now triggers a Random-48 VMAC reseed and reconnect instead of failing the connection permanently; reseed fails closed on RNG errors, and retry eligibility is preserved across primary-restore probes.
- SC hub duplicate Device-UUID replacement: a device reconnecting after an unclean drop now reclaims its stale session slot instead of being NAK'd until hub restart.
- Failover hub is now used after mid-life reconnect exhaustion (previously only attempted on cold start; the reconnect path carried a TODO), and the primary hub is probed and restored after a failover.
- Heartbeats are sent only when the link is idle (AB.6.3), and a `Heartbeat-ACK` must correlate to the outstanding request - stale, foreign, or malformed ACKs no longer keep a dead link alive; hub-side heartbeat-ACK timeouts are enforced.
- Fatal `BVLC-Result` NAKs, receive errors, heartbeat send failures, and reconnect exhaustion now tear the transport down and set the connection state to `Disconnected` - no more phantom-`Connected` states on a dead link; pending-connect correlation state is cleared on handshake decode/IO errors.

### BACnet/SC - Protocol Conformance (Annex AB.2, AB.3)

- `BVLC-Result` payloads (Error Class / Error Code / Error Details) are parsed and surfaced instead of skipped.
- Malformed SC frame option chains (bad length, truncated, unterminated) are rejected instead of accepted.
- SC data options: outbound encoding, inbound exposure as `ReceivedNpdu::data_attributes` (new `DataAttribute` type + default-bodied `TransportPort::send_*_with_data_attributes` methods), rejection of unsupported must-understand options, and preservation of data attributes across router and hub forwarding.
- Hub hardening: exact ConnectRequest payload-length validation, advertised `Max-BVLC-Length` enforced on inbound and relayed frames, and inbound frames carrying an Originating VMAC rejected (the hub stamps the registered VMAC).
- TLS 1.3 is enforced for SC configurations.
- New `ErrorCode` constants 139-151 (Clause 5.4.6 / Annex AB), including `NODE_DUPLICATE_VMAC` and `NOT_A_BACNET_SC_HUB`.

### BACnet/IP - Annex J

- The BIP UDP socket now binds `INADDR_ANY` (the configured interface IP is retained for the advertised MAC and I-Am source) so subnet and limited broadcasts reach the socket on Linux, with a startup probe of the interface IP.
- Forwarded-NPDU rebroadcast loops are prevented and Original-Broadcast local echo is suppressed.
- BBMD: Distribute-Broadcast-To-Network forwarding failures return a NAK, FDT entries are purged by a timer task with lifecycle-edge validation, BDT persistence survives restart, and management ACK wire format and ACLs are validated.

### WASM (browser bindings)

- BACnet/SC heartbeat keepalive scheduling after Connect-Accept, disconnect handling, and callback cleanup; SC data attributes are routed and preserved through the WASM layer.

### Server

- New `BACnetServer::i_am_broadcaster()` / `broadcast_i_am()` and `BipServerBuilder::vendor_id(u16)` for host-driven I-Am announcement.

### Dependencies

- `pyo3` upgraded to 0.29 (resolves audit advisories).

## [0.9.0]

### Spec Compliance - Codec Strictness (ASHRAE 135-2020 Clauses 20.1.2.7, 20.1.2.8, 20.1.6.x)

- Fixed Finding 4: ConfirmedRequest max-APDU and SegmentACK window fields are now validated instead of silently accepting reserved or out-of-range wire values.
- Fixed Finding 6: BVLL/BVLC encoders now reject frame lengths that cannot fit in the 16-bit BACnet length field instead of truncating.
- Fixed Finding 7: fixed-width primitive decoders now reject incorrect lengths and trailing bytes for ObjectIdentifier, Date, Time, Real, Double, and overlong application-tag values.

### Spec Compliance - Confirmed Notification TSM (ASHRAE 135-2020 Clause 5)

- Fixed Finding 8: confirmed EventNotification delivery now uses the server TSM with timeout and retry handling instead of fire-and-forget sends.
- Fixed Finding 8: server-side confirmed notification acknowledgments are keyed by `(peer, invoke_id)` so responses from different peers cannot collide on a shared invoke ID.

### Spec Compliance - Segmentation (ASHRAE 135-2020 Clauses 5, 20.1.2.4, 20.1.2.5, 20.1.6.x)

- Fixed Finding 1: `split_payload` now errors when a payload would require more than the 256 sequence numbers representable by BACnet segmentation instead of falling back to an oversized unsegmented payload.
- Fixed Finding 2: segmented ComplexACK responses now honor the client's `max-segments-accepted` value and abort with `BUFFER_OVERFLOW` when the response cannot fit.
- Fixed Finding 3: server-side segmented ConfirmedRequest receive state now validates proposed window size, ACKs only at the negotiated window boundary or final segment, and sends negative SegmentACKs for sequence gaps.
- Fixed Finding 5: negative SegmentACK retransmission now resumes at `ack_seq + 1` on both client and server send paths.
- Fixed Finding 9: routed confirmed requests now enter the segmented send path when they exceed the local APDU limit, and routed responses are matched with a routed endpoint TSM key instead of only the next-hop router MAC.

### Performance

- Fixed Finding 10: notification send paths now freeze `BytesMut` payload buffers directly instead of copying them through `to_vec()` before constructing `Bytes`.

### Changed

- **API break**: APDU/BVLL/BVLC encoder entry points now return `Result` where wire-length or field validation can fail.
- **API break**: `bacnet-encoding::segmentation::split_payload` now returns `Result<Vec<Bytes>, Error>` so callers must handle zero-payload-capacity and over-256-segment failures explicitly.
- **Behavior change**: primitive decoder strictness now rejects malformed encodings that were previously accepted with trailing bytes.

### Engineering — CI guardrails

- Workspace lints centralized: `unsafe_code = "deny"` (workspace floor; per-site `#[allow]` + `// SAFETY:` comments on all 23 FFI sites in `bacnet-transport`), `missing_docs = "warn"`, `unused_must_use = "deny"`, plus clippy `todo`/`dbg_macro = "deny"` and `print_*` warnings.
- `rust-toolchain.toml` (channel 1.95.0) and `rustfmt.toml` pin the dev/CI environment.
- New CI jobs: `cargo audit` (advisory database), `check-no-secrets.sh` (AWS keys, private keys, Slack/GitHub/`sk-*` tokens), `check-file-size.sh` (700-LOC cap, warn-only until track-2 splits land).
- `--locked` added to clippy/test/wasm-check so `Cargo.lock` updates can't slip in silently.

### Engineering — Modularity (700 LOC cap)

- 34 source files split into focused modules so every tracked `*.rs` is below the 700 non-empty / non-comment line cap.
- `[lints]` re-export discipline preserved: every type, function, and macro is reachable at its previous import path; no API renames in this pass.
- `.github/scripts/check-file-size.sh` flipped from warn-only (`CHECK_FILE_SIZE_WARN=1`) to strict — the cap is now enforced on every PR.

### Workspace reorganization

The HTTP/MCP gateway and BTL compliance test harness were extracted into dedicated repositories. The remaining workspace focuses purely on the BACnet protocol stack: types, encoding, services, transport, network, client, server, objects, plus the Python and WASM bindings and the CLI.

- **`bacnet-gateway`** — moved to [`jscott3201/rusty-bacnet-mcp`](https://github.com/jscott3201/rusty-bacnet-mcp). Same crate name (`bacnet-gateway`); consumes the published `bacnet-*` library crates from crates.io. `default-features` flipped to `["http", "mcp"]` to make the binary's natural shape the default for the standalone repo.
- **`bacnet-btl`** — moved to [`jscott3201/rusty-bacnet-btl-harness`](https://github.com/jscott3201/rusty-bacnet-btl-harness). Same crate name (`bacnet-btl`); consumes the published `bacnet-*` library crates from crates.io. Direct `bacnet-network` dep dropped (transitive via client/server).

### Removed (from this workspace)
- `crates/bacnet-btl/` directory.
- `crates/bacnet-gateway/` directory.
- `docs/btl.md` (now in the BTL harness repo).
- `docs/gateway.md` (now in the MCP repo).
- `examples/docker/Dockerfile.btl` and `examples/docker/docker-compose.btl.yml` (BTL Docker assets — now in the BTL harness repo).

### Notes
- Library crate APIs changed from 0.8.1 where called out in this changelog.
- Python (`rusty-bacnet`) and WASM (`bacnet-wasm`) bindings unchanged.
- CLI (`bacnet-cli`) unchanged.

## [0.8.1]

### Security
- Bumped `rustls-webpki` from 0.103.10 to 0.103.13 to address [RUSTSEC](https://rustsec.org/) advisory: panic reachable prior to CRL signature verification. Applications not using Certificate Revocation Lists were not exposed.
- Bumped `rand` 0.10.0 → 0.10.1 and `rand` 0.9.2 → 0.9.4 to address [RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097): `ThreadRng` unsoundness when a custom logger calls `rand::rng()` during reseed. Triggering preconditions are not present in this stack, but the patched versions are pulled in for defense in depth.

### Fixed
- **bacnet-gateway**: use the configured BIP port for the client transport instead of port 0 (which let the OS assign an ephemeral port). Devices replying to the standard BACnet port could not reach the gateway client. Thanks to @chappo (PR #8).
- **bacnet-client**: gate `Ipv6Addr` import behind the `ipv6` feature to fix unused-import warning when the feature is disabled.
- **bacnet-gateway**: drop unused `property_value_to_json` import in REST objects handler.

### Documentation
- **Benchmarks.md**: refreshed with results from a clean run across all 9 Criterion suites.

## [0.8.0]

### Spec Compliance — BBMD & Router (ASHRAE 135-2020 Annex J, Clause 6)

Deep-dive review of the BBMD and Router implementations identified 22 spec compliance issues. All fixed.

#### Router — Congestion & Reachability (Clause 6.6.3)
- **Fixed** router forwards traffic to busy networks — now checks `effective_reachability()` before forwarding and rejects with reason 2 (ROUTER_BUSY) per Clause 6.6.3.6
- **Fixed** router forwards traffic to permanently unreachable networks — now rejects with reason 1 per Clause 6.6.3.5
- **Fixed** Router-Busy-To-Network handler — now uses `mark_busy()` with 30-second auto-clear timer per Clause 6.6.3.6 (was permanent until Router-Available)
- **Fixed** Router-Available-To-Network handler — now uses `mark_available()` per Clause 6.6.3.7
- **Fixed** Router-Busy/Available not re-broadcast to other ports — now re-broadcasts per Clause 6.6.3.6/7
- **Fixed** Reject-Message-To-Network removed routes — now differentiates by reason: reason 1 marks permanently unreachable (keeps entry), reason 2 marks busy with 30s timer per Clause 6.6.3.5
- **Fixed** unknown network message types silently dropped — now sends Reject with reason 3 (UNKNOWN_MESSAGE_TYPE) per Clause 6.6.3.5
- **Added** `busy_until: Option<Instant>` to `RouteEntry` for timestamp-based busy auto-clear
- **Added** `effective_reachability()` with inline deadline check (avoids 90-second worst-case from sweep granularity)
- **Added** `mark_busy()`, `mark_available()`, `mark_unreachable()`, `clear_expired_busy()` to `RouterTable`
- **Added** message-too-long framework — `max_apdu_length` captured per port for future size validation

#### Router — Route Management (Clause 6.6.3.2/3)
- **Fixed** I-Am-Router-To-Network not re-broadcast when no new routes learned — now re-broadcasts unconditionally per Clause 6.6.3.3
- **Fixed** anti-flapping logic blocked spec-required route updates from different ports — replaced `add_learned_stable` with `add_learned_with_flap_detection` that always accepts updates per Clause 6.6.3.2 ("last message wins") but logs rapid changes for operator visibility
- **Fixed** `touch()` never called — learned routes now refreshed on every route lookup during forwarding, preventing active routes from being purged by the 5-minute aging sweep
- **Added** flap detection fields (`flap_count`, `last_port_change`) to `RouteEntry` for observability

#### Router — Network Messages (Clause 6.4)
- **Added** Initialize-Routing-Table-Ack handler — learns routes from peer ACK responses per Clause 6.4.8
- **Added** Network-Number-Is handler — detects and logs network number conflicts per Clause 6.6.3.12
- **Added** explicit match arm for security messages (0x0A-0x11) — prevents incorrect rejection
- **Changed** Establish-Connection-To-Network log level from `debug` to `info` with "not implemented" note

#### BBMD (Annex J)
- **Fixed** BBMD not included in its own BDT — `ensure_self_in_bdt()` auto-inserts local BBMD entry on `set_bdt()` per J.4.2
- **Fixed** non-BBMD Forwarded-NPDU uses wrong source_mac — now uses originating address from frame (spec J.2.5) instead of UDP sender address; fixes cross-BBMD unicast for non-BBMD nodes
- **Fixed** non-BBMD silently drops Distribute-Broadcast-To-Network — now sends NAK (0x0060) per J.4.5
- **Fixed** Register-Foreign-Device with empty payload silently defaults TTL=0 — now validates payload >= 2 bytes and NAKs if short
- **Added** BDT persistence — optional file-backed persistence via `set_bdt_persist_path()` using BDT wire encoding (no serde dependency)
- **Improved** `forward_npdu` yields every 32 sends to avoid starving the recv loop with large FDT (up to 512 entries)

#### TSM (Clause 5.4)
- **Fixed** invoke ID leak on task cancellation — `TsmGuard` drop guard in `confirmed_request_inner` cleans up invoke IDs if the tokio task is aborted before normal completion

### Spec Compliance — Transport Layer (ASHRAE 135-2020 Clauses 7-9, Annexes J, U, AB)

Deep-dive review of all five transport implementations (BIP, BIPv6, BACnet/SC, Ethernet, MS/TP) identified 34 spec compliance issues. All addressed (31 fixed, 3 deferred as future features).

#### MS/TP — State Machine (Clause 9.5)
- **Fixed** IDLE state timeout used T_usage_timeout (20ms) instead of T_no_token (500ms) — node declared token lost 25x too quickly, causing premature token generation and bus collisions
- **Fixed** WAIT_FOR_REPLY did not transition to DONE_WITH_TOKEN on receiving a reply — added ~255ms unnecessary latency to every confirmed MS/TP request
- **Fixed** NoToken entry from PassToken timeout missing T_slot*TS per-station offset — multiple stations could simultaneously generate tokens
- **Fixed** no source address validation on reply frames in WAIT_FOR_REPLY — a frame from the wrong station could be incorrectly accepted as a reply
- **Fixed** ReplyPostponed frames (type 0x07) silently discarded — now transitions to DONE_WITH_TOKEN per Clause 9.5.6
- **Added** T_frame_abort tracking — discards partial frames when inter-byte gap exceeds 60 bit times per Clause 9.3
- **Added** `expected_reply_source` field to `MasterNode` for reply frame validation

#### BACnet/IPv6 — VMAC & Address Resolution (Annex U)
- **Fixed** Virtual-Address-Resolution wire format — was 10 bytes with duplicate VMAC payload, now 7 bytes per Clause U.2.7
- **Fixed** Virtual-Address-Resolution-ACK — now accepts and encodes requester's destination VMAC (10 bytes per Clause U.2.7A)
- **Fixed** `send_unicast` derived destination VMAC from IPv6 address bytes — now uses VMAC address table reverse lookup per Clause U.5
- **Fixed** decoder only extracted destination VMAC for OriginalUnicast — now also extracts for AddressResolution, AddressResolutionAck, VirtualAddressResolutionAck
- **Fixed** `derive_vmac_from_device_instance` did not mask to 22 bits per Clause H.7.2
- **Added** `VmacTable` — VMAC-to-address mapping with learn-on-receive from all incoming frames per Clause U.5
- **Added** Address-Resolution and Address-Resolution-ACK handlers in recv loop
- **Added** `Bip6BroadcastScope` enum — configurable broadcast multicast scope (LinkLocal/SiteLocal/OrganizationLocal), default SiteLocal
- **Added** `Bip6ForeignDeviceConfig` — foreign device registration with TTL/2 re-registration, Distribute-Broadcast-To-Network in FD mode
- **Added** BVLC-Result handling in recv loop with NAK logging

#### BACnet/SC — Client (Annex AB)
- **Fixed** HeartbeatAck included unnecessary originating/destination VMACs — now omitted per AB.2.11
- **Fixed** BVLC-Result parsing used payload-presence heuristic — now properly parses Result Code byte (0x00=ACK, 0x01=NAK) with error class/code extraction
- **Fixed** ConnectAccept message_id not verified against ConnectRequest — now rejects mismatched responses per AB.3.1.3
- **Fixed** `stop()` aborted task without DisconnectRequest — now sends DisconnectRequest with 2-second timeout before abort, clears shared state
- **Added** Device UUID parsing from ConnectAccept payload (bytes 6..22), stored as `hub_device_uuid`
- **Added** `build_heartbeat_ack()` method on `ScConnection` (extracted from inline recv loop construction)
- **Added** `pending_connect_message_id` field for response verification

#### BACnet/SC — Hub (Annex AB)
- **Fixed** ConnectRequest accepted with >= 6 bytes — now requires exactly 26 bytes per AB.2.9, NAKs short payloads with MESSAGE_INCOMPLETE
- **Fixed** pre-handshake messages silently dropped — now returns BVLC-Result NAK
- **Fixed** unknown function codes silently ignored — now returns BVLC-Result NAK
- **Fixed** broadcast relay was sequential — now parallel via `join_all` with per-client 5-second timeout
- **Added** per-client `max_npdu` tracking from ConnectRequest — oversized NPDUs rejected on unicast relay
- **Added** hub heartbeat initiation — periodic sweep (30s interval, 60s idle threshold) sends HeartbeatRequest to idle clients, removes clients on send failure
- **Added** `HubClient` struct with `sink`, `max_npdu`, `last_activity` fields
- **Added** `build_bvlc_result_nak()` helper for consistent NAK construction

#### BACnet/SC — TLS (Annex AB.7.4)
- **Changed** all `ClientConfig` builders to use `builder_with_protocol_versions(&[&TLS13])` — spec requires TLS 1.3

#### Ethernet — LLC Commands (Clause 7.1)
- **Added** XID and TEST command/response handling — Clause 7.1 "shall" requirement
- **Added** `build_xid_response()` and `build_test_response()` frame builders
- **Added** `check_llc_control()` helper for raw LLC control byte inspection
- **Changed** BPF filter widened to accept UI, XID, and TEST control bytes (was UI only)
- **Fixed** recv loop broke permanently on any error — now classifies transient (EAGAIN, EINTR, ENOBUFS) vs fatal errors

#### BIP — Foreign Device (Annex J)
- **Improved** BVLC-Result NAK handling — REGISTER_FOREIGN_DEVICE_NAK and DISTRIBUTE_BROADCAST_TO_NETWORK_NAK now logged at error level with specific messages

#### Cross-Cutting Transport Improvements
- **Fixed** BIP, BIP6, Ethernet `start()` leaked recv task and socket on double call — now returns error via `Option::take()` guard, matching SC/MS/TP/Loopback pattern
- **Fixed** MS/TP and SC used `Error::Encoding` for "transport not started" — now uses `Error::Transport(NotConnected)`, matching BIP/BIP6/Ethernet
- **Fixed** BIP, BIP6, Ethernet recv loops used `.await` on bounded channel send — now uses `try_send()` with warn log, preventing recv loop stall on slow consumers
- **Fixed** MS/TP `stop()` left node queue and state intact — now clears queue and resets state
- **Fixed** SC `stop()` left `ws_shared` and `connection` alive — now clears after disconnect
- **Added** named `NPDU_CHANNEL_CAPACITY` constants in all transports (256 for BIP/BIP6/Ethernet/Loopback, 64 for SC/MS/TP) with documented rationale
- **Changed** `bip6` module feature-gated behind `ipv6` feature flag — consistent with `ethernet` and `sc-tls` gating; propagated to bacnet-client, bacnet-java, bacnet-btl, bacnet-cli, benchmarks

### Spec Compliance — Stack-Wide (ASHRAE 135-2020 Clauses 5, 6, 12, 13, 15, 16, 20)

Deep-dive review of encoding, types, services, objects, client, server, and network layers identified 43 spec compliance issues. All critical/high/medium fixed.

#### Encoding & APDU (Clause 20)
- **Fixed** SegmentAck window size not clamped to 1-127 range on decode — now clamps with warning log per Clause 20.1.6
- **Fixed** reserved max_apdu values silently accepted — now logs warning for non-standard values

#### Types & Enums (Clause 21)
- **Fixed** LifeSafetyOperation enum ordering — reset=4, reset-alarm=5, reset-fault=6, unsilence=7 per Table 12-54
- **Added** LifeSafetyMode OEO values (15-19) per 135-2020 addendum
- **Added** DaysOfWeek bitflags type for schedule encoding
- **Added** 11 new BACnetPropertyStates variants (UnsignedValue, DoorAlarmState, Action, DoorSecuredStatus, DoorStatus, DoorValue, TimerState, TimerTransition, LiftCarDirection, LiftCarDoorCommand)

#### Services (Clauses 13-16)
- **Fixed** TextMessage tags — messagePriority and message use context tags [2] and [3] (were [3] and [4])
- **Fixed** ReinitializeDevice password validation — SIZE(1..20) per Clause 16.4.1.1.5
- **Added** `message_text: Option<String>` field to EventNotificationRequest with encode/decode per Clause 13.8.1
- **Added** `RecipientProcess` struct and `enrollment_filter` field to GetEnrollmentSummaryRequest

#### Objects (Clause 12)
- **Fixed** StatusFlags IN_ALARM never set — all 9 event-capable object types (AI/AO/AV/BI/BO/BV/MSI/MSO/MSV) now compute IN_ALARM from `event_detector.event_state`
- **Added** `compute_status_flags()` helper function for consistent StatusFlags computation across object types
- **Added** ValueSourceTracking fields (VALUE_SOURCE, LAST_COMMAND_TIME) to AV, BO, BV, MSO, MSV
- **Added** `set_overridden()` default method on BACnetObject trait

#### Client (Clause 5.4)
- **Fixed** per-window SegmentAck — tracks window position for correct sequence acknowledgment
- **Fixed** duplicate segment handling in segmented response reassembly
- **Fixed** negative SegmentAck uses `wrapping_sub(1)` for correct sequence arithmetic
- **Added** Abort on unsupported segmented response when `segmented_response_accepted` is false
- **Added** `segmented_response_accepted` parameter threading through dispatch_apdu/handle_segmented_complex_ack
- **Added** device table auto-purge every 5 minutes for stale entries

#### Server
- **Fixed** COV notification `ack_required` flag — `notify_type == NotifyType::ALARM` (was `!= ACK_NOTIFICATION`)
- **Fixed** DCC DISABLE now accepted — all 3 EnableDisable values work correctly per 135-2020
- **Fixed** COVProperty cancel now calls `unsubscribe_property()` instead of `unsubscribe()`
- **Fixed** RPM handler resolves device wildcard via `resolve_device_wildcard()`
- **Fixed** GetEnrollmentSummary priority lookup reads from notification class object (was hardcoded 0)
- **Fixed** intrinsic reporting silently non-functional — EVENT_ENABLE stored as BitString but read via `read_unsigned()`; added `read_event_enable()` helper handling both types
- **Fixed** schedule tick passes UTC offset parameter for correct time computation
- **Fixed** EventNotificationRequest now includes `message_text: None` field
- **Added** `days_to_date()` helper for full datetime in trend log records

#### Network (Clause 6)
- **Fixed** remote broadcast self-delivery — router now delivers broadcast to local network layer
- **Fixed** `is_network_message` passthrough in routing (was hardcoded false)
- **Fixed** proprietary network messages (type >= 0x80) with DNET now forwarded correctly
- **Fixed** Init-Routing-Table-Ack uses actual port_index (was hardcoded)

### Python Bindings Improvements

- **Rewritten** `.pyi` type stubs from scratch (826 → 1598 lines) — all 47 client methods, 62+ server methods, correct exception names, CovNotification class, PropertyValue constructors, all 65 ObjectType constants
- **Added** `time_synchronization()` and `utc_time_synchronization()` methods
- **Added** `who_is_directed()` for unicast WhoIs
- **Added** auto-routing methods: `read_property_from_device()`, `read_property_multiple_from_device()`, `write_property_to_device()`, `write_property_multiple_to_device()`
- **Added** `add_device()` for manual device table population
- **Added** `discover(timeout_ms)` convenience method — combines WhoIs + sleep + discovered_devices
- **Added** `PropertyValue.date()`, `.time()`, `.bit_string()`, `.list()` static constructors
- **Added** structured error attributes — `BacnetProtocolError.error_class`/`.error_code`, `BacnetRejectError.reason`, `BacnetAbortError.reason`
- **Added** `dcc_password` and `reinit_password` parameters to `BACnetServer` constructor

### Added
- **New crate: `bacnet-gateway`** — HTTP REST API and MCP (Model Context Protocol) server for BACnet networks
  - REST API at `/api/v1/` with endpoints for device discovery, property read/write, local object CRUD, and health check
  - MCP server at `/mcp` with 10 tools for LLM-driven BACnet interaction
  - MCP reference knowledge base — 9 static resources plus per-object-type drill-down templates
  - Pluggable authentication with bearer token, TOML configuration with CLI overrides
  - Feature-gated: `http`, `mcp`, `bin` — zero web deps by default
- **`LoopbackTransport`** in `bacnet-transport` — in-process transport for gateway client/server composition
- **RS-485 GPIO direction control** — `GpioDirectionPort<S>` wrapper with configurable `post_tx_delay_us`, kernel RS-485 ioctl on `TokioSerialPort`
- **Client batch operations** — `read_property_from_devices()`, `read_property_multiple_from_devices()`, `write_property_to_devices()` with `buffer_unordered(max_concurrent)` for concurrent multi-device I/O
- **Client auto-routing** — `resolve_device()` helper + `_from_device` variants for RP, RPM, WP, WPM
- **Server concurrent dispatch** — spawns per-request tasks for ConfirmedRequest/UnconfirmedRequest, enabling concurrent `db.read()` from multiple clients
- **Architecture documentation** — `docs/architecture.md`, expanded `docs/rust-api.md`, `docs/gateway.md`, `docs/btl.md`, `docs/wasm-api.md`

### Changed
- **Dependencies updated** — criterion 0.5→0.8, tokio-tungstenite 0.28→0.29, rand 0.9→0.10, rustyline 15→17, toml 0.8→1.0, rcgen 0.13→0.14, aws-lc-sys 0.38→0.39, rustls-webpki 0.103.9→0.103.10
- **Security advisories resolved** — aws-lc-sys X.509 name constraints bypass, CRL distribution point logic errors; rustls-webpki CRL scope check

### Removed
- **Java/Kotlin bindings** — removed `bacnet-java` crate, `uniffi-bindgen` crate, `java/` Gradle project, `examples/kotlin/`, and all associated CI jobs (no user base; maintenance burden)

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
