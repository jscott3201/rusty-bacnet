# Design: optional 135-2024 (Protocol_Revision 30) support with revision profiles

Status: **proposed**, for maintainer feedback. Nothing here is implemented
and nothing is decided; the questions at the end are the ones this document
needs answered.
Created: 2026-11-26
Sources: ANSI/ASHRAE 135-2024 (consolidated text and History of Revisions),
the individual 135-2020 addenda bx, ch, ci, cj, ck, cm, cn, co, cp, cq, cs, and
ANSI/ASHRAE 135.1-2025. These are licensed documents held outside this
repository; this document cites clause numbers and does not reproduce their
text.
Related: [conformance and support reporting](conformance-reporting.md).

Method: seven AI-assisted, read-only reviews of the code against the
standards, one per area (revision architecture, rev 19-22 baseline, objects
and enumerations, behaviour addenda, transport and segmentation,
authentication and authorization, test standard), merged by hand into this
document. No builds or tests were run. Items marked *inferred* were reasoned
from code structure or adjacent text and need confirmation before work starts.

Reviewed revision: the detailed review ran on a downstream line at
`a3edf6e20`, which is 526 commits behind `dev`. The findings in the summary
below were re-checked on `dev` at `4ade57174` and hold there. The per-item
findings in the workstream tables were not all re-checked, so they name files
without line numbers and each needs confirming on `dev` before it becomes an
issue.

## Goal

Let a device, router, hub or client built on this stack claim
Protocol_Revision 30 (135-2024) when configured to, while keeping selectable
profiles for revision 22 (135-2020) and revisions 19-21 (135-2016 with its
2018 and 2019 addenda) so that it interoperates with, and can be tested as, an
older device.

There is no 135-2018 edition. "2018 compatibility" in this document means
revision 20, the addenda approved in June 2018. Revision 19 is 135-2016 and
revision 21 is the August 2019 addenda.

## Revision map

| Rev | Addenda | Content relevant to this stack |
|---|---|---|
| 19 | 135-2016 | Baseline for the oldest profile |
| 20 | bd, be, bi, bk, bl, bm, bn, bq | Staging; audit reporting and DCC change; property id range; MS/TP |
| 21 | br, bs, bt, bu, bw | Units; Null writes; life safety re-alert; array of list; time series format |
| 22 | bj, by, bz (135-2020) | BACnet/SC; Clause 24 removed; Who-Am-I and You-Are |
| 23 | cd | TLS 1.3 cipher profile for SC |
| 24 | bv, ca, cc, ce | Schedule write-every; Color objects; SC Network Port; MS/TP terms |
| 25 | cf | Data attributes; Every Segment |
| 26 | ch, ck, cn, cq, cs | Segmentation fixes; units; CA interchange format |
| 27 | bx, ci | Device Address Proxy; error-code clarifications |
| 28 | cj, co | Lighting restore and toggle; reliability and parameter consistency |
| 29 | cp | Authentication and authorization |
| 30 | cm (135-2024) | Energy Services Interface (web services only) |

## Summary of findings

1. **The revision is a compile-time constant.** `PROTOCOL_REVISION = 22`
   (`crates/bacnet-types/src/lib.rs`) feeds the Device object and PICS
   directly. Nothing in the server, client or transports branches on the local
   or the peer's revision. The only override is wrapping the Device object, as
   `crates/bacnet-server/src/pics/protocol_revision_tests.rs` does.
2. **The current revision-22 claim is already inconsistent.** The stack serves
   Color and Color Temperature objects, Lighting Output trims and colour
   links, and advertises object-type bits 63 and 64, all of which are
   revision 24. Clause 23 lets a device borrow only error codes, units, BIBBs,
   profiles and data links from later revisions; object types, properties and
   services must match the claimed revision. 135.1-2025 Clause 5 checks this
   statically.
3. **One wire-level enumeration conflict exists today.** `NetworkPortCommand`
   has generate-csr-file = 8 and validate-changes = 9
   (`crates/bacnet-types/src/enums/network_port.rs`), which matches
   addendum cc. Addendum bx took 8 for restart-device-discovery and the 2024
   text has 9 and 10 for the certificate commands. The meaning of 8 and 9
   depends on the peer's revision.
4. **Much of the 22-to-30 delta is already present or is a no-op.** Addendum
   ca objects, ch segmentation fixes, the cd TLS profile, error codes up to
   229 and the rev 24+ property identifiers up to 4194337 are in the code.
   Addenda ck, cq and cm need no protocol work in this stack.
5. **The large missing pieces are** the BACnet/SC Network Port object (cc),
   Every Segment data-attribute handling (cf), lighting restore and toggle
   (cj), the revision-dependent error codes (ci, co), and all of
   authentication and authorization (cp).
6. **Some revision 19-22 items are not implemented**, so the 2020 claim has
   gaps of its own: Who-Am-I and You-Are beyond the codec, the
   CHANGE_OF_LIFE_SAFETY algorithm, Global Group reliability, unsubscribed
   COV.
7. **135.1-2025 has no tests for revisions 27-30** and its highest revision
   condition is 24. Of about 1,660 tests, 176 mention Protocol_Revision,
   mostly at old thresholds. Revision differences are exercised almost
   entirely by the Clause 5 EPICS consistency rules, which this project
   cannot run today because it generates no EPICS and declares no BIBBs.

## Design: revision profiles

### Rules the design must satisfy

- 135-2024 Clause 23: object types, properties, services and non-exempt
  enumerations must not exceed the claimed revision.
- Clause 12.1.3.1: clients should take a peer's Protocol_Revision into
  account when a required property may be absent.
- Annex K: client-side BIBBs require handling only up to the client's own
  claimed revision. Bit strings must cover the complete range for the claimed
  revision.
- 135.1-2025 Clause 5 rules (i), (k), (l), (m): no properties from later
  revisions, exact bit-string lengths for the declared revision, no
  properties removed by that revision.

### Shape

- `bacnet_types::revision` (no_std): `ProtocolRevision(u16)` with named
  constants, and predicates `allows_object_type`, `allows_service`,
  `allows_property`, `services_bits`, `object_types_bits`.
- Introduction data: an `introduced_in` revision on each `PropertyMetadata`
  row (properties are reused across object types, so a global table is
  wrong), plus small tables for object types, services and the gated
  enumeration values.
- Carrier: `DeviceIdentity::with_protocol_revision`, since `DeviceIdentity`
  is already the single source for Device declarations. `DeviceObject`,
  `device_view`, service dispatch, the PICS generator and the firmware Device
  read the profile from it.
- Peer side: a `PeerProfile` cache in `bacnet-client` and `bacnet-ops` keyed
  by device, holding Protocol_Revision (or "absent") and
  Protocol_Services_Supported. Services supported stays the primary signal,
  as today. Decoding stays liberal at every profile: the local revision
  changes what the stack emits and claims, never what it accepts.
- Surfaces: Rust builder method; Python `protocol_revision=` keyword on
  server and endpoint constructors with stub updates; CLI
  `--protocol-revision`; MCP describe output; firmware configuration value.
- Not Cargo features. Features are additive and cannot express a maximum
  revision. Use them only to compile out whole subsystems such as
  authorization on embedded targets.
- Default stays 22 until the rev 30 exit criteria are met, then moves to 30.

### What each profile changes

| Behaviour | Rev 19-21 | Rev 22 | Rev 30 |
|---|---|---|---|
| Protocol_Services_Supported length | per revision | 49 bits | 50 bits |
| Object types advertised | per revision; no Staging or audit below 20 | up to 62 | up to 64 |
| Color objects, LO trims and colour links | hidden | hidden | served |
| Property ids 4194304 and above in Property_List | filtered below 20 | rev 22 set only | all |
| Who-Am-I and You-Are; Device_UUID; SC | off below 22 | on | on |
| DCC DISABLE | accepted below 20 | denied | denied |
| SC port Network_Type | n/a | proprietary value | SECURE_CONNECT |
| NetworkPortCommand 8, 9, 10 | n/a | cc numbering | 2024 numbering |
| Every Segment checking | n/a | lenient | enforced with rev 25+ peers |
| INVALID_ARRAY_SIZE, INVALID_VALUE_IN_THIS_STATE on Reliability | old codes | old codes | new codes |
| Lighting restore and toggle; Last_On_Value | hidden | hidden | served |
| Authorization properties, AuthRequest, SC Hello | off | off | available, enforcement opt-in |

Clarifications that only narrow vendor-defined behaviour (ch, most of co,
ci-1, SC NAK codes) apply at every profile and are not gated.

## Workstreams

Effort: S is days, M is one to two weeks, L is several weeks. Flags: **G**
must be gated by profile, **U** adopt unconditionally, **A** purely additive.

### A. Revision profile foundation

Everything gated depends on this stream.

| ID | Work | Effort |
|---|---|---|
| A1 | `ProtocolRevision` type and predicates in `bacnet-types`; keep `PROTOCOL_REVISION` as the default | S |
| A2 | `introduced_in` on property metadata rows and tables for object types, services, gated enum values; cross-check test against spec-derived data | L |
| A3 | Carry the profile on `DeviceIdentity`; merge the two hard-coded 49-bit builders (`bacnet-objects/src/device/mod.rs`, `bacnet-server/src/device_view.rs`) into one profile-aware helper; size both bit strings by revision | M |
| A4 | Server enforcement: object registration refuses or filters types newer than the profile; Property_List, RPM ALL/REQUIRED/OPTIONAL and direct reads filter properties; dispatch rejects services outside the profile | M |
| A5 | `PeerProfile` cache and per-peer mapping hooks in client and ops | M |
| A6 | Python, CLI, MCP and firmware surfaces; `.pyi` stubs | M |
| A7 | Guard test: fail when any emitted type, property, service or gated enum value is newer than the claimed revision | S |
| A8 | Immediate correction: at revision 22, hide Color, Color Temperature, trims, colour links and bits 63/64. Delivered by A3, A4 once A2 covers lighting | S |

A2 is the highest-risk item: roughly 500 properties across 65 object types.
Source the data from the standard's per-property footnotes and the History of
Revisions, and check it mechanically (see H2) instead of by review alone.

### B. Baseline gaps, revisions 20-22

Required for an honest revision 22 claim and for any later one.

| ID | Addendum | Finding | Work | Flag | Effort |
|---|---|---|---|---|---|
| B1 | bz | Codec only; client can send Who-Am-I; no You-Are send, no server handling, not in executed services | Server execution of You-Are and Who-Am-I, client You-Are API, unconfigured-device mode | G below 22 | M |
| B2 | bt | CHANGE_OF_LIFE_SAFETY not evaluated (`bacnet-device-core/src/event/enrollment/pass/detect.rs`); Life Safety objects have no intrinsic reporting | Implement the algorithm with re-alert transitions and intrinsic reporting | U | M-L |
| B3 | bu | Global Group Reliability fixed at NO_FAULT_DETECTED (`bacnet-objects/src/group.rs`) | Evaluate member communication failure | U | S-M |
| B4 | bu | No unsubscribed COV (COVU_Period, COVU_Recipients); restart-procedure COV unverified | Implement or record as unsupported optional | A | M |
| B5 | br | No intrinsic fault reporting on Lighting Output | Add event rows and fault path | A | M |
| B6 | bq | Absentee_Limit not modelled; access event ordering not enforced (*inferred*) | Model the property; order events | U | S |
| B7 | bi | DCC DISABLE correctly denied at rev 20+; a rev 19 profile would need a real DISABLE state | Only if a rev 19 server profile is in scope (question Q3) | G | M |
| B8 | br | A stranded multi-state shrink over the network is refused where the clause expects a reliability fault | Decide and document | U | S |
| B9 | br | Event-type and message-text wording (13.8, 13.9) not reviewed | Review | - | S |

Confirmed implemented: bd Staging, bi audit reporting, bk identifier range,
bl clarifications, bm MS/TP, br Null writes and wildcard instances, bj
BACnet/SC, by Clause 24 removal. bp and bw are out of scope (no web services
or CSML in this stack; bw is a file format, see E7).

### C. Revision 23-24

| ID | Addendum | Finding | Work | Flag | Effort |
|---|---|---|---|---|---|
| C1 | cd | TLS 1.3 only, required suite and curve present (`bacnet-transport/src/sc_tls/tls_config.rs`) | None | U | - |
| C2 | cd, cp | Hub dial enforces host name and EKU by default; AB.7.4 allows no checks beyond its list and cp forbids SAN as a connection criterion | Make name and EKU checks opt-in; optional setting to offer secp256r1 first | U | S |
| C3 | ca | Implemented and tested; served under a rev 22 claim | Gate through A8; fix "New in 135-2020" doc comments | G | S |
| C4 | bv | Only the property id exists (`bacnet-types/src/enums/property_id.rs`) | Boolean property, forced write on each schedule action and clock change, datatype entry, Python keyword | G | S-M |
| C5 | cc | No SC Network Port object; only ids and two state enums exist | SC Network Port with 12.56.78-102 properties, constructed types (hub, hub-function and direct connections, failed requests), Current_Health, Command and Command_Validation_Result, hooks into the SC transport for status and configuration. Network_Type is proprietary below rev 24 and SECURE_CONNECT from 24 (AB.1.8) | G | L |
| C6 | cc, bx | Command numbering conflict (finding 3) | 2024 numbering locally; per-peer mapping for rev 24-26 peers via A5; add restart-device-discovery. Confirm against published errata first | G | S |
| C7 | cc | Heartbeat and reconnect timers settable by API only; no credential hot swap | Expose through C5; reconnect on certificate change or issuer removal | A | M |
| C8 | cc | Building blocks only for certificate replacement | CSR generation on the device, File object workflow, Clause 19.8 procedure; client side for NM-CC-A and NM-SCCM-A | A | L |
| C9 | ce | Old MS/TP names throughout Rust and Python APIs; wire unchanged | New names as primary, old as deprecated aliases; needs one-value-many-names support in `bacnet_enum!` | U | S-M |

### D. Revision 25-26

| ID | Addendum | Finding | Work | Flag | Effort |
|---|---|---|---|---|---|
| D1 | cf | `DataAttribute` stores bit 6 as `must_understand` (`bacnet-transport/src/port.rs`) | Rename to every-segment semantics; keep wire value | U | S |
| D2 | cf | Segmentation never reads data attributes; INCONSISTENT_ATTRIBUTES defined, never sent | Repeat every-segment attributes on all segments; compare on receipt; abort per 5.4.4.4 and 5.4.5.2. Enforce only with rev 25+ peers; tolerate first-segment-only Secure Path from rev 22 SC peers | G | M |
| D3 | ch | Client and server match the corrected state machines; addendum examples are tests | None | U | - |
| D4 | cn | 11 units missing (47828, 47829, 47870, 47876, 47877, 47886, 47887, 47925-47928); kelvin and joules-per-hour renames not accepted by name | Add values and aliases | U | S |
| D5 | cs | No CA interchange archive support | CLI tool for Annex AA.2 archives; depends on C8 | A | M |
| D6 | ck, cq | Editorial ASN.1; JSON short forms in the web services data model | None | - | - |

### E. Revision 27-28

| ID | Addendum | Finding | Work | Flag | Effort |
|---|---|---|---|---|---|
| E1 | ci | Event Enrollment accepts event and fault parameters for algorithms it never runs | One supported-algorithm table shared by write path and evaluator; refuse with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED | U | S |
| E2 | ci | INVALID_ARRAY_SIZE (227) never used; about 12 sites return NO_SPACE_TO_WRITE_PROPERTY, VALUE_OUT_OF_RANGE or WRITE_ACCESS_DENIED | Route array-resize refusals through one profile-aware helper | G | M |
| E3 | ci | SC NAK codes: over-long messages get INCONSISTENT_PARAMETERS; MESSAGE_INCOMPLETE and HEADER_ENCODING_ERROR largely unused | Align codes in `sc_frame` | U | S |
| E4 | co | Reliability write errors differ from 13.2.2.3 at about 14 copied sites (`bacnet-objects/src/reliability_inhibit.rs` and the per-object copies) | Consolidate, then return INVALID_VALUE_IN_THIS_STATE at rev 28+ | G | S-M |
| E5 | co | No CONFIGURATION_ERROR for crossed limits or overlapping alarm and fault value lists in Event Enrollment, analog family, AO, Pulse Converter, multi-state, CharacterString Value, Access Door, Life Safety | Shared conflict predicate per family feeding reliability evaluation. Do E1 first | U | M |
| E6 | cj | Entirely missing: operations 11-14, Present_Value -4.0 to -7.0, BLO toggle, required Last_On_Value and Default_On_Value | Enum values, engine operations, properties with persistence, Python keywords | G | M |
| E7 | ci, bw | No time-series exchange format | Optional CLI or Python import and export for DM-TSDI-A and DM-TSDE-A | A | M-L |
| E8 | bx | Device Address Proxy entirely missing; router has no Who-Is or I-Am handling | Optional; see question Q5. Property ids and codec (S), Device and Network Port properties (M), router interception, synthesised I-Am and rate limit (L), discovery timers (M), MS/TP subordinate proxy (L) | A | L+ |

Confirmed compliant: ci Schedule, Accumulator Scale, UNSUPPORTED_OBJECT_TYPE
use, relaxed COV BIBBs; co two-stage reliability evaluation.

### F. Revision 29: authentication and authorization

`bacnet_objects::access_control::evaluate_authorization` is physical access
control (Clause 12.31) and is unrelated to this addendum.

Required for any revision 29+ claim, whether or not the device authorizes:

| ID | Work | Flag | Effort |
|---|---|---|---|
| F1 | Enumerations: property ids 4194343-4194349, confirmed service choice 34, services-supported bit 49 and 50-bit string, authorization enums and scope bit string. Error codes 207-229 already present | G for the bit string, rest U | S |
| F2 | SC Hello destination option on Connect-Request and Connect-Accept; record peer capabilities per connection. Per-profile switch and a per-hub override for peers that mishandle destination options | G | S-M |
| F3 | Data-link identity rules (17.3.4) in node, hub and router: parse `bacnet://N` SANs with hub and router grants; add Identity on original messages; strip Identity relayed by unauthorized peers. The hub relay is byte-opaque today and needs a non-opaque path. Depends on D2 | G | M-L |

Required only when claiming the authorization BIBBs:

| ID | Work | BIBB | Effort |
|---|---|---|---|
| F4 | Codecs for access token, scope, constraint, policy, server, status and AuthRequest request, ACK and error; Integer32 | all | M |
| F5 | Device properties Authorization_Policy, _Groups, _Server, _Status, _Scopes, _Cache with persistence; posture; non-secure write restriction | AA-NAT-B | M |
| F6 | Target enforcement: scope classification of services and properties, extension of `ServerPolicy` to reads and infrastructure services, token validation with ES256, SECURITY error codes, Hint on responses, silent discard for unconfirmed requests | AA-AT-B | L |
| F7 | Client: public token attachment, token cache, AuthRequest initiation and retry on scope errors; Python and CLI | AA-SAC-A, AA-DAC-A | M |
| F8 | Authorization server: AuthRequest execution, signing, key rotation, denial notification | AA-AS-B, B-AS | L, optional |

Enforcement stays opt-in at every profile; the default posture must remain
open or every legacy peer loses write access. `aws-lc-rs` under `sc-tls`
covers P-256 verification and signing; embedded targets need a no_std
alternative.

Specification problems to resolve with the committee or by interop before F6:

- Signature encoding (DER or raw r and s) and the exact signed octets are
  not stated.
- NOT_AUTHENTICATED and INVALID_SIGNATURE are used but have no enumeration
  value; INACTIVE_TOKEN has a value and no description.
- The Identity option table shows bit 6 clear while the text calls it an
  every-segment option.
- No normative mapping from services and properties to scopes exists.
- The alphabetical error-code list assigns no-default-scope twice; the
  numeric list, which the code follows, is consistent.

### G. Revision 30

Addendum cm adds only Annex AD, a data tree for BACnet Web Services. It
defines no objects, properties, services or BIBBs. This stack has no web
services implementation, so no work is required to report revision 30.

### H. Conformance evidence and testing

Current state: large Rust and Python suites, fuzz targets and an operator-run
MS/TP bench method, with tests citing clauses of 135 but with no traceability
to 135.1 clauses. The PICS generator produces a readable Annex A style
document, not a 135.1 EPICS, and the project decided on 2026-10-25 to state
no BIBBs ([conformance reporting](conformance-reporting.md)). This stream
does not assume that decision changes; Q4 asks.

| ID | Work | Effort |
|---|---|---|
| H1 | Run the existing suites under each profile (22 and 30 at minimum) once stream A lands | S |
| H2 | Clause 5 consistency checker: rules (a)-(r) of 135.1-2025 as tests against the live Device and generated EPICS, per profile. This is where revisions differ and is the mechanical check for A2 | M |
| H3 | EPICS generator in the Clause 4 format: test database, fail times, value restrictions, window sizes; parse its own output | M-L |
| H4 | PICS corrections: `DeviceProfile` names that are not in Annex L (B-OP, B-ROUTER, B-SC); allow several profiles; add the 2024 Annex A blocks for BACnet/SC, authentication options and device address proxying; heading by edition | M |
| H5 | Clause-traceable matrix: tag tests with a 135.1 clause and revision range; generate clause by profile by pass, skip or missing. Start with the 176 revision-conditioned tests | M |
| H6 | Test-device harness over the in-process transport and frame tap, mirroring the test language (transmit, receive with wildcards, verify, wait). Port order: 7.1, 7.2, 9.18-9.23; then COV and segmentation; then object tests for claimed types; then initiation tests; then Clause 10, 12.3, 12.4; then 12.5 SC | L |
| H7 | Project-owned tests for revisions 27-30, tagged with the 135-2024 clause since no method of test exists | M per feature |
| H8 | External references: interoperability fixtures against other stacks (bacnet4j, bacpypes3, BACnet Stack); a BTL-style tool consuming the EPICS; assess the external `rusty-bacnet-btl-harness` for reuse | M, ongoing |
| H9 | Fuzz targets for NPDU and SC BVLC; malformed-PDU corpus for 13.4 | S |

Realistic profile targets by code evidence: B-GENERAL and B-ASC are met in
code and lack only evidence; B-AAC and B-BC need the event, schedule and
backup test bodies; B-RTR, B-BBMD and B-SCHUB need a hosted Device and the
network tests. Minimum BIBB sets themselves vary with revision (Annex L
footnotes), so any BIBB table must take the profile as input.

## Phases

| Phase | Content | Exit criterion |
|---|---|---|
| 0 | A1-A8, C6, H1, H2 | Selectable profile; rev 22 build passes the Clause 5 checker with no later-revision items; command numbering resolved |
| 1 | B1-B6, B8, B9, E1, E3, E5, C2, C9, D1, D4 | Rev 22 claim holds against this audit; unconditional clarifications in |
| 2 | C3-C5, C7, D2, H3, H4 | A device can truthfully report revision 26 |
| 3 | E2, E4, E6, F1-F3 | A device can truthfully report revision 30 without authorization BIBBs; default may move to 30 |
| 4 | F4-F7, C8, D5, H5-H9 | Authorization target and client BIBBs; certificate management; traceable test matrix |
| 5 | F8, E7, E8, B7 | Optional profiles: authorization server, time series, device address proxy, rev 19 server |

Phase 3 is the point at which "135-2024 compliant" can be stated for a
device that does not claim the optional BIBBs. Phases 4 and 5 extend what can
be claimed.

## Questions for the maintainer

- **Q0. Direction.** Is optional revision 30 support, with selectable older
  profiles, something the project wants at all, and if so does the profile
  design above fit where the Device, PICS and firmware code are heading?
- **Q1. Revision 22 and Color objects.** Hide them at 22 (this document), or
  raise the default claim to 24 once C4 and C5 land. Hiding changes behaviour
  for existing deployments that use Color objects under the current claim.
- **Q2. Profile mismatches.** Refuse to register objects newer than the
  profile, or register and filter them from the wire.
- **Q3. Scope of the pre-2020 profile.** Client tolerance and server
  filtering of types, properties and services (this document), or full server
  emulation including DCC DISABLE (B7).
- **Q4. BIBB statements.** A 135.1 EPICS and the Clause 5 checks need a BIBB
  list, which the 2026-10-25 decision excludes. Either H3 and the BIBB rules
  of H2 are dropped (the bit-string, property and service rules still work
  without BIBBs), or that decision is revisited with a reviewed,
  hand-maintained, profile-aware table.
- **Q5. Optional functional areas.** Device Address Proxy (E8), authorization
  server (F8), time series exchange (E7): in or out.
- **Q6. Sequencing and ownership.** Stream A touches most crates. Which
  pieces should become issues on the roadmap first, and which are welcome as
  outside contributions? The three that stand alone today are the command
  numbering (C6), the revision 24 items served under a revision 22 claim
  (A8), and the unconditional clarifications in phase 1.

## Risks

- Introduction-revision data entry (A2) is large and error-prone; H2 is the
  mitigation and must land with it.
- Each profile multiplies the test matrix. Limit continuous runs to 22 and
  30 and run the older profiles on a schedule.
- Behavioural gating scattered across crates. Keep it to the listed error
  codes, Every Segment, Hello, Network_Type and command numbering; route each
  through one helper.
- Unresolved ambiguities in addendum cp (stream F list) can produce a
  conformant implementation that does not interoperate.
- First-generation SC peers may mishandle destination options or a
  HelloRetryRequest. Both have per-peer switches in this document; neither risk
  has been tested.
- The test standard lags the protocol standard by six revisions, so
  revision 27-30 evidence will be project-defined until ASHRAE publishes
  tests.

## Not verified

- The 135-2020 text was not available; statements about what revision 22
  required come from the 2024 History of Revisions and addendum markup.
- Addenda bv, ca, cc, cd, ce and cf were read from the consolidated text
  only.
- Object-types bit-string length by revision (63 bits at 22, 65 from 24) is
  inferred from Annex K and the enumeration.
- Whether INVALID_ARRAY_SIZE applies to index-0 and wrong-length writes to
  fixed arrays, and whether the Reliability write rule covers the in-service
  case.
- Annex K and L changes from addendum ca against the PICS generator.
- Null-write acceptance across every object type; trend and event-log
  timestamp behaviour; hub VMAC in Connect-Accept.
