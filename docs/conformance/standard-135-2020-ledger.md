# BACnet Standard 135-2020 Conformance Ledger

> DRAFT internal support evidence. This ledger is not a BTL certification claim, a formal PICS, or a formal BIBB declaration.

Optional target Maximum_Send_Delay/Send_Now, bounded batching, captured historical
loss contexts and the cancellation-safe three-second drain are tracked by the
in-progress `BACNET-12-AUDIT-REPORTER-DELAY` row (#783). See
[the selected runtime contract](../delayed-target-audit.md). Captured failure-bit
filtering remains a qualified partial-profile policy; broader #345 stays open.

AV/BV object-owned policy (#781) extends the existing Audit row with independently
optional live properties and effective target filtering. Supported server
`write_local` shares the observer; physical Input sampling/raw DB authoring remain
separate. The object-specific absent/NULL priority inheritance interpretation
conflicts with generic §19.6.3 and is documented in the [Rust API](../rust-api.md#object-owned-avbv-audit-policy).
This does not promote the row or global evidence pins; #345/#782 remain open.

## Local APDU receive declaration

Refs #893: `BACNET-12-LOCAL-APDU-CAPACITY` records the bounded directional
transport/server contract. Clause12.11.18 supplies the raw Device declaration
and minimum 50; Clause16.10.3 carries that value in I-Am. Clause20.1.2.5 supplies
the six header codes. Startup clamps the configured ceiling against stable local
capacity and validates the selected Device before transport start. Both live
I-Am paths revalidate the current selection; queued broadcasts check at execution.
Confirmed COV, Audit and Event origins floor only their header field (raw 1474 to 1024).
Client canonical configuration and dynamic egress remain separate.

SC node Connect/Connect-Accept and intake use NPDU 1478 to carry local APDU 1476;
BVLC, remote, routed and Hub-forwarding limits remain independent. Evidence
includes actual transport Hub intake, both real-TLS direct directions,
asymmetric link controls, raw I-Am and decoded notification headers, plus the
binding-owned MS/TP Device constructor without serial I/O. This does not qualify
MS/TP hardware, all custom transports or complete Annex AB. Global review pins
and unrelated row statuses remain unchanged.

## Scope

- Standard: ANSI/ASHRAE Standard 135-2020.
- Reviewed at: 2026-09-17.
- Implementation evidence SHA reviewed: `b4c845caf920db279b0aefbd2824ac1348bba1cb`.
- Machine-readable source: `docs/conformance/bacnet-135-2020.json`.
- The SHA above identifies the last repository-wide review. Individual rows may cite later PR reviews that update narrower evidence.
- Current scope: RB-01 baseline reconciliation (R0) at dev b4c845c. The corrected-2020 target is ANSI/ASHRAE 135-2020 plus the 2024-04-29 Errata Summary for the supported subset. Owner-approved: (1) corrected 2020 baseline (optional later addenda and external qualification remain separate); (2) no CP authenticated-origin expansion — baseline keeps unknown-origin plus hardened denial; (3) MS/TP is non-routing standard-frame only (MAX_STANDARD_MPDU_DATA 501); extended-frame/COBS routing is not claimed. The Audit query corrected contract (filter BOOLEAN to BACnetSuccessFilter, cursor Unsigned32 to Unsigned64) is recorded in `BACNET-13-AUDIT-WIRE-MODELS` of the machine-readable ledger; the RB-02 codec migration and RB-20 runtime/Python migration are done, while #345 stays open for broader reporting/forwarding review. Per-feature status stays separate from any whole-product Protocol_Revision claim; no Protocol_Revision or workspace version change (workspace stays 0.11.0). Preserved prior owner decisions: #431 transport-neutral composition above sibling roles; #195 publish bacnet-cli in the next release (delivery gap remains open). No protocol code changes in this tranche.
- Addenda/errata status: ASHRAE 135-2020 Errata Summary 2024-04-29 (v1) reviewed for the supported subset. Item 7 (Clause 21.6, p. 886): successful-actions-only corrected from BOOLEAN (struck through, removed) to BACnetSuccessFilter (italic, added), tags [7]/[4]. Item 8 (Clause 21.2.3, p. 865): start-at-sequence-number corrected from Unsigned32 (struck through, removed) to Unsigned64 (italic, added), tag [2] OPTIONAL. Both items visually verified from the rendered errata p. 3 (strikeout = removed, italics = added per the p. 1 convention); not inferred from concatenated text extraction. The implementation encodes the corrected BACnetSuccessFilter/u64 contract after the RB-02 codec and RB-20 runtime/Python migrations; `BACNET-13-AUDIT-WIRE-MODELS` remains `implementation-present-needs-source-review` pending broader Audit review.
- PR-0808 evidence row: `BACNET-12-ALERT-ENROLLMENT-TABLE-12-61` is `supported-with-clause-evidence` for the served object model only; it is not an Alert evaluator or notification-generation claim.

## Multiple target Audit Reporters

Refs #782 extends the in-progress `BACNET-13-AUDIT-WIRE-MODELS` row with
1–64 configured target Reporters, enabled nominal lowest-instance election,
per-Reporter overlap health/loss contexts, and one global admission budget.
Live Rust configuration setters and aggregate updates capture changes atomically;
Python authors the plural set before startup. Supported network Reporter WRITE
attempts retain exact provenance and known failures; no-op/failure retention is
an explicit local policy. Mandatory self fallback does not create nominal overlap.
The [target contract](../target-audit-reporters.md) links behavioral evidence and
separates local policy from clause requirements. Source mode remains exactly one,
network configuration remains limited, and parent #345/global pins/statuses stay unchanged.

## Registered NORMAL B/IP Network Port

The `BACNET-12-REGISTERED-BIP-PORT` row covers explicit single-port association
(#785), independently of the configured snapshot row (#867). Clauses 15.5.2 and
15.7.2 select the receiving Network Port for wildcard instance 4194303; successful
ACKs use concrete identity under 15.5.1.2/15.7.3.2, including target Audit identities
under Tables 19-4/19-5. Full-server RP/RPM and endpoint RP have real UDP fixtures;
mixed RPM retains inline missing-registration errors and accessible properties.
[Routed fixtures](../../crates/bacnet-endpoint/src/registered_port_routed_tests.rs)
exercise both owners with an unrelated remote source network and full-server
request reassembly; the endpoint preserves its segmentation-not-supported refusal.

[Wire and capacity fixtures](../../crates/bacnet-endpoint/src/registered_port_wire_tests.rs)
cover post-bind port zero, selected versus unregistered/shared databases, multiple
configured rows, concrete/Device controls, successful Audit targets, valid exactly
1476-byte WP and independently sized RP ACKs through both owners. A separate local
BVLL fixture verifies the 1482-byte frame; these results do not establish blanket
oversized rejection. `APDU_Length` (399) remains independent of Device property 62.
[Lifetime fixtures](../../crates/bacnet-endpoint/src/registered_port_lifetime_tests.rs),
[final-network-owner coverage](../../crates/bacnet-server/src/server/network_port_tests.rs)
and [socket ownership coverage](../../crates/bacnet-transport/src/bip/registration_tests.rs)
exercise cancellation, failure, cleanup panic/retry, bare Drop and admitted reads.
[Installed Python coverage](../../crates/rusty-bacnet/tests/test_endpoint_bound_address.py)
checks explicit selection and active-only actual address reporting.

The instance range 1–255 is local policy. The transport must remain NORMAL B/IP,
with a concrete configured unicast interface; activation configuration is read-only
and bound Out_Of_Service/structural changes are refused.
[#875](https://github.com/jscott3201/rusty-bacnet/issues/875) delivered bounded
single-link nonrouter Network Number controls for the full server and endpoint
on NORMAL B/IP: answers when the number is known and valid local-broadcast
learning with configured-source precedence. See `BACNET-06-NONROUTER-NETWORK-NUMBER`
in the [evidence rows](support-summary.md#ledger-rows) and the
[control contract](../rust-api.md#local-network-number-controls).
This is not complete active Network Port profile conformance; #863 remains the
same-device multiport/router residual. #879 extended the controls, without a
registered port, to the other built-in links (sections below). Pending
configuration, rebind, BBMD or foreign-device registration authority, DHCP and
configured numbers on other links are not claimed. See the [Rust contract](../rust-api.md#registered-bip-network-port).
Global evidence pins remain unchanged.

## SC local nonrouter Network Number controls

The SC slice of #879 extends `BACNET-06-NONROUTER-NETWORK-NUMBER` to full
servers and shared endpoints using the existing owner and bounded control worker.
Clauses 6.4.19–6.4.20 and 6.5.2.2 admit local unicast or broadcast What-Is;
known owners reply by local broadcast. Only logical-broadcast NNI teaches state.
SC starts UNKNOWN without configured SC registration and never borrows another
Network Port object. Configured-source precedence and learned response flag zero
are unchanged. A direct What-Is replies through the Hub broadcast path.

[Constrained TLS tests](../../crates/bacnet-endpoint/tests/sc_network_numbers.rs)
cover both owners through `AnyTransport`, actual BVLC destination/payload bytes,
unknown/unicast/routed/malformed refusals and disconnected-Hub shutdown.
[Full-server gates](../../crates/bacnet-server/src/server/sc_network_number_tests.rs)
and [endpoint gates](../../crates/bacnet-endpoint/src/sc_network_number_tests.rs)
separately prove blocked single-writer behavior, control saturation/closure,
APDU handler and admitted Audit ACK progress, caller-owned Number cancellation,
stop cancellation and joined cleanup. They are not OS/TLS backpressure tests.
A started send may have reached wire; detached ordinary APDU ownership is unchanged.

An NPDU from a direct-connection peer is never logical group: it may query but
cannot teach. #863 multiport routing and #518 SC control-origin authorization
remain separate.
No SC configured Network Port, authenticated Hub-relayed leaf, whole Annex AB,
or complete Network Port profile is claimed. Global evidence pins stay unchanged.

## B/IPv6 full-server local Network Number controls

The IPv6 slice of #879 opts normal and configured foreign-device full servers
into the same bounded nonrouter owner. Both start UNKNOWN without borrowing an
unrelated Network Port object. Normal replies are OriginalBroadcast NNI on the
selected link; foreign replies are DBTN to the configured BBMD. A trusted
Forwarded-NPDU is a logical broadcast even on a unicast UDP hop. Existing Annex U
selected-link, VMAC, destination/interface and configured-BBMD admission remain.

[External Rust wire tests](../../crates/bacnet-integration-tests/tests/ipv6_network_numbers.rs)
cover UNKNOWN, both query delivery forms, learned flag zero, configured-source
precedence, unicast/routed/malformed/invalid refusals, wrong-BBMD refusal and
stop/port reuse. Normal mode also reconstructs UNKNOWN on the same port.
The explicit `bacnet-integration-tests/ipv6` feature and ignored-test invocation
are required on an isolated IPv6 link; ordinary CI does not execute these tests.
[Installed Python qualification](../../crates/rusty-bacnet/tests/qualifications/ipv6_network_numbers.py)
uses the public full server and actual multicast intake/output on the same Linux
topology. An independent observer container captures response bytes; no duplicate
filter compensates for sender-namespace multicast reflection. These wire tests
are separate from the unchanged deterministic owner lifecycle gates above.

No IPv6 endpoint builder, number setter, configured IPv6 Network Port, Python
foreign-device API or complete Annex U/Network Port profile is claimed. The
other links have their own slices, summarised below. Global evidence pins stay
unchanged.

## Local Network Number owners per link (#879)

#879 is complete for every built-in nonrouter link. Each transport opts in
through `TransportPort::supports_local_nonrouter_number_controls`; the full
server, shared endpoint and standalone client consume that one capability and
enable the network layer's single pre-start control receiver, with no
per-transport branch. Only the in-process `LoopbackTransport` keeps the false
default. No owner sends a startup announcement: Clause 6.4.20 asks that of
configured routers. Clause 6.4.19 lets a query arrive by local unicast or
broadcast; Clause 6.4.20 lets only a local broadcast teach.

| Link | Owners | Logical broadcast on ingress | Reply egress | Evidence label |
|---|---|---|---|---|
| B/IP NORMAL | server, endpoint, client | Original-Broadcast; Forwarded-NPDU from any UDP sender (compatibility policy) | Original-Broadcast | real UDP loopback; a registered port supplies the configured number |
| B/IP BBMD | server, endpoint, client | Original-Broadcast, Forwarded-NPDU from a BDT peer, DBTN from a registered foreign device | Original-Broadcast, also forwarded to BDT/FDT (#937) | real UDP loopback; broadcast capture on Linux |
| B/IP foreign device | server, endpoint, client | Forwarded-NPDU from any UDP sender (compatibility policy) | DBTN to the configured BBMD | real UDP loopback |
| B/IPv6 normal | server, client | multicast Original-Broadcast or Forwarded-NPDU | multicast Original-Broadcast on the selected link | isolated Linux IPv6 link, ignored in ordinary CI; installed Python qualification |
| B/IPv6 foreign device | server, client | Forwarded-NPDU from the configured BBMD | DBTN to that BBMD | isolated Linux IPv6 link, ignored in ordinary CI |
| BACnet/SC | server, endpoint, client | Hub-relayed broadcast VMAC only | Hub broadcast, including for direct queries | real constrained-TLS Hub and direct peers |
| MS/TP | server, endpoint, client | frames to station 255 | DataNotExpectingReply to station 255 | LoopbackSerial simulator in both execution modes; no RS-485 timing |
| Linux Ethernet | server, client | all-FF destination | all-FF LLC frame | two-container NET_RAW virtual link, opt-in; no physical LAN |

There is no B/IPv6 or Ethernet endpoint builder and no Python Ethernet or B/IPv6
foreign-device API. Shared-endpoint BBMD/foreign modes remain experimental.
Configured numbers exist only through a registered NORMAL B/IP Network Port;
every other link starts UNKNOWN.

## Endpoint WriteProperty source WRITE

Refs #852 extends the bounded source profile under the still-in-progress #345
tracker. Evidence: `crates/bacnet-endpoint/src/source_write_tests.rs`,
`source_write_preflight_tests.rs`, `source_write_lifecycle_tests.rs`,
`source_write_queue_tests.rs` and installed `test_endpoint_write_property.py`.
Clause 15.9, 19.2.1 and 19.6/Table 19-4/19-5 govern the direct B/IP request,
required caller-declared commandability, omitted effective priority 16,
source filtering, captured identity/result and whole 0–32-byte Target_Value.
Tests decode real request/notification traffic, peer terminals, empty Recipient_List
success and scalar Error, 32/33 bounds, framing/prewire refusal, retries, caller
cancellation, stop/drop, recipient changes and shared resource budgets. Additional
regressions require matching Error service choice in requester and notification
leases, and retract canceled no-Reporter writes both queued and in transport
while preserving audited-worker and explicitly detached-send ownership. One shared
requester and one SourceAudit owner serve RP/RR/RPM/WP; read-family semantics are
retained. WPM, routed writes, other transport source profiles, standalone ownership
and full Audit/BIBB/BTL qualification remain unclaimed.

## Endpoint RPM source READ

Refs #780 extends the in-progress `BACNET-19-SOURCE-READ-PROPERTY` row with
Rust/Python endpoint RPM. Its profile is 1–64 explicit references on concrete
objects, with nonempty lists and unsegmented request/ACK bounds. Index zero is
valid; inline errors may omit the requested index, while successful values must
echo it. Complete ordered correlation precedes per-occurrence value-free records.
One operation shares its lease, worker, timestamp and recipient snapshot; whole
failures fan out as a documented local representation of attempted references.
Only a unique successful Device identity establishes operation-local Target Device.

[Wire/lifecycle coverage](../../crates/bacnet-endpoint/src/source_rpm_tests.rs),
[bounded congestion coverage](../../crates/bacnet-endpoint/src/source_rpm_congestion_tests.rs)
and [installed Python coverage](../../crates/rusty-bacnet/tests/test_endpoint_rpm.py)
support this subset. Typed synchronous QueueFull is known local resource loss;
a full-queue summary retains its count in the existing bounded coalescer until
capacity returns. Closed/shutdown and ambiguous attempted delivery remain separate.
Clause15.7 (printed742–744/PDF744–746), Table19-5 (printed823/PDF825), and High Volume
(printed825/PDF827) ground these distinctions. The server scalar-error index
producer fix remains #789; broader Audit #345 stays open. Global pins and row
statuses remain unchanged.

## ReadProperty ACK identity and source attribution

Refs #784 extends the in-progress `BACNET-19-SOURCE-READ-PROPERTY` evidence.
Clause 15.5.1.2 (printed739/PDF741) and 15.5.2 (printed740/PDF742) ground one
shared standalone direct/routed and endpoint ACK correlation rule. Property and
array index match exactly; Device/Network Port instance4194303 aliases accept a
same-type concrete peer-reported object. [Client wire tests](../../crates/bacnet-endpoint/tests/read_property_correlation.rs)
include the bundled server's Device alias and controlled Network Port replies.
The registered receiving-port mapping is documented below (#785).

[Source wire tests](../../crates/bacnet-endpoint/src/source_property_identity_tests.rs)
prove concrete successful Target Object attribution. A validated concrete Device
ACK also establishes Target Device for that operation (RP or RR), per Table 19-4
(printed822/PDF824). Other object successes and failures retain the direct
address. Without a valid ACK, preserving the requested object/alias is the local
representation of attempted identity; no remote instance is inferred. Records
stay value-free, with one record across canceled retries and the original
recipient snapshot. No discovery cache or broader Audit support is added.
[Installed Python coverage](../../crates/rusty-bacnet/tests/test_read_property_correlation.py)
checks both client surfaces. Row status and global evidence pins are unchanged.

## Endpoint ReadRange source READ

Refs #771 extends `BACNET-19-SOURCE-READ-PROPERTY` without changing its in-progress
status or global evidence pins. Clause15.8 (printed745–748/PDF747–750) and
19.6.5/Table19-5 (printed823/PDF825) ground one value-free source READ record for
one attempted object/property range request, independent of returned item count.
[Real B/IP tests](../../crates/bacnet-endpoint/src/source_range_tests.rs) cover all
Rust range forms, empty/multiple-item ACKs, both delivery modes, exact terminal
failures, segmented refusal, cancellation/retries, recipient snapshots and Drop.
The shared RP/RR core preserves the existing operation/lease/notification owners.

[Transactional codec tests](../../crates/bacnet-services/src/read_range_validation_tests.rs)
reject selectors, index zero, invalid counts and nonconcrete ByTime components
before output. Zero position/sequence references remain valid. Concrete date/time
component bounds reuse the decoder; no extra calendar/weekday rule is imposed.
[Installed Python tests](../../crates/rusty-bacnet/tests/test_endpoint_read_range.py)
cover the actual shared EndpointClient/standalone API and typed raw-byte ACK shape.
Python supports all-items/position/sequence, not ByTime or source Reporter setup.
Endpoint responses remain unsegmented; Audit reporting remains direct B/IP only.
Source RPM/WP and wider Audit work remain open under #345.

## Python standalone mutation policy

`BACNET-LOCAL-MUTATION-POLICY` records the bounded #768 binding evidence, with
`in-progress` status and no global pin or broader conformance promotion.
The constructor selects the native `Permissive`/`DenyAll` authority before I/O;
[installed B/IP tests](../../crates/rusty-bacnet/tests/test_mutation_policy.py)
cover representative property/object/list/file/COV decisions, exact denials,
state preservation, permissive controls, reads and trusted local writes.
The [native matrix](../../crates/bacnet-server/src/server/requests/mutation_policy_tests.rs)
owns exhaustive ten-service/authorizer/counter coverage. This is local operator
policy, not certificate-principal authorization; DCC, ReinitializeDevice,
LifeSafety, Audit and endpoint Device writes remain separate. See
[the policy contract](../mutation-policy.md); #524 remains open.

## Hub reciprocal WebSocket Close

Scoped `BACNET-AB-SC-WEBSOCKET-TLS` evidence, Refs #776: an observed peer Close
uses the existing connection lease to flush Tungstenite's queued reciprocal
frame. [Real mutual-TLS tests](../../crates/bacnet-transport/src/sc_hub/peer_close_tests.rs)
assert the allowed code/reason before EOF for registered, upgraded pre-Connect,
and graceful Disconnect-Ack-wait peers. Close without a Disconnect-Ack keeps the
shutdown outcome forced. Retirement and identity-checked registry removal precede
the existing local five-second sink acquisition/flush bound; tests cover held
sinks, capacity recovery, replacement safety and canceled forceful-stop joining.
Forceful abort can forgo the reply. This covers AB.7.5.5 (PDF1412/printed1410)
and RFC6455 sections5.5.1/7.1.2 only for these paths; it makes no TLS
`close_notify`, complete close-status mapping or broader Annex AB claim.
The row status and global evidence pins remain unchanged.

## Hub transit relay budget

Scoped `BACNET-AB-SC-CONNECTION-STATE` evidence, Refs #774 under #476.
One validated `relay_send_budget` / Python `relay_send_budget_ms` setting replaces
the pre-1.0 unicast-only API without aliases. The default remains five seconds;
positive whole-millisecond and monotonic representation checks precede file I/O
or bind. It bounds acquisition plus send for NPDU/opaque unicast, each concurrent
broadcast recipient, and forwarded BVLC-Result. Probe, control, cleanup and
graceful-shutdown policy remain distinct. Timeouts do not retire, retry or
fabricate a Result; buffered bytes cannot be retracted. Captured-registration
retirement still protects replacements. Outcome counters remain unicast-only,
with their existing broadcast/Result exclusions.

[Real TLS/paused-clock tests](../../crates/bacnet-transport/src/sc_hub/relay_budget_tests.rs)
prove healthy fanout while two recipients are blocked, one concurrent configured
budget, source progress, no replay, retained peers and later delivery; addressed
Result and replacement-under-held-sink cases use the same runtime paths.
The same module establishes sixteen peers and closes them concurrently, verifies
registration/resource reclamation, recovers capacity and joins Hub shutdown.
Existing sequential stress coverage remains. [Installed Python tests](../../crates/rusty-bacnet/tests/test_sc_hub_probe_policy.py)
exercise the renamed setting with all transit wire families; Rust owns the
deterministic held-sink deadline proof. Signature/stub and invalid-before-I/O
tests migrate together. Global pins/statuses remain unchanged; parent #476
acceptance is reconciled separately, with no broader Annex AB claim here.

## Hub outcome status

Scoped `BACNET-AB-SC-CONNECTION-STATE` evidence, Refs #770 under #476.
Rust and Python expose one fixed, redacted, saturating per-start outcome snapshot.
Registration counts selected collision/capacity refusals and committed UUID
replacement; ordered accept limits and TLS/WebSocket/Connect timeouts count at
their actual decision points. Eligible NPDU/opaque unicast counts missing targets,
length limits, send timeout/error; malformed, pre-registration, stale-source,
self/local, broadcast and forwarded Result paths are excluded. There is no
successful-send inference from retired-sink skips. Heartbeat counts only actual
matching-registration removal, including generation exhaustion. Counters do not
affect policy and independent field reads are not transactional.

Evidence: [TLS outcome cases](../../crates/bacnet-transport/src/sc_hub/outcome_tests.rs),
[deadline/admission ordering](../../crates/bacnet-transport/src/sc_hub/deadline_commit_tests.rs),
[heartbeat races](../../crates/bacnet-transport/src/sc_hub/heartbeat_generation_tests.rs),
[actual established-peer same-address/config restart](../../crates/bacnet-transport/src/sc_hub/graceful_tests.rs),
and [installed Python outcomes](../../crates/rusty-bacnet/tests/test_sc_hub_conflict_admission.py).
Existing stress/cleanup coverage and Rust post-stop snapshots remain. Python
status still raises before start/after stop; no new lifecycle or shutdown count
is implied. Global review pins and row status remain unchanged. #476 acceptance is reconciled separately; unified relay-budget evidence is
recorded above. No broader
conformance or certificate-principal authorization claim is added.

## Hub operator timing and broadcast policy

Refs #769 under #476. The accepting Hub's optional outbound probe is a local
liveness policy. Base 135-2020 AB.6.3 (PDF1407 / printed1405) assigns idle
Heartbeat-Request initiation and configurable 3–300s timing to the initiating
peer; the accepting peer responds with ACK. The local Hub probe does not replace
that node contract or establish broader Annex AB conformance.

[One monotonic timing owner](../../crates/bacnet-transport/src/sc_hub/timing.rs)
provides checked millisecond scan/idle/ACK/send policy (defaults 30/60/5/5s).
Idle and ACK ages must be strictly exceeded at a scan; pending age starts at
reservation. Sequential sends can delay later scans; missed ticks are skipped,
not replayed. Matching ACK/activity and pending clearing share the registry lock;
wrong-ID/invalid ACKs cannot refresh activity. The public configuration also sets
one separate transit relay acquisition-plus-send budget (default5s), without
retry, fabricated Result, or timeout-only retirement. It includes NPDU/opaque
unicast, each concurrent broadcast recipient, and forwarded Result. Initiating-node
heartbeat and Hub probe/control/cleanup/graceful policies remain separate.

[Paused-clock real TLS tests](../../crates/bacnet-transport/src/sc_hub/probe_tests.rs)
cover exact strict age boundaries, held-sink budgets, missed-scan behavior, and
forceful/graceful task cleanup. [Unicast tests](../../crates/bacnet-transport/src/sc_hub/unicast_deadline_tests.rs)
exercise default and custom budgets through public configuration. Existing ACK,
generation, and replacement regressions remain. [Installed Python tests](../../crates/rusty-bacnet/tests/test_sc_hub_probe_policy.py)
observe custom probes and the existing native sender/global rate drops with exact
wire/count reconciliation; constructor tests cover invalid bounds before I/O.
Representation bounds are local policy, not BACnet-specified probe ranges.
Global pins and row status remain unchanged; see the outcome evidence above. #476 remains open.

## Hub conflict-aware admission

Scoped evidence for `BACNET-AB-SC-CONNECTION-STATE`, Refs #767 under #476.
Base 135-2020 AB.6.2.3 (PDF 1406 / printed 1404) requires accepting a known
Device UUID and closing its incumbent. That remains the default. Explicit
operator refusal is a local security policy before protocol acceptance, not a
claim that the standard requires duplicate-UUID rejection.

The existing locked registration decision supplies one fixed classification to
Rust admission policy: initial, same UUID/same VMAC, same UUID/moved VMAC, or a
VMAC owned by a different UUID. No incumbent identity or certificate fields are
added. Python's static `deny_uuid_replacement` mode uses that same authority;
no Python callback runs under the registry lock. Denial preserves the incumbent
and uses the existing RESOURCES/OTHER NAK/admin-denial counter. Standard collision
and capacity rules still follow Allow; the original commit deadline, panic-deny,
retirement and shutdown ownership are retained.

Evidence: [real TLS conflict and concurrent admission](../../crates/bacnet-transport/src/sc_hub/conflict_admission_tests.rs),
[installed Python default/refusal modes](../../crates/rusty-bacnet/tests/test_sc_hub_conflict_admission.py),
and [constructor rejection before I/O](../../crates/rusty-bacnet/tests/test_sc_hub_lifecycle.py).
Global review pins and row status are unchanged. Fixed-shape outcomes are documented above; certificate-principal authorization and
broader Annex AB qualification are not claimed.

## Target Device Audit recipient

The `BACNET-13-AUDIT-WIRE-MODELS` row records the target-only portion of #728.
The corrected Device requirement uses 135-2020 §12.11 and the 2024-04-29 errata
item 25 (PDF page 7): the recipient is required/writable when Audit Reporting is
supported; Device Audit_Level and Auditable_Operations remain optional.
[The implemented contract](../device-audit-recipient.md) covers typed initial
provision, active metadata, local/direct/WP/WPM old/new admission, route limits,
shared sequence and health generations, and ownership through shutdown quiescence.
Mandatory change delivery at Audit_Level NONE or with ordinary WRITE disabled is
an explicit interpretation of the specific property requirement, not an assertion
that the general filter text unambiguously settles precedence.

Evidence includes [recipient runtime tests](../../crates/bacnet-server/src/server/audit_recipient_tests.rs),
[real B/IP delivery](../../crates/bacnet-integration-tests/tests/audit_reporter/device_recipient.rs),
and [installed Python contracts](../../crates/rusty-bacnet/tests/test_audit_api.py).
Endpoint source evidence includes [public local recipient delivery](../../crates/bacnet-endpoint/tests/source_recipient_public.rs)
and [startup cleanup cancellation](../../crates/bacnet-endpoint/src/source_start_cleanup_tests.rs).
The target and endpoint source migrations complete the bounded #728 recipient
contract. Row status and the global review pin remain unchanged. Other Address/link
choices, durable delivery and broader Audit review remain open under #345.

## Node Address-Resolution accepting capability

Scoped correction to `BACNET-AB-SC-CONNECTION-STATE` (Refs #733). Base
135-2020 AB.3.3 (printed 1395 / PDF 1397) requires
COMMUNICATION/OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (`7/45`) from a node that
cannot take inbound direct connections. An accepting node may ACK an empty
URI list. The 2024-04-29 errata makes no relevant correction.

[Node admission](../../crates/bacnet-transport/src/sc/address_resolution.rs)
uses the same current-live capability as Advertisement: registered listener,
matching VMAC/UUID, running listener and open direct/application NPDU intakes.
Using current availability for stopped listeners is **local policy**, not an
additional normative unsupported-implementation rule. URI configuration alone
never grants capability; a later stop cannot recall an already-built ACK.
Replies retain copied IDs, origin-addressed/peer-addressed envelopes and response
silence. Denial precedes activity refresh/probe clear and uses the existing
remaining heartbeat rejection budget and expiry retirement. Positive ACK sends
retain their existing best-effort behavior.

[Real listener tests](../../crates/bacnet-transport/src/sc/address_resolution_capability_tests.rs)
cover empty/known URIs, IDs, addressing, identity mismatch and stopped/dropped
listener or application intake. Raw no-listener, malformed and response-silence
matrices remain separate. Direct discovery interoperability now registers real
accepting listeners. Existing held/late/immediate-error rejection tests include
capability denial. [Installed-native vectors](../../crates/rusty-bacnet/tests/test_sc_unknown_function.py)
retain exact replies, strict invalid-heartbeat ordering barriers and independent
later ReadProperty/heartbeat controls. No full AB.3.3/Annex AB/PICS/BTL or
platform/OS-backpressure qualification is claimed; row status and immutable
baseline provenance remain unchanged.

## Hub Address-Resolution transit

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
#519 remains open/partial; 68 rows /19 supported rows, statuses, global provenance
and #517 A1–A6 remain unchanged. This explicit selection supersedes only the
Address-Resolution hub-transit exclusion in the earlier slice-time supplements.
Their NODE and other known-family exclusions remain, including Advertisement
`0x04`, Advertisement-Solicitation `0x05` and Proprietary `0x0C`.

| Contract / source | Implementation | Evidence |
|---|---|---|
| Base Standard 135-2020 AB.2.6/.1 (PDF1389–1390 / printed1387–1388) and AB.2.7/.1 (PDF1390 / printed1388): Request and ACK are unicast; ACK responds to Request and its URI list may use zero bytes. AB.2 (PDF1385 / printed1383) forbids responding to broadcasts/responses. | [Hub handler](../../crates/bacnet-transport/src/sc_hub/handler.rs) admits only registered, origin-absent, destination-present unicast `0x02/0x03` before activity. Broadcast/self/explicit-origin transit is silent. Every nonforwardable ACK is silent, including pre-registration and peer-local ACK. | [Raw mTLS tests](../../crates/bacnet-transport/src/sc_hub/resolution_transit_tests.rs): compiled empty Request/ACK RED/GREEN, zero/max IDs, 48 raw options/body vectors, 192 preregistered and 168 registered local/rejected cases with fresh two-variant fixtures, ordered no-echo barriers and complete snapshots. |
| AB.5.1 (PDF1400 / printed1398), AB.5.3.2 (PDF1402 / printed1400): matching current peer only, no self echo, registered source stamp and destination removal. | [Private opaque relay](../../crates/bacnet-transport/src/sc_hub/opaque_relay.rs) extracts the existing Unknown mechanics without widening caller admission. Function/ID/options/body bytes survive exactly; no URI parser or endpoint validation. Only final encoded recipient BVLC limits apply, not Max-NPDU. Missing/zero-unmapped targets silently drop. | Exact cap/+1, ingress 5705/5706, ACK URI-shaped body over1497 with Max-NPDU=1; MU/MoreOptions/empty HeaderData preservation. Extra Request payload, Data Options and non-UTF8 are opaque-transit tests, **not endpoint format conformance**. |
| AB.3.1.1–.3 (PDF1395 / printed1393) and AB.2.4.1 (PDF1389 / printed1387): response address/ID, no Result-on-Result, marker zero for non-option diagnostic. | [Local policy](../../crates/bacnet-transport/src/sc_hub/resolution_transit.rs) retains eligible Request 7/150 (`UNEXPECTED_DATA`), original ResultFor2/ID, marker zero, no options/detail, destination=valid origin or absent, same socket only. Broadcast/reserved origins suppress it; pre-registration explicit unicast never transits/commits. This unsupported hub-local profile is **not AB.3.3 node behavior** or full field-validation/diagnostic-priority conformance. | Local/preregistration address/options matrices distinguish 7/150 from Unknown 7/143; ACK never generates Result. Existing immediate write errors remain ignored/continue. |
| AB.2 response rule and guarded Result return. | [Result relay](../../crates/bacnet-transport/src/sc_hub/relay.rs) relays ResultFor2 and ResultFor3 ACK/NAK under destination-match unicast with stamped origin and no self echo (AB.5.1/AB.5.3.2); 0x03 answers the AR initiator as a unicast response (AB.2.7) via the hub (AB.4.1). Existing syntax/DataOptions/address/registration/ownership/BVLC/5s/retirement guards remain. New-family no-self-echo leaves unrelated EncapsulatedNpdu self behavior unchanged. | 192 preregistered and 192 registered Result cases, detailed UTF-8 NAK/raw option preservation, malformed/DataOptions suppression and exact cap/+1. Earlier Unknown/all-other-known characterizations remain. |

- **Scoped local activity/lifecycle policy:** local/rejected Request and ACK do not
  refresh activity or clear/reseed probes, mutate identity/limits/registry or extend
  deadlines. Accepted transit follows existing NPDU/Unknown activity, including
  missing/capped recipient drops; self drops precede activity. Pending ACK timeout
  was already independent, not an ACK-evasion fix. [Lifecycle tests](../../crates/bacnet-transport/src/sc_hub/resolution_transit_lifecycle_tests.rs)
  cover held local Request NAK, Request/ACK/ResultFor2 sends, source/target
  retirement and replacement, healthy owners, stop/join, original absolute 5s
  preregistration deadline and capacity release. No new timers or write budgets.
- **Installed native DEV:** [public native hub + raw mTLS A/B](../../crates/rusty-bacnet/tests/test_sc_hub_resolution_transit.py)
  checks Request→ACK (including empty URI list), ResultFor2 ACK/NAK, exact origin
  bytes, silence/caps and real native third-peer ReadProperty before/after. Raw B
  owns AR endpoint semantics; the native NODE answers valid AR-Requests with its configured-or-empty ACK only while accepting direct connections; otherwise it returns 7/45 (Refs #733).
- **Excluded:** AB.3.3 node URI response/unsupported-optionality semantics and
  AB.4.1 direct connections (PDF1396–1397 / printed1394–1395), URI discovery,
  validation, dialing, general known-function
  forwarding, support promotion and full Annex AB claims. Authenticated peers only;
  cancellation is not rollback, logical retirement is not OS closure, held-sink
  evidence is not OS backpressure. Cooperative runtime/state-lock assumptions,
  no native 60s expiry, no preauthentication/MITM or release qualification remain.

## Hub Unknown transit and Result return

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
#519 remains open/partial; 68 rows /19 supported rows, all statuses, global
provenance, historical supplements and #517 A1–A6 remain unchanged. This slice
selects only the Unknown family previously excluded from the node supplement.

| Contract / source | Implementation | Evidence |
|---|---|---|
| Base Standard 135-2020 AB.5.1 (PDF1400 / printed1398), AB.5.3/.1 (PDF1401 / printed1399), AB.5.3.2/.3 (PDF1402 / printed1400): independent hub endpoint, matching-peer unicast or all-other-peer broadcast, no source echo, stamp origin and remove unicast/retain broadcast destination. AB.5.4 (PDF1403 / printed1401) describes the sending connector envelope; its receiving explicit-destination drop is a NODE rule. | [Hub handler](../../crates/bacnet-transport/src/sc_hub/handler.rs) classifies registered Unknown `0x0D..0xFF` with absent origin and present destination before activity. [Opaque relay](../../crates/bacnet-transport/src/sc_hub/opaque_relay.rs) reuses raw wire transformation, recipient capture and retirement-aware sends; missing/self targets are silent. | [Independent mTLS raw vectors](../../crates/bacnet-transport/src/sc_hub/unknown_transit_tests.rs): compiled RED/GREEN, all 243 codes, unicast/broadcast to two recipients, IDs zero/max, empty/nonempty bodies, MU/MoreOptions/empty HeaderData preserved exactly, no source echo. |
| AB.3.1.5 (PDF1396 / printed1394), AB.3.1.2/.3 (PDF1395 / printed1393), AB.2 and AB.2.4.1: unknown local unicast NAK/discard, original raw ResultFor/ID, response address and marker zero; no broadcast response. | Destination-absent registered Unknown is local; pre-registration never transits, including explicit unicast destinations. Eligible 7/143 NAK has destination=valid origin or absent, no options/detail, same socket only. Explicit-origin registered transit, broadcast local rejection and reserved origins are silent. Unknown diagnostic priority and reserved-origin suppression are owner-local policy. | Address/options/body matrices, ordered no-fanout barriers and complete snapshots; pre-registration cannot commit/route to another peer and later valid Connect still works. Known fallback stays 7/150. |
| Unknown body is opaque, not an NPDU; recipient encoded BVLC cap remains applicable. | No NPDU interpretation or Max-NPDU cap. Existing ingress cap remains; final wire length includes broadcast origin insertion. | Exact cap and cap+1, ingress 5705/5706, body over1497 with peer Max-NPDU=1, unicast and broadcast selective drops; Result BVLC cap exact/+1. |
| AB.3.1.1/.2/.3: no Result-on-Result, response address and original ID. | [Result relay](../../crates/bacnet-transport/src/sc_hub/relay.rs) widens only ResultFor EncapsulatedNpdu to also Unknown; existing malformed/address/registration/ownership/limit/timeout guards remain, with Unknown-only self-drop. | Exact ACK and detailed UTF-8 NAK/options; invalid Result, broadcast, spoofed origin, missing/unknown/self target, pre-registration and other known ResultFor silence. |

- **Activity and lifecycle are scoped local policy:** local/rejected/pre-registration
  Unknown never refreshes activity. Accepted transit follows the existing NPDU
  rule even for missing/oversized recipient drops; self-target drops occur before
  activity. Pending heartbeat identity/deadline is never cleared or reseeded.
  The fixed behavior is idle-probe deferral: pending ACK timeout was already
  independent of activity, not an ACK-timeout-evasion fix or universal AB.6.3 rule.
  [mTLS lifecycle tests](../../crates/bacnet-transport/src/sc_hub/unknown_transit_lifecycle_tests.rs)
  cover held local NAK/transit/ACK/NAK return, source and target replacement or
  retirement, healthy owners, joined stop, and held pre-registration NAK expiry
  under the original absolute Connect deadline with capacity release/recovery.
  Immediate local NAK send errors retain ignored-error/continue behavior. No new
  global NAK deadline; broadcast keeps per-target 5s waits, unicast existing
  retirement, Result its existing 5s wait.
- **Installed native DEV:** [raw peer → public native hub → real native NODE → raw peer](../../crates/rusty-bacnet/tests/test_sc_hub_unknown_transit.py)
  asserts the NAK origin is the NODE, not a hub-local fallback, plus original
  function/ID, native broadcast silence, raw fanout/options/caps and healthy
  ReadProperty before and after. NODE production and its ResultForUnknown fatal
  policy remain unchanged.
- **Limits:** authenticated TLS peers only. Cancellation is not rollback or
  immediate physical closure; held-sink evidence is not OS backpressure. Existing
  cooperative-runtime/state-lock assumptions remain. No native 60-second expiry,
  preauthentication/MITM, release qualification or full Annex AB claim. General
  known-function forwarding (Address-Resolution/Advertisement/Proprietary),
  known-function liveness, all-write/rate policy, capacity/API/clock changes,
  graceful shutdown and support promotion remain excluded.

## Node unknown-function admission

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
#519 remains open/partial; 68 rows /19 supported rows, statuses, global provenance,
historical evidence and #517 A1–A6 remain unchanged. Only established NODE receive
admission is changed; hub fallback/forwarding and handshake remain excluded.

| Contract / source | Implementation | Evidence |
|---|---|---|
| Base Standard 135-2020 AB.3.1.5 (PDF1396 / printed1394): unknown unicast gets COMMUNICATION/BVLC_FUNCTION_UNKNOWN and is discarded. AB.2 (PDF1385 / printed1383) lists known 0x00..0x0C and prohibits broadcast responses. | [Private node guard](../../crates/bacnet-transport/src/sc/unknown_function.rs) admits only successful wire decodes of Unknown (0x0D..0xFF) to this rejection path. It does not interpret options, payload or NPCI. | [Independent raw vectors](../../crates/bacnet-transport/src/sc/unknown_function_tests.rs): compiled missing-NAK RED/GREEN, all 243 unknown codes, 0x0D/0x42/0xFF address/options/payload matrix and malformed generic-wire silence. |
| AB.3.1.2/.3 (PDF1395 / printed1393), AB.2.4.1 (PDF1389 / printed1387): response address, original ID/ResultFor, marker zero for a non-option error. | Eligible unicast has no destination: absent origin returns connection-local; valid origin becomes response destination. Exact 7/143 (0x008F), no options/error detail. | Independent exact NAK bytes with ID zero/max, absent/valid origins, empty/nonempty payload, MU Destination/Data Options, MoreOptions and empty/nonempty HeaderData. |
| AB.5.4 (PDF1403 / printed1401) drops explicit nonbroadcast destinations; AB.2 forbids broadcast replies. Reserved-origin suppression is owner-local policy. | Every explicit destination (broadcast/local/other/zero) and zero/broadcast origin is silently discarded without activity, probe or NPDU effects. Absent origin is allowed, not a missing NPDU source fault. | Ordered non-activity barriers detect extra replies/delivery; expired-budget tests prove silent decisions do not construct writes or fabricate expiry. |
| Owner-local unknown-first diagnostic and no-activity policy, not a universal priority over AB.3.1.4 or universal invalid-frame accounting from AB.6.3 (PDF1407 / printed1405). | Existing control/source/MU/empty gates exclude Unknown; fifth guard runs before activity/pending clear. All known functions, including Proprietary and known-but-unhandled, retain existing behavior. | Real std::Instant bursts before/after the first probe, original timeout/probe preservation, matching ACK and valid NPDU recovery. Known-code controls, all-state direct handle_received purity (including IDs/queued ACK), handshake silence and Result-for-Unknown ACK/NAK/malformed fatal-policy checks. |

- **Fifth bounded node path:** [RejectionBudget](../../crates/bacnet-transport/src/sc/rejection.rs)
  keeps the full remaining accepted-activity budget, strict pre/post-poll cutoff,
  huge-u64 safety and prompt send-error log/discard behavior. No per-frame timeout,
  new setting, unbounded write or clock policy. [Deadline vectors](../../crates/bacnet-transport/src/sc/rejection_deadline_tests.rs)
  and the [production TLS write-lock test](../../crates/bacnet-transport/src/sc_tls/rejection_deadline_tests.rs)
  include the fifth path. [Fresh recovery](../../crates/bacnet-transport/src/sc/rejection_recovery_tests.rs)
  adds an unknown case while retaining original MU/empty cases, failed-probe
  identity/limit exclusion and no further transport I/O on the retired socket.
  PR605's three-path and PR606's fourth-path evidence below remain slice-time evidence.
- **Installed native DEV:** [raw fake hub to native node](../../crates/rusty-bacnet/tests/test_sc_unknown_function.py)
  uses TLS1.3/client authentication, exact wire/silence, known-function controls,
  healthy ReadProperty and explicit fresh starts. This is not the native hub's
  fallback NAK. No Python short-heartbeat knob or native 60-second timing claim.
- **Limits:** authenticated peer path only, not preauthentication/MITM. Cancellation
  is not rollback; logical retirement is not immediate OS closure. Cooperative
  runtime, available state locks and caller-supplied fresh connectors remain
  assumptions. TLS lock evidence is not OS backpressure or hard real-time proof.
  Public connection mutation, hub/general forwarding, known-function liveness,
  handshake/probing, codec/raw send, all-write budgets, rate policy, graceful
  shutdown, CI/dependencies and release qualification remain excluded.
  No full Annex AB claim or support promotion.

## Empty Encapsulated-NPDU admission

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
#519 remains open/partial; all 68 rows, 19 supported rows, statuses, global
provenance, historical tranches and #517 A1–A6 acceptance remain unchanged.

| Contract / source | Implementation | Evidence |
|---|---|---|
| Base Standard 135-2020 AB.2.5/.1 (PDF 1389 / printed 1387) requires an NPDU payload; AB.3.1.5 (PDF 1396 / printed 1394) specifies absent-required-payload rejection. | [Pure presence predicate](../../crates/bacnet-transport/src/sc_frame/npdu.rs), independent of generic codec syntax. Exactly zero, not a two-byte floor or NPCI/APDU decoding. | [Node vectors](../../crates/bacnet-transport/src/sc/empty_npdu_tests.rs): compiled RED delivery, GREEN, raw option boundaries, one-byte and valid-NPDU compatibility, metadata/source delivery, pure direct-state drop and codec preservation. |
| AB.3.1.2/.3 (PDF 1395 / printed 1393), AB.2.4.1 (PDF 1389 / printed 1387): response address/ID, marker zero for a non-option error. AB.2 (PDF 1385 / printed 1383): broadcast silence. | [Node admission](../../crates/bacnet-transport/src/sc/empty_npdu.rs) after existing control/source/MU gates; eligible unicast replies to the valid originating VMAC with COMMUNICATION/PAYLOAD_EXPECTED (7/149, 0x0095). [Registered hub](../../crates/bacnet-transport/src/sc_hub/handler.rs) replies connection-locally, never to a spoofed source. | Exact independent NAK bytes, source/MU combined faults, broadcast silence, and [hub routing/options matrix](../../crates/bacnet-transport/src/sc_hub/empty_npdu_tests.rs). |
| AB.5.4 (PDF 1403 / printed 1401): drop explicit nonbroadcast node destinations. Existing hub routing-envelope filters remain. | The new node guard is silent for explicit destinations. Registered hub empties with Originating VMAC or no Destination VMAC stay silent. Pre-registration OTHER behavior is unchanged. Hub options including MU remain opaque. | Node destination matrix; compiled RED hub relay; two-recipient no-fanout barriers, unknown/zero destinations, preregistration characterization and later Connect, exact positive option relay, healthy Result/heartbeat/disconnect. |
| Owner-approved local admission policy, not universal invalid-frame accounting required by AB.6.3 (PDF 1407 / printed 1405). | Empty NPDUs cannot refresh activity or clear pending probes. [Direct connection](../../crates/bacnet-transport/src/sc/connection.rs) drops without mutation inside existing NPDU/state ownership; transport owns NAKs. | Real std::Instant node burst/timeout; hub activity/lease/identity/limits/probe snapshots and independent heartbeat sweep. |

- **Fourth bounded node rejection path:** missing payload reuses the original
  remaining accepted-activity budget, prompt-error behavior and logical retirement
  / fresh-only recovery from PR605, after source then MU. No budget reset, fabricated
  silent expiry, API change or all-writes guarantee. The [deadline suite](../../crates/bacnet-transport/src/sc/rejection_deadline_tests.rs)
  and [production TLS write-lock gate](../../crates/bacnet-transport/src/sc_tls/rejection_deadline_tests.rs)
  now include this fourth path; [fresh recovery](../../crates/bacnet-transport/src/sc/rejection_recovery_tests.rs)
  also tests failed-probe exclusion and identity/limits publication for it.
- **Hub lifecycle:** [mTLS held-NAK tests](../../crates/bacnet-transport/src/sc_hub/empty_npdu_retirement_tests.rs)
  exercise existing sticky retirement and same-identity replacement interrupting
  the supervised handler, with healthy recipients/replacement owners preserved.
  Registered Connect deadlines remain retired; cleanup retains its existing
  five-second close bound. No new global hub NAK deadline is claimed.
- **Installed native DEV evidence:** [two independent seams](../../crates/rusty-bacnet/tests/test_sc_empty_npdu.py)
  use the public native hub/server and raw mTLS clients to prove no forwarding,
  then a raw fake hub sends empties directly toward a native NODE (not through the
  hub guard again). Exact responses, healthy ReadProperty and explicit fresh starts
  are tested. Rust supplies timing/non-delivery evidence; Python has no exposed
  short default heartbeat knob and does not claim a native 60-second expiry test.
- **Limits:** authenticated TLS peers, not preauthentication or MITM proof. Source/MU
  order is preserved local behavior, not a newly asserted normative priority.
  Cancellation is not rollback of buffered bytes or already-admitted sends;
  logical retirement is not immediate OS closure. Cooperative runtime/state-lock
  assumptions remain; TLS lock tests are not OS backpressure or hard real-time
  proof. Positive payload validation, codec/encoding policy, pre-registration
  oddities, other function/liveness/rate policies, Address-Resolution forwarding,
  all-write budgets, graceful shutdown, CI/dependencies, performance and release
  qualification remain excluded. No full Annex AB claim or support promotion.

## Rejection NAK budget and fresh-only recovery

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
#519 remains open/partial. All 68 rows, 19 supported rows, statuses, global
provenance, historical tranches and closed #513/#517 acceptance remain unchanged.
PR605 superseded only the three blocked-NAK gaps in the MU slice below; the
empty-NPDU supplement adds the fourth path and the unknown-function supplement
adds the fifth, without broadening other writes.

- **Base Standard 135-2020:** AB.3.1.4/.5 (PDF 1395–1396 / printed 1393–1394)
  provide the existing NAK context; response IDs/addressing and raw option markers
  are unchanged. AB.6.1/.2 (PDF 1403–1405 / printed 1401–1403) describe reconnect
  and failure/IDLE transitions. AB.6.3 (PDF 1407 / printed 1405) does **not**
  prescribe this write budget or retirement policy. AB.3.1.4's remaining-parts
  qualification and the existing receive-drop policy remain; no new whole-message
  or full Annex AB conformance claim is made.
- **Owner-approved local policy:** [rejection budget](../../crates/bacnet-transport/src/sc/rejection.rs)
  bounds actual control/source/unsupported-MU/missing-NPDU-payload/unknown-function NAK sends by the original
  accepted-activity heartbeat budget. Late/repeated rejected frames do not reset
  it; elapsed budgets have no positive floor and cannot poll a send as fresh.
  Checks before/after polling prevent accepting completion after the cutoff.
  Large valid `u64` settings avoid new Instant-addition overflow by chunking timer
  registration, not resetting the budget. Silent/nonrejection decisions never
  acquire a deadline or fabricate an expiry. Immediate send errors retain their
  existing log/discard behavior; rejected frames do not refresh activity, clear a
  pending probe or dispatch.
- **Ownership/compatibility:** [retirement/recovery](../../crates/bacnet-transport/src/sc/recovery.rs)
  publishes Disconnected before recovery, drops the NAK future, and prevents
  further transport-initiated receive/write/handshake/stop/restore-disconnect on
  that socket. The retained initial primary is removed only if it is the retired
  socket; no unbounded poisoned-Arc history is kept. Existing connector factories
  must supply fresh connections (a caller contract, not introspection of hidden
  shared driver internals). Without a factory, the retired socket is **not reused**.
  An unused preconfigured failover remains eligible. Existing retry eligibility,
  limits, probe identity, publication and primary/failover recovery order remain;
  no reconnect configuration means no automatic recovery. No fresh eligible
  socket means remaining disconnected. Outstanding restore-disconnect work is
  canceled and joined on retirement; new public sends fail or use a fresh
  successfully published socket.
- **Cancellation is not rollback:** futures-util **0.3.33** `SinkExt::send` feeds
  then flushes; tokio-tungstenite/tungstenite **0.29.0** may retain accepted frames
  and flush them on later I/O. Tokio **1.53.1** timeouts may poll immediately ready
  futures even after a deadline. See the pinned [send source](https://docs.rs/futures-util/0.3.33/src/futures_util/sink/send.rs.html),
  [sink implementation](https://docs.rs/tokio-tungstenite/0.29.0/src/tokio_tungstenite/lib.rs.html),
  [write contract](https://docs.rs/tungstenite/0.29.0/tungstenite/protocol/struct.WebSocket.html#method.write)
  and [timeout contract](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout_at.html).
  Buffered bytes and application sends admitted before disconnection are not
  rolled back; NAK success is not claimed at expiry. The send slot may retain an
  old Arc until fresh publication or stop/drop, and external/in-flight references
  may retain it longer: logical retirement is **not immediate OS closure**.
- **Evidence/limits:** [real-clock deadline tests](../../crates/bacnet-transport/src/sc/rejection_deadline_tests.rs)
  retain PR605 compiled RED/GREEN evidence for its three paths; current vectors
  additionally cover missing payload and unknown functions. Tests cover full/remaining
  budgets, strict cutoff, immediate errors, silence and huge accepted settings.
  [Recovery tests](../../crates/bacnet-transport/src/sc/rejection_recovery_tests.rs)
  track socket identity and simulate retained bytes to detect later flush/retry;
  they cover fresh dials, unused failover, poisoned-primary restore prevention,
  failed probes, public admission and task cleanup.
  [Real TLS tests](../../crates/bacnet-transport/src/sc_tls/rejection_deadline_tests.rs)
  gate the production write mutex using public Rust timing. This is **not OS
  backpressure** proof. [Installed-native smoke](../../crates/rusty-bacnet/tests/test_sc_rejection_deadline.py)
  exercises exact NAKs, healthy ReadProperty and explicit fresh starts, not the
  default 60-second expiry or automatic native reconnect. Timing assumes a
  cooperative, timer-enabled runtime and available public state locks, not a
  hard real-time deadline under CPU starvation or an application-held connection
  lock. Heartbeat Request/ACK, Disconnect-ACK and public-send deadlines, general
  backpressure, Address-Resolution-ACK, general forwarding and graceful shutdown
  remain excluded. No API/dependency/configuration or support promotion.

## MU-rejection liveness accounting

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
#519 remains open/partial. All 68 rows, 19 supported rows, statuses, global
provenance, historical tranches and closed #513/#517 acceptance remain unchanged.

- **Base Standard 135-2020 source:** AB.3.1.4 (PDF 1395–1396 / printed
  1393–1394) requires COMMUNICATION/HEADER_NOT_UNDERSTOOD for an unsupported MU
  Destination Option on unicast and no Result for broadcast. Unknown MU-clear
  Destination Options are ignored; Data Options are forwarded unaltered.
  AB.3.1.2/.3 on the same pages specify response addressing and echoed IDs.
  AB.2 (PDF 1385 / printed 1383) supplies broadcast/response-silence context.
- **Owner-approved local admission policy:** [node receive ordering](../../crates/bacnet-transport/src/sc/mod.rs)
  now performs the existing MU rejection before refreshing accepted-BVLC activity
  or clearing the pending heartbeat. AB.6.3 (PDF 1407 / printed 1405) does not
  explicitly mandate universal invalid-frame accounting. This is not such a claim,
  nor a new whole-message conformance rule: AB.3.1.4 also requires processing
  remaining parts as required; this slice preserves the existing receive-drop
  policy. Control/source validation and matching Heartbeat-ACK precedence remain.
- **Evidence:** [real-time Rust regressions](../../crates/bacnet-transport/src/sc/mu_liveness_tests.rs)
  cover independent marker variants (More Options, empty/nonempty Header Data,
  multiple options), exact addressed NAKs, broadcast silence, no dispatch,
  source/control precedence, unchanged pending probe, matching-ACK recovery,
  original idle/timeout decisions under rejected traffic and actual state-watch
  timeout/redial recovery. MU-clear Destination Options, MU Data Options, valid
  NPDUs and Heartbeat-Request retain normal activity behavior.
  [TLS wire coverage](../../crates/bacnet-transport/src/sc_tls/mu_liveness_tests.rs)
  and [installed-native ReadProperty smoke](../../crates/rusty-bacnet/tests/test_sc_mu_liveness.py)
  exercise rejection and healthy recovery, not default native heartbeat expiry.
- **Slice-time limitation:** before the rejection-NAK budget supplement above,
  all timing claims required receive loop progress.
  `data_attributes.rs` awaits the NAK send inside the receive arm, as do existing
  source/control rejection helpers. `TlsWebSocket::send` awaits its write lock
  and sink without an explicit deadline. A blocked write can stall timer polling;
  blocked-send/backpressure handling is a documented follow-up, not fixed here.
  No timeout/cancellation/offload, adapter, clock, select-loop, retry-budget or
  all-function deadline change is included. Post-`handle_received` drops retain
  their existing activity accounting. General forwarding/Address-Resolution-ACK,
  diagnostic-rate policies, capacity floors and graceful shutdown remain excluded.
  No addenda, API, support promotion or full Annex AB claim is made.

## Accepting hub unsolicited-response silence

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
Only unsolicited Connect-Accept (`0x07`) and Disconnect-ACK (`0x09`) at the
accepting hub are included. #519 remains open/partial; the 68 rows, 19 supported
rows, global provenance, historical tranches and closed #513/#517 acceptance
are unchanged. This is not full Annex AB or universal response conformance.

- **Base Standard 135-2020 source:** AB.2 (PDF 1385 / printed 1383) prohibits
  replies to response messages. AB.2.11 and AB.2.13 (PDF 1391–1392 / printed
  1389–1390) define these connection-peer responses. AB.6.2/AB.6.2.3 (PDF 1403,
  1406–1407 / printed 1401, 1404–1405) give the accepting peer its connect wait
  and state transitions; Disconnect-ACK belongs to Disconnecting. The current
  hub has no Disconnect-ACK waiter: retirement uses WebSocket Close and stop
  remains forceful. No graceful-disconnect state machine is added.
- **Scoped local admission policy:** after generic decode and ownership checks,
  [hub dispatch](../../crates/bacnet-transport/src/sc_hub/handler.rs) silently
  discards these two functions before admission, activity refresh or dispatch,
  regardless of ID, envelope, options or payload. No NAK, registration, UUID/
  limits change, replacement, pending-probe clear/reseed or deadline extension
  results. Excluding them from activity is owner-approved local liveness policy,
  not a claim that AB.6.3 (PDF 1407 / printed 1405) forbids timer refresh for
  every invalid message. Existing generic/control malformed-message silence stays.
- **Evidence:** [independent raw wire/barrier tests](../../crates/bacnet-transport/src/sc_hub/response_silence_tests.rs)
  cover valid and malformed fields/options/payloads including Must Understand,
  zero/stale/matching IDs, before and after registration, exact lease/UUID/limits/
  activity/probe preservation, original idle/probe expiry, and later valid
  registration, Heartbeat-ACK, Heartbeat-Request, Disconnect-Request, NPDU and
  Result relay. [Authenticated TLS lifecycle tests](../../crates/bacnet-transport/src/sc_hub/response_silence_lifecycle_tests.rs)
  retain the absolute connect deadline under repeated responses, release its
  admission, and preserve all 256 registry owners before genuine replacement
  at capacity. [Installed-native public hub smoke](../../crates/rusty-bacnet/tests/test_sc_hub_response_silence.py)
  checks ordered silence, subsequent registration and surviving ReadProperty.
- **Exclusions:** Address-Resolution-ACK is not included: AB.5.1/AB.5.3.2 (PDF
  1400/1402 / printed 1398/1400) require separate general forwarding analysis.
  Result relay and matching Heartbeat-ACK retain their dedicated paths. Other
  fallback behavior, node-wide liveness, direct connections, diagnostic-rate
  policy, capacity floors, public APIs and raw codecs are unchanged. No addenda,
  performance measurement, platform/release qualification or support promotion.

## Received zero-capacity admission

Current-dev scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #519).
**Zero-only local policy:** received Connect-Request and Connect-Accept must
advertise nonzero Max-BVLC and Max-NPDU. This is not a universal minimum-capacity
conformance claim. #519 remains open/partial; the closed #513/#517 acceptance and
the immutable PR #601 identity closeout below remain valid historical evidence.
All 68 rows, 19 supported rows, statuses and global August 13/SHA provenance remain.

- **Source:** licensed base 135-2020 AB.2.10–11 (printed 1389–1390/PDF 1391–1392)
  defines each capacity and the fixed 26-byte payload; those field clauses do not
  supply a universal positive peer floor. Zero's range classification is an
  owner-approved local validation policy. AB.3.1.2/.4/.5 (printed 1393–1394/PDF
  1395–1396) informs addressing, errors and MU handling; AB.2 (printed 1383/PDF
  1385) prohibits replies to responses. AB.6.2 (printed 1401–1403/PDF 1403–1405)
  supplies the connect wait/state context. No source edition/addendum expansion.
- **Request:** eligible rejection is COMMUNICATION/PARAMETER_OUT_OF_RANGE (7/80),
  marker zero, with existing envelope addressing and broadcast/reserved-source
  suppression. Rejection precedes activity refresh, registry/capacity decisions,
  UUID replacement and limit commit. New malformed peers close; registered repeats
  preserve sink, identity, limits and liveness. Length/envelope/identity precedence
  remains; zero capacities precede MU in local diagnostic selection.
- **Accept:** silently discarded, with no NAK, state/identity/limit commit or
  Connected publication. Pending request and local identity remain unchanged; the
  original connect deadline is not extended. Later valid recovery is permitted;
  zero-only/flood traffic expires the original wait. Invalid-plus-wrong-ID is
  discarded while otherwise-valid wrong-ID remains terminal. Failed probes do not
  retire a healthy failover peer or poison its effective APDU budget.
- **Compatibility boundary:** all positive values remain accepted by this check,
  including 1/1, 1/65535, 65535/1, 65535/65535, 1200/480, 300/1476 and 1476/1476.
  Tiny positive values are not proof of serviceability or conformance. Stronger
  positive floors and Max-NPDU/Max-BVLC relationship checks are deferred. AB.5.1's
  hub forwarding/distribution capacity is not a blanket advertised-node floor;
  PR #549's 5705-byte full BVLC budget and hub NPDU 1497 remain unchanged;
  its historical node NPDU 1476 default becomes 1478 after #893. Adapter caps
  and independent outgoing budgets remain separate. Generic codecs and
  constructors still permit zero syntax; post-start public mutation is excluded.
- **Evidence:** [independent wire vectors](../../crates/bacnet-transport/src/sc_frame/connect_test_support.rs)
  cover either/both zero fields and error/suppression combinations; [direct and
  async Accept tests](../../crates/bacnet-transport/src/sc/connect_validation_tests.rs)
  cover transactional snapshots and paused-clock deadlines; [reconnect probes](../../crates/bacnet-transport/src/sc/reconnect_validation_tests.rs)
  preserve active state and recover without reseeding. [Real TLS Accepts](../../crates/bacnet-transport/src/sc_tls/connect_accept_tests.rs),
  [mTLS hub admission/capacity/repeats](../../crates/bacnet-transport/src/sc_hub/peer_uuid_tests.rs),
  [held-NAK deadlines](../../crates/bacnet-transport/src/sc_hub/deadline_commit_tests.rs)
  and [installed-native Python client/server/hub tests](../../crates/rusty-bacnet/tests/test_sc_zero_limits.py)
  cover the runtime boundary. The JSON row adds only scoped anchors/supplement;
  its historical tranche notes remain unchanged. Other #519 functions, diagnostic
  rate/liveness gaps, positive floors, full Annex AB/PICS/BTL and performance claims
  remain outside this slice.

## Received peer UUID admission

Scoped supplement to `BACNET-AB-SC-CONNECTION-STATE` (Refs #517), superseding only
the earlier slices' wire-admission exclusion. The machine-readable row adds
focused code/test anchors; its historical tranche notes and global provenance
remain unchanged. All 68 rows, 19 supported rows and existing statuses are retained.

- **Local security policy:** reject all-zero Device UUIDs in received
  Connect-Request at the hub, after TLS/WebSocket setup and before activity,
  registry/capacity decisions, commit or replacement. This is a compatibility break
  for legacy nil-request senders, not a pre-dial check or a claim that RFC 4122
  leaves nil UUID syntax undefined. Nonzero bits remain opaque; no version/variant,
  generation, lifetime storage or certificate binding is added. The
  [identity acceptance closeout](#device-identity-acceptance-closeout) below records
  the owner-approved resolution without broadening this policy.
- **Source:** licensed base Standard 135-2020 AB.1.5.2–3 (printed 1382/PDF 1384),
  AB.2.10–11 (printed 1389–1390/PDF 1391–1392), and AB.3.1.2/.4/.5 (printed
  1393–1394/PDF 1395–1396). The existing parameter-range classification returns
  COMMUNICATION/PARAMETER_OUT_OF_RANGE (7/80) for eligible unicast and discards the
  request. Length and forbidden-envelope checks still precede identity checks;
  identity precedes unsupported MU. Multi-fault precedence and reserved-source
  suppression remain repository interpretations, not new normative claims.
- NAK addressing uses the envelope source, never the proposed payload VMAC/UUID;
  marker zero and broadcast/reserved-envelope-source suppression remain. Nil plus
  a colliding VMAC is a range error, not Duplicate-VMAC, so no Random-48 reseed.
  A new malformed peer closes without admission/commit. Registered malformed
  repeats preserve sink/identity, negotiated limits, heartbeat and activity.
  Intended valid same-UUID replacement and valid-repeat handling remain unchanged.
- Generic encode/decode and manual raw sending still permit nil syntax.
  **Connect-Accept with a zero UUID is silently discarded** after TLS/WebSocket
  setup, extending the local nonzero policy to initiating peers. AB.2 (printed
  1383/PDF 1385) prohibits replies to response messages, including Connect-Accept;
  the AB.3.1.5 range classification is a local diagnostic, not a wire NAK. This
  retains the node's existing malformed-Accept silence, including envelope/MU
  multi-fault cases. Nil precedes MU in local diagnostic selection.
- Rejection preserves pending state, peer identity/limits, local UUID/VMAC and
  retry fields, without publishing Connected or resetting the absolute connect
  deadline. A later valid Accept can complete the same handshake; nil-only/flood
  traffic expires the original wait (AB.6.2/.2, printed 1401–1403/PDF 1403–1405).
  Invalid-plus-wrong-ID is discarded; valid-shape wrong-ID remains a terminal
  mismatch. Failed primary probes preserve the active failover/send limits; nil
  Accept never triggers the matching Connect-Request Duplicate-VMAC NAK reseed.
- Independent wire vectors, request/accept and MU matrices, real mTLS rejection
  and suppression, all 256 registered peers at capacity, repeated nil requests,
  healthy relay, held-NAK absolute deadlines, and installed-native Python raw-peer
  rejection plus surviving ReadProperty are the bounded Request evidence.
  Accept evidence adds transactional state snapshots, every single UUID bit and
  all-ones positives, paused-clock deadlines, failover/primary restoration, real
  TLS/WebSocket vectors and installed-native Python client/server mTLS rejection,
  later-valid recovery and timeout. This is not a new connection-lifecycle or
  caller-owned storage guarantee; raw primary socket retention remains unchanged.
  No full Annex AB/PICS/BTL, new addenda or performance claim is made.

### Device identity acceptance closeout

**Owner-approved scoped resolution of [#517](https://github.com/jscott3201/rusty-bacnet/issues/517):**
this proposed closeout accepts the delivered startup and received-peer guards,
with caller-owned provisioning/storage and the exclusions below. Closing the
scoped issue resolves accidental default/nil identity at those entry points and
peer admission; it is not literal all-public-API coverage or a new runtime change.
Issue closure is part of this proposed closeout, not a claim that GitHub is already
closed before merge.

**Runtime source baseline:**
[`bde599405c38e2ceb62e23ee628a0f15d1ac9fe2`](https://github.com/jscott3201/rusty-bacnet/tree/bde599405c38e2ceb62e23ee628a0f15d1ac9fe2),
unchanged by this docs/test closeout. Evidence below refers to that immutable
runtime and the merged node [#594](https://github.com/jscott3201/rusty-bacnet/pull/594),
hub [#595](https://github.com/jscott3201/rusty-bacnet/pull/595), raw-start
[#596](https://github.com/jscott3201/rusty-bacnet/pull/596), Request
[#597](https://github.com/jscott3201/rusty-bacnet/pull/597) and Accept
[#600](https://github.com/jscott3201/rusty-bacnet/pull/600) slices. Python lifecycle
methods below are in `NodeIdentityMtlsTests`.

| ID | Acceptance criterion and disposition | Code/document contract | Existing test evidence |
|---|---|---|---|
| A1 | **Met within scope:** distinct nodes coexist with distinct UUIDs and non-colliding VMACs. | [Hub registration](../../crates/bacnet-transport/src/sc_hub/helpers.rs), `hub_client_registration_decision`, distinguishes UUID replacement from VMAC collision. | [`test_distinct_nodes_and_same_uuid_replacement_leave_other_node_usable`](../../crates/rusty-bacnet/tests/test_sc_hub_mtls.py) checks real ReadProperty `72.5` before/after replacement in both client/server roles. |
| A2 | **Met at approved startup boundaries:** omitted/default/all-zero UUIDs are refused; not every low-level public path. | Rust fixed `[u8; 16]` types enforce length; [raw start](../../crates/bacnet-transport/src/sc/mod.rs) and the startup map below enforce nonzero identity before their owned I/O. Python [owned conversion](../../crates/rusty-bacnet/src/sc_identity.rs) also checks length. | [`test_sc_uuid_validation_precedes_file_and_socket_io`](../../crates/rusty-bacnet/tests/test_sc_node_identity.py), [`test_uuid_required_length_zero_and_vmac_errors_precede_io`](../../crates/rusty-bacnet/tests/test_sc_hub_identity.py) and [`local_hub_identity_rejected_before_bind_on_every_start_api`](../../crates/bacnet-transport/tests/sc_hub_tls.rs) cover missing/wrong-length/zero and no-dial/no-bind boundaries. |
| A3 | **Met for supplied bytes:** reuse across supported lifecycle/reconnect and intended same-UUID replacement. This is not application disk-storage qualification. | [`reset_for_connect_retry`](../../crates/bacnet-transport/src/sc/reconnect.rs) preserves the local UUID; [hub registration](../../crates/bacnet-transport/src/sc_hub/helpers.rs) replaces the same UUID even with a different VMAC. | [`test_uuid_owned_wire_bytes_across_stop_start_and_recreation`](../../crates/rusty-bacnet/tests/test_sc_hub_mtls.py) mutates the input bytearray after copying and verifies three lifecycles; [`test_hub_owned_identity_survives_stop_start_and_fresh_object`](../../crates/rusty-bacnet/tests/test_sc_hub_mtls.py) checks exact Accept identity. A1 observes incumbent Close and a surviving other node. [Reconnect tests](../../crates/bacnet-transport/src/sc/reconnect_validation_tests.rs) retain UUID/limits through nil-Accept failover, failed primary probes and timeout/redial. |
| A4 | **Met by the documented provisioning boundary alternative**, not changed-UUID detection after restart. | [Rust provisioning](../rust-api.md#sc-device-uuid-migration) and [Python provisioning](../python-api.md#sc-device-uuid-migration) require predeployment generation, durable storage and the same bytes for the device lifetime. No library backend/history or enforced lifetime immutability. | A3 proves in-memory reuse, not persistence. The [closeout documentation guard](../../crates/bacnet-integration-tests/tests/conformance_ledger.rs) checks this explicit alternative and the linked contracts; detection without application history is not claimed. |
| A5 | **Met:** hub Connect-Accept carries the configured nonzero hosting device UUID. | All four [Rust hub starts](../../crates/bacnet-transport/src/sc_hub.rs) share pre-bind UUID/VMAC validation; [Accept emission](../../crates/bacnet-transport/src/sc_hub/handler.rs) uses that configured identity. | [`strict_hub_start_family_requires_mutual_tls13_and_preserves_uuid`](../../crates/bacnet-transport/tests/sc_hub_tls.rs), the A2 pre-bind test, and the A3 Python hub test check rejection and exact Accept UUID/VMAC. Request nil rejection and silent Accept discard are additional guards, not bit-profile proof. |
| A6 | **Preserved:** existing Rust client #92 validation and type propagation. | [`validate_identity`](../../crates/bacnet-client/src/client/mod.rs) checks reserved VMACs then zero UUID before TLS lookup/dial; reconnect validation remains first. | [`sc_client_builder_sends_configured_vmac_and_device_uuid`](../../crates/bacnet-client/src/client/sc_builder_tests.rs), [`sc_client_builder_rejects_reserved_vmac_before_connect`](../../crates/bacnet-client/src/client/sc_builder_tests.rs) and [`sc_client_builder_rejects_broadcast_vmac_and_zero_device_uuid`](../../crates/bacnet-client/src/client/sc_builder_tests.rs). |

**Startup map and exclusions:** [Rust client](../../crates/bacnet-client/src/client/mod.rs)
validates VMAC/UUID before TLS dial; [Rust server](../../crates/bacnet-server/src/server/sc_builder.rs)
validates UUID before binding-table lookup/TLS dial, but its VMAC validation may
follow dialing. All four Rust hub starts validate UUID/VMAC before bind. Python
node credential-presence checks and [hub CA-first checks](../../crates/rusty-bacnet/src/hub.rs)
retain precedence before identity preflight/file I/O. The
[`parse_sc_device_uuid_arg`](../../crates/bacnet-cli/src/transport.rs) CLI parser
requires fixed-width hex and nonzero bytes; the standalone
[hub](../../benchmarks/src/bin/bacnet_sc_hub.rs) and
[device](../../benchmarks/src/bin/sc/device.rs) check identity before credential I/O.
Raw `ScTransport::start` checks reconnect, heartbeat, then identity before owned
I/O/socket-take, but cannot undo caller-owned WebSocket creation/dialing.
`ScConnection::new`, `build_connect_request`, generic codec/manual raw sending and
post-start mutation through `ScTransport::connection()`'s `Arc<Mutex<ScConnection>>`
remain outside the guarantee. There is no new rollback or lifetime enforcement.

**Source contract and deferred policy:** the unchanged licensed base 135-2020
source review covers AB.1.5.3 (printed 1382/PDF 1384), AB.2's prohibition on replies
to responses (printed 1383/PDF 1385), and AB.6.2 wait/replacement behavior (printed
1401–1403/PDF 1403–1405). RFC 4122 defines structured
[variant](https://www.rfc-editor.org/rfc/rfc4122.html#section-4.1.1) and
[version](https://www.rfc-editor.org/rfc/rfc4122.html#section-4.1.3) fields and the
[all-zero nil form](https://www.rfc-editor.org/rfc/rfc4122.html#section-4.1.7).
The structural recommendation is **not fully implemented** as a bit filter:
all nonzero 128-bit values remain opaque, including reserved variants. Nonzero is
not an RFC 4122 bit-profile guarantee. Stronger structural enforcement is explicitly
**deferred/excluded** by the owner, not marked as passed. Applications must provision
an appropriate RFC 4122 identity before deployment, durably store it and reuse it
for the device lifetime. No certificate binding, changed-identity history,
full Annex AB/PICS/BTL, new addenda or RFC 9562 qualification is claimed.

**Evidence reuse:** Runtime evidence is reused because runtime code is unchanged:
PR #600's recorded checks include 4,479/4,492 Rust workspace passes and 85 installed
Python tests with 1,163 subtests. This closeout adds documentation/anchor guards,
with no fresh native/platform qualification, deployment-storage validation or
conformance certification. The 68 rows, 19 supported rows, all statuses, source
JSON and generated PICS/BIBBs/support summary, global August 13 review SHA and historical tranche
evidence remain unchanged; slice-time open/exclusion notes below are historical,
not the current issue disposition.

## Status Taxonomy

| Status | Meaning |
|---|---|
| `in-progress` | Ledger/support artifact exists but is not complete evidence. |
| `implementation-present-needs-conformance-tests` | Source anchors exist; clause-specific positive tests are incomplete. |
| `implementation-present-needs-negative-tests` | Source anchors exist; malformed/unsupported-path tests are incomplete. |
| `implementation-present-needs-security-tests` | Source anchors exist; security/TLS/auth/fail-closed tests are incomplete. |
| `implementation-present-needs-timeout-tests` | Source anchors exist; deterministic timeout tests are incomplete. |
| `implementation-present-needs-state-machine-audit` | Source anchors exist; state transition audit/tests are incomplete. |
| `implementation-present-needs-window-tests` | Source anchors exist; segmentation/window tests are incomplete. |
| `implementation-present-needs-source-review` | Source appears present; detailed clause review is still needed. |
| `implementation-present-needs-platform-tests` | Source appears present; platform or hardware-adjacent evidence is needed. |
| `supported-with-clause-evidence` | Positive tests, anchors, and public claims support this row. |
| `deferred-pending-owner-decision` | Support direction requires an explicit owner decision. |
| `unsupported-by-design` | Intentionally unsupported with documented rationale. |
| `unknown-pending-source-review` | No reviewed implementation evidence yet. |

## Clause 4 Architecture

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-4-ARCHITECTURE
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-4-ARCHITECTURE` | Clause 4 | P2 | `implementation-present-needs-source-review` | Workspace crates and `docs/architecture.md` establish the current architecture map. |
<!-- END ledger-rows -->

## Clause 5 Application Layer

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-5-TSM-CLIENT
BACNET-5-TSM-SERVER
BACNET-5-SEGMENTATION-WINDOW
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-5-TSM-CLIENT` | Clause 5.4.4 | P1 | `implementation-present-needs-state-machine-audit` | Issue #379 covers Clause 5.4.4.4, and issue #380 covers Clause 5.4.4.3. Client TSM phase and activity-generation transitions stop RequestTimer after segment zero is saved, serialize segment admission against retry authorization without holding the TSM lock across transport I/O, and reject stale SegmentTimer expiry after qualifying activity. Each registration has an immutable owner identity that qualifies timer expiry, cancellation, receive-state activity/reset/completion, and dispatch-owned cleanup across immediate Invoke ID reuse. Both outgoing request paths enter SEGMENTED_CONF for a segmented response; its receive timer spans four APDU segment timeouts, restarts on segment activity, returns the local `TSM_TIMEOUT` ABORT.indication, promptly reclaims reassembly state without inbound traffic, and sends no peer PDU. Under `crates/bacnet-client/src`, code anchors are `tsm.rs`, `client/lifecycle.rs`, `client/requests.rs`, `client/segmentation.rs`, and `client/transaction_cleanup.rs`. Focused evidence in `client/request_timer_tests.rs`, `client/segmented_receive_lifecycle_tests.rs`, `client/segmented_timeout_tests.rs`, and `tsm/tests.rs` covers retry bounds and races, request-timer handoff, segment activity, receive timeout, cancellation under lock contention, outgoing segmented-request cancellation, stale timer generations, and delayed cleanup after key reuse. The broader Clause 5.4.4 transition matrix still needs audit. |
| `BACNET-5-TSM-SERVER` | Clause 5.4.5 | P1 | `implementation-present-needs-state-machine-audit` | Server segmented transaction identity follows Clauses 5.3.5.3 and 6.2: valid routed SNET/SADR plus Invoke ID is stable across immediate routers, while local transactions use immediate MAC plus Invoke ID. `crates/bacnet-server/src/server/mod.rs` owns the private key helper; `lifecycle.rs`, `dispatch.rs`, and `segmentation.rs` use it for request reassembly, peer Abort removal, SegmentACK/Abort dispatch, and segmented ComplexACK sender registration. Replies still use the current or captured immediate router MAC with NPDU DNET/DADR, never the canonical empty-MAC sentinel. Focused evidence in `server/tests.rs`, `server/segmentation_tests/request_reassembly.rs`, and `server/segmentation_tests/routing_overlap.rs` covers the identity matrix, request continuation and reply routing across routers, immediate peer-Abort cleanup, routed sender matching/mismatch, and canonical replacement without stale cleanup. Existing send-side evidence remains: SegmentACKs with the server bit set are ignored; either ACK flavor naming the current segment advances; only a negative ACK naming the immediately preceding segment retransmits the current segment; stale and out-of-range ACKs are ignored; timeout retries retransmit before idle cleanup without a final timeout Abort; dispatch stays nonblocking; active senders are capped; and replacement cancels the older sender. Remaining gaps include full server TSM transition audit, configured APDU segment timeout/retry exposure, and broader Reject/Error mapping coverage. |
| `BACNET-5-SEGMENTATION-WINDOW` | Clauses 5.2-5.4 | P1 | `implementation-present-needs-window-tests` | Client SEGMENTED_CONF and server SEGMENTED_REQUEST now use the Clause 5.4.2.2 `DuplicateInWindow` predicate corrected by Addendum 135-2020ch. Modulo-256 unit vectors and loopback receive tests cover window one, windows greater than one, exactly `ActualWindowSize` silent duplicates before a NAK, immediate out-of-order NAKs, client/server baseline resets, ACK roles and fields, and payload integrity. Remaining gaps include multi-segment send-window behavior, full-transfer modulo-256 wrap evidence, max APDU/max segment boundaries, and broader TSM transition coverage. |
<!-- END ledger-rows -->

## Clause 6 Network Layer

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-6-NPDU-CONTROL
BACNET-6-ROUTER-MESSAGES
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-6-NPDU-CONTROL` | Clause 6.2 | P1 | `implementation-present-needs-negative-tests` | NPDU codec and network layer paths exist. |
| `BACNET-6-ROUTER-MESSAGES` | Clauses 6.4-6.6 | P1 | `implementation-present-needs-conformance-tests` | Router code and stress benchmark paths exist. |
<!-- END ledger-rows -->

## Clauses 7-11 Data Links

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-7-ETHERNET-LLC
BACNET-8-ARCNET
BACNET-9-MSTP-FRAMES
BACNET-10-PTP
BACNET-11-LONTALK
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-7-ETHERNET-LLC` | Clause 7 | P2 | `implementation-present-needs-platform-tests` | Ethernet transport claim exists; platform tests remain open. |
| `BACNET-8-ARCNET` | Clause 8 | P3 | `unknown-pending-source-review` | No public support claim found in the initial scan. |
| `BACNET-9-MSTP-FRAMES` | Clause 9.3 | P2 | `implementation-present-needs-source-review` | MS/TP frame and transport paths exist. |
| `BACNET-10-PTP` | Clause 10 | P3 | `unknown-pending-source-review` | No public support claim found in the initial scan. |
| `BACNET-11-LONTALK` | Clause 11 | P3 | `unknown-pending-source-review` | No public support claim found in the initial scan. |
<!-- END ledger-rows -->

## Clauses 12-19 Objects, Services, And Procedures

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-13-ACKED-TRANSITIONS-NETWORK-OWNERSHIP
BACNET-12-OBJECT-MODEL
BACNET-12-ALERT-ENROLLMENT-TABLE-12-61
BACNET-12-CALENDAR-PROPERTY-SET
BACNET-12-SCHEDULE-EVALUATION
BACNET-12-SCHEDULE-WRITES
BACNET-12-SCHEDULE-RELIABILITY
BACNET-12-PROPERTY-METADATA-CORE
BACNET-12-ESCALATOR-STATUS-WRITABILITY
BACNET-12-ELEVATOR-GROUP-LANDING-CALLS
BACNET-12-ELEVATOR-GROUP-PROPERTY-SET
BACNET-12-LIFT-CAR-MOVING-DIRECTION
BACNET-12-LIFT-PROPERTY-SET
BACNET-12-ESCALATOR-PROPERTY-SET
BACNET-12-ACCESS-DOOR-DOOR-VALUE
BACNET-12-ACCESS-CREDENTIAL-PROPERTY-SET
BACNET-12-ACCESS-CREDENTIAL-REQUIRED-ROWS
BACNET-12-ACCESS-DOOR-PULSE-TIMING
BACNET-12-UNDEFINED-PROPERTY-ROWS
BACNET-12-LIFE-SAFETY-GLOBAL-GROUP-REQUIRED-ROWS
BACNET-12-GLOBAL-GROUP-ARRAY-ENCODINGS
BACNET-12-COMMAND-STRUCTURED-VIEW-ARRAYS
BACNET-12-LIFE-SAFETY-OUT-OF-SERVICE-SIMULATION
BACNET-12-LIFE-SAFETY-APPLICATION-VALUES
BACNET-12-LOOP-PROPERTY-SET
BACNET-12-REQUIRED-ROWS-UNITS-PULSE-LIGHTING
BACNET-12-DEVICE-MAX-SEGMENTS
BACNET-12-DEVICE-ACTIVE-COV-SUBSCRIPTIONS
BACNET-12-DEVICE-ACTIVE-COV-MULTIPLE-SUBSCRIPTIONS
BACNET-12-NOTIFICATION-FORWARDER-WITHDRAWAL
BACNET-12-CHANNEL-WITHDRAWAL
BACNET-15-WRITEGROUP-SERVER-WITHDRAWAL
BACNET-12-RECIPIENT-LIST-FRAMING
BACNET-12-EVENT-PARAMETERS-FRAMING
BACNET-12-OOS-RELIABILITY-WRITABILITY
BACNET-13-COV-OBJECT-CRITERIA
BACNET-12-RELINQUISH-DEFAULT-WRITABILITY
BACNET-12-BINARY-LIGHTING-OPERATIONS
BACNET-12-REFERENCE-PROPERTY-WRITABILITY
BACNET-12-TIME-DELAY-NORMAL
BACNET-13-EVENT-ENROLLMENT-EVALUATOR
BACNET-15-ARRAY-INDEX-GATING
BACNET-15-WP-EVENT-FIELD-VALIDATION
BACNET-15-STRUCTURED-WRITE-DECODE
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-13-ACKED-TRANSITIONS-NETWORK-OWNERSHIP` | Clause 12.1.2; Acked_Transitions property paragraphs and Tables 12-2 (Analog Input), 12-3 (Analog Output), 12-4 (Analog Value), 12-6 (Binary Input), 12-8 (Binary Output), 12-10 (Binary Value), 12-14 (Event Enrollment), 12-21 (Multi-state Input), 12-22 (Multi-state Output), 12-23 (Multi-state Value), and 12-61 (Alert Enrollment); Clauses 13.2.3, 13.2.5, and 13.5; Clauses 15.9 and 15.10; Annex K Table K-17 footnote 1 | P1 | `supported-with-clause-evidence` | On the eleven supported Analog Input/Output/Value, Binary Input/Output/Value, Multi-state Input/Output/Value, Event Enrollment, and Alert Enrollment types, `Acked_Transitions` is exposed in `Property_List`, reads as a three-bit BitString, and is network read-only. The property-specific read-only rule controls over Clause 12.1.2's general implementor option; a valid whole-property WriteProperty returns PROPERTY / WRITE_ACCESS_DENIED without mutation, generated PICS remains readable/non-writable, and WPM reports the exact failed reference while retaining its successful prefix. Transition commit, AcknowledgeAlarm or local acknowledgment indications, and detection-disable reset retain internal mutation ownership. Annex K requires presence rather than modification. Evidence: `pics::acked_transitions_policy_tests`, `handlers/tests/wpm_prefix_commit.rs`, `event_enrollment/tests/same_state.rs`, `handlers/tests/acknowledge_alarm_ee.rs`, and object detection-reset tests. This evidence-only row makes no public claim or runtime/API change; it does not complete #123 or #175, expand event notification/BIBBs, or claim AcknowledgeAlarm for Binary, Multi-state, or Alert Enrollment (current overrides are Analog Input/Output/Value and Event Enrollment only). |
| `BACNET-12-OBJECT-MODEL` | Clauses 12-19 | P1 | `implementation-present-needs-conformance-tests` | Object model, server handlers, and existing PICS generator paths exist. |
| `BACNET-12-ALERT-ENROLLMENT-TABLE-12-61` | Clause 12.52 and Table 12-61; Clause 21 BACnetNotifyType; Clause 15.7 ReadPropertyMultiple | P1 | `supported-with-clause-evidence` | Alert Enrollment serves exactly the twelve required rows plus optional `Description`. `Present_Value` is the read-only last-source `ObjectIdentifier`; `record_alert_source` changes only that source. `Notify_Type` defaults `ALARM`, accepts `ALARM`/`EVENT`, and rejects `ACK_NOTIFICATION`, invalid, wrong-type, and overwide values without mutation. Property_List, RPM selectors, PICS, ordered-prefix WPM behavior, and Rust/Python migration tests are exact. `Status_Flags`, `Out_Of_Service`, and `Reliability` are absent. No Alert evaluator, notification generation, or optional-property completeness is claimed. |
| `BACNET-12-CALENDAR-PROPERTY-SET` | Clause 12.9 and Table 12-11; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#984). Calendar serves only Table 12-11 rows: the six required rows plus optional `Description`. `Description` and `Date_List` are its writable properties. `Status_Flags`, `Event_State`, `Out_Of_Service` and `Reliability` are absent from `Property_List`, the metadata, RPM `ALL`/`OPTIONAL` and PICS, and RP, RPM and WP on them return PROPERTY / UNKNOWN_PROPERTY (breaking: the first three used to read fixed values and refuse writes with WRITE_ACCESS_DENIED). COV, event-notification and alarm/event summary consumers already treat these properties as absent. Evidence: `crates/bacnet-objects/src/schedule/calendar_metadata.rs`, `crates/bacnet-objects/src/computed_status_flags_tests.rs::calendar_has_no_status_flags_to_compute`, `crates/bacnet-server/src/handlers/tests/{read_rpm,property_metadata}/calendar.rs`. Scope: the served property set and Present_Value. #1029: Present_Value is evaluated from the bound Device clock's local date on every read, TRUE when any `Date_List` entry matches (the shared `bacnet_types::calendar` matching: wildcards, odd/even months, last/odd/even days, open-ended ranges, week-of-month 1-9), FALSE without a clock; `set_present_value` is removed. Evidence: `crates/bacnet-objects/src/schedule/calendar_tests.rs`, `crates/bacnet-types/src/calendar/tests.rs`, `crates/bacnet-server/src/schedule_tests.rs`. The `Date_List` wire form, writes and value checks are `BACNET-21-CALENDAR-ENTRY-CHOICE` (#996, #1029). The audit, tag and profile rows are not claimed. |
| `BACNET-12-SCHEDULE-EVALUATION` | Clause 12.24.4 Present_Value, 12.24.6 Effective_Period, 12.24.7 Weekly_Schedule, 12.24.8 Exception_Schedule and 12.24.9 Schedule_Default; Clause 21 BACnetSpecialEvent and BACnetTimeValue; Clause 12 note on unspecified dates | P1 | `supported-with-clause-evidence` | #1028. Within `Effective_Period` (ends included, unspecified ends open) Present_Value is the best-priority special event in effect today whose current value is not NULL (inline calendar entry matching, or the referenced Calendar TRUE; the lower array index breaks a tie), else today's weekly entry if not NULL, else `Schedule_Default`. A list's current value is its latest time-value at or before now. Outside the period nothing is calculated or written. Entering it (start-up included) writes even an unchanged value. Time-values are typed, so Present_Value and the target writes carry the scheduled datatype, at `Priority_For_Writing` (locally settable, default 16); NULL relinquishes. Breaking: before, every exception applied whatever its period, Calendar references and `Effective_Period` were ignored, NULL counted as a value, and Present_Value was an Octet String of the raw bytes. Evidence: `crates/bacnet-objects/src/schedule/evaluation_tests.rs`, `crates/bacnet-server/src/schedule_tests.rs`, `crates/bacnet-endpoint/src/source_reporter_schedule_tests.rs`. Network writes are `BACNET-12-SCHEDULE-WRITES` and the CONFIGURATION_ERROR evaluation `BACNET-12-SCHEDULE-RELIABILITY`. Present_Value writes while Out_Of_Service are `BACNET-12-SCHEDULE-WRITES` (#1055). Not covered: remote references, and timing finer than the 60-second pass. |
| `BACNET-12-SCHEDULE-WRITES` | Clause 12.24.6 Effective_Period, 12.24.7 Weekly_Schedule and 12.24.8 Exception_Schedule (printed310-311/PDF312-313), 12.24.10 List_Of_Object_Property_References and 12.24.11 Priority_For_Writing (printed311/PDF313), recalculation and Present_Value writability in 12.24.4 (printed309/PDF311), 12.24.14 Out_Of_Service (printed312/PDF314); Clauses 15.1, 15.2, 15.9.1.3 and 15.10; Clause 21 BACnetDailySchedule, BACnetSpecialEvent and BACnetDateRange | P1 | `supported-with-clause-evidence` | #1057. WP, WPM and `write_local` accept `Weekly_Schedule`, `Exception_Schedule` and `Effective_Period` whole, and one array element by index, decoded with the shared constructed codecs and checked by the setters' own functions: a non-specific time or an event priority outside 1 to 16 is VALUE_OUT_OF_RANGE (the codec decodes any Unsigned priority, #1087), a repeated time in one list DUPLICATE_ENTRY, an element of another datatype INVALID_DATA_TYPE, a malformed one INVALID_DATA_ENCODING, and a refusal changes nothing. `Weekly_Schedule` stays seven days (other counts VALUE_OUT_OF_RANGE, index 0 WRITE_ACCESS_DENIED); `Exception_Schedule` index 0 resizes it with empty events, up to 1,024 (NO_SPACE_TO_WRITE_PROPERTY, `add_exception` too). A committed write to a Schedule runs its evaluation at once through the tick's code, with COV for the targets. DATATYPE_NOT_SUPPORTED is never returned; mixed datatypes show in Reliability. #1055: while `Out_Of_Service` is TRUE, Present_Value takes any primitive value, NULL included (INVALID_DATA_TYPE otherwise; WRITE_ACCESS_DENIED in service), and every accepted write goes to the references at `Priority_For_Writing` in that same pass, NULL relinquishing, ahead of any calculated value and without a clock; neither the tick nor a content write replaces it, and the return to service hands back to the calculation at once. Evidence: `crates/bacnet-objects/src/schedule/write_tests.rs`, `crates/bacnet-objects/src/schedule/out_of_service_tests.rs`, `crates/bacnet-server/src/server/schedule_write_tests.rs`, `crates/bacnet-server/src/schedule_tests.rs`. Not covered: network writes of the references and `Priority_For_Writing`. |
| `BACNET-12-SCHEDULE-RELIABILITY` | Clause 12.24.13 Reliability, 12.24.12 Status_Flags and 12.24.14 Out_Of_Service (printed311-312/PDF313-314); output rule in 12.24.4 (printed309/PDF311); reference writes in 12.24.10 (printed311/PDF313) | P1 | `supported-with-clause-evidence` | #1056. Reliability is CONFIGURATION_ERROR, with Status_Flags FAULT, while the non-NULL values in `Weekly_Schedule`, `Exception_Schedule` and `Schedule_Default` are not all of one datatype, checked on every change (setters and network writes), on the return to service and in the fault detector's pass. The object clears only a fault it raised, so an application-applied or simulated Reliability stays. A misconfigured Schedule keeps writing its references, since 12.24.4 sends every change and 12.24.13 only reports. A Present_Value written while out of service counts for nothing in the check, and a simulated Reliability doesn't hold its write back (#1055). Evidence: `crates/bacnet-objects/src/schedule/reliability_tests.rs`, `crates/bacnet-objects/src/schedule/out_of_service_tests.rs`. Not covered: whether each referenced property accepts the datatype, which needs the target objects and waits on a design decision. |
| `BACNET-12-PROPERTY-METADATA-CORE` | Clause 12.6, Table 12-6 (pp. 189-190); Clause 12.42, Table 12-49 (pp. 444-445); Clause 15.7.3.1 (p. 743); Annex A (pp. 964-965) | P1 | `in-progress` | Time Value, Binary Input, Life Safety Point (`POINT_BASE` 18 rows), and Life Safety Zone (`ZONE_BASE` 16 rows) now carry canonical effective-property rows with base R/W/O, presence-condition, and implemented-write classifications. Their legacy property-list projection, RPM ALL/REQUIRED/OPTIONAL expansion, and existing PICS flags derive from those rows; `Property_List` remains canonical but is omitted from the legacy projection and RPM expansion. Life Safety rows mirror dispatch exactly: `Mode` carries table W and accepts only the modes in `Accepted_Modes`, which is network read-only like `Silenced` and `Operation_Expected`, and `Tracking_Value` (on both objects since #1092) and `Reliability` are `WhenOutOfService`, writable only while Out_Of_Service is TRUE as footnote 1 of both tables asks (#1108, `BACNET-12-LIFE-SAFETY-OUT-OF-SERVICE-SIMULATION`). Every other object type retains the prior empty-metadata fallback, so #261 remains open. Evidence: `property_metadata_tests`, RPM metadata-selector tests, PICS metadata projection, and explicit Analog Input/Date Value fallback tests. |
| `BACNET-12-ESCALATOR-STATUS-WRITABILITY` | Clause 12 general property conformance rules; Clause 12.60 Table 12-78 and Out_Of_Service; Clause 15.9.1.3; Clause 21 BACnetEscalatorMode, BACnetEscalatorOperationDirection, and BACnetEscalatorFault; Clause 23.1 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#401). `Power_Mode`, `Operation_Direction`, `Escalator_Mode`, `Energy_Meter`, `Fault_Signals`, and `Passenger_Alarm` accept the same validated writes in service and while `Out_Of_Service` is TRUE. `Passenger_Alarm` defaults FALSE. `Fault_Signals` stores typed `EscalatorFault` values and reads a `List` of `Enumerated`; empty, wire-singleton, and unique multi-value sets accept named 0..=8 and proprietary 1024..=65535 values. Reserved, oversized, duplicate, mistyped, and non-finite values fail atomically with the Clause 15.9.1.3 PROPERTY errors. The service decoder's zero-element exception is limited to `Fault_Signals` and Calendar `Date_List` (#996); indexed access remains rejected. A successful empty `Fault_Signals` WPM prefix stays committed when a later write fails, while a refused write leaves the prior set unchanged. `EscalatorObject::is_writable_property` matches `Description`, `Out_Of_Service`, and these six routes, without advertising `Object_Name`. Evidence: `crates/bacnet-objects/src/elevator/escalator.rs`, `crates/bacnet-objects/src/elevator/tests/escalator_status_writability.rs`, `crates/bacnet-server/src/handlers/{write_property.rs,tests/escalator_writes.rs}`. This is not a complete Escalator-object conformance claim: the served property set is `BACNET-12-ESCALATOR-PROPERTY-SET` (#1022), and Reliability simulation, an initialized `Energy_Meter_Ref`, intrinsic reporting and physical-device arbitration remain out of scope. |
| `BACNET-12-ELEVATOR-GROUP-LANDING-CALLS` | Clause 12.58, Table 12-76 and 12.58.9-12.58.10; Clause 21 BACnetLandingCallStatus and BACnetLiftCarDirection; Clause 23.1 Table 23-1; Clause 15.9.1.3 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#980). `Landing_Call_Control` holds one `BACnetLandingCallStatus` (floor [0], direction [1] or destination [2], optional floor-text [3], unframed) and `Landing_Calls` a `BACnetLIST` of them, replacing a raw Enumerated and an Unsigned count (breaking on the wire; the unused public `BACnetAssignedLandingCalls` is removed). `Landing_Call_Control` stays writable (12.58.10) and refuses non-constructed values with INVALID_DATA_TYPE, undecodable or trailing bytes with INVALID_DATA_ENCODING, and a well-formed call whose floor-number or destination exceeds 255 or whose direction is reserved or above 65535 with VALUE_OUT_OF_RANGE (Clause 15.9.1.3), atomically, over WP and WPM. `Landing_Calls` is read-only and application-owned (`set_landing_calls`); a control write doesn't add to it. Before any write the control reads floor 0 / UNKNOWN. Evidence: `crates/bacnet-encoding/src/constructed/tests/landing_call_status.rs`, `crates/bacnet-objects/src/elevator/tests/landing_calls.rs`, `crates/bacnet-server/src/handlers/tests/elevator_landing_calls.rs`. Not covered: the object's property set (`BACNET-12-ELEVATOR-GROUP-PROPERTY-SET`, #997), the lift-group presence footnote, and dispatching. |
| `BACNET-12-ELEVATOR-GROUP-PROPERTY-SET` | Clause 12.58 and Table 12-76; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#997). Elevator Group serves only Table 12-76 rows. `Machine_Room_ID` (required `BACnetObjectIdentifier`) names the Positive Integer Value object holding the machine room's number, or instance 4194303 when there is none; it reads over RP and RPM, refuses WP with WRITE_ACCESS_DENIED, and the application sets it with `set_machine_room_id`, which refuses any other object type with VALUE_OUT_OF_RANGE. `Group_ID` (required Unsigned8) stays writable, is stored as `u8`, and refuses a write above 255 with VALUE_OUT_OF_RANGE without mutation. `Status_Flags`, `Out_Of_Service` and `Reliability` are absent from `Property_List`, the metadata, RPM `ALL`/`OPTIONAL` and PICS, and RP, RPM and WP on them return PROPERTY / UNKNOWN_PROPERTY (breaking: they used to read fixed or stored values and `Out_Of_Service` took writes). The object has no COV, event state or intrinsic reporting, and COV, event-notification and alarm/event summary consumers already treat these properties as absent. Evidence: `crates/bacnet-objects/src/elevator/{metadata.rs,tests/group_properties.rs}`, `crates/bacnet-server/src/handlers/tests/{elevator_properties.rs,read_rpm/elevator.rs,property_metadata/elevator.rs}`, `crates/bacnet-server/src/pics/property_metadata_tests/elevator.rs`. Scope: the served property set only; no claim that the `Machine_Room_ID` object exists, nor for the lift-group presence footnote, the audit, tag and profile rows, or dispatching. |
| `BACNET-12-LIFT-CAR-MOVING-DIRECTION` | Clause 12.59 and Table 12-77; Clause 21 BACnetLiftCarDirection; Clause 23.1 Table 23-1; Clause 15.9.1.3 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#998). `Car_Moving_Direction` is stored as `LiftCarDirection` (#932 pattern) and accepts the six named values 0..=5 and proprietary 1024..=65535; reserved 6..=1023 and values above 65535 fail with VALUE_OUT_OF_RANGE and non-Enumerated values with INVALID_DATA_TYPE, atomically, over the object API and WP. The old check admitted only 0..=3, numbered as if 1 were STOPPED, so DOWN, UP_AND_DOWN and proprietary values were refused and a new Lift read NONE (1); it now reads STOPPED (2). The property description lists four of the six named values; the check follows the datatype and leaves the choice to the application. Evidence: `crates/bacnet-objects/src/elevator/tests/lift_car_moving_direction.rs`, `crates/bacnet-server/src/handlers/tests/{elevator_properties.rs,read_rpm/elevator.rs}`. The Lift's other Table 12-77 rows and datatypes are `BACNET-12-LIFT-PROPERTY-SET` (#1021). Not covered: `Car_Assigned_Direction` and intrinsic reporting. |
| `BACNET-12-LIFT-PROPERTY-SET` | Clause 12.59 and Table 12-77, including Out_Of_Service, Energy_Meter and Energy_Meter_Ref; Clause 12.1.5.1; Clause 21 BACnetDoorStatus, BACnetLandingDoorStatus, BACnetAssignedLandingCalls, BACnetLiftCarCallList, BACnetLiftCarDirection, BACnetLiftCarDoorCommand, BACnetLiftCarMode, BACnetLiftCarDriveStatus, BACnetLiftFault, BACnetEngineeringUnits and BACnetDeviceObjectReference; Clause 23.1; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1021). The Lift serves only Table 12-77 rows, in table order, each with its table datatype. It gains the required `Elevator_Group` (Elevator Group instance 4194303 until set), `Group_ID` and `Installation_ID` (Unsigned8), `Passenger_Alarm` (Boolean, FALSE) and `Fault_Signals` (a `BACnetLIST` of `LiftFault`: named 0..=16 and proprietary 1024..=65535, no repeats), and `Car_Load_Units` (PERCENT until set), which the table requires alongside `Car_Load`. `Car_Position` is an Unsigned8 and refuses a write above 255 with VALUE_OUT_OF_RANGE without mutation; `Car_Load` is a REAL that refuses non-finite values; `Car_Door_Status` is a `BACnetARRAY` of `BACnetDoorStatus` and `Landing_Door_Status` a `BACnetARRAY` of `BACnetLandingDoorStatus` (new `bacnet-types` type and `bacnet-encoding` codec), both one element per car door, and `Floor_Text` is a `BACnetARRAY`; all three take an array index. `Tracking_Value` and `Floor_Number` are not table rows and now fail RP, RPM and WP with UNKNOWN_PROPERTY (breaking, as are the four datatype changes). The membership rows, `Car_Load_Units`, the door arrays and `Floor_Text` are read-only over the network; the application sets them with `LiftObject` setters that refuse another object type, units above 65535, a reserved door status, or a `Landing_Door_Status` size other than `Car_Door_Status`'s, and `set_car_door_status` resizes `Landing_Door_Status` to the door count. `Car_Position`, `Car_Moving_Direction`, `Car_Load`, `Passenger_Alarm`, `Energy_Meter` and `Fault_Signals` take writes in service and out of service. The door arrays take simulation writes while `Out_Of_Service` is TRUE (#1035). #1052 adds the optional `Assigned_Landing_Calls`, `Making_Car_Call`, `Registered_Car_Call` and `Car_Door_Command` (per-door `BACnetARRAY`s resized with the door count; new `BACnetAssignedLandingCalls` and `BACnetLiftCarCallList` types and codecs) and `Car_Assigned_Direction`, `Car_Door_Zone`, `Car_Mode`, `Next_Stopping_Floor` and `Car_Drive_Status`, all application-owned in service through `LiftObject` setters and written over the network only while out of service, as items (c) and (d) of the Out_Of_Service description require. Evidence: `crates/bacnet-objects/src/elevator/{lift.rs,membership.rs,doors.rs,door_values.rs,car_state.rs,metadata.rs,tests/lift_properties.rs,tests/lift_door_simulation.rs,tests/lift_car_calls.rs,tests/lift_car_state.rs,tests/group_membership.rs}`, `crates/bacnet-encoding/src/constructed/tests/{landing_door_status.rs,assigned_landing_calls.rs,lift_car_call_list.rs}`, `crates/bacnet-server/src/handlers/tests/{elevator_properties.rs,elevator_properties/lift_simulation.rs,read_rpm/elevator.rs,property_metadata/elevator.rs}`, `crates/bacnet-server/src/pics/property_metadata_tests/elevator.rs`. Not covered: the unserved optional rows (`Car_Door_Text` and the deck rows), the `Floor_Text` size rule, `Elevator_Group` consistency with the group's `Group_Members`, intrinsic reporting and dispatching. |
| `BACNET-12-ESCALATOR-PROPERTY-SET` | Clause 12.60 and Table 12-78, including Energy_Meter and Energy_Meter_Ref; Clause 21 BACnetDeviceObjectReference; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1022). The Escalator serves its Table 12-78 rows in table order with their table datatypes. It gains the required `Elevator_Group` (Elevator Group instance 4194303 until set), `Group_ID` and `Installation_ID` (Unsigned8, 0 until set), read over RP and RPM and refused over WP with WRITE_ACCESS_DENIED; the application sets them with `EscalatorObject::set_elevator_group` (refuses any other object type with VALUE_OUT_OF_RANGE), `set_group_id` and `set_installation_id`, shared with the Lift through one membership model. `Energy_Meter_Ref` is now an uninitialized `BACnetDeviceObjectReference` (Accumulator instance 4194303, no device) instead of an empty OctetString (breaking on the wire). Evidence: `crates/bacnet-objects/src/elevator/{escalator.rs,membership.rs,metadata.rs,tests/group_membership.rs}`, `crates/bacnet-server/src/handlers/tests/{elevator_properties.rs,read_rpm/elevator.rs,property_metadata/elevator.rs}`, `crates/bacnet-server/src/pics/property_metadata_tests/elevator.rs`. Not covered: an initialized `Energy_Meter_Ref` and its `Energy_Meter` rule, `Elevator_Group` consistency with the group's `Group_Members`, the event, intrinsic-reporting, audit, tag and profile rows, and dispatching. |
| `BACNET-12-ACCESS-DOOR-DOOR-VALUE` | Clause 12.26 and Table 12-30 (Present_Value, Priority_Array, Relinquish_Default); Clauses 12.26.4 and 12.26.11; Clause 21 BACnetDoorValue; Clause 19 command prioritization; Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#979, #1073). The Access Door stores `Present_Value`, its `Priority_Array` slots and `Relinquish_Default` as `DoorValue` (#932 pattern), and `AccessDoorObject::set_relinquish_default` takes a `DoorValue`. A `Present_Value` command accepts LOCK, UNLOCK, PULSE_UNLOCK and EXTENDED_PULSE_UNLOCK (0..=3); `Relinquish_Default` accepts only LOCK and UNLOCK, since Clause 12.26.11 keeps the pulses out of it (#1073). Any other Enumerated fails with VALUE_OUT_OF_RANGE and a non-Enumerated value with INVALID_DATA_TYPE, leaving the value, the priority array and the default unchanged, over the object API and WP. Breaking: the `Present_Value` arm used to store any Enumerated, and `Relinquish_Default` used to accept the two pulses. In-range values read the same on the wire. Evidence: `crates/bacnet-objects/src/access_control/{door.rs,typed_value_tests.rs,tests.rs}`, `crates/bacnet-server/src/handlers/tests/{access_typed_values.rs,read_rpm/access_topology.rs,wpm_state_ownership.rs}`. Pulse timing, the pulse-time rows and `Current_Command_Priority` are `BACNET-12-ACCESS-DOOR-PULSE-TIMING`. Not covered: the unlock-delay row, the value-source rows, and the footnoted Out_Of_Service writability of `Door_Status` and `Lock_Status`. |
| `BACNET-12-ACCESS-CREDENTIAL-PROPERTY-SET` | Clause 12.35 and Table 12-40 (Credential_Status); Clause 12.35.8; Clause 21 BACnetBinaryPV; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#979, #1073). `Credential_Status` is a `BinaryPV` derived from `Reason_For_Disable`: INACTIVE while the list holds a reason, ACTIVE when it is empty (#1073). It is read-only, so WP on it fails with WRITE_ACCESS_DENIED (breaking: #979 let a write store 0 or 1), and a new credential reads ACTIVE (breaking: it read INACTIVE). `Credential_Disable` is the property an operator writes to disable it. `ResolvedEnum::from_property` names it as a `BinaryPV`. `Present_Value` is not a Table 12-40 row and is removed: it is absent from `Property_List`, the metadata, RPM `ALL`/`OPTIONAL` and PICS, and RP, RPM and WP on it return PROPERTY / UNKNOWN_PROPERTY (breaking). It came with the 0.1.0 import as a separate, unsynchronised copy of the active/inactive state that `Credential_Status` carries, and #678 recorded it as an implementation-extra optional writable row. Evidence: `crates/bacnet-objects/src/access_control/{credential.rs,credential_tests.rs,metadata_identity.rs,typed_value_tests.rs}`, `crates/bacnet-server/src/handlers/tests/{access_typed_values.rs,read_rpm/access_identity.rs,property_metadata/access_identity.rs}`, `crates/bacnet-server/src/pics/property_metadata_tests/access_identity.rs`, `crates/bacnet-types/src/enums/resolve.rs`. The other required rows and the array datatypes are `BACNET-12-ACCESS-CREDENTIAL-REQUIRED-ROWS`. The served `Out_Of_Service`, which Table 12-40 doesn't list, went in #1064 (`BACNET-12-UNDEFINED-PROPERTY-ROWS`). |
| `BACNET-12-ACCESS-CREDENTIAL-REQUIRED-ROWS` | Clause 12.35 and Table 12-40 (Global_Identifier, Reason_For_Disable, Authentication_Factors, Activation_Time, Expiration_Time, Credential_Disable, Assigned_Access_Rights); Clauses 12.35.5, 12.35.9, 12.35.10, 12.35.11, 12.35.12, 12.35.13 and 12.35.18; Clause 21 BACnetAssignedAccessRights, BACnetCredentialAuthenticationFactor, BACnetAuthenticationFactor, BACnetAuthenticationFactorType, BACnetAccessAuthenticationFactorDisable, BACnetAccessCredentialDisable and BACnetAccessCredentialDisableReason; Clause 15.5 ReadProperty and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1073). The credential serves every required row. `Global_Identifier` is a writable Unsigned32. `Reason_For_Disable` is a read-only list joining the application's reasons (`add_disable_reason`, `remove_disable_reason`), the reason the current `Credential_Disable` stands for, and DISABLED_NOT_YET_ACTIVE or DISABLED_EXPIRED judged against the database clock on each read (nothing without a usable clock frame). `Credential_Disable` (named values and 64..=65535), `Activation_Time` and `Expiration_Time` (a specific moment or all X'FF'; a partly specified one is VALUE_OUT_OF_RANGE) are writable. `Assigned_Access_Rights` and `Authentication_Factors` are read-only BACnetARRAYs of the Clause 21 element types, readable whole, by element or by size (breaking: they read as a count and a list of octet strings), and set through checked `set_assigned_access_rights` and `set_authentication_factors`; new `bacnet-encoding` codecs carry the elements. Evidence: `crates/bacnet-objects/src/access_control/{credential.rs,credential_rules.rs,credential_tests.rs,metadata_identity.rs}`, `crates/bacnet-encoding/src/constructed/{access_credential.rs,tests/access_credential.rs}`, `crates/bacnet-types/src/{constructed/access.rs,enums/access.rs,enums/tests/access_production.rs}`, `crates/bacnet-server/src/handlers/tests/{access_required_rows.rs,read_rpm/access_identity.rs}`. Not covered: the optional rows and the reasons only they drive, resizing the arrays over the network, and that a referenced Access Rights object exists. |
| `BACNET-12-ACCESS-DOOR-PULSE-TIMING` | Clause 12.26 and Table 12-30 (Door_Pulse_Time, Door_Extended_Pulse_Time, Door_Open_Too_Long_Time, Current_Command_Priority); Clauses 12.26.4, 12.26.16, 12.26.17, 12.26.19 and 12.26.39; Clause 21 BACnetOptionalUnsigned; Clause 19 command prioritization; Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1073). The door serves `Door_Pulse_Time`, `Door_Extended_Pulse_Time` and `Door_Open_Too_Long_Time` as writable Unsigned32 tenths of a second (defaults 5 s, 15 s, 30 s) and `Current_Command_Priority`, NULL on the default. A PULSE_UNLOCK or EXTENDED_PULSE_UNLOCK slot is relinquished once its pulse time has passed, by the server's existing monotonic operation task (the one Binary Lighting Output egress uses), with COV; a pulse below a live command or with a zero time is relinquished at once, and rewriting or relinquishing the slot cancels it. Evidence: `crates/bacnet-objects/src/access_control/{door.rs,door_pulse_tests.rs,metadata_topology.rs}`, `crates/bacnet-server/src/server/{binary_lighting_lifecycle.rs,access_door_pulse_task_tests.rs}`, `crates/bacnet-server/src/handlers/tests/{access_required_rows.rs,read_rpm/access_topology.rs}`. Not covered: `Door_Unlock_Delay_Time` and the door-open-too-long alarm. |
| `BACNET-12-UNDEFINED-PROPERTY-ROWS` | Clause 12 property tables: Tables 12-5 (Averaging), 12-12 (Command), 12-14 (Event Enrollment), 12-16 (File), 12-17 (Group), 12-24 (Notification Class), 12-31 (Event Log), 12-32 (Load Control), 12-34 (Structured View), 12-36 (Access Point), 12-37 (Access Zone), 12-38 (Access User), 12-39 (Access Rights) and 12-40 (Access Credential); the Status_Flags descriptions of Clauses 12.10, 12.12, 12.21, 12.27, 12.28, 12.33, 12.34 and 12.35; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1064). A sweep of every object type's served rows (metadata plus a read and write probe of each standard identifier) against its Clause 12 table removed the rows no table defines: Event Log `Out_Of_Service` and `Log_Interval`; `Out_Of_Service` on Command, Event Enrollment, Notification Class, Load Control, Access Credential and Access Rights; `Status_Flags`, `Reliability` and `Out_Of_Service` on File, Group and Structured View; `Present_Value`, `Status_Flags`, `Event_State`, `Reliability` and `Out_Of_Service` on Averaging; `Present_Value`, `Assigned_Access_Rights` and `Out_Of_Service` on Access User; `Present_Value` on Access Point; `Present_Value` and `Access_Doors` on Access Zone. They are absent from `Property_List`, the metadata, RPM `ALL`/`OPTIONAL` and PICS, and RP, RPM and WP on them return PROPERTY / UNKNOWN_PROPERTY (breaking). All came with the 0.1.0 import (7efc670a) and were recorded as compatibility or implementation-extra rows by the metadata migrations; none was a documented extension, so they go as #984, #985, #997 and #979 did. Types with `Status_Flags` but no `Out_Of_Service` hold the OUT_OF_SERVICE flag FALSE through the `no_out_of_service` form of `read_common_properties!`; a write of the stray property used to set it. The Event Enrollment evaluator no longer skips an enrollment on `Out_Of_Service`; `Event_Detection_Enable` still suspends it. Credential Data Input keeps `Out_Of_Service`, which Table 12-43 defines. Evidence: `crates/bacnet-objects/src/undefined_property_rows_tests.rs`, `crates/bacnet-server/src/handlers/tests/undefined_property_rows.rs`, the per-family metadata, `read_rpm` and PICS tests, and `crates/bacnet-server/src/event_enrollment/tests/out_of_range.rs`. Not covered: Color and Color Temperature (no 135-2020 table), the required rows these objects still don't serve, and datatypes of the remaining rows. |
| `BACNET-12-LIFE-SAFETY-GLOBAL-GROUP-REQUIRED-ROWS` | Clause 12.15, Table 12-18 (Life Safety Point) and Clauses 12.15.12 (Mode) and 12.15.13 (Accepted_Modes); Clause 12.16, Table 12-19 (Life Safety Zone) and Clauses 12.16.5 (Tracking_Value), 12.16.12 and 12.16.13; Clause 12.50, Table 12-57 (Global Group) and Clauses 12.50.9 (Event_State) and 12.50.10 (Member_Status_Flags); Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1092, the Life Safety and Global Group items). Life Safety Point and Zone serve `Accepted_Modes`, a read-only list that starts as every standard `LifeSafetyMode` and that `set_accepted_modes` replaces; a WP or WPM of `Mode` naming a mode off the list returns PROPERTY / VALUE_OUT_OF_RANGE and leaves `Mode` unchanged (breaking: any Enumerated used to be stored), while the local `set_mode` is unchecked. Life Safety Zone serves `Tracking_Value` on the Point's model: read-only in service (out-of-service writes are `BACNET-12-LIFE-SAFETY-OUT-OF-SERVICE-SIMULATION`), set by `set_tracking_value` or a reset commit (the Zone reset context and commit gained `tracking_value`), and part of the property-COV surface, so SubscribeCOVProperty on it no longer returns NOT_COV_PROPERTY. Global Group serves `Event_State`, a fixed NORMAL without intrinsic reporting, and `Member_Status_Flags`, the OR of the `Status_Flags` values held in `Present_Value`, computed on each read from the application-filled store so it follows every `Present_Value` update. Each row is in `Property_List`, the metadata, RPM `ALL`/`REQUIRED` and PICS. Evidence: `crates/bacnet-objects/src/life_safety/accepted_modes_tests.rs`, `crates/bacnet-objects/src/group/member_status_flags_tests.rs`, `crates/bacnet-server/src/handlers/tests/life_safety_mode_writes.rs`, the zone COV pins in `crates/bacnet-server/src/server/life_safety_cov_tests/event_state_pins.rs`, and the per-family metadata, `read_rpm` and PICS tests. `BACNET-12-GLOBAL-GROUP-ARRAY-ENCODINGS` covers the `Present_Value` and `Group_Members` datatypes. Not covered: the Global Group intrinsic reporting and COVU rows, and the optional Life Safety rows still unserved. |
| `BACNET-12-GLOBAL-GROUP-ARRAY-ENCODINGS` | Clause 12.50, Table 12-57 (Global Group) and Clauses 12.50.5 (Group_Members), 12.50.7 (Present_Value), 12.50.7.1 and 12.50.10 (Member_Status_Flags); Clause 21 BACnetDeviceObjectPropertyReference and BACnetPropertyAccessResult; Clause 12.1.5.1 array properties; Clause 15.5 ReadProperty and Clause 15.7 ReadPropertyMultiple | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1107). Global Group serves `Group_Members` as an array of `BACnetDeviceObjectPropertyReference` and `Present_Value` as an array of `BACnetPropertyAccessResult`, one element per member: the member's reference, then the value read in `[4]` or the error class and code in `[5]` (breaking: the references went out as application-tagged values with NULL fillers and the values bare). The application stores an `AccessResult` per member in `GlobalGroupObject::present_value`; a member with none reads PROPERTY / VALUE_NOT_INITIALIZED and results past the last member are not served. Index 0 of `Group_Members`, `Present_Value` and `Group_Member_Names` reads the size, 1 to N one element, and past N PROPERTY / INVALID_ARRAY_INDEX; any index used to return the whole array. `Member_Status_Flags` counts only bit-string values held for `Status_Flags` members. Evidence: `crates/bacnet-encoding/src/constructed/tests/property_access_result.rs`, `crates/bacnet-objects/src/group/array_tests.rs`, `crates/bacnet-objects/src/group/member_status_flags_tests.rs` and the Global Group `read_rpm` tests, RPM `ALL` included. Not covered: writable `Group_Members` with its resizing rules, and acquisition of member values by the server. |
| `BACNET-12-COMMAND-STRUCTURED-VIEW-ARRAYS` | Clause 12.10, Table 12-12 (Command) and Clause 12.10.8 (Action); Clause 12.29, Table 12-34 (Structured View) and Clauses 12.29.7 (Subordinate_List) and 12.29.8 (Subordinate_Annotations); Clause 21 BACnetActionList, BACnetActionCommand and BACnetDeviceObjectReference; Clause 12.1.5.1 array properties; Clause 15.5 ReadProperty and Clause 15.7 ReadPropertyMultiple | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1135). Command serves `Action` as an array of `BACnetActionList`, each element its `BACnetActionCommand` writes framed in `[0]` (breaking: the elements went out as application-tagged Octet Strings of opaque bytes, and `CommandObject::set_action` now takes typed lists and refuses a priority outside 1 to 16 with VALUE_OUT_OF_RANGE). Structured View serves `Subordinate_List` as an array of `BACnetDeviceObjectReference` (breaking: bare application-tagged object identifiers before; `add_subordinate` takes a reference or an `ObjectIdentifier` for a local one) and `Subordinate_Annotations` as an array of CharacterString. Index 0 of each reads the size, 1 to N one element in the octets the whole-array read concatenates, and past N PROPERTY / INVALID_ARRAY_INDEX; any index used to return the whole array. All three stay read-only, whole or indexed. Evidence: `crates/bacnet-encoding/src/constructed/tests/action_list.rs`, the Command and Structured View object tests, their `read_rpm` tests and `crates/bacnet-server/src/server/array_element_wire_tests.rs` over B/IP. Not covered: running the actions on a Present_Value write, `Action_Text` and the other optional Structured View arrays, writable `Subordinate_List`, and the Channel object, which isn't modeled. |
| `BACNET-12-LIFE-SAFETY-OUT-OF-SERVICE-SIMULATION` | Clause 12.15, Table 12-18 footnote 1 and Clauses 12.15.4, 12.15.5, 12.15.10 and 12.15.11 (Life Safety Point); Clause 12.16, Table 12-19 footnote 1 and Clauses 12.16.4, 12.16.5, 12.16.10 and 12.16.11 (Life Safety Zone); Clause 13.3.8 CHANGE_OF_LIFE_SAFETY; Clause 15.9.1.3 WriteProperty errors; Clause 21 BACnetLifeSafetyState and BACnetReliability | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1108). While Out_Of_Service is TRUE, Life Safety Point and Zone take WP, WPM and `write_local` of `Tracking_Value` (an Enumerated naming a standard `BACnetLifeSafetyState` or one from 256 to 65535) and `Reliability` (a named value or one from 64 to 65535); another number is VALUE_OUT_OF_RANGE, another datatype INVALID_DATA_TYPE, and in service both stay WRITE_ACCESS_DENIED (breaking: every write used to be refused). Metadata marks both `WhenOutOfService`, so the PICS lists them writable. Entering out of service sets the object's own values aside and the return to service restores them; meanwhile `set_tracking_value` and a reset commit's `tracking_value` update the value set aside, and the new `set_reliability_internal` is refused. A committed simulation write notifies through the existing Life Safety snapshots (Tracking_Value subscribers for a new Tracking_Value, every subscriber when Reliability flips FAULT). The object never derives Present_Value from Tracking_Value, so a simulated value latches nothing; a reset executor's context carries the served value. Silenced and Operation_Expected are untouched, and no event follows, since CHANGE_OF_LIFE_SAFETY monitors Present_Value and these objects run no intrinsic reporting. Evidence: `crates/bacnet-objects/src/life_safety/out_of_service_tests.rs`, `crates/bacnet-objects/src/reliability_writability_tests.rs`, `crates/bacnet-server/src/handlers/tests/life_safety_oos_writes.rs`, `crates/bacnet-server/src/server/life_safety_cov_tests/simulation.rs`, and the Life Safety metadata and PICS tests. #1123 added the runtime application route for both values (`BACNET-12-LIFE-SAFETY-APPLICATION-VALUES`), whose `Tracking_Value` follows the same set-aside rule. Not covered: the FAULT_LIFE_SAFETY fault algorithm, and `Reliability_Evaluation_Inhibit`, which these objects don't serve. |
| `BACNET-12-LIFE-SAFETY-APPLICATION-VALUES` | Clause 12.15 and Table 12-18 (Clauses 12.15.4, 12.15.5 and 12.15.11, Life Safety Point); Clause 12.16 and Table 12-19 (Clauses 12.16.4, 12.16.5 and 12.16.11, Life Safety Zone); Clause 13.1, Table 13-1 and Table 13-1a (Life Safety COV); Clause 13.3.8 CHANGE_OF_LIFE_SAFETY; Clause 21 BACnetLifeSafetyState | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1123). Once the server holds a Life Safety Point or Zone, `BACnetServer::set_present_value_local` sets its `Present_Value` and the new `BACnetServer::set_tracking_value_local` its `Tracking_Value` (Python: the same names), through `set_present_value_internal` and the new `BACnetObject::set_tracking_value_internal` hook, forwarded by `SourceReporter` (breaking: `set_present_value_local` used to answer OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED for these objects). Each takes an Enumerated `LifeSafetyState`, standard or from 256 to 65535; another number is VALUE_OUT_OF_RANGE, another datatype INVALID_DATA_TYPE, an unknown object UNKNOWN_OBJECT and another object type OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Each sets only its own property: `Silenced` and `Operation_Expected` stay put and neither value is derived from the other, so latching until reset is the application's (its reset executor sees what the route left). `Present_Value` is taken in or out of service, since clients never write it; a `Tracking_Value` sent out of service replaces the value set aside (`BACNET-12-LIFE-SAFETY-OUT-OF-SERVICE-SIMULATION`). Both run `write_local`'s Life Safety snapshots (Present_Value to SubscribeCOV and Present_Value property subscribers, Tracking_Value to its property subscribers) and its post-write event pass, which raises nothing for the built-in objects. Evidence: `crates/bacnet-objects/src/life_safety/application_tests.rs`, `crates/bacnet-endpoint/src/source_reporter_life_safety_tests.rs`, `crates/bacnet-server/src/server/life_safety_application_tests.rs` and `crates/rusty-bacnet/tests/test_life_safety_runtime_values.py`. Not covered: a runtime route for `Silenced` and intrinsic `CHANGE_OF_LIFE_SAFETY` reporting. |
| `BACNET-12-LOOP-PROPERTY-SET` | Clause 12.17 and Table 12-20 (Controlled_Variable_Units, Action, Priority_For_Writing; footnotes 1 to 3 for the gain-constant units rows); Clause 21 BACnetAction and BACnetEngineeringUnits; Clause 12.1.5.1; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#1062). The Loop serves every required Table 12-20 row. It gains `Controlled_Variable_Units`, `Action` and `Priority_For_Writing`, and `Proportional_Constant_Units`, `Integral_Constant_Units` and `Derivative_Constant_Units`, which the table requires alongside the gain constants it already served. `Action` is a writable `BACnetAction`, DIRECT until written; a write other than DIRECT (0) or REVERSE (1) fails with VALUE_OUT_OF_RANGE, a non-Enumerated value with INVALID_DATA_TYPE, and neither changes it. The four units rows (NO_UNITS until set) and `Priority_For_Writing` (16 until set) refuse writes with WRITE_ACCESS_DENIED; the application sets them with `LoopObject` setters that refuse units above 65535 or a priority outside 1 to 16 with VALUE_OUT_OF_RANGE. `Property_List`, the metadata, RPM `ALL`/`REQUIRED`/`OPTIONAL` and the PICS list every row, now in table order (breaking). A Loop's `Action` takes no array index (PROPERTY_IS_NOT_AN_ARRAY); `ACTION` is classified as an array only on Command. The Loop doesn't run its algorithm or command the `Manipulated_Variable_Reference` target, so `Action` and `Priority_For_Writing` describe the application's algorithm and change nothing in the object. Evidence: `crates/bacnet-objects/src/loop_obj/{metadata.rs,property_set_tests.rs}`, `crates/bacnet-objects/src/traits/defaults.rs`, `crates/bacnet-server/src/handlers/tests/{loop_properties.rs,read_rpm/loop_program.rs}`, `crates/bacnet-server/src/pics/property_metadata_tests/loop_program.rs`. Not covered: writing Present_Value to the `Manipulated_Variable_Reference` target at `Priority_For_Writing` (including while Out_Of_Service), resolving `Controlled_Variable_Reference` or `Setpoint_Reference` (#1063 gave Controlled_Variable_Value an application route instead), the optional Bias, Maximum_Output and Minimum_Output rows, intrinsic reporting, and the audit, tag and profile rows. Python's `add_loop` takes the read-only rows as keyword-only arguments, checked by the same setters. |
| `BACNET-12-REQUIRED-ROWS-UNITS-PULSE-LIGHTING` | Clauses 12.39, 12.43 and 12.44 with Tables 12-46, 12-50 and 12-51 (Units); Clause 12.23 and Table 12-27, including 12.23.5 Present_Value, 12.23.13 Adjust_Value and 12.23.14-12.23.17; Clause 13.1 and Table 13-1 (Pulse Converter row, COV_Period); Clauses 12.54 and 12.55 with Tables 12-64 and 12-69, including 12.54.17, 12.54.18, 12.54.39 and 12.55.32; Clause 21 BACnetEngineeringUnits, BACnetDateTime and BACnetOptionalUnsigned; Clause 15.5 ReadProperty, Clause 15.7 ReadPropertyMultiple and Clause 15.9.1.3 WriteProperty errors | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL`, from the #1092 list the #1064 sweep produced; each row checked against the table images. Integer, Positive Integer and Large Analog Value serve the required `Units` (NO_UNITS until `set_units`, read-only over the network, values above 65535 refused). The Pulse Converter serves the required `Count`, `Update_Time`, `Count_Change_Time` and `Count_Before_Change` (read-only) and `COV_Period` (constant 0, meaning no periodic notifications, required because the object reports COV). In service `Present_Value` is `Count` times `Scale_Factor`; `add_pulses` accumulates input and stamps `Update_Time`; an `Adjust_Value` write stores the value, keeps the old count in `Count_Before_Change`, takes the truncated quotient over `Scale_Factor` off `Count` and stamps `Count_Change_Time`, or fails with VALUE_OUT_OF_RANGE and changes nothing. Going out of service freezes and decouples `Present_Value`. SubscribeCOV notifications carry `Update_Time` (Table 13-1). Lighting Output serves `Default_Ramp_Rate` (100.0) and `Default_Step_Increment` (1.0), writable within 0.1 to 100.0, and both lighting objects serve `Current_Command_Priority` from the shared `common::current_command_priority`. All join `Property_List`, the metadata, RPM `ALL`/`REQUIRED` (`COV_Period` in `OPTIONAL`) and the PICS (breaking). Evidence: `crates/bacnet-objects/src/value_types/tests/units.rs`, `crates/bacnet-objects/src/accumulator/pulse_converter/{tests.rs,metadata_tests.rs}`, `crates/bacnet-objects/src/lighting/{required_rows_tests.rs,metadata.rs}`, `crates/bacnet-server/src/handlers/tests/{property_metadata/value_units.rs,pulse_converter_writes.rs,lighting_required_rows.rs}`, the accumulator and lighting `read_rpm`, metadata and PICS tests, and `crates/bacnet-server/src/server/pulse_converter_cov_tests.rs`. Not covered: Lighting Output `Default_Fade_Time` (still a constant 0, below its range), `Current_Command_Priority` and `COV_Increment` on the commandable value types, periodic COV notifications, and feeding `Count` from `Input_Reference`. |
| `BACNET-12-DEVICE-MAX-SEGMENTS` | Clause 12.11, Table 12-13 | P1 | `implementation-present-needs-conformance-tests` | Split child of `BACNET-12-OBJECT-MODEL` (#379). `DeviceObject::new` omits `Max_Segments_Accepted` for `NO_SEGMENTATION`, derives 1 for exact `SEGMENTED_TRANSMIT`, and retains 65 for `SEGMENTED_RECEIVE`, `SEGMENTED_BOTH`, and unknown non-NONE raw values. `device::tests::mode_derived_max_segments_accepted` covers property reads, `Property_List`, and raw `Segmentation_Supported` readback. Boundary: `DeviceConfig` and `ServerConfig` remain independent; this slice neither changes nor adds enforcement for the server's 256-segment sequence-space limit; client timers are excluded. |
| `BACNET-12-DEVICE-ACTIVE-COV-SUBSCRIPTIONS` | Clause 12.11, Table 12-13 and 12.11.31; Clause 12.1.5.2; Clauses 20 and 21 BACnetCOVSubscription, BACnetRecipientProcess, BACnetRecipient, BACnetObjectPropertyReference, ReadProperty-ACK, and ReadAccessResult productions | P1 | `in-progress` | Split child of `BACNET-12-OBJECT-MODEL` (#183, #813). The server-owned `CovSubscriptionTable` is the sole live authority for SubscribeCOV and SubscribeCOVProperty entries; `DeviceObject` no longer holds a competing manual list. Network RP, budgeted RPM (explicit, repeated, wildcard-Device, `ALL` and `OPTIONAL` rows) and `BACnetServer::read_local` resolve the selected local Device's value through one request-local projection: under the database read guard, the COV table read guard samples one monotonic instant and copies non-expired entries without purging, then is released before object reads or encoding. Direct subscribers encode as network 0 plus source MAC; routed subscribers encode the remote NPDU source, not the router MAC. Whole-object entries name Present_Value (Access_Event for Access Point, Table 13-1 footnote 1); single-property entries keep the property and absent, zero or element index. Indefinite lifetimes report zero, finite lifetimes report rounded-up seconds, and reached deadlines are omitted before the periodic purge. The increment appears only for a numeric monitored value, using the notification rule (explicit override, else the object's current `COV_Increment` for numeric Present_Value). Wire tests decode each entry across initial empty, direct and routed acceptance, renewal, exact cancellation, expiry, DeleteObject, exact peer cleanup, rejected requests, repeated-reference and selector agreement under a concurrent renew/cancel race, read-only and non-array rejection, selected-Device scope and stopped-server reporting; codec golden vectors remain in bacnet-encoding. Limits: raw `ObjectDatabase`/`DeviceObject` reads and the low-level RP/RPM helpers return the standalone empty list, the composition responder has no COV table, and SubscribeCOVPropertyMultiple contexts belong to `Active_COV_Multiple_Subscriptions` (#814). Not a full COV or BTL claim. |
| `BACNET-12-DEVICE-ACTIVE-COV-MULTIPLE-SUBSCRIPTIONS` | Clause 12.11, Table 12-13 footnote 18 and Active_COV_Multiple_Subscriptions; Clause 13.16.2; Clause 12.1.5.2; Clauses 20 and 21 BACnetCOVMultipleSubscription, BACnetRecipientProcess, BACnetRecipient, BACnetPropertyReference, ReadProperty-ACK, and ReadAccessResult productions | P1 | `in-progress` | Split child of `BACNET-12-OBJECT-MODEL` (#814). The server-owned `CovSubscriptionTable` is the canonical owner of SubscribeCOVPropertyMultiple contexts: every Multiple reference enters through `subscribe_multiple`, which stores the admitting request's maximum notification delay on each reference and refreshes the expiry and delay of the exact context (endpoint, process, form) together, last write wins; the generic `subscribe` rejects Multiple identities before any table effect. The delay is reported only and never delays a notification. Network RP, budgeted RPM (explicit, repeated, wildcard-Device, `ALL` and `OPTIONAL` rows) and `BACnetServer::read_local` share the `Active_COV_Subscriptions` request-local projection: one sampled instant under the database then table read guard, table guard released before object reads or encoding. Each entry is one context with its recipient (network 0 plus source MAC, or the remote NPDU source when routed), process, form, rounded-up remaining lifetime (the notification Time_Remaining primitive), maximum notification delay and references grouped by object with property, index, numeric increment in use and timestamped flag, in the shared recipient/coordinate order. Reached deadlines, references on deleted objects and contexts left without references are omitted; an accepted empty-spec finite request adds no row. `Active_COV_Subscriptions` never lists Multiple references. Wire tests decode every field across both forms of one recipient, direct and routed recipients, empty-spec renewal, additive re-subscription, single-reference and whole-context cancellation, expiry, DeleteObject, exact peer cleanup, atomic rejected re-subscriptions leaving references/lifetime/delay unchanged, RP/RPM/read_local agreement under a concurrent race, read-only/non-array rejection, selected-Device scope and stopped-server reporting; codec golden vectors are hand-assembled. Pre-fix reproduction at dev 59c28e2a: Property_List omitted the property and RP returned PROPERTY/UNKNOWN_PROPERTY before and after an accepted request. Limits: contexts are always finite; Property_List inclusion ignores a `set_services_supported` override that drops the service; one remote subscriber reached through two routers forms two exact-endpoint contexts with the same encoded recipient; raw object reads, low-level RP/RPM helpers and the composition responder report the standalone empty list; delayed Multiple delivery under Max_Notification_Delay is not implemented. Not a full COV or BTL claim. |
| `BACNET-12-NOTIFICATION-FORWARDER-WITHDRAWAL` | Clause 12.51 (pp. 497-503), Table 12-58 (p. 500); Clause 13.2.5.1 (p. 643); Clause 21 BACnetEventNotificationSubscription and BACnetProcessIdSelection productions (pp. 904, 924) | P1 | `unsupported-by-design` | Bundled Device objects omit type 51 from `Protocol_Object_Types_Supported`; the public Rust placeholder and Python registration method are removed because they provided no forwarding behavior. The object-type constants, CLI remote-identifier parsing, and generic recipient codecs remain as wire vocabulary, not support claims. Evidence: `crates/bacnet-objects/src/device/tests.rs::read_protocol_object_types_supported`. Reintroduction requires full Clause 12.51 forwarding, subscription, filtering, and anti-loop/port/network behavior. |
| `BACNET-12-CHANNEL-WITHDRAWAL` | Clause 12.53 (pp. 508-517), Table 12-62 (pp. 509-510) | P1 | `unsupported-by-design` | Bundled Device objects omit type 53, and public Rust/Python Channel construction is removed because the placeholder lacked the typed value, member/control-group, propagation, coercion, write-status, and applicable delay behavior. Channel enums, property identifiers, and array metadata remain protocol vocabulary. Evidence: `device::tests::read_protocol_object_types_supported`. Full Clause 12.53 behavior is required before reintroduction. |
| `BACNET-15-WRITEGROUP-SERVER-WITHDRAWAL` | Clause 15.11 (pp. 757-758); Clause 19.2.1.6 (p. 809) | P1 | `unsupported-by-design` | Bundled devices omit WriteGroup service bit 40, and the server has no inbound dispatch arm or handler. The codec, client initiation, and enums remain protocol vocabulary. Evidence: the Device service negative, dispatch cross-check, and PICS negative. Reintroduction requires matching/group-zero, priority, supported Inhibit Delay behavior, and state-mutation integration evidence. |
| `BACNET-12-RECIPIENT-LIST-FRAMING` | Clause 12.21, Clause 21 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL`. `Recipient_List` reads/writes on the Notification Class object use the Clause 21 `BACnetDestination` framing: seven application-tagged members in order concatenated as a `BACnetLIST`, `BACnetRecipient` discriminated by context tag (`device [0]` primitive ObjectIdentifier, `address [1]` constructed `BACnetAddress`). The generic recipient codec remains interoperability vocabulary and is not Notification Forwarder object support. Golden vectors cover the device, address (nonzero network), and broadcast (zero-length MAC) forms plus the MSB-first decode of a Monday-only `valid-days`; an 8-entry list round-trips at codec and object level (Annex K.2.25 AE-CRL-B minimum-entries posture); negatives reject recipient context tag [2], truncated members, unbalanced recipients, and Unsigned16 network-number overflow. Decode is strict end to end: element sequence, the fixed-width bit-string members (`valid-days`/`transitions` must be exactly one content octet with the production's unused-bit counts 1/5), and whole-list routing (a malformed stored list fails closed — no prefix delivery). #1125: the pre-#152 flat form (`PropertyValue::List` of seven-value entries) is gone; a local write of it, an empty flat list included, fails with PROPERTY / INVALID_DATA_TYPE, and routing treats a custom class serving it as an invalid list. Network writes were always framed, and neither the Python bindings nor the CLI built the flat form. The write path and routing share one capped framed decoder. #1098: the list holds at most 32 destinations (`MAX_RECIPIENT_LIST_DESTINATIONS`, four times the AE-CRL-B minimum of 8); a WriteProperty, WritePropertyMultiple or `write_local` past the cap fails with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY naming the first destination that doesn't fit, which AddListElement reports as NO_SPACE_TO_ADD_LIST_ELEMENT at the request element that brought it, and `add_destination` returns `Result`. #1124: an address MAC is at most 18 octets (`BACnetAddress::MAX_MAC_LEN`). Clause 21 leaves the OCTET STRING unbounded; the longest network-layer address in Table 6-2 is 7 octets (LonTalk Neuron_ID) and the longest link address this stack uses is B/IPv6's 18 (IPv6 address and UDP port). A destination's recipient decodes through `decode_configured_recipient`, which refuses a longer MAC, so a WriteProperty or WritePropertyMultiple fails with PROPERTY / INVALID_DATA_TYPE, AddListElement with a ChangeList-Error naming the element, and `add_destination` with the same code; a destination is then at most 47 octets and a full list at most 1,504. Routing applies the 32-destination cap to every class: a custom NOTIFICATION_CLASS object serving a longer list yields `RecipientListTooLong`, the server logs a warning, counts the transition in `EventNotificationCounters::recipient_list_too_long` (#1142) and sends nothing for that transition (never a prefix), and decoding stops at the first destination past the cap. The generic `decode_recipient` stays unbounded, since COV subscription lists and audit records report source addresses learned off the network. Evidence: `crates/bacnet-encoding/src/constructed/recipient.rs`, `crates/bacnet-encoding/src/constructed/tests/recipient.rs`, `crates/bacnet-objects/src/notification_class/{mod,recipient_list}.rs` + tests (`tests/recipient_list_cap.rs`, `tests/routed_list.rs`), `crates/bacnet-server/src/server/requests/{mutation_list_element_number_tests,recipient_mac_bound_tests}.rs`, `crates/bacnet-server/src/server/event_recipient_routing_tests.rs`, `crates/bacnet-server/src/server/event_recipient_routing_tests/suppression_counters.rs`. Remaining: interop evidence against a second implementation and indexed-array write semantics (Tranche K / #260); Notification Forwarder remains unsupported under `BACNET-12-NOTIFICATION-FORWARDER-WITHDRAWAL`. |
| `BACNET-12-EVENT-PARAMETERS-FRAMING` | Clause 12.12, Clause 21 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL`. `Event_Parameters`/`Fault_Parameters` read/write use the full Clause 21 ASN.1 CHOICE framing of `BACnetEventParameter`/`BACnetFaultParameter` (opening/closing context-tag pairs, context-tagged members, explicitly tagged inner CHOICEs, `fault-out-of-range` inner CHOICEs discovered by application tag, `BACnetPropertyStates` elements with the 135-2020 CHOICE tags). Golden vectors cover every modeled event/fault alternative; per-alternative round-trips; Opaque preserves unmodeled alternatives through a raw scanner (byte-for-byte for the constructed forms; the `none [20] NULL` primitive decodes to `Opaque{20, []}` and re-encodes in the constructed form — value-equivalent, tag form differs); decode rejects omitted/deprecated/reserved tags 6/7/12/19; framed writes with trailing garbage bytes are rejected; truncated/unbalanced negatives covered at codec and object level; the legacy flat and `OctetString`-to-`Opaque` write fallbacks are retained per #129 and still tested. Unmodeled alternatives stored as Opaque never reach the legacy little-endian evaluator (only the 0xFF-sentinel legacy form does). `BACnetPropertyStates` models every 135-2020 alternative, including signed integer-value `[41]` and the extended-value `[63]` tag/value formula; rejects reserved tags, malformed Boolean values, wrong tag forms, numeric values above `u32`, signed integers wider than `i32`, and malformed constructed proprietary bodies; and preserves primitive or constructed proprietary tags 64..=254. Event/fault parameter and notification parameter paths share the same fallible codec, while the retained flat compatibility form uses the corrected discriminants. The Event Enrollment evaluator compares Boolean, signed, unsigned, enumerated, extended, and primitive proprietary states in separate monitored value domains and retains a domain-tagged condition identity. Evidence reviewed in #353: `crates/bacnet-types/src/constructed/{property_states.rs,event_parameter/}`, `crates/bacnet-encoding/src/constructed/mod.rs` + `constructed/tests/`, `crates/bacnet-services/src/alarm_event/` + tests, `crates/bacnet-objects/src/event_enrollment/` + tests, `crates/bacnet-server/src/event_enrollment/` + tests. |
| `BACNET-12-OOS-RELIABILITY-WRITABILITY` | Clause 12.17 Table 12-20 footnote 7 and 12.17.9 Out_Of_Service (Loop Present_Value and Reliability); Clause 12 Out_Of_Service property texts (12.2/12.3/12.4/12.6/12.7/12.8/12.19/12.21/12.22 families); Clause 12.24 Schedule Reliability_Evaluation_Inhibit text; Clauses 12.15 and 12.16, Tables 12-18 and 12-19 footnote 1 (Life Safety Point and Zone); Clause 12.25 Table 12-29 and Clause 12.30 Table 12-35 (Trend Log / Trend Log Multiple); Clause 21 BACnetReliability | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#240, #252). On object types carrying Out_Of_Service and Reliability, Reliability is network-writable exactly when Out_Of_Service is TRUE (the simulation grant quoted in each carrier's Out_Of_Service property text; Loop Table 12-20 carries it as footnote 7) and validated against the BACnetReliability production: the named set derives from `Reliability::ALL_NAMED` (0..=25, 11 reserved) plus the vendor-proprietary range 64..=65535, so a future addendum constant flips from refused to accepted without a second edit — proven by an exhaustive 0..=65536 equivalence sweep with the boundary derived from the enum. In-service writes refuse PROPERTY / WRITE_ACCESS_DENIED (Clause 15.9.1.3); the evaluated value is saved on the FALSE→TRUE edge and restored on the TRUE→FALSE edge; `set_reliability_internal` carries the complementary ownership guard. The #240 sweep: Schedule's Reliability_Evaluation_Inhibit text anticipates the out-of-service client write and gets the gate (its previously unconditional store is gone); Trend Log (Table 12-29 Reliability O) and Trend Log Multiple (Table 12-35 Reliability O) carry no writability footnote and their Reliability_Evaluation_Inhibit paragraphs end without the write provision, so Trend Log's ungated store now refuses PROPERTY / WRITE_ACCESS_DENIED and Trend Log Multiple's default denial is pinned as deliberate. Evidence: `crates/bacnet-objects/src/{common.rs,loop_obj.rs,schedule/mod.rs,life_safety/out_of_service.rs,trend/}`, `crates/bacnet-objects/src/reliability_writability_tests.rs` (thirteen carriers), `crates/bacnet-server/src/handlers/tests/write_validation.rs`, `crates/bacnet-objects/src/computed_status_flags_tests.rs`, `crates/bacnet-objects/src/loop_obj/tests.rs`, `crates/bacnet-server/src/server/loop_cov_tests.rs`. #978: Loop, Schedule, Trend Log and Trend Log Multiple now compute Status_Flags instead of returning flags fixed at construction. Loop and Schedule derive FAULT from an evaluated or simulated Reliability, OUT_OF_SERVICE from Out_Of_Service and IN_ALARM from their fixed NORMAL Event_State, and a Loop COV subscriber is notified when either write changes Status_Flags. Trend Log and Trend Log Multiple derive only FAULT and IN_ALARM and hold OVERRIDDEN and OUT_OF_SERVICE FALSE per Clauses 12.25.30 and 12.30.5, so Trend Log's then non-standard compatibility Out_Of_Service never reached its flags. #978 also gave Calendar an always-clear Status_Flags; #984 removed it, with Event_State and Out_Of_Service, because Table 12-11 defines none of them (`BACNET-12-CALENDAR-PROPERTY-SET`); #997 likewise removed Status_Flags, Out_Of_Service and Reliability from Elevator Group, whose Table 12-76 has none (`BACNET-12-ELEVATOR-GROUP-PROPERTY-SET`). #985: Loop Present_Value, which footnote 7 also covers, joins the gate: network-writable only while Out_Of_Service is TRUE (WRITE_ACCESS_DENIED in service; metadata `WhenOutOfService`, so the PICS marks it writable), and the application route (`set_present_value_internal`, reached through `BACnetServer::set_present_value_local`) is refused while Out_Of_Service is TRUE. #985 also removed Trend Log's writable and Trend Log Multiple's fixed-FALSE Out_Of_Service rows, since Tables 12-29 and 12-35 define no Out_Of_Service; reads and writes now fail PROPERTY / UNKNOWN_PROPERTY. #1064 likewise removed Event Log's writable Out_Of_Service, which Table 12-31 doesn't define (`BACNET-12-UNDEFINED-PROPERTY-ROWS`). #1108: Life Safety Point and Zone are the twelfth and thirteenth carriers (footnote 1 of Tables 12-18 and 12-19), with `set_reliability_internal` added under the complementary guard and the save and restore going through `write_out_of_service_with_reliability_restore`; their `Tracking_Value` is `BACNET-12-LIFE-SAFETY-OUT-OF-SERVICE-SIMULATION`. Limitations: Trend Log reliability has no internal evaluator route yet; carriers that deny by default for lack of an arm (lighting/escalator/lift/access-zone families) are untriaged for the inverse gap. |
| `BACNET-13-COV-OBJECT-CRITERIA` | Clause 13.1 and Table 13-1 (the Access Door, Access Point with footnote 1, Credential Data Input, Load Control, Loop, Pulse Converter and Staging rows, and SubscribeCOV admission for every listed type; no Averaging row) with Table 13-1a; Clause 12.17 and Table 12-20 (Loop Controlled_Variable_Value, COV_Increment footnote 4); Clause 12.62 and Table 12-80 (Staging COV_Increment footnote 3); Clauses 13.14 and 13.15; Clause 12.5 and Table 12-5 (Averaging samples and the datatypes they may have) | P1 | `supported-with-clause-evidence` | Split child of `BACNET-13-COV-SUBSCRIPTIONS` (#985, #988). A Loop's SubscribeCOV notification carries Present_Value, Status_Flags, Setpoint and Controlled_Variable_Value and is sent when Present_Value moves by at least COV_Increment or Status_Flags changes; a Staging object's carries Present_Value, Status_Flags and Present_Stage and is also sent on any Present_Stage change, including the Status_Flags change of a target-plan completion that changes Reliability. Both serve a writable COV_Increment (non-negative finite REAL); Loop serves the required read-only Controlled_Variable_Value. #1063: a running server's application feeds it with `BACnetServer::set_controlled_variable_value_local` (also in Python), a finite REAL that other objects refuse with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED and that Out_Of_Service doesn't block, through the `set_controlled_variable_value_internal` hook and the local-write COV path; a SubscribeCOVProperty on it is notified, and a SubscribeCOV report carries the new value the next time it fires (`server/loop_controlled_variable_tests.rs`). #1083: an Averaging object, which Table 13-1 doesn't list, refuses SubscribeCOV but admits SubscribeCOVProperty and SubscribeCOVPropertyMultiple through the new `BACnetObject::supports_subscribe_cov_property` (default `supports_cov`, which the default `supports_cov_property` now follows); its properties report by Table 13-1a without Status_Flags. A running server's application feeds it samples with `BACnetServer::add_averaging_sample_local` (also in Python) through the `add_averaging_sample_internal` hook: a BOOLEAN (0 or 1), Signed, Unsigned, Enumerated or finite REAL, converted to REAL; other datatypes fail with INVALID_DATA_TYPE, non-finite REALs with VALUE_OUT_OF_RANGE and other objects with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. The statistics and sample counts change together and notify property subscriptions (`server/averaging_sample_tests.rs`, `averaging/sample_tests.rs`). The server reads the extra values through `BACnetObject::cov_reported_properties` (default keyed by object type) under the Present_Value borrow, omits a declared property missing from Property_List, keeps only trigger values (Present_Stage) in the delivered observation, and sends nothing when a listed value fails to read. A WriteProperty runs a Staging plan before its own fanout, so a stage change and the completion's flags can share one notification. Local reading of Table 13-1a: SubscribeCOVProperty reports the selected property with Status_Flags, so a Present_Value property subscription carries neither Setpoint nor Present_Stage. Evidence: `crates/bacnet-objects/src/{traits.rs,traits/defaults.rs,loop_obj.rs,staging.rs}`, `crates/bacnet-server/src/cov/{reported.rs,observation.rs}`, `crates/bacnet-server/src/server/cov_notifications.rs`, `crates/bacnet-server/src/server/{loop_cov_tests.rs,staging_cov_tests.rs}`, `crates/bacnet-objects/src/loop_obj/tests.rs`, `crates/bacnet-objects/src/staging/tests.rs`. #1061: every type Table 13-1 lists that the stack builds takes SubscribeCOV; Access Point, Credential Data Input and Load Control refused it before. The `cov_reported_properties` default now covers every row with extra values: Access Door Door_Alarm_State (trigger); Access Point Access_Event_Tag, Access_Event_Time (trigger), Access_Event_Credential and Access_Event_Authentication_Factor; Credential Data Input Update_Time (trigger); Load Control Requested_Shed_Level, Start_Time, Shed_Duration and Duty_Window (all triggers); Pulse Converter Update_Time (reported only, #1092). An Access Point report leads with Access_Event, which doesn't trigger, instead of Present_Value; the server picks that leading value by type (`cov/reported.rs` `lead`), the same property Active_COV_Subscriptions names. Credential Data Input's Present_Value stays a trigger, which adds no report since Clause 12.36 moves Update_Time on every Present_Value update. Unserved rows (Access_Event_Credential, Access_Event_Authentication_Factor, Duty_Window; #1092) are left out of the reports. Door_Alarm_State, Update_Time and the Access Point event rows have no runtime route yet; the new setters (`AccessDoorObject::set_door_alarm_state`, `AccessPointObject::set_access_event`, `CredentialDataInputObject::set_update_time`) work before the server holds the object, so the wire tests (`server/table_13_1_cov_tests.rs`, `server/pulse_converter_cov_tests.rs`) replace the stored object and run the fanout; `cov_criteria_tests.rs` pins admission and the reported rows per type. Not claimed: the Access Door rows Table 12-30 makes writable while Out_Of_Service, a Load Control Start_Time write route, the datatypes of Credential Data Input Present_Value and of the BACnetTimeStamp and BACnetShedLevel rows (reported as ReadProperty serves them), and the server following Loop's Controlled_Variable_Reference. Averaging's sliding window and no-sample INF / NaN / -INF values (#1092) and the server's own sampling of its Object_Property_Reference (#1144) are `BACNET-12-AVERAGING-WINDOW`; the server samples fan out to property subscribers as a background commit (`server/averaging_sampling_tests.rs`). Loop's other required rows are `BACNET-12-LOOP-PROPERTY-SET` (#1062). |
| `BACNET-12-RELINQUISH-DEFAULT-WRITABILITY` | Clause 12.3 Table 12-3 (Analog Output), Clause 12.7 Table 12-8 (Binary Output), Clause 12.8 Table 12-10 (Binary Value), Clause 12.19 Table 12-22 (Multi-state Output), Clause 12.20 Table 12-23 (Multi-state Value), Clause 12.26 Table 12-30 (Access Door), Clause 12.54 Table 12-64 (Lighting Output), Clause 12.55 Table 12-69 (Binary Lighting Output), Clause 12 value object tables; Clause 19 command prioritization | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#270). `Relinquish_Default` is network-writable and locally settable on the commandable object types — permitted writability implemented, not a conformance upgrade: the conformance tables carry it R or O and permit but do not require writes. Each arm routes through a validated local `set_relinquish_default` and re-resolves Present_Value from the priority array afterward, so an all-NULL array adopts the new default immediately while a live command still outranks it. Validation is property- and type-specific: finite Real (AO/AV), BinaryPV 0/1 (BO/BV), Unsigned 1..=Number_Of_States (MSO/MSV; shrink interplay stays a local matter per Clauses 12.19/12.22 and is not auto-adjusted — CONFIGURATION_ERROR reporting tracks #226), finite Real in 0..=100 (Lighting Output), BACnetDoorValue LOCK or UNLOCK (Access Door; #1073 narrowed it from 0..=3 to the two Clause 12.26.11 admits), and per-type extraction plus a finite check for Large Analog Value (Double). Binary Lighting Output `Relinquish_Default` is restricted to BACnetBinaryLightingPV OFF(0) or ON(1), deliberately separate from Present_Value operation inputs and steady Priority_Array OFF/ON/NULL storage. Values outside OFF/ON refuse atomically with PROPERTY / VALUE_OUT_OF_RANGE, wrong datatype remains PROPERTY / INVALID_DATA_TYPE, an active ordinary command still outranks the default, and all-NULL recaptures it. Direct evidence is `binary_lighting_output_relinquish_default_{accepts_off_on_and_recaptures_present_value,rejects_non_binary_values_atomically,wrong_type_is_atomic}` and `binary_lighting_output_active_command_outranks_relinquish_default`; focused wire evidence is `binary_lighting_output_relinquish_default_{accepts_off_on_over_write_property,rejects_non_binary_values_over_write_property}` plus `binary_lighting_output_failed_wpm_restores_default_present_value_and_priority`. WARN, WARN_OFF, WARN_RELINQUISH, and STOP are now evidenced separately by BACNET-12-BINARY-LIGHTING-OPERATIONS; this row remains bounded to the OFF/ON Relinquish_Default proof. With #182 the datetime-paired value types (DateTime Value, Clause 12.38 Table 12-45; DateTime Pattern Value, Clause 12.46) join the network-writable set: the BACnetDateTime pair decodes from application-tagged Date+Time in the loop decoder, and `datetime_value_relinquish_default_write_recaptures_present_value` replaces the pinned-denial test. Evidence also remains in the per-type pins across `crates/bacnet-objects/src` plus `crates/bacnet-server/src/handlers/tests/{write_validation,multi_element_writes}.rs`; all twelve value types go through `define_value_object_commandable!` with per-type `rd_validate` and required metadata-derived property presence/writability. Issue #821 removes the unused private access selector and no-metadata fallbacks while preserving the validated write path. The Access Door Present_Value arm, which used to accept any Enumerated, now shares the `Relinquish_Default` check (#979, `BACNET-12-ACCESS-DOOR-DOOR-VALUE`), and the door's `property_list` now lists PRIORITY_ARRAY and RELINQUISH_DEFAULT. |
| `BACNET-12-BINARY-LIGHTING-OPERATIONS` | Clause 12.55 and Table 12-70, including 12.55.4.1 and 12.55.10.1; Clause 19.2 command prioritization; Clause 21 BACnetBinaryLightingPV | P1 | `supported-with-clause-evidence` | Binary Lighting Output stores only steady OFF/ON/NULL priority values while interpreting WARN, WARN_OFF, WARN_RELINQUISH, and STOP as operations. The object owns at most one process-local arm-relative monotonic deadline; eligible WARN_OFF/WARN_RELINQUISH retain the existing highest-priority ON slot for snapshotted Egress_Time, zero time mutates synchronously, same-priority STOP cancels without changing the slot, and validated same/higher non-STOP commands complete the prior terminal mutation before the incoming command. Direct Priority_Array[index] OFF/ON/NULL compatibility follows the same halt-before-incoming ordering, while lower writes leave the operation and deadline intact. A hidden database-bound Tokio monotonic source gives arming and lifecycle expiry one time domain without pre-arm charging; delayed wakes use actual time. Expiry clones COV-readable terminal state with the mutation under the database lock, then generic single/multiple encoding and network delivery use the immutable snapshot after lock release, so intervening commands cannot rewrite the terminal payload. A successful WPM prefix that arms, halts, or updates command state remains committed if a later write fails; the failing write itself is mutation-free. Evidence: `crates/bacnet-objects/src/lighting/{binary.rs,binary_tests.rs,binary_direct_tests.rs}`, `crates/bacnet-objects/src/{database.rs,traits.rs}`, `crates/bacnet-server/src/handlers/tests/binary_lighting_operations.rs`, and `crates/bacnet-server/src/server/{binary_lighting_lifecycle.rs,binary_lighting_task_tests.rs,cov_notifications.rs,lifecycle.rs,shutdown.rs}`. Logical blink observation is internal/test-only: no physical output, public callback, Python API, persistence, restart replay, README/PICS/BIBB, or hardware claim. |
| `BACNET-12-REFERENCE-PROPERTY-WRITABILITY` | Clause 12.17 with Table 12-20 (Loop), Clause 12.23 with Table 12-27 (Pulse Converter Input_Reference), Clause 12.5 Table 12-5 (Averaging Object_Property_Reference - BACnetDeviceObjectPropertyReference), Clause 21 BACnetObjectPropertyReference / BACnetSetpointReference productions | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#182). The Loop `Controlled_Variable_Reference`, `Manipulated_Variable_Reference`, and `Setpoint_Reference` properties and the Pulse Converter `Input_Reference` are `BACnetObjectPropertyReference` — a SEQUENCE of primitive context-tagged members [0]/[1]/[2] (Setpoint_Reference nested `[0]` inside the `BACnetSetpointReference` production). A strict typed codec in `crates/bacnet-encoding/src/constructed/object_property_reference.rs` (built on the tranche-J DOPR body decoder, then narrowed) covers the bare members and the wrapped Setpoint form: full consumption is required, and a device-qualifying member [3] is rejected — unlike `BACnetDeviceObjectPropertyReference`, these references are local-device only. Each object arm (Loop ×3, Pulse Converter ×1, and after the review round Averaging ×1) accepts BOTH the legacy local `List([ObjectIdentifier, property-id, Unsigned?])` form — exact shape, no silent member dropping, the property-id member tolerated as Unsigned OR Enumerated (Averaging's flat form carries Unsigned and keeps it on reads; the framed form stays the Clause 21 encoding) — and the framed network form handed over as one or more `ApplicationData` elements by the multi-element service decode, via a shared arm helper (`crates/bacnet-objects/src/reference.rs`): wrong value datatype → PROPERTY / INVALID_DATA_TYPE; malformed framing, device qualification, or an unknown trailing context tag → PROPERTY / INVALID_DATA_ENCODING (Clause 15.9.1.3 pairings) with the stored reference untouched. `Null` still clears, and the read arms now carry the reference's optional array index as a third list element. Review round: the adversary lane's blocker on the Averaging arm (silent member drop / member retype to no-index / `as u32` truncation, newly network-reachable via the loop decode) is fixed by routing that arm through the same shared decode; the runtime+dataflow empty-frame finding is fixed — the `BACnetSetpointReference` frame with its OPTIONAL member absent (`0x0E 0x0F`, the Clause 12.17 case using the stored Setpoint) now CLEARS `Setpoint_Reference` exactly like a `Null` write instead of drawing INVALID_DATA_ENCODING on a conformant peer (`decode_setpoint_reference` returns `Option`); the dataflow PICS incoherence is fixed by exact `is_writable_property` overrides on Pulse Converter (PRESENT_VALUE / SCALE_FACTOR / ADJUST_VALUE / INPUT_REFERENCE + DESCRIPTION / OUT_OF_SERVICE / COV_INCREMENT) and Averaging (OBJECT_PROPERTY_REFERENCE + DESCRIPTION / OUT_OF_SERVICE) — both ALSO stop advertising OBJECT_NAME, which no arm routes (truth-toward-arms PICS correction; writability probes pinned in `pics/truth_source_tests.rs`). Evidence: codec golden vectors + negatives (`constructed/object_property_reference/tests.rs`), the arm-helper shape matrix (`reference/tests.rs`), per-arm object tests (`loop_obj.rs`, `accumulator.rs`, `averaging.rs`), and wire-level WriteProperty / WritePropertyMultiple tests incl. in-order prefix commit and failure preservation (`crates/bacnet-server/src/handlers/tests/reference_writes.rs`). Limitation: reads still emit the flat application-tagged list form rather than re-framing the context tags — the encode-side framing of these properties is unchanged by this tranche (wire-minimal); interop evidence against a second implementation remains open. |
| `BACNET-12-TIME-DELAY-NORMAL` | Clause 13.3.2 CHANGE_OF_STATE, Clause 13.3.4 COMMAND_FAILURE, Clause 13.3.6 OUT_OF_RANGE (pTimeDelayNormal definitions and condition letters); Clause 12.2 Table 12-2 (Analog Input, O5), 12.3 Table 12-3 (Analog Output, O4), 12.4 Table 12-4 (Analog Value, O6), 12.6 Table 12-6 (Binary Input, O7), 12.7 Table 12-8 (Binary Output, O6), 12.8 Table 12-10 (Binary Value, O8), 12.18 Table 12-21 (Multi-state Input, O5), 12.19 Table 12-22 (Multi-state Output, O3), 12.20 Table 12-23 (Multi-state Value, O6) | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#225). `Time_Delay_Normal` (property 356) is the `pTimeDelayNormal` backing store on the nine intrinsic-reporting object types, and direction selection lives once in `crates/bacnet-objects/src/event.rs` (`delay_toward`): every indication into an OFFNORMAL state — including offnormal→offnormal re-indication (13.3.2 (c), 13.3.6 (d)/(g)) — is seeded with `Time_Delay`, while the sustained-condition return to NORMAL (13.3.2 (b), 13.3.4 (b), 13.3.6 (e)/(h)) is seeded with `Time_Delay_Normal`, whose absent value takes on `Time_Delay` exactly as the fallback text requires (detector and object constructors keep it `None`, never an error). Mid-delay re-seed uses the current target's delay; condition-revert cancellation is unchanged (a revert to the confirmed state leaves nothing pending). The shared read arm returns the effective delay, the shared write arm validates Unsigned within the u32 span (PROPERTY / INVALID_DATA_TYPE or VALUE_OUT_OF_RANGE on refusal, stored value preserved), and `property_list` + `is_writable_property` + PICS share the one truth source. Conformance-code verification against the extract handled its two-column interleave by count-based alignment (AI 42↔42: `Time_Delay`=O3,5 at line 11845, `Time_Delay_Normal`=O5 at 11860; AO 45↔45; AV 44↔44; BI 37↔37; BV 44↔44; MSI 37↔37; MSV 39↔39), MSO Table 12-22 read in exact name/type/code triples (O3 at 19412-19414), and every O-coded footnote body confirmed as restricting those properties to objects with intrinsic-reporting support (MSO's footnote-3 marker was lost to the interleave but pinned by the EAI=O3,4 / REI=O5 cross-checks). No table marks the property writable (W); acceptance mirrors `Time_Delay` (#229 rationale — otherwise the asymmetry is not commissionable). Reset mirrors `Time_Delay` (the EDE=FALSE restoration path touches neither delay; nothing invented). Evidence: `event/time_delay_normal_tests.rs` (nine detector-level tests), the extended shared round-trip helpers in `binary/` and `multistate/` `generic_event_properties.rs` (BI/BV/MSI/MSV/MSO), `analog/tests/input.rs` (AI/AO/AV round-trip + fallback + AI asymmetric gating), `binary/tests/command_failure.rs` (BO asymmetric gating), and wire-level `crates/bacnet-server/src/handlers/tests/write_validation.rs::time_delay_normal_round_trips_over_write_property_and_read_property`. Limitation (pre-existing, deferred to the evaluator tranche): OUT_OF_RANGE's no-delay returns when a limit is disabled (13.3.6 (c)/(f)) are not separately modeled — a LIMIT_ENABLE write disabling an active limit still returns through the deadband path gated by `pTimeDelayNormal`. |
| `BACNET-13-EVENT-ENROLLMENT-EVALUATOR` | Clause 12.12 with Table 12-14 (Event Enrollment Object Type; configured Event_Type excludes CHANGE_OF_RELIABILITY; Time_Delay_Normal Unsigned, conformance O, extract 16444-16446, property text 16887-16889); Clause 12.12 Table 12-15 (Time_Delay -> pTimeDelay mapping for every evaluated algorithm); Clause 13.2.2.1 (changed nonzero Reliability while FAULT is a To-Fault re-entry); Clause 13.2.2.1.4 (transition actions incl. the same-state rule); Clause 13.2.3 (Acked_Transitions on a received transition); Clause 13.2.5.2 Table 13-3 and Clause 13.2.5.3 (CHANGE_OF_RELIABILITY whenever From or To is FAULT); Clause 13.3 common introduction and 13.3.1/13.3.2/13.3.3/13.3.5/13.3.6 direction rules with the pTimeDelayNormal fallback; Figure 13-10 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (tranche C2: #163, #166, #137). The Event Enrollment evaluator honors both delays: `Event_Parameters.Time_Delay` gates OFFNORMAL-directed transitions and the EE object's new optional `Time_Delay_Normal` property gates NORMAL-directed ones (read-back falling back to `Time_Delay` exactly as Clause 13.3 requires; writable as Unsigned per the Clause 12.1.2 option; in `Property_List` and PICS). The object-owned (in-memory, internal-channel, reset by `Event_Detection_Enable` FALSE) pending countdown advances once per `event_enrollment_task` interval: revert cancels, redundant observations never re-seed, a changed target re-seeds with the new direction's delay, and a mid-pending parameter change (per-pass fingerprint over framed parameters + effective TDN + event type) cancels and re-gates. Per Clause 13.2.2.1.4 the actions run for same-state transitions too: the SPECIFIC state is stored (never collapsed to OFFNORMAL), `Acked_Transitions` is maintained per Clause 13.2.3 via a new guarded internal trait method (clear when the referenced Notification Class's `Ack_Required` marks the direction ack-owed, else set; unresolvable class = not-required), and the transition is emitted with its `Event_Enable`-scoped `distribute`. CHANGE_OF_STATE condition (c) is implemented via the retained last-offnormal-causing value (also object-owned); CHANGE_OF_BITSTRING's (c) is deliberately not (documented; no bitstring baseline is retained). CHANGE_OF_VALUE tracks the Clause 13.3.3 detection baseline: both criteria compare against the value at the last indicated NORMAL transition (REAL `|current − baseline| >= pIncrement`, positive only; BIT STRING masked-bit change), the sole indication is NORMAL→NORMAL per Figure 13-10, and the first observed sample initializes the baseline without indicating — the clause's explicit local matter. Evidence: `crates/bacnet-server/src/event_enrollment/{mod.rs,algorithms.rs}`, `crates/bacnet-objects/src/event_enrollment/mod.rs`, `crates/bacnet-objects/src/traits.rs`; tests `event_enrollment/tests/{delays,same_state,change_of_value,compat}.rs`, `server/event_enrollment_task_tests.rs` (start_paused lifecycle), objects `event_enrollment/tests/time_delay_normal.rs`, with pre-existing suites (`time_delay: 0` fixtures) green unchanged except the two tests that pinned the pre-#137 absolute-magnitude COV behavior (rewritten). Manually killed mutants: swapped delay directions (3 tests), always-reseed countdown (11 tests). Fences: no notification send (#127), no `Event_Time_Stamps`/`Event_Message_Texts` (#264), no `Status_Flags` algorithm input (pStatusFlags takes the spec's all-FALSE absent-value fallback; Table 12-15.1 fetching is tranche-E follow-up), no intrinsic-detector changes, no legacy-`Opaque` behavior change. Remaining gaps: OUT_OF_RANGE 13.3.6 (c)/(f) no-delay returns on a disabled limit (inherited from `BACNET-12-TIME-DELAY-NORMAL`'s limitation); EE fault precedence (no Reliability term evaluated) unchanged and out of scope; downstream custom EE-typed objects that skip the new internal channel fail closed (TD=0 transitions as before; nonzero delays and COV inert). PR-#290 review round absorbed: (B1) delays converted to wall-clock seconds via `ceil(delay_secs / interval_secs)` (the countdown previously counted passes); (B2) each arm names its reachable state set (OOR/FL {NORMAL, HIGH_LIMIT, LOW_LIMIT}; COS/COBS {NORMAL, OFFNORMAL}; COV {NORMAL}) and recovers foreign states by indicating the NORMAL-perspective computation through the delay-gated actions path (COV's recovery installs the sample as baseline); (B3) EE implements `acknowledge_alarm` (idempotent set per 13.2.3) with OBJECT/NO_ALARM_CONFIGURED while detection is disabled (Table 13-10), closed by the wire-level `handlers/tests/acknowledge_alarm_ee.rs` round trip through `handle_get_event_information`; (F4) fingerprint cancellation persisted before later exits (A→B→A round trip cannot resume); (F5) monitored reference folded into the fingerprint; (F2) COBS matcher compares the full `max(mask, value)` width zero-filled; (N2) COS identity strictness re-anchored on 13.3.2 (c). |
| `BACNET-15-ARRAY-INDEX-GATING` | Clause 15.5.1.3, Clause 15.9.1.3 (with Clause 12.1.5); Clause 15.7.3.2.2.2 (printed744/PDF746); Clause15.10.1.3 (printed754-755/PDF756-757) | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#190, #260, #266). ReadProperty, ReadPropertyMultiple, WriteProperty, and WritePropertyMultiple reject an array index supplied for a property that is not a BACnetARRAY with PROPERTY / PROPERTY_IS_NOT_AN_ARRAY; the classification is one per-object query (`BACnetObject::is_array_property`, default keyed by the Clause 12 object tables), so it cannot drift between services. Identifier-stable BACnetARRAY properties modeled in-tree admit an index (OBJECT_LIST Table 12-13, PROPERTY_LIST, STATE_TEXT Tables 12-21/12-22/12-23, PRIORITY Table 12-24, WEEKLY_SCHEDULE/EXCEPTION_SCHEDULE Table 12-28, EVENT_TIME_STAMPS/EVENT_MESSAGE_TEXTS in the analog/binary/multi-state object tables, PRIORITY_ARRAY, TAGS Annex Y, SUBORDINATE_LIST/SUBORDINATE_ANNOTATIONS Table 12-34, GROUP_MEMBERS/GROUP_MEMBER_NAMES Table 12-57, STAGES/STAGE_NAMES/TARGET_REFERENCES Table 12-80); identifier-stable BACnetLIST properties (DATE_LIST Table 12-11, LIST_OF_GROUP_MEMBERS Table 12-17, RECIPIENT_LIST Table 12-24, LOG_BUFFER Tables 12-29/12-31, DEVICE_ADDRESS_BINDING and ACTIVE_COV_SUBSCRIPTIONS Table 12-13) and all scalars reject it, since Clause 12.1.5.2 makes ReadRange the only positional access to a BACnetLIST; the type-dependent identifiers classify by object type (ACTION array on Command per Table 12-12 but a single BACnetAction on Loop per Table 12-20, #1062; ALARM_VALUES/FAULT_VALUES array on CharacterString/BitString Value per Tables 12-44/12-47, LIST_OF_OBJECT_PROPERTY_REFERENCES array on Channel per Table 12-62 but a list on Schedule/Timer per Tables 12-28/12-75, PRESENT_VALUE BACnetARRAY on Global Group per Table 12-57). `Event_Time_Stamps` and `Event_Message_Texts` now implement the Clause 12.1.5.1 BACnetARRAY[3] read contract across all nine modeled intrinsic-reporting families: omitted index returns the ordered three transition slots, index 0 returns Unsigned(3), indexes 1..3 return one element, and larger indexes return PROPERTY / INVALID_ARRAY_INDEX. `Event_Time_Stamps` preserves the Clause 21 BACnetTimeStamp CHOICE and Clause 20.2 tag identities (Time [0], SequenceNumber [1], DateTime [2]) through object reads and RP/RPM encoding; `Event_Message_Texts` remains application-tagged CharacterString. `EVENT_MESSAGE_TEXTS_CONFIG` remains absent because no object-side model exists. Other array-typed identifiers whose object types are not modeled in-tree (ACTION_TEXT, VALUE_SOURCE_ARRAY, ...) stay rejected until their object-side modeling lands, so coverage is complete for the modeled set rather than for every array in the standard. The RP whitelist that admitted six always-list identifiers is gone; RPM gates per reference with inline error elements while sibling references succeed; WP gates before decode/mutation; WPM applies writes in order and retains the successful prefix before a gated failure. Indexed `Recipient_List` reads and writes fail with the same gate classification, replacing the tranche-J INVALID_DATA_TYPE stopgap; the framed wire form is unchanged. #266: omitted-index (whole-array, Clause 12.1.5.1) `Priority_Array` writes are PROPERTY / WRITE_ACCESS_DENIED — a mappable Result(-) — while indexes 0 and 17 stay INVALID_ARRAY_INDEX, pinned at both macro sites and over the network WP/WPM paths. Evidence: `crates/bacnet-objects/src/event/history.rs`, `crates/bacnet-encoding/src/primitives/mod.rs`, `crates/bacnet-server/src/handlers/{read_property,write_property}.rs`, `crates/bacnet-server/src/handlers/tests/{array_index_gating,read_event_arrays,read_rpm}.rs`, and analog/binary/multi-state event-history tests. Limitations: every array the in-tree objects serve reads one element per index (Global Group since #1107, Structured View and Command since #1135, BACNET-12-COMMAND-STRUCTURED-VIEW-ARRAYS), and Channel has no object in-tree; CharacterString/BitString Value ALARM_VALUES/FAULT_VALUES remain classified as arrays; effective write absence is handled at the indexed gate. #182 review round: the five reference-typed properties (Loop ×3, Pulse Converter Input_Reference, Averaging Object_Property_Reference) are pinned rejecting indexed ReadProperty with the same PROPERTY / PROPERTY_IS_NOT_AN_ARRAY pair (`reference_properties_reject_indexed_read_property`). #881: at the existing indexed WP/WPM service gate only, nonempty effective metadata that omits the property yields PROPERTY/UNKNOWN_PROPERTY before value decode, WPM authorization and write observer/source/Audit hooks. Absence-first is the selected local simultaneous-fault interpretation of Clauses15.9-15.10 (printed752-755/PDF754-757), not an explicit normative precedence rule. Empty optional custom metadata retains classifier/writer delegation; served scalars and BACnetLISTs retain PROPERTY_IS_NOT_AN_ARRAY and served arrays keep object-owned write/count/range rules. Encoded AI and B/IP raw5555 plus unprovisioned Staging names cover valid/malformed values, while present Staging arrays, read-only B/IP DNS and a custom vendor array supply controls. WPM preserves its Description prefix and exact failed object/property/index, with no failing/suffix authorization, observer, source write or Audit record. A running-server fixture retains actual successful Audit delivery and source-aware writing. Unindexed writes, direct object calls, outer WP authorization and read service precedence are unchanged. Evidence: `handlers/tests/{indexed_write_presence,indexed_write_effects}.rs` and `server/audit_reporter_indexed_absence_tests.rs` under `crates/bacnet-server/src`. |
| `BACNET-15-WP-EVENT-FIELD-VALIDATION` | Clause 15.9.1.3 (WriteProperty error table) with Clause 21 BACnetNotifyType / BACnetEventTransitionBits / BACnetLimitEnable productions | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#255; behavior change — previously-accepted invalid writes now fail). WriteProperty validation of the shared intrinsic-reporting event properties matches the Clause 15.9.1.3 error table: `Notify_Type` (BACnetNotifyType, a closed {alarm, event, ack-notification} production, Clause 21) refuses out-of-production values with PROPERTY / VALUE_OUT_OF_RANGE (a value outside the property's permitted range) instead of storing them — an accepted `Enumerated(99)` previously read back as 99 and could reach the wire as the notification's notifyType; membership derives from `NotifyType::ALL_NAMED` so a future addendum constant widens the gate without a second edit. `Event_Enable` / `Limit_Enable` (BACnetEventTransitionBits = 3 bits, BACnetLimitEnable = 2 bits, Clause 21) require the canonical encoding of their fixed-width production — exactly one content octet with 8−N declared unused bits, the form the read path emits — refusing any other declared shape (an 8-bit string where fewer bits are defined, extra or missing content octets, a half-octet string; for Limit_Enable even Event_Enable's valid 3-bit shape) with PROPERTY / INVALID_DATA_ENCODING (an encoding incompatible with the property's datatype) instead of silent mask-and-normalize; an empty content now reports INVALID_DATA_ENCODING rather than INVALID_DATA_TYPE. Applies to the generic macro arms (AI/AO/AV/BI/BO/BV/MSI/MSO/MSV), the analog macro's Limit_Enable, and the own arms on EventEnrollment and AlertEnrollment, via a shared objects-layer helper (`common::check_fixed_width_bit_string`) that reports protocol errors — the decoding-layer sibling in `bacnet-encoding` is intentionally not reused across layers. Evidence: object-level probes across `crates/bacnet-objects/src/{analog/tests/input.rs,binary/tests/generic_event_properties.rs,multistate/tests/generic_event_properties.rs,event_enrollment/tests/enrollment.rs}` and wire-level `crates/bacnet-server/src/handlers/tests/write_validation.rs`. Follow-up family: the Notification Class recipient-list decoder still masks transition/day bit strings without consulting declared counts. |
| `BACNET-15-STRUCTURED-WRITE-DECODE` | Clause 15.9 WriteProperty (15.9.1.2 Result(+), 15.9.1.3 Result(-)), Clause 15.10 WritePropertyMultiple, Clause 20.2.1 (concatenated elements) | P1 | `supported-with-clause-evidence` | Split child of `BACNET-12-OBJECT-MODEL` (#182; behavior change — previously-dropped trailing bytes now decide the write). The WriteProperty/WritePropertyMultiple value decoder consumed exactly one application-tagged primitive and silently discarded the rest, so no structured or `BACnetLIST` property value could reach an object arm whole. `decode_write_property_value` (`crates/bacnet-server/src/handlers/write_property.rs`) now loop-decodes until the payload is exhausted — the exact mirror of `encode_property_value`'s `List` flattening: one element → scalar `PropertyValue`, more than one → `PropertyValue::List`, and full consumption is REQUIRED — a partial or undecodable trailing element is PROPERTY / INVALID_DATA_ENCODING (Clause 15.9.1.3) and an empty payload is refused, while a well-formed extra element reaches a scalar arm and fails its shape as INVALID_DATA_TYPE; in every case the stored value is proven unchanged. The RECIPIENT_LIST verbatim special-case keeps its precedence (the object layer owns its framed `BACnetLIST` codec), and the context-tagged behavior is preserved: framed CHOICE properties (Event_Parameters/Fault_Parameters) still arrive as a single `ApplicationData`, and context-tagged member productions (the Clause 12.17 references) arrive as one `ApplicationData` element per member — see `BACNET-12-REFERENCE-PROPERTY-WRITABILITY`. A value TLV-truncated so far that framing breaks is refused earlier by the service request's own [3]/[4] walk (a framing error, never silent). Unlocks: MSI `Alarm_Values` whole-list writes (`whole_list_write_property_decodes_all_elements`, was a pinned failure) and the datetime-paired value properties (DateTime Value PV / priority-array entries / Relinquish_Default, completing #270 — see `BACNET-12-RELINQUISH-DEFAULT-WRITABILITY`). WPM ordered-prefix execution covers structured values (`wpm_reference_write_commits_in_order_and_keeps_prefix_on_failure`). Evidence: `crates/bacnet-server/src/handlers/write_property.rs` + `handlers/tests/multi_element_writes.rs` + `handlers/tests/reference_writes.rs`, `handlers/list.rs` tests. Known residues (follow-ups): AddListElement/RemoveListElement still break their `listOfElements` loop on an undecodable element instead of erroring (#182's wider scope); Enumerated contents wider than four octets truncate to u32 (#277, untouched here); and properties declared `SEQUENCE OF` beyond the four reference arms (e.g. `LIST_OF_OBJECT_PROPERTY_REFERENCES`) keep their pre-existing arm-level behavior. |
<!-- END ledger-rows -->

`BACNET-12-ALERT-ENROLLMENT-TABLE-12-61` supersedes the generic
`BACNET-15-WP-EVENT-FIELD-VALIDATION` wording for Alert Enrollment's
`Notify_Type`: that configurable property accepts only `ALARM` and `EVENT`;
`ACK_NOTIFICATION` remains acknowledgement-flow output vocabulary.

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-15-WPM-ORDERED-PREFIX-ERROR
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-15-WPM-ORDERED-PREFIX-ERROR` | Clause 15.10 and 15.10.1.3 (WritePropertyMultiple service procedure and Result(-)); Clause 18.9 (Reject reasons); Clause 21 (Error and BACnetObjectPropertyReference productions) | P1 | `supported-with-clause-evidence` | Service 16 parses and executes complete write attempts in wire order, retains the successful prefix, stops on the first failure, and emits the formal `[0] Error` plus `[1] BACnetObjectPropertyReference` Result(-) body for semantic or post-prefix syntax failures. Pre-write malformed bodies use the narrow classified Reject reason; post-prefix malformed bodies use SERVICES / INVALID_TAG and an exact completed reference or the documented DEVICE:4194303 / ALL / no-index local sentinel. Response emission precedes committed-prefix generic and exact Life Safety COV. Low-level `ErrorPdu.error_data` retains provenance; legacy generic WPM errors and high-level class/code projections remain compatible. Empty requests/property lists remain no-op. This is a bounded service correction (`Refs #242`), not a broad PICS/BIBB or Acked_Transitions claim. |
<!-- END ledger-rows -->

This row governs Service 16 references in earlier rows. Retained object
snapshot hooks are compatibility evidence, not bundled-server behavior;
Service 16 no longer calls those hooks.

`BACNET-15-STRUCTURED-WRITE-DECODE`'s generic empty-payload refusal has two
property-specific exceptions: Clause 12.60 permits an empty `Fault_Signals`
BACnetLIST and Clause 12.9 an empty Calendar `Date_List` (#996), which the
service decoder delivers as an empty `PropertyValue::List`.

### C3 Partial Evidence

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-13-EVENT-DISABLE-WPM-PREFIX
BACNET-13-LIFE-SAFETY-OPERATION
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-13-EVENT-DISABLE-WPM-PREFIX` | Clause 12.52 Table 12-61 (Alert Enrollment Event_State, Acked_Transitions, Event_Detection_Enable); Clause 13.2.2.1 disabled-state initial conditions; Clause 13.3 pTimeDelayNormal fallback; Clause 15.10 ordered WritePropertyMultiple procedure | P1 | `in-progress` | Alert Enrollment applies the modeled disabled-state initial conditions. Object-owned snapshot tokens remain a directly testable compatibility surface, but bundled-server Service 16 does not invoke them: a successful Event_Detection_Enable or Time_Delay_Normal prefix write remains committed when a later write fails, and the currently failing write must be mutation-free. The required Alert property projection is covered separately; no evaluator claim is made. |
| `BACNET-13-LIFE-SAFETY-OPERATION` | LifeSafetyOperation service procedures and errors; Life Safety COV reporting; Life Safety Point/Zone Present_Value, Status_Flags, Tracking_Value, Silenced, and Operation_Expected property requirements | P0 | `implementation-present-needs-conformance-tests` | Partial operational-state evidence only (Refs #177; remains open). Fail-closed authorization receives immediate and routed requester identity; Requesting Source remains untrusted text. Built-in Point/Zone execute authorized silence/unsilence with exact `Operation_Expected` arming. Reset variants execute only through configured application-owned synchronous executors and commit only supplied modeled values before clearing `Operation_Expected`; no physical state is inferred. The bounded process-local confirmed-request tracker resends the byte-identical recorded response for exact duplicates within 60 seconds / 256 entries before authorization/execution; requests over 64 KiB execute untracked; replay is retained by design and is not durable actuation idempotency. Detailed operation outcomes and WP/WPM/`write_local`/live-Schedule snapshots retain actual deltas. Whole-object Life Safety COV triggers only for `Present_Value`/`Status_Flags` and reports exactly both; property COV reports the supported subscribed property plus one `Status_Flags` and fans out on an actual status change. Point and Zone both support `Present_Value`, `Status_Flags`, `Tracking_Value`, `Silenced`, and `Operation_Expected`; the Zone gained `Tracking_Value` in #1092, and a Zone reset commit may set it as a Point commit can. The built-in Point/Zone `Status_Flags` `IN_ALARM` projection from `event_state` is latent: no `event_state` setter exists, so `IN_ALARM` transitions are unreachable; `Event_State` is purely intrinsic per R1 outcome (b) with no LSO-driven change. Initial/re-subscription notifications use the same payload sets after ACK; cancellation is quiet. Served property model is pinned by executable metadata: Point `POINT_BASE` 18 rows and Zone `ZONE_BASE` 16 rows drive `Property_List`, required sets, and writability with exact PICS projection tests; `Silenced` and `Operation_Expected` are network read-only. `Mode` writes are checked against `Accepted_Modes` (#1092, `BACNET-12-LIFE-SAFETY-GLOBAL-GROUP-REQUIRED-ROWS`). `Tracking_Value` and `Reliability` take writes while Out_Of_Service is TRUE, and a simulated change notifies through the same snapshots (#1108, `BACNET-12-LIFE-SAFETY-OUT-OF-SERVICE-SIMULATION`). The application sets `Present_Value` and `Tracking_Value` at runtime through `set_present_value_local` and `set_tracking_value_local`, which notify through the `write_local` snapshots (#1123, `BACNET-12-LIFE-SAFETY-APPLICATION-VALUES`). Gaps retained: no BIBB, profile, or device-advertisement claim; broad intrinsic `CHANGE_OF_LIFE_SAFETY` event algorithm deferred. |
<!-- END ledger-rows -->

## Clauses 20-21 Encoding And Formal APDUs

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-20-ENCODING
BACNET-21-FORMAL-APDUS
BACNET-21-TIMESTAMP-CHOICE
BACNET-21-CALENDAR-ENTRY-CHOICE
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-20-ENCODING` | Clause 20 | P1 | `implementation-present-needs-negative-tests` | Encoding modules and tests exist. |
| `BACNET-21-FORMAL-APDUS` | Clause 21 | P1 | `implementation-present-needs-conformance-tests` | APDU and service modules exist. |
| `BACNET-21-TIMESTAMP-CHOICE` | Clause 21 (BACnetTimeStamp), Clause 20.2.1.5 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-21-FORMAL-APDUS`. Resolves the #259 dual-codec defect per the Clause 20.2.1 tag-form rules: `time [0]` tags the primitive base type `Time`, so every producer/consumer now shares one codec — `time [0]` as a primitive context tag 0 of length 4 (raw Time octets), `sequence-number [1]` constrained to Unsigned (0..65535) by its public representation and hostile-wire validation, `datetime [2]` as an opening/closing tag 2 pair around application-tagged Date/Time. The bare CHOICE form serves `SEQUENCE OF` contexts (GetEventInformation-ACK `eventTimeStamps`); the field-wrapped form serves BACnetTimeStamp-tagged fields (audit, alarm acknowledgment, and event notification) byte-identically to before. COVNotificationMultiple instead carries independent request DateTime and per-value Time fields. Golden vectors, 0/65535 boundary acceptance, 65536 and 3-octet overflow rejection, wrong-outer-tag / wrong-class-bit / truncated negatives, and a cross-call-site primitives↔GetEventInformationAck matrix run in-tree. Evidence: `crates/bacnet-encoding/src/primitives/mod.rs` + `primitives/tests.rs`, `crates/bacnet-services/src/alarm_event/get_event_information.rs` + `alarm_event/tests/get_event_information_timestamps.rs`. |
| `BACNET-21-CALENDAR-ENTRY-CHOICE` | Clause 21 BACnetCalendarEntry, BACnetDateRange, BACnetWeekNDay, BACnetSpecialEvent, BACnetDailySchedule and BACnetTimeValue; Clauses 20.2.1.3.2 and 20.2.1.5; Clause 12.9 Date_List; Clause 12.24 Effective_Period, Weekly_Schedule and Exception_Schedule; Clauses 15.1.1.3, 15.2, 15.8 and 15.9.1.3 | P1 | `supported-with-clause-evidence` | Split child of `BACNET-21-FORMAL-APDUS` (#996). Each `BACnetCalendarEntry` travels under its CHOICE tag: date [0] (four-octet primitive), date-range [1] (frame around two application Dates) or weekNDay [2] (three-octet primitive). Calendar `Date_List` is the concatenation of its entries; Schedule `Exception_Schedule` elements are whole special events (calendar-entry [0] frame or calendar-reference [1], the [2] time-value frame, event-priority [3] decoded as any Unsigned and range-checked 1..16 by the Schedule object, #1087), `Weekly_Schedule` elements are [0] daily-schedule frames, and `Effective_Period` is two application Dates. One codec in `bacnet-encoding::constructed` (`calendar.rs`, `schedule.rs`) serves objects and services; `bacnet-services::schedule` and the raw eight-octet `BACnetDateRange::encode`/`decode` are removed. `Date_List` is network-writable: WP/WPM replace it (empty included), AddListElement/RemoveListElement edit it, and written non-entries are INVALID_DATA_TYPE, undecodable entries INVALID_DATA_ENCODING and more than 1024 entries NO_SPACE_TO_WRITE_PROPERTY (NO_SPACE_TO_ADD_LIST_ELEMENT for AddListElement), atomically. The list services use a calendar-entry element codec, so RemoveListElement matches by value and a malformed element is INVALID_DATA_TYPE on either service. One list element per entry lets ReadRange address entries by position. Breaking on the wire: entries were an application Date or Octet String, `Exception_Schedule` dropped the period and sent the priority as an application Unsigned, `Weekly_Schedule` days were Time/Octet String pairs, and `Effective_Period` was an Octet String, NULL until set (now the range with both dates unspecified). Evidence: `crates/bacnet-encoding/src/constructed/tests/{calendar,schedule}.rs`, `crates/bacnet-objects/src/schedule/{calendar_tests,tests}.rs`, `crates/bacnet-server/src/handlers/tests/calendar_date_list.rs`, `crates/bacnet-server/src/handlers/tests/read_rpm/{calendar,schedule}.rs`. #1029: a well-formed entry with an octet out of its Clause 21 range (month 1-14, day 1-34, weekday 1-7, week-of-month 1-9, date-range ends each a specific date or wholly unspecified) is VALUE_OUT_OF_RANGE on WP and WPM, and on AddListElement a ChangeList-Error naming that entry's position, atomically; RemoveListElement has no such error, and since no stored entry can be out of range it fails LIST_ELEMENT_NOT_FOUND at that position. #1028: time-values are typed `PropertyValue`s; the encoders refuse a constructed value. Present_Value evaluation is `BACNET-12-CALENDAR-PROPERTY-SET` and `BACNET-12-SCHEDULE-EVALUATION`. Network writes of the Schedule properties decode with these codecs (`BACNET-12-SCHEDULE-WRITES`, #1057). The shared list-service element rules are `BACNET-15-LIST-ELEMENT-SEMANTICS` (#1027). |
<!-- END ledger-rows -->

## Annex A PICS

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-A-PICS
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-A-PICS` | Annex A | P1 | `in-progress` | `bacnet-server::pics` exists; generated draft summary is not a certification claim. |
<!-- END ledger-rows -->

## Annex J BACnet/IP

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-J-BVLC-FUNCTION-CODES
BACNET-J-ORIGINAL-UNICAST-NPDU
BACNET-J-ORIGINAL-BROADCAST-NPDU
BACNET-J-FORWARDED-NPDU
BACNET-J-BBMD-BDT
BACNET-J-FOREIGN-DEVICE-FDT
BACNET-J-NAT-TRAVERSAL
BACNET-J-IP-MULTICAST
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-J-BVLC-FUNCTION-CODES` | Annex J.2 | P0 | `implementation-present-needs-conformance-tests` | J-01 covers Annex J.2 constants through `0x0B`, representative frames, malformed type/length, unknown function passthrough, deleted-value passthrough, and current decoder policy for extra bytes beyond BVLC Length. J-02 adds exact two-byte BVLC-Result parsing, unknown result-code passthrough, malformed result rejection, and sender/expected-function correlation for pending management responses. J-13 adds reproducible local and base/head benchmark runners for BIP Criterion suites with ignored raw artifacts and a machine-readable result-row schema. |
| `BACNET-J-ORIGINAL-UNICAST-NPDU` | Annex J | P0 | `implementation-present-needs-negative-tests` | J-04 Original-NPDU evidence covers UDP sender B/IP MAC delivery and self-originated frame suppression; directed reply-path integration remains open. |
| `BACNET-J-ORIGINAL-BROADCAST-NPDU` | Annex J | P0 | `implementation-present-needs-negative-tests` | J-04 Original-NPDU evidence covers BBMD Original-Broadcast local delivery, BDT/FDT Forwarded-NPDU fanout with original sender preservation, and no local Forwarded-NPDU echo. J-09 adds platform socket evidence for INADDR_ANY broadcast reception and SO_BROADCAST Original-Broadcast-NPDU sends. J-10 adds B/IP send-path evidence that Clause 6 global-broadcast and remote-broadcast NPDUs preserve DNET, DLEN 0 broadcast DADR, Hop Count, and APDU bytes unchanged inside Annex J Original-Broadcast-NPDU. Additional malformed/unsupported-path coverage remains open. |
| `BACNET-J-FORWARDED-NPDU` | Annex J | P0 | `implementation-present-needs-negative-tests` | J-04 covers originating-address parsing, truncated-address rejection, BBMD delivery with originating B/IP source MAC, FDT fanout preserving the origin, local rebroadcast for unicast peer arrivals, directed-broadcast peer local-rebroadcast suppression, no onward forwarding to other BDT peers, and Original-Broadcast BDT/FDT Forwarded-NPDU emission. Non-BDT Forwarded-NPDU evidence verifies rejection before local delivery, local rebroadcast, BDT fanout, or FDT fanout. DBTN evidence covers registered foreign-device fanout to local broadcast, BDT peer, and non-origin FDT peer while preserving the origin, excluding origin echo, checking for no extra duplicate frames, and returning `X'0060'` when deterministic local Forwarded-NPDU forwarding fails. |
| `BACNET-J-BBMD-BDT` | Annex J.4/J.5 | P0 | `implementation-present-needs-conformance-tests` | J-03 covers read/write BDT caller paths, replacement semantics, malformed Write-BDT NAK without table mutation, self-entry insertion without overflow, and directed-broadcast forwarding target calculation. J-04 adds Forwarded-NPDU BDT mask behavior, no onward forwarding to other BDT peers, non-BDT sender rejection before BDT fanout, and Original-Broadcast one-peer fanout without local echo. DBTN fanout covers a registered foreign-device request forwarded to a BDT peer plus local broadcast and FDT targets without extra duplicate frames. J-07 adds project ACL evidence for the legacy Write-BDT path: listed management senders can update the table, and unlisted senders receive the standard Write-BDT NAK without table mutation. J-08 adds BDT persistence evidence: successful Write-BDT stores the current wire-format BDT for restart load, and invalid persisted bytes fall back to the configured BDT without accepting malformed state. J-11 adds Read-BDT-Ack payload validation evidence for Annex J.2.4 `N*10` BDT entry sizing. J-12 adds raw BBMD Read-BDT-Ack wire evidence for ACK function code and `N*10` BDT entry bytes. J-13 adds reproducible BBMD stress benchmark A/B runner evidence with ignored raw artifacts and a machine-readable result-row schema. |
| `BACNET-J-FOREIGN-DEVICE-FDT` | Annex J.5 | P0 | `implementation-present-needs-conformance-tests` | J-03 covers FDT read/register/delete caller paths, re-registration, zero-TTL and malformed TTL NAKs, exact Delete-FDT payload length, max TTL remaining-time capping, expiry purge, source exclusion, and unregistered DBTN NAK without local delivery. J-04 Original-Broadcast evidence covers one registered FDT target; DBTN evidence covers registered origin plus peer FDT fanout, source preservation, no echo to the originating foreign device, no extra duplicate FDT frames, and `X'0060'` when forwarding cannot be completed. Non-BDT Forwarded-NPDU evidence verifies no FDT fanout. J-06 adds a BBMD-owned timer purge task for Annex J.5.2.3, covers expiration without an inbound BVLC request, and covers re-registration resetting the entry before the purge task removes it. J-07 adds project ACL evidence for Delete-FDT: listed management senders can delete registered entries, and unlisted senders receive the standard Delete-FDT NAK without removing the entry. J-11 adds Read-FDT-Ack payload validation evidence for Annex J.2.8 `N*10` FDT entry sizing. J-12 adds raw BBMD Read-FDT-Ack wire evidence for ACK function code and `N*10` FDT entry fields. J-13 adds reproducible foreign-device/BBMD stress benchmark A/B runner evidence with ignored raw artifacts and a machine-readable result-row schema. |
| `BACNET-J-NAT-TRAVERSAL` | Annex J.7.5 | P0 | `deferred-pending-owner-decision` | Current IPv4 B/IP surfaces expose interface, port, broadcast address, BDT/FDT management, and foreign-device registration, but no reviewed global B/IP address field, NAT mode flag, BBMD/router logical-port model, or NAT-specific originating-address rewrite. Follow-up work item `019f0ff8-14c0-7013-9721-3bc5fe0356de` tracks the owner decision and implementation plan if support is later claimed. |
| `BACNET-J-IP-MULTICAST` | Annex J.8 | P0 | `deferred-pending-owner-decision` | Current IPv4 B/IP transport sends local broadcasts to a configured IPv4 broadcast address and has no B/IP-M multicast group membership, multicast send, or B/IP-M BBMD group configuration. BACnet/IPv6 multicast evidence belongs to Annex U, not Annex J. Follow-up work item `019f0ff8-14e7-7681-8738-032683da62df` tracks the owner decision and implementation plan if support is later claimed. |
<!-- END ledger-rows -->

## Annex K BIBBs

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-K-BIBBS
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-K-BIBBS` | Annex K | P1 | `in-progress` | Generated draft is a starting point only; detailed service mapping remains open. |
<!-- END ledger-rows -->

## Annex L Profiles

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-L-PROFILES
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-L-PROFILES` | Annex L | P2 | `in-progress` | Profile evidence must be derived from ledger/PICS rows later. |
<!-- END ledger-rows -->

## Annex U BACnet/IPv6

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-U-IPV6-BVLL
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-U-IPV6-BVLL` | Annex U | P2 | `implementation-present-needs-conformance-tests` | B/IP6 codec and benchmark paths exist. Current selected-link startup and source/destination/interface ownership have isolated Linux ULA wire tests (auto/explicit, three group scopes, two-link rejection, collision/lifecycle and foreign BBMD source/trust) plus fresh installed Python Who-Is/I-Am/client discovery. macOS lo0 qualifies multicast intake and unicast/control source only; Windows is compile-checked, not runtime-qualified. Unique link-local/zone selection has unit evidence only. External fixtures are explicitly opt-in; full Annex U conformance remains unqualified. See `crates/bacnet-transport/tests/ipv6_selected_link/README.md`. |
<!-- END ledger-rows -->

## Annex AB BACnet/SC

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-AB-SC-FRAME
BACNET-AB-SC-BVLC-RESULT
BACNET-AB-SC-DATA-ATTRIBUTES
BACNET-AB-SC-CONNECTION-STATE
BACNET-AB-SC-HUB-CONNECTOR
BACNET-AB-SC-WEBSOCKET-TLS
BACNET-AB-SC-HEARTBEAT
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-AB-SC-FRAME` | Annex AB.2 | P0 | `implementation-present-needs-negative-tests` | AB-01 adds transport codec evidence for reserved control bit rejection, Header Option Type `1..31` enforcement, AB.2.17 destination/data option marker decoding, VMAC field ordering, option count cap enforcement, option length/data truncation rejection, and unterminated option-chain rejection. Received Encapsulated-NPDUs enforce Destination Option Must Understand semantics: an unsupported Must Understand option returns BVLC-Result NAK COMMUNICATION/HEADER_NOT_UNDERSTOOD for unicast, echoes the complete offending marker, routes the Result to the source through the hub, and drops broadcast without a result. The hub changes the VMAC fields while copying validated option and payload bytes, so an empty Header Data field is not normalized away before destination processing. Must Understand clear does not block delivery. Addenda 135-2020cf/cp were checked: cf renames Data Options bit 6 to Every Segment without changing the marker bit position, and cp adds standard header option types 2..5 accepted by the generic `1..31` parser. Must Understand processing for other BVLC-SC functions, non-Result function-specific payload semantics, Every Segment segmentation behavior, and direct-connection behavior remain open. |
| `BACNET-AB-SC-BVLC-RESULT` | Annex AB.2.4 | P0 | `implementation-present-needs-conformance-tests` | AB-02 adds typed BVLC-Result ACK/NAK payload parsing for transport. ACK fixtures use the Proprietary-Message function per addendum 135-2020ci; NAK fixtures mirror AB.2.17 examples with and without UTF-8 Error Details, including the Figure AB-6 multibyte UTF-8 details bytes. The hub relays a source-addressed, syntactically valid Encapsulated-NPDU Result to the originating node and drops peer Results for other functions. Connection handling keeps ACK and Encapsulated-NPDU NAK messages nonfatal, retains existing disconnect handling for other NAKs, and disconnects on malformed Result without generating a Result response. Received unicast Encapsulated-NPDUs with an unsupported Must Understand Destination Option return COMMUNICATION/HEADER_NOT_UNDERSTOOD with the complete offending option marker. Negotiated Max-BVLC-Length resource-limit evidence is tracked under `BACNET-AB-SC-CONNECTION-STATE`. Correlation for permitted Encapsulated-NPDU Results, Error Details resource limits, standard-function ACK diagnostics, full AB.3.1.5 Result generation, and direct-connection response routing remain open. |
| `BACNET-AB-SC-DATA-ATTRIBUTES` | Annex AB.3.4 | P0 | `implementation-present-needs-conformance-tests` | Native receive-side, outbound send, router local-delivery, and router forwarding evidence covers BACnet/SC Encapsulated-NPDU Data Options. Receive paths convert every Data Option, including unknown options with bit 6 set, into generic transport `DataAttribute` values, preserving option type, the stored bit-6 flag, and option data on `ReceivedNpdu`; `NetworkLayer` exposes those attributes on `ReceivedApdu`; router local-delivery and forwarding retain them; and non-SC transports emit empty `data_attributes` on receive. Native SC no longer applies Destination Option Must Understand rejection rules to Data Options. Destination Option rejection and BVLC-Result evidence is tracked under the frame and result rows. Outbound helpers encode SC `DataAttribute` values as Data Options while validating option type `1..31`, the 64-option cap, and u16 option-data lengths. Native negative tests enforce the Annex AB.2.3.1 Secure Path type-1 Must Understand and no-Header-Data marker constraints before message-ID allocation or BVLC-SC frame emission. Remaining work includes Addendum 135-2020cf Every Segment semantics and a context-specific name for the stored Data Option bit-6 flag, high-level helper address resolution/target VMAC behavior, and formal conformance tests. |
| `BACNET-AB-SC-CONNECTION-STATE` | Annex AB.6.2 | P0 | `implementation-present-needs-state-machine-audit` | AB-03 adds native Connect-Accept message-id matching evidence, strict 26-byte Connect-Accept payload handling, negotiated Max-BVLC/Max-NPDU limits, and native async handshake failure/disconnect evidence. AB-07 adds Device UUID replacement, superseded WebSocket close, duplicate-VMAC NAK signaling for new-UUID collisions, and live generated-certificate evidence around replacement and invalid connected-state transitions. This tranche adds Annex H.7.3 Random-48 VMAC generation, initiating-peer Connect-Request BVLC-Result NAK COMMUNICATION/NODE_DUPLICATE_VMAC (7/151) detection, native failover retry evidence using the reselected VMAC, generic Connect-Request NAK no-reseed evidence. Remaining gaps include adapter-boundary AWAITING_WEBSOCKET evidence, accepting-peer connect-wait timeout, Disconnect wait in `ScTransport::stop`, and reserved VMAC error signaling. |
| `BACNET-AB-SC-HUB-CONNECTOR` | Annex AB.5 | P0 | `supported-with-clause-evidence` | AB-06 adds hub connector/forwarding evidence: hub-bound Encapsulated-NPDU uses Destination VMAC present and Originating VMAC absent; local receive drops hub-relayed non-broadcast Destination VMAC; hub rejects Originating-VMAC-present or missing-Destination forwarding attempts; unicast selects only the matching VMAC and unknown unicast has no recipient; broadcast targets all current hub connections except origin; relay preserves Message ID/payload/destination options/data options, adds sender Originating VMAC, strips Destination VMAC for unicast, and preserves broadcast Destination VMAC. Live three-client WebSocket evidence covers A-to-B unicast, A-to-unknown discard, and A broadcast to B/C but not A. AB-07 adds live WebSocket evidence that known Device UUID replacement moves hub reachability to the replacement VMAC, prevents old-VMAC unicast delivery, requires a Close frame for the superseded connection, prevents marked-superseded source or recipient sinks from relaying after replacement wins, and preserves peer reachability after rejecting a connected client's second Connect-Request. Transport connector evidence covers primary timeout/failover, established primary loss with reconnect exhaustion, active send-path swap to failover, and primary restoration while failover is active. AB.5.1 live generated-certificate WebSocket evidence in `sc_hub::ws_capacity_tests` covers a 1497-octet NPDU plus 4192 octets of encoded Destination Options and Data Options through both unicast (5699-octet relayed BVLC) and broadcast (5705-octet relayed BVLC), preserving options and payload. The hub advertises 5705-octet Max-BVLC and 1497-octet Max-NPDU capacities and drops a 1498-octet NPDU before relay or activity refresh. Recipient-limit decision tests cover independent NPDU and final encoded BVLC bounds; unicast and broadcast relay apply each recipient's negotiated limits, so the node's historical 1476-octet NPDU default (1478 after #893) is not a hub capacity limit. Existing 31 minimum-size Destination Options plus 31 minimum-size Data Options remain covered by relay-helper and live WebSocket unicast tests, with helper broadcast preservation. Direct-connection unsupported-classification evidence covers rejecting `dc.bsc.bacnet.org` at the hub WebSocket subprotocol boundary and returning COMMUNICATION/UNEXPECTED_DATA, not BVLC_FUNCTION_UNKNOWN, for a connected client's Address-Resolution. Configured hub URI evidence covers preserving the configured `wss` authority, port, path, and query at the production `TlsWebSocket::connect` parse boundary; malformed or hostless configured hub URIs are rejected before any WebSocket/TLS dial. Addendum cc affects AB.5.3.1 metadata, not these forwarding, replacement, reconnect-failover, option-preservation, configured/malformed hub URI, or direct-connection unsupported-classification behaviors. No remaining hub connector gaps are tracked in this row. |
| `BACNET-AB-SC-WEBSOCKET-TLS` | Annex AB.7 | P0 | `implementation-present-needs-security-tests` | AB-04 retains pre-dial `wss`-only checks, `hub.bsc.bacnet.org` negotiation and negative offers, binary BVLC-SC sends, text close `1003` and optional Ping/Pong. AB-05 retains generated-certificate mTLS/TLS 1.3 negotiation and missing/wrong-issuer/expired/not-yet-valid peer, wrong SAN, malformed PEM, mismatched-key and TLS 1.2 denials. PRs #585–#592 (runtime assessed at `982cc9f`) address [#513](https://github.com/jscott3201/rusty-bacnet/issues/513)'s original entry-point configuration acceptance: mandatory `ScHubTlsConfig`/`ScNodeTlsConfig`, explicit nonempty CA and matching operational credentials, Python/CLI/standalone preflight before bind/dial, and strict hub peer verification. File-helper rejection/retry, real ReadProperty, installed native Python and isolated Docker pair evidence are delivered; the credential closeout did not rerun them. Insecure hub/example modes and `sc_latency`/`sc_throughput` were retired, not renamed or retained; historical benchmark numbers are not new mTLS measurements. The broad row remains `implementation-present-needs-security-tests`: TLS application-profile/cipher-suite evidence, revocation when configured and separate PKI/identity-profile work remain. No new addendum 135-2020cd validation, full Annex AB/PICS/BTL promotion, or direct-connection support beyond dial-out is claimed. Refs #615 PR3a adds the dial-out-only direct primitive `TlsWebSocket::connect_direct` (`wss` plus `dc.bsc.bacnet.org` with the same `ScNodeTlsConfig` operational-certificate policy and pre-dial URI validation); accept-side, discovery trigger, and routing over direct connections remain excluded and the row status is unchanged. Node credentials are offered when requested/compatible: a trusted server with no CertificateRequest can complete; normal resumption may omit certificate retransmission. Local configuration is not per-connection presentation proof or remote-verifier attestation; custom `WebSocketPort` implementations are outside the built-in guarantee. CA membership is not BACnet operation authorization or certificate-to-VMAC/UUID binding. The [#517](https://github.com/jscott3201/rusty-bacnet/issues/517) node-first slice requires caller-provisioned nonzero 16-byte UUIDs in `ScServerBuilder` and Python client/server before I/O. Generated-certificate tests cover distinct native Python nodes with ReadProperty, intended same-UUID replacement with the unrelated node still usable, owned wire bytes over Python stop/start/recreation, and Rust server wire bytes over fresh builds and reconnect. The hub-local slice extends explicit nonzero UUID configuration to all four Rust ScHub startup APIs, Python ScHub and the standalone/Docker hub. ScHub::start appends the required UUID argument; one shared Rust check rejects zero UUID or reserved UNKNOWN/BROADCAST local VMAC before bind. Python preserves five positional slots and CA-first diagnostics, copies keyword-only device_uuid, and validates identity before credential-file I/O; the standalone hub requires --device-uuid and retains its default VMAC. Independent Connect-Accept vectors check the hosting port VMAC and hosting device UUID (base 2020 AB.2.11 and AB.6) through all four Rust routes, Python owned-buffer mutation/stop-start/recreation, and actual binary restart/read tests. Compose uses an explicit stable TEST-ONLY hub UUID, not a deployment fallback in the binary. Base 2020 AB.1.5.3 lifetime provisioning/storage remains the caller's responsibility; changed UUIDs cannot be detected without application history. Raw ScTransport::new(ws, vmac) retains its unstarted zero placeholder, but start requires with_device_uuid and rejects all-zero UUID/all-zero or broadcast local VMAC after reconnect and heartbeat validation, before transport-owned I/O or startup state changes. Repeated rejection retains sockets; UUID repair via the existing setter can retry on the same owned WebSocket. This cannot undo caller-owned dials or promise generic endpoint rollback/all-field repair. Independent AB.2.10 byte oracles cover initial start, reconnect, initial/established failover, primary restore and legitimate duplicate-VMAC reselection without UUID changes. This is startup enforcement, not lifetime immutability against later application mutation through public connection(); pure ScConnection/manual WebSocket use and later handshake validation are excluded. Wire admission, peer replacement and UUID version/variant policy are unchanged; no general VMAC bit-shape policy is added. #517 remains open for residual identity work; no lifetime guarantee or row-status promotion. Refs #956: `ScTransport::connection()` is no longer public (test-only), so applications can no longer change the live connection identity through the transport; they read the link state through `connection_state_changes()`. The startup checks above are unchanged and this is still not a lifetime guarantee: internal duplicate-VMAC reselection and pure ScConnection/manual WebSocket use stay outside them. Row status is unchanged. |
| `BACNET-AB-SC-HEARTBEAT` | Annex AB.6.3 | P0 | `implementation-present-needs-timeout-tests` | AB.6.3 heartbeat evidence covers no-VMAC Heartbeat-Request/Heartbeat-ACK construction, native Heartbeat-ACK message-id/no-VMAC validation, production `ScTransport::start` rejection of configurable heartbeat intervals outside 3..300 seconds, production rejection of disconnect timeouts that are not greater than the heartbeat interval, periodic heartbeat initiation after idle inbound BVLC activity, inbound BVLC activity resetting the liveness timer, native timeout/send-error disconnect behavior, and hub-initiated heartbeat tracking that sends idle Heartbeat-Requests, records the pending Message ID, clears pending state only for a matching Heartbeat-ACK, and removes the hub client when the pending ACK exceeds the hub timeout. Accelerated native heartbeat tests use a private test-only timing override so production builders keep Annex AB.6.3 range enforcement at start. Remaining gap: formal Annex AB.6.3 timeout conformance tests; the native heartbeat timeout evidence relies on a test-only timing override. |
<!-- END ledger-rows -->

## Explicit Deferred Or Unsupported Annexes

<!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate
BACNET-O-ZIGBEE
-->
| Row ID | Anchor | Priority | Status | Evidence |
|---|---|---|---|---|
| `BACNET-O-ZIGBEE` | Annex O | P3 | `unknown-pending-source-review` | No public support claim found in the initial scan. |
<!-- END ledger-rows -->

## Follow-Up Backlog

Rows not marked `supported-with-clause-evidence` are follow-up work. The next Annex J tranche should continue BBMD/BDT/FDT lifecycle evidence, including remaining forwarding-loop prevention negative cases and management/table edge cases. NAT traversal and B/IP-M multicast are now explicitly tracked as deferred owner-decision rows.

### COV successful preparation ordering (#826)

The existing `BACNET-13-COV-SUBSCRIPTIONS` row now records a local concurrency
policy: complete eligible unconfirmed observations reserve a checked table-owned
ticket before later waits, and successful sends commit the entire baseline only
when newer than that live reference's last successful ticket. Failed or cancelled
newer work cannot prevent older success. Confirmed admission timing, lifecycle
fences and per-reference qualification remain unchanged; #896 later moved
confirmed completion to the subscriber's Ack, under a ticket from the same
counter. This is not a claimed Standard tie-break or an original-write,
event-time, byte-order or remote-receipt guarantee. A supplied Binary Lighting terminal snapshot prepared later can win
even when its object state is older. Regression anchors in the machine ledger
cover held completions, capture-before-await, per-reference overlap, failure,
exhaustion and the supplied-snapshot limit; row status is unchanged.

## Hub certificate bindings

The `BACNET-AB-HUB-CERTIFICATE-BINDINGS` row records #800's opt-in installation
policy under Annex AB.7.4. A verified exact leaf DER SHA-256 fingerprint must
match one configured UUID/allowed-VMAC group before locked registration commit.
Offline reservations, same-CA unauthorized leaves, listed rotation, callback
conjunction, incumbent relay, missing identity, prebind conflicts and joined
shutdown have native fixtures; installed Python exercises the same validator and
real TLS paths through frozen named groups. No-map CA-valid admission remains
intentional. See [Rust](../rust-api.md#hub-certificate-bindings) and
[Python](../python-api.md#hub-certificate-bindings) for the exact contract.
This neither authenticates relayed operations end to end nor closes the full SC
security profile; #518/#524 remain separate. Accepted-direct identity is
covered by the narrower evidence below.

## Direct peer membership

`BACNET-AB-SC-DIRECT-MEMBERSHIP` records the bounded #851 outcome against
135-2020 AB.4.2/AB.4.2.1, AB.6.2.1/AB.6.2.3 and AB.2.4.1. Accepted and outbound
direct peers share UUID/VMAC ownership; same-UUID replacement publishes after
successful Accept and fences stale receive/cleanup generations. Real TLS tests
cover duplicate VMAC, changed-VMAC replacement, cross-role conflicts/races,
M=1 capacity and pending/physical saturation, expiry/eviction and discovery
teardown. Deterministic write seams test failed/cancelled/timed-out Accept
against a real TLS incumbent; queued complete work survives replacement.

The accepted M/pending M/physical 2M bounds, compound collision precedence and
capacity RESOURCES/OTHER signal are local policy. Public migration is described
in [the Rust API](../rust-api.md#direct-peer-membership-and-limits). Different
CA-valid certificates may claim the same UUID under this membership policy.
This row does not promote the broader connection-state audit or claim the
separate request-principal isolation row below, #524 response confinement, BTL certification,
external interoperability or hardware qualification. Idle outbound workers observe
remote EOF/Close and handle Disconnect control with generation-specific cleanup.
Ordinary bidirectional application NPDU routing is qualified by the narrower
[direct traffic row](#bidirectional-direct-traffic) below.

The direct-membership race evidence distinguishes a locally unique winner while
peer sockets remain open from deterministic crossed replacement at two endpoints.
Crossed replacement can close both sockets; the tests require generation-specific
cleanup, recovered physical capacity, and successful fresh demand after normal
URI backoff. Local write success alone does not establish remote NPDU delivery.

## Accepted direct request identity

`BACNET-AB-SC-DIRECT-PRINCIPAL` records #803's accepted-direct identity boundary.
Annex AB.7.4 supplies TLS connection authentication; exact leaf-DER SHA-256,
process-lifetime connection incarnations, request ownership and authorizer policy
are the selected local contract. This does not make certificate-to-UUID/VMAC
mapping mandatory or enable such installation policy for direct connections.

The listener captures its verified leaf before WebSocket upgrade, fails closed
without that chain, and combines it with #851's committed generation under the
existing NPDU admission fence. The sealed snapshot reaches the network queue,
mutation/WPM and LSO authorizers, generic duplicate admission, LSO replay, and
receive reassembly/cancellation. Already admitted complete A work may finish under
A's original snapshot after replacement; newly arriving retired-socket frames
remain fenced. Direct identities partition partial contexts and duplicate/replay
entries without expanding existing global or claimed-peer capacity bounds.
Hub admission has its own scope-only channel variant; Hub-relayed and unverified
ingress do not gain a downstream direct principal.

Real TLS tests use different same-CA leaves and same-leaf reconnects claiming
identical UUID/VMAC and routed addresses. Listener-produced envelopes pass through
the actual network/server loop with explicit queue/database barriers. They prove
fresh authorization for byte-identical WP/LSO requests across direct incarnations,
pending same-socket suppression, WPM order and old snapshot retention,
independent segment completion/cancellation, and unchanged reassembly capacity.
Queued-A segment controls were admitted before retirement. Separate tests cover
exact DER hashing, absent-chain refusal, TLS resumption, redaction, and sealed
construction. Independent negative controls omit the duplicate identity partition
or restore address-wide Abort sweeping and fail the corresponding regressions.

The original #803 response observer was a transport send boundary. #888 updates
these fixtures to decode live replies on their actual TLS socket while retaining
identity/state assertions; stale A completion has no address fallback. The
separate server-response row below supplies the bounded confinement evidence.
This identity row does not qualify #886 bidirectional/outbound application NPDU intake, Hub-relayed
end-to-end identity, full Annex AB conformance, BTL certification, or external
interoperability. #876 removes generic completed retention: completed same-socket
WP reauthorizes, while completed LSO still replays without reauthorization.
Global evidence pins and broad security-row status are unchanged. See the
[Rust API and public breaks](../rust-api.md#accepted-direct-tls-identity).


## Ordinary confirmed transaction lifetimes

`BACNET-5-TSM-SERVER` adds #876 evidence for Clauses 5.3.5.1–3 and
5.4.5.3–4 without promoting the broad server TSM row. Ordinary exact duplicate
detection has bounded pending-only ownership: 256 tracked entries and a 64 KiB
service-request limit, with normal admission when detection is unavailable.
Local and routed canonical keys and #803's accepted-direct leaf/incarnation
partition remain intact. LSO completed replay and Audit receipt policies remain
separate.

Full-server tests count actual object execution and hold transport futures after
observing encoded NPDU issuance. Pending WP duplicates remain suppressed; after
local issuance, byte-identical WP and a fresh Invoke ID execute while the old
send and request-task permit remain held. Direct and routed controls cover
SimpleACK, ComplexACK, Error, Reject and oversize Abort. MS/TP tests distinguish
successful encoded handoff from a closed receiver; encoding failure, transport
failure, cancellation and task-capacity rejection release ownership without
inventing successful delivery. Network tests distinguish lazy future creation,
encoding failure and first-poll issuance.

A full-server segmented RP test lets the parent finish, retains duplicate
ownership through the final ACK wait, then proves immediate reuse after the
final ACK. Isolated tests exercise send failure, remote Abort, cancellation,
virtual-clock timeout, rejected child spawn and generated terminal Abort with a
held transport result. Reused ownership survives late cleanup of the old child.
Real TLS #803 tests retain A/B/C leaf/incarnation assertions and WPM/reassembly
isolation; completed same-socket WP now reauthorizes while completed LSO replays.

Evidence: `confirmed_issuance_tests.rs`, `confirmed_response_lifetime_tests.rs`,
`confirmed_segmented_lifetime_tests.rs`, `confirmed_tracker_tests.rs` under
`crates/bacnet-server/src/server/`, and `crates/bacnet-network/src/issuance_tests.rs`.
These observations establish local issuance and ownership, not physical send,
peer receipt, reply-socket affinity, segmented-response control confinement,
full TSM conformance, external interoperability or BTL certification.


## Accepted direct server responses

`BACNET-AB-SC-SERVER-RESPONSE` records #888, a bounded child of #524.
Annex AB.4.2/AB.6 supplies direct-connection and current-membership context;
Clauses 5.2.1.1–3 supply response sizing and Clause 5 supplies confirmed
response/segmentation state. Confinement to an
original accepted socket is selected local policy, not a normative promise of
historical-socket delivery or an exhaustive Annex AB claim.

The sealed response capability travels separately from Copy/Eq/Hash provenance
through listener, network/router local delivery, queued server dispatch and
segment-zero reassembly. Every native server confirmed terminal response, overload Abort,
LSO replay, segmented response/retry/terminal Abort, and receive SegmentACK/Abort
uses that original writer. Direct missing/mismatched/stale authority fails
closed without mutable-address, replacement, Hub or dial fallback. Complete
admitted execution retains its old authorization snapshot. Segmented-send
controls include direct principal/incarnation while non-direct canonical keys,
MS/TP handoff and #876 pending-only ownership remain unchanged.

Real TLS evidence in `direct_response_tests.rs`, `direct_response_segment_tests.rs`,
`direct_response_owner_tests.rs` and `direct_principal_segment_tests.rs` covers
live terminal responses, held A/replacement B with distinct same-CA leaves,
same-leaf reconnect, exact LSO replay/fresh reauthorization, missing/mismatched
capabilities, overload, retry/terminal Abort, peer ACK/Abort isolation, receive
NAK/ACK and saved segment-zero route. Baseline red observed a data-bearing reply
at generic egress, not proven disclosure to another peer. Green tests decode A
or B TLS data and use a generic-egress spy plus deterministic completion barriers.

`direct_response_budget_tests.rs` adds Clause 5.2.1.2 (printed 30–31/PDF 32–33)
evidence: response selection includes the original peer's independent NPDU/BVLC
limits, the encoded local/routed header, requester APDU acceptance and local cap.
A 479-byte ComplexACK over a 480-byte NPDU/484-byte BVLC path becomes two fitting
segments; an exactly fitting 478-byte ACK stays unsegmented. Independent BVLC
and six-byte routed-DADR limits, exact payload reassembly, final ACK cleanup and
identical InvokeID reuse are checked over real TLS. No-segmentation, segment-count,
segment-header and Abort-too-small controls preserve bounded failure with no
fallback. Tiny positive receive limits are admitted-input robustness cases, not
certification of every such endpoint advertisement. Sizing snapshots survive
retirement but confer no send authority; admitted authorization remains intact.

`direct_response_tests.rs` in transport checks queue capacity 64, weak membership,
negotiated NPDU/complete BVLC limits, cancellation and bounded waiting.
`direct_response_worker_tests.rs` exercises production writer scheduling,
blocked-write timeout and queued owner-seal rejection independently of future
destruction. Its deterministic ready-control stream enqueues a response during
the first Ping/Pong poll: the adapter returns after one frame, then selects the
queued write. The pre-repair control consumed all eight controls and a binary
frame in one turn; this proves a bound violation, not an indefinite network
stall. Paused-clock controls preserve Connect and binary-activity idle deadlines.
The real TLS lifecycle fixture verifies Ping/Pong handling before Connect and
before an NPDU response. Already-started writes may complete and cannot be recalled.
`direct_response_lifecycle_tests.rs` uses real TLS to prove registered transport
stop/abort/drop seals a retained listener handle and joins zero physical sockets.
Network/server scopes seal irreversibly before shutdown waits; the retained
listener remains a cleanup handle, not authority to keep accepting or replying.

Unconfirmed Who-Is/Who-Has discovery replies retain ordinary routing outside
this row. This row excludes inbound client/shared-endpoint consumers (#889),
outgoing client transaction ownership, ordinary bidirectional direct application traffic
(#886), Hub-relayed end-to-end identity, Python direct-entry support, full Annex
AB/PICS/BTL and external interoperability. Global evidence pins and broad row
statuses remain unchanged. [Public API and lifecycle break](../rust-api.md#accepted-direct-server-responses).


## Accepted-direct client and endpoint replies

Scoped row `BACNET-AB-SC-INBOUND-CONSUMER-RESPONSE` (GitLab #889, under #524)
extends the separate server response row to two inbound consumers only. The
selected original-socket policy is local: BACnet allows response path switching;
Annex AB.4.2/AB.6 do not mandate this historical-socket policy. Clause 5.2.1.2
(printed 30–31 / PDF32–33) supplies the minimum path/receiver APDU sizing rule;
AB.2.10.1/AB.2.11.1 carry the separate receive capacities. No broad row status
or global source pin changes.

The standalone client's confirmed COV/Event Ack/Reject, unsupported-service
Reject and segmented-request Abort now preserve the admitting `ResponseRoute`.
The shared endpoint's existing ReadProperty/authorized Device WriteProperty
responder carries the same authority through its existing bounded egress queue.
Direct provenance **or any capability** is classified before `reply_tx`; missing,
mismatched, retired and sealed routes never fall back through a one-shot, claimed
address, replacement, Hub or new dial. Ordinary non-direct/no-capability client
failed prompt handoff still falls back; endpoint prompt handoff still completes
even when its receiver closed. Group requests and COV `NoResponse` stay silent.
Direct Data Options remain empty; ordinary endpoint data attributes are preserved.

[Real TLS controls](../../crates/bacnet-endpoint/tests/direct_replies.rs) exercise
the public `BACnetClient` and live `EndpointSession` through a queue of genuine
listener-admitted envelopes and a generic-egress spy. Baseline failures observed
actual generic response selection, not disclosure to another peer. The
[authority matrix](../../crates/bacnet-endpoint/tests/direct_replies/authority.rs)
first proves A's socket works, holds complete A work, commits distinct-leaf B
with the same UUID/VMAC, then releases A and uses B's fresh reply as an ordered
completion barrier. Invalid capability/prompt-channel combinations are silent;
valid direct capability wins over an otherwise usable prompt channel.
[Notification controls](../../crates/bacnet-endpoint/tests/direct_replies/notifications.rs)
cover COV policy Ack/Reject/NoResponse, event Ack, malformed notification Reject,
segmented Abort and admitted A notification delivery after replacement.
[Device readback](../../crates/bacnet-endpoint/tests/direct_replies/execution.rs)
proves retired/missing-route A writes still authorize and commit under A's saved
identity, denied B writes cannot commit, and current B responses remain usable.

[Budget controls](../../crates/bacnet-endpoint/tests/direct_replies/budget.rs)
compare exact 479-byte ComplexACKs with a 478-byte local APDU budget, independent
NPDU/BVLC restrictions and six-byte routed destination overhead. The endpoint
uses its existing `SEGMENTATION_NOT_SUPPORTED` Abort; it gains no segmented send.
Ordinary ingress still accepts the same response under request-only APDU480.
Tiny admitted limits that cannot fit an Abort/reply fail bounded without fallback;
these are robustness cases, not certification of every advertised endpoint limit.
Sizing follows service execution and never revokes committed work.

[Queue and lifecycle controls](../../crates/bacnet-endpoint/tests/direct_replies/lifecycle.rs)
hold the sole endpoint egress owner, prove queue-full/closed behavior, cancel a
queued response while its original A connection remains current, then require a
fresh ordered reply. Live session stop/drop cancels queued original replies and
retained role handles. Saved capability clones do not retain listener membership.
The feature-independent [owned-command control](../../crates/bacnet-endpoint-core/src/endpoint_egress_deadline_tests.rs)
preserves cancellation without a deadline. Existing MS/TP prompt/deferred,
ordinary data-attribute and requester tests remain controls. Already-started
writes cannot be recalled; local completion is not peer receipt.

The real TLS integration suite requires explicit `bacnet-endpoint/sc-tls`.
The standard hosted workspace command enabling only `bacnet-transport/sc-tls`
does not select those endpoint-feature-gated cases; focused local evidence is
recorded separately. Outgoing client transaction/retry policy and controls
(#890), native notification traffic (#891), ordinary bidirectional direct traffic
(#886), Hub-relayed end-to-end identity, Python direct entry, full Annex AB/PICS/BTL
and external interoperability remain excluded. These source changes postdate
published 0.11.0. See the [public scope](../rust-api.md#accepted-direct-client-and-endpoint-replies).


## Bidirectional direct traffic

`BACNET-AB-SC-BIDIRECTIONAL-DIRECT` records #886 against Annex AB.4.2.1–2
(established direct preference), AB.6.2 (direct connection behavior), AB.7.4
(TLS authentication), and Clause 5.4 (permitted response path switching).
Established accepted and outbound membership supplies one current ordinary-send
route. Discovery can be disabled while accepted peers remain routable. Broadcast
stays Hub-only. The private built-in TLS dialer captures the verified exact leaf
before WebSocket upgrade, then joins it to the committed membership incarnation
and matching original-response capability. Full and resumed TLS tests verify the
same authenticated leaf and distinct incarnations; resumption need not resend
certificates. Custom factories remain application send-only and cannot mint identity.

One socket worker owns a shared 64-item ordinary/reply FIFO plus at most one
active write. Ready reads, including Ping/Pong, alternate with bounded writes.
Queue saturation returns an error rather than duplicating through the Hub;
uncertain started writes cannot fall back. Definitely unstarted retired work may
make a fresh route decision. Peer NPDU and encoded BVLC limits apply to both
write classes; the existing original-response sizing includes local/routed headers.
Stop/abort/drop and discovery disable retire outbound owners, with stop joining
workers. Accepted membership survives discovery disable. Already admitted work
retains immutable authority, while new retired-generation intake is fenced.

Real TLS baseline controls failed for accepted ordinary route selection and
outbound application intake. Green tests observe actual TLS replies through public
client, BACnetServer and EndpointSession paths, both server segmentation directions,
negotiated direct limits, shared queue saturation, cancellation, teardown,
replacement, resumed identity, custom-factory non-admission and uncertain-write
non-replay. The public consumer fixture advertises current Hub5705/1497 capacities;
small direct-peer capacity cases are separate. The endpoint integration binary
requires explicit `bacnet-endpoint/sc-tls`, which the lean hosted feature list does
not enable by itself.

Standalone client and native NotificationTransactions controls accept matching
Hub/direct path switches under unchanged peer-address/Invoke-ID correlation,
reject wrong-service terminals and release the exact owner. The native notification
fixture runs its real worker with actual network envelopes and production admission
API; it does not qualify a complete COV subscription lifecycle. Replacement B with
the same claimed address may complete A's pending outgoing transaction. Existing
service/direction/phase checks remain, without a same-leaf continuity assertion.
Original replies to incoming confirmed requests keep the selected #888/#889
confinement policy; optional strict outgoing route filtering is a separate choice.
No full Annex AB, Hub-relayed end-to-end identity, Python direct-entry, external
interoperability or certification claim follows. Published0.11.0 predates these APIs.
See [Rust routing and migration](../rust-api.md#bidirectional-direct-traffic).
