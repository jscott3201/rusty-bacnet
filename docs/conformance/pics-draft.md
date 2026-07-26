# Draft BACnet PICS Support Evidence

> DRAFT internal support evidence. Generated from `docs/conformance/bacnet-135-2020.json`; this is not a BTL certification claim or formal PICS/BIBB declaration.

This draft summarizes implementation evidence that may feed a future formal Protocol Implementation Conformance Statement. It intentionally stays below a certification claim.

## Data Link And Network Rows

| ID | Anchor | Status | Code Anchors |
|---|---|---|---|
| `BACNET-7-ETHERNET-LLC` | Clause 7 | implementation-present-needs-platform-tests | `crates/bacnet-transport/src/ethernet*`, `crates/bacnet-transport/Cargo.toml` |
| `BACNET-9-MSTP-FRAMES` | Clause 9.3 | implementation-present-needs-source-review | `crates/bacnet-transport/src/mstp_frame.rs`, `crates/bacnet-transport/src/mstp`, `crates/bacnet-transport/src/mstp/tests.rs` |
| `BACNET-J-BVLC-FUNCTION-CODES` | Annex J.2 | implementation-present-needs-conformance-tests | `crates/bacnet-transport/src/bvll.rs`, `crates/bacnet-transport/src/bip/mod.rs`, `crates/bacnet-types/src/enums/bvll.rs` |
| `BACNET-J-ORIGINAL-UNICAST-NPDU` | Annex J | implementation-present-needs-negative-tests | `crates/bacnet-transport/src/bvll.rs`, `crates/bacnet-transport/src/bip` |
| `BACNET-J-ORIGINAL-BROADCAST-NPDU` | Annex J | implementation-present-needs-negative-tests | `crates/bacnet-transport/src/bvll.rs`, `crates/bacnet-transport/src/bip` |
| `BACNET-J-FORWARDED-NPDU` | Annex J | implementation-present-needs-negative-tests | `crates/bacnet-transport/src/bvll.rs`, `crates/bacnet-transport/src/bbmd.rs`, `crates/bacnet-transport/src/bip/io.rs` |
| `BACNET-J-BBMD-BDT` | Annex J.4/J.5 | implementation-present-needs-conformance-tests | `crates/bacnet-transport/src/bbmd.rs`, `crates/bacnet-transport/src/bip/mod.rs`, `crates/bacnet-transport/src/bip/io.rs`, `crates/bacnet-cli/src/shell/bbmd.rs` |
| `BACNET-J-FOREIGN-DEVICE-FDT` | Annex J.5 | implementation-present-needs-conformance-tests | `crates/bacnet-transport/src/bbmd.rs`, `crates/bacnet-transport/src/bip/io.rs`, `crates/bacnet-cli/src/shell/bbmd.rs` |
| `BACNET-J-NAT-TRAVERSAL` | Annex J.7.5 | deferred-pending-owner-decision | `crates/bacnet-transport/src/bip/mod.rs`, `crates/bacnet-transport/src/bip/io.rs`, `crates/bacnet-transport/src/bbmd.rs`, `crates/bacnet-client/src/client/mod.rs`, `crates/bacnet-server/src/server/mod.rs`, `crates/bacnet-cli/src/transport.rs` |
| `BACNET-J-IP-MULTICAST` | Annex J.8 | deferred-pending-owner-decision | `crates/bacnet-transport/src/bip/mod.rs`, `crates/bacnet-transport/src/bip/io.rs`, `crates/bacnet-transport/src/bbmd.rs`, `crates/bacnet-transport/src/bip6/port.rs`, `crates/bacnet-cli/src/transport.rs`, `README.md`, `docs/rust-api.md` |
| `BACNET-U-IPV6-BVLL` | Annex U | implementation-present-needs-conformance-tests | `crates/bacnet-transport/src/bip6`, `crates/bacnet-types/src/enums/bvll.rs` |
| `BACNET-AB-SC-FRAME` | Annex AB.2 | implementation-present-needs-negative-tests | `crates/bacnet-transport/src/sc_frame.rs` |
| `BACNET-AB-SC-BVLC-RESULT` | Annex AB.2.4 | implementation-present-needs-conformance-tests | `crates/bacnet-transport/src/sc_frame.rs`, `crates/bacnet-transport/src/sc_frame/result.rs`, `crates/bacnet-transport/src/sc/mod.rs`, `crates/bacnet-transport/src/sc/result_tests.rs` |
| `BACNET-AB-SC-DATA-ATTRIBUTES` | Annex AB.3.4 | implementation-present-needs-conformance-tests | `crates/bacnet-transport/src/port.rs`, `crates/bacnet-transport/src/any.rs`, `crates/bacnet-transport/src/sc/data_attributes.rs`, `crates/bacnet-transport/src/sc/data_attribute_tests.rs`, `crates/bacnet-transport/src/sc/mod.rs`, `crates/bacnet-transport/src/sc/tests.rs`, `crates/bacnet-network/src/layer.rs`, `crates/bacnet-network/src/router/mod.rs`, `crates/bacnet-network/src/router/forwarding.rs`, `crates/bacnet-network/src/router/tests/data_attributes.rs` |
| `BACNET-AB-SC-CONNECTION-STATE` | Annex AB.6.2 | implementation-present-needs-state-machine-audit | `crates/bacnet-transport/src/sc/mod.rs`, `crates/bacnet-transport/src/sc/connect_result.rs`, `crates/bacnet-transport/src/sc/failover.rs`, `crates/bacnet-transport/src/sc/handshake.rs`, `crates/bacnet-transport/src/sc/random48.rs`, `crates/bacnet-transport/src/sc/primary_restore_tests.rs`, `crates/bacnet-transport/src/sc/result_tests.rs`, `crates/bacnet-transport/src/sc/receive_state_tests.rs`, `crates/bacnet-transport/src/sc/tests.rs`, `crates/bacnet-transport/src/sc_hub.rs`, `crates/bacnet-transport/src/sc_hub/tests.rs`, `crates/bacnet-types/src/enums/protocol.rs` |
| `BACNET-AB-SC-HUB-CONNECTOR` | Annex AB.5 | supported-with-clause-evidence | `crates/bacnet-transport/src/sc/failover.rs`, `crates/bacnet-transport/src/sc/mod.rs`, `crates/bacnet-transport/src/sc/handshake.rs`, `crates/bacnet-transport/src/sc_hub.rs`, `crates/bacnet-transport/src/sc_hub/relay.rs`, `crates/bacnet-transport/src/sc_tls.rs` |
| `BACNET-AB-SC-WEBSOCKET-TLS` | Annex AB.7 | implementation-present-needs-security-tests | `crates/bacnet-transport/src/sc_frame.rs`, `crates/bacnet-transport/src/sc_hub.rs`, `crates/bacnet-transport/src/sc_tls.rs`, `crates/rusty-bacnet/src/tls.rs`, `benchmarks/src/sc_helpers.rs`, `benchmarks/src/bin/bacnet_sc_hub.rs`, `benchmarks/src/bin/bacnet_device.rs` |
| `BACNET-AB-SC-HEARTBEAT` | Annex AB.6.3 | implementation-present-needs-timeout-tests | `crates/bacnet-transport/src/sc/mod.rs`, `crates/bacnet-transport/src/sc_frame.rs`, `crates/bacnet-transport/src/sc_hub.rs`, `crates/bacnet-transport/src/sc_hub/connection.rs`, `crates/bacnet-transport/src/sc_hub/heartbeat.rs` |

## PICS/Profile Rows

| ID | Anchor | Status | Notes |
|---|---|---|---|
| `BACNET-12-OBJECT-MODEL` | Clauses 12-19 | implementation-present-needs-conformance-tests | Initial family row only; later work should split high-claim services and object families into detailed rows. |
| `BACNET-A-PICS` | Annex A | in-progress | This ledger does not claim certification. |
| `BACNET-L-PROFILES` | Annex L | in-progress | No profile certification claim is made by this seed. |
