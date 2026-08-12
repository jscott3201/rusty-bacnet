# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Tranche-Q 135-2020 enumeration parity (#253): the `bacnet-types` enum
  catalogue now covers the Clause 21 productions the audit found missing,
  and enumerated resolution promotes their raw wire values by property.

  - `LifeSafetyState` gains the production tail — non-default-mode (24)
    through test-oeo-unaffected (34) — without renumbering any existing
    value; life-safety states past 23 no longer display as bare numbers.
  - New `EscalatorOperationDirection` (unknown / stopped / up-rated-speed /
    up-reduced-speed / down-rated-speed / down-reduced-speed): the
    Escalator object's `operation_direction` (477) now resolves and
    displays by name instead of falling through to `ResolvedEnum::Unknown`.
    The object still stores a raw `u32`; retyping is follow-up work.
  - New `BinaryLightingPV` (off / on / warn / warn-off / warn-relinquish /
    stop) and `LightingTransition` (none / fade / ramp); `transition` (385)
    now resolves. No resolve arm exists for `BinaryLightingPV` — it types
    only object-type-dependent present values, which stay numeric by
    policy. Binary Lighting Output docs no longer cite a nonexistent
    "fade-on" value (4 is warn-relinquish; 5 is stop); its write
    validation behavior is unchanged.
  - Access family: new `AuthenticationStatus`,
    `AuthorizationExemption`, `AccessZoneOccupancyState`, and `DoorValue`,
    with `authentication-status` (260), `authorization-exemptions` (364),
    and `occupancy-state` (296) resolving by name. `AccessDoorObject`'s
    `Relinquish_Default` validation now bounds against
    `DoorValue::EXTENDED_PULSE_UNLOCK` instead of a literal `0..=3`, so the
    write domain cannot drift from the Clause 21 production.
  - New `ProgramError` (normal / load-failed / internal / program /
    other), `RestartReason` (unknown..activate-changes, 0-8),
    `Maintenance` (none..need-service-inoperative, 0-3), and
    `Relationship` (unknown/default plus the even/odd forward/reverse
    pairs 2-29, mirroring the production's pairing note). The properties
    `reason-for-halt` (100), `last-restart-reason` (196), and
    `maintenance-required` (158) resolve as scalars;
    `subordinate-relationships` (489, a BACnetARRAY) and
    `authorization-exemptions` (364, a BACnetLIST) resolve element-wise
    through `resolve_value`'s list recursion — the same convention the
    pre-existing `accepted-modes` (175) arm follows. `represents` (491)
    deliberately has no arm: the Structured View object types it as
    `BACnetDeviceObjectReference`, not `BACnetRelationship`.
  - Two comment corrections binding comments to the production text:
    `NetworkType::NON_BACNET` was removed in version 1, revision 18 (not
    "protocol revision 16"), and `EventType`'s tag-6/tag-7 gap note now
    mirrors the Clause 21 production (tag 6 kept clear for the
    `complex-event-type` CHOICE of `BACnetNotificationParameters`; tag 7
    deprecated).

### Changed

- WriteProperty and WritePropertyMultiple now decode the ENTIRE
  `propertyValue` payload instead of exactly one application-tagged
  element: the decoder loops until the input is exhausted — the mirror of
  `encode_property_value`'s `List` flattening — so one element yields the
  scalar `PropertyValue` as before and more than one yields
  `PropertyValue::List`. Full consumption is required: a partial or
  undecodable trailing element, and an empty payload, fail the write with
  PROPERTY / INVALID_DATA_ENCODING (Clause 15.9.1.3; malformed payloads
  previously surfaced as SERVICES/OTHER from a propagated raw decoding
  error) instead of being silently dropped, and a well-formed extra
  element reaches a scalar arm to be refused as INVALID_DATA_TYPE — in
  every refusal the stored value is untouched. Context-tagged behavior is
  unchanged: framed CHOICE properties (`Event_Parameters`,
  `Fault_Parameters`) still arrive as one `ApplicationData`, and
  context-tagged member productions (the Loop/Accumulator reference
  properties) arrive as one `ApplicationData` element per member for the
  object arm to reassemble. (#182)

- Pulse Converter and Averaging PICS writability advertising now mirrors
  their `write_property` arms exactly (the truth invariant the other
  overrides follow): Pulse Converter advertises PRESENT_VALUE /
  SCALE_FACTOR / ADJUST_VALUE / INPUT_REFERENCE plus DESCRIPTION /
  OUT_OF_SERVICE / COV_INCREMENT (INPUT_REFERENCE was previously
  advertised read-only while the arm accepted it), and Averaging
  advertises OBJECT_PROPERTY_REFERENCE + DESCRIPTION / OUT_OF_SERVICE
  (its arm was likewise unadvertised). Both also stop advertising
  OBJECT_NAME, which no arm routes — a write was and is refused — so
  this is a truth-toward-arms correction, flagged for visibility like
  the Loop/Schedule corrections in #276. (panel round on #182)

- Derive the Reliability write-validation set from `Reliability::ALL_NAMED`
  instead of a restated literal: the named range was copied into
  `is_reliability_value_valid` as `0..=10 | 12..=25 | 64..=65535` from the
  Clause 21 production when the enum was short, and the two could silently
  drift — the next addendum value would gain a constant in `bacnet-types`
  while the write path kept rejecting it. The predicate now checks membership
  in the enum's named set plus the explicit vendor-proprietary range, so a
  new constant flips its value from rejected to accepted with no second edit;
  11 (reserved) and 26..=63 stay refused, the boundary matrix
  (11/25/26/63/64/65535/65536) is unchanged, and the nine consumers and
  their tests are untouched. No wire behavior changes. (#252)

- Analog event timestamp and message storage is consolidated into a shared
  implementation. Unindexed reads and the detection-disable resets are
  unchanged; indexed reads of `Event_Time_Stamps` / `Event_Message_Texts` on
  the analog types previously ignored the array index and returned all three
  elements, and now follow the same Clause 12.1.5.1 handling as the six types
  under *Added* — count at index 0, one element at 1–3, `INVALID_ARRAY_INDEX`
  beyond. (#235)

- The Device object's `Protocol_Services_Supported` is now derived from the
  set of services the server dispatch actually executes, instead of a stale
  hardcoded constant whose comment mislabeled three bits. The property gains
  twenty executed-but-undeclared services (Who-Is and Who-Has among them),
  drops four initiate-only bits (i-am, i-have,
  confirmed/unconfirmed-event-notification — Clause 12.11 ties the property
  to *executed* services), and is now sized for the full Clause 21 production
  through you-Are (bit 48), so subscribe-cov-property-multiple (bit 41) is
  representable. The PICS executor column derives from the same constant
  (`bacnet_objects::device::EXECUTED_SERVICES`), a cross-check test ties both
  to the dispatch table, and service choice → services-supported bit mapping
  is now explicit via `ServiceSupported::from_confirmed_choice`/
  `from_unconfirmed_choice`. Deployments embedding a different dispatch
  surface can override via `DeviceObject::set_services_supported`. (#192)

- **Breaking (wire format):** all ≤8-bit BACnet bit strings are now encoded
  MSB-first per Clause 20.2.10 — the first defined bit occupies bit 7 (`0x80`)
  of the octet. Previously `Event_Enable`/`Acked_Transitions` (here and in
  GetEventInformation ACKs) and `Recipient_List`'s `valid_days`/`transitions`
  were packed LSB-first within the octet, so TO_OFFNORMAL/TO_NORMAL arrived
  swapped and the `valid_days` week was reversed for conformant peers
  (Monday/Sunday inverted). Both encode and decode changed together; peers
  that interoperated with the old bytes (including older releases of this
  stack) will see transition and day masks bit-reversed until upgraded.
  `Status_Flags`, `Limit_Enable`, `Ack_Required`,
  `Protocol_Object_Types_Supported` and `Protocol_Services_Supported` were
  already MSB-first and are unchanged. The conversion now lives in
  `bacnet_types::bitstring::pack_octet`/`unpack_octet` (with typed
  `to_bacnet()` on `EventTransitionBits`/`LimitEnable`), tested against
  asymmetric spec vectors (`TO_OFFNORMAL → 0x80`, `monday → 0x80`). (#203)

### Added

- Structured property values are writable over WriteProperty /
  WritePropertyMultiple: MSI `Alarm_Values` whole-list writes (consecutive
  application-tagged BACnetLIST elements now reach the arm as
  `PropertyValue::List` with per-element validation; the tranche pin test
  flips from pinned-failure to success), DateTime Value `Present_Value`
  and `Priority_Array` entry writes from an application-tagged Date+Time
  pair, and `Relinquish_Default` on DateTime Value and DateTime Pattern
  Value (Clause 12.38 / 12.46) — the last two tranche-L1 exclusions,
  completing #270's permitted writability across all twelve commandable
  value types. (#182)

- Network-writable structured reference properties: Loop
  `Controlled_Variable_Reference` / `Manipulated_Variable_Reference` /
  `Setpoint_Reference` (Clause 12.17) and Pulse Converter `Input_Reference`
  (Clause 12.23) accept their Clause 21 wire forms through a new strict
  `BACnetObjectPropertyReference` codec in bacnet-encoding
  (`constructed::object_property_reference`: bare `[0]`/`[1]`/`[2]`
  context-tagged members with full consumption, device-qualifying member
  `[3]` rejected — these references are local-device only) and its
  `BACnetSetpointReference` `[0]`-frame companion. The four arms share an
  objects-layer decode helper (`bacnet-objects::reference`) that keeps the
  legacy local `List([ObjectIdentifier, Enumerated, Unsigned?])` form, and
  the read arms now carry the reference's optional array index as a third
  list element. Malformed framed writes refuse PROPERTY /
  INVALID_DATA_ENCODING, wrong-datatype writes INVALID_DATA_TYPE, and
  WritePropertyMultiple in-order commit / rollback is proven on a failing
  reference request. (#182)

- The empty `BACnetSetpointReference` frame (`0x0E 0x0F`) is accepted on
  Loop `Setpoint_Reference`: the production's member is OPTIONAL and its
  absence is the Clause 12.17 "fixed setpoint" state, so a conformant
  peer writing the absent member now clears the reference exactly like a
  `Null` write (previously refused PROPERTY / INVALID_DATA_ENCODING).
  The framed codec's `decode_setpoint_reference` accordingly returns
  `Option<BACnetObjectPropertyReference>`. (panel round on #182)

- Make `Relinquish_Default` writable over the network and add a validated
  local `set_relinquish_default` on the commandable object types: Analog
  Output, Analog Value, Binary Output, Binary Value, Multi-state Output,
  Multi-state Value, Lighting Output, Binary Lighting Output, Access Door,
  and the commandable value-object types (Integer, Positive Integer and
  Large Analog scalars among them). Validation mirrors each type's
  Present_Value write — finite Real on the analog types (0..=100 for
  Lighting Output), BinaryPV 0/1, Unsigned 1..=Number_Of_States (whose
  shrink interplay stays "a local matter" per Clauses 12.19 / 12.22 and is
  deliberately not auto-adjusted), BinaryLightingPV 0..=4, and the Access
  Door's BACnetDoorValue production 0..=3 (lock, unlock, pulse-unlock,
  extended-pulse-unlock) — and a store re-resolves Present_Value from the
  priority array, so an all-NULL array immediately adopts the new default
  while a live command still outranks it. The conformance tables (e.g.
  Tables 12-3, 12-8, 12-22, 12-64, 12-69, 12-30) permit rather than require
  the writability, so this documents permitted-writability implemented, not
  a conformance upgrade. The datetime-paired value types (DateTime Value,
  DateTime Pattern Value) shipped with the local setter only; #182 below
  now makes them network-writable too. These writes were previously
  refused; `Property_List` is unchanged. (#270)

- Model `Event_Time_Stamps` and optional `Event_Message_Texts` on Binary
  Input/Output/Value and Multi-state Input/Output/Value objects. Both appear in
  `Property_List` and return to their initial values when
  `Event_Detection_Enable` is disabled. Indexed reads follow Clause 12.1.5.1:
  index 0 returns the count, indices 1–3 return one element, and later indices
  return `INVALID_ARRAY_INDEX`; element encoding remains in the interim
  flattened form pending #171. (#235, #230, #258)

- Name the missing tail values of two enumerations, completing them against
  their 135-2020 productions: `BackupAndRestoreState` gains `BACKUP_FAILURE`
  (5) and `RESTORE_FAILURE` (6) — states a device legitimately reports during
  Clause 19.1 backup/restore procedures — and `Reliability` gains
  `MULTI_STATE_OUT_OF_RANGE` (25). Both are open enums, so the raw values
  already round-tripped; what changes is that they now display by name
  (previously a device reporting backup-failure showed as the bare number
  `5`, including through `resolve_value` on Backup_And_Restore_State).
  Purely additive, no wire-format change. (#246, #241)
- `FaultDetector` gained a private field for warning suppression and is therefore
  no longer constructible with struct-literal syntax. `FaultDetector { comm_timeout }`
  becomes a compile error; use `FaultDetector::new(comm_timeout)`, which has been
  available all along. `comm_timeout` remains `pub` and readable and writable as
  before. This is a source break in a published crate and is called out separately
  from the trait addition below, because the compiler error it produces names a
  private field rather than the change that caused it.
- Add `BACnetObject::set_reliability_internal` as the trusted lifecycle route for
  object reliability evaluation, distinct from the network `WriteProperty`
  route. Ownership is symmetric: clients may write `Reliability` only while
  `Out_Of_Service` is TRUE, and internal evaluation may write it only while
  `Out_Of_Service` is FALSE. Entering simulation saves the evaluated value and
  leaving restores it, avoiding a transient NO_FAULT_DETECTED window on analog
  objects while still discarding a client simulation; an object restored
  already out of service falls back to NO_FAULT_DETECTED when no saved value
  exists. The `bacnet-objects` crate is a public dependency;
  downstream `BACnetObject` implementors registered as Analog Input, Analog
  Output, or Analog Value must override this method to keep server fault
  detection working. The `OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED` default leaves
  their fault-detection update unapplied and emits a warning. Both public and
  internal routes enforce the same BACnetReliability value set; repeated
  identical internal-route failures are warned once per object until the error
  changes or a write succeeds.
- Add `Event_Detection_Enable` to Analog Input, Analog Output, Analog Value, Binary Input, Binary Value, Multi-state Input and Multi-state Value (ASHRAE 135-2020 Clauses 12.2, 12.3, 12.4, 12.6, 12.8, 12.18 and 12.20). It carries conformance code **O** on all seven with a bidirectional footnote pair — "These properties are required if the object supports intrinsic reporting" and "These properties shall be present only if the object supports intrinsic reporting" — so on types that do support intrinsic reporting, as these seven do, its presence is required rather than optional. With Binary Output and Multi-state Output already covered, all nine intrinsically-reporting types now model the property. The property gates both intrinsic-reporting entry points, and a write of FALSE establishes the Clause 13.2.2.1 initial conditions for the state each type actually carries: `Event_State` NORMAL, `Acked_Transitions` all-set, no pending Time_Delay countdown, and — on the types that model them, now all nine — `Event_Time_Stamps` and `Event_Message_Texts` restored as well. The reset runs on any write of FALSE rather than on the TRUE→FALSE edge, because an edge-only reset never fires for an object constructed or restored with the property already FALSE.

  **On Binary Input, Binary Value, Multi-state Input and Multi-state Value the invariant was established only partially by this change, and not because those types were exempt.** Clause 13.2.2.1 names `Event_Time_Stamps` and `Event_Message_Texts` alongside `Acked_Transitions`. The conformance footnote pair that makes `Event_Detection_Enable` required on an intrinsically-reporting object makes `Event_Time_Stamps` required as well; `Event_Message_Texts` carries only the presence-restriction footnote and is optional. Those four types did not model either history property at that point; they gain both in the entry above. The gap predated this change: all four already called `impl_intrinsic_reporting!` and therefore already supported intrinsic reporting without the timestamps their footnote requires. This change closed the `Event_Detection_Enable` omission and left the storage omissions to #235, alongside #230 (the identical omission on Multi-state Output) — both now closed by the entry above.

  **The default is TRUE, and it deliberately differs from Binary Output and Multi-state Output, which stay FALSE.** 135-2020 specifies no default for this property anywhere, so this is a project choice. The event-algorithm half is inert on a fresh object: `OutOfRangeDetector` starts with `LimitEnable::NONE`, and `ChangeOfStateDetector` with empty `alarm_values`. The fault half is not inert — `fault_step` runs first and depends only on `Reliability`. On Analog Input/Output/Value, enabling the server `FaultDetector` can derive a non-normal `Reliability` from `Min_Pres_Value` / `Max_Pres_Value` and thereby produce a ToFault transition before the event algorithm is commissioned. Binary Input/Value and Multi-state Input/Value have no route that can set `Reliability`, so their fault path remains inert. TRUE preserves the pre-change behavior precisely: all seven previously used the ungated macro arm, so both their event and fault paths always ran; FALSE would silently suppress a fault path that already works on the three analog types. Binary Output and Multi-state Output stay FALSE because their previously nonfunctional intrinsic reporting became live with COMMAND_FAILURE and needs a commissioning gate.

  **A FALSE default would also have been observable on the wire.** Clause 13.12's GetEventInformation Service Procedure phrases its predicate as objects "that do not have an Event_Detection_Enable property with a value of FALSE", while Clauses 13.10 and 13.11 say an object with the property FALSE "shall be ignored" — so an object *lacking* the property is summarized, and an object *carrying* it as FALSE is not. Defaulting these seven to FALSE would therefore have removed them from GetAlarmSummary, GetEnrollmentSummary and GetEventInformation with no change in `bacnet-server` at all. That absence-means-included case is pinned by its own test.

  Observable consequence of the addition itself: each object's `Property_List` gains an entry, so array indices after the insertion point shift and a ReadPropertyMultiple of ALL returns one more element. A ReadPropertyMultiple of REQUIRED is unaffected, since it expands from the `required_properties()` trait default that no object type overrides.

  The analog half of the reset — `Event_Time_Stamps` and `Event_Message_Texts` — is written but cannot yet change anything, because nothing on these objects ever populates those fields on a transition (#123). It is guarded by direct unit tests that seed the fields rather than by a behavioral test that would pass either way, so the guard becomes live when #123 lands.
- Add `BACnetObject::is_writable_property`, `BACnetObject::is_createable`, and `BACnetObject::is_deleteable` trait methods (with conservative defaults that preserve current behavior for unmigrated object types) so PICS generation and runtime dispatch share one truth source for property writability and object createability. The `bacnet-objects` crate is a public dependency; downstream implementors of `BACnetObject` now have these methods available to override (defaults preserve existing PICS output).
- Add `BACnetServer::write_local` as the server-owned local-mutation entry point. It writes a property under the database lock (routing `OBJECT_NAME` through the name-uniqueness check and index refresh, like the network handler) and then fires the same post-write COV and event notifications as a network `WriteProperty`. The Python `write_property_local` binding now delegates to it.
- Add `BACnetObject::tick_intrinsic_reporting` as the periodic (1-second) counterpart to `evaluate_intrinsic_reporting`. The production server now runs a 1-second `intrinsic_reporting_task` that calls `tick_intrinsic_reporting` on every object to advance pending Time_Delay countdowns and fire confirmed transitions, mirroring the per-write `evaluate_intrinsic_reporting` (probe) path. Object implementors overriding intrinsic reporting should implement both methods together.
- Add `BACnetObject::set_event_state_internal` as the internal lifecycle path for the algorithmically-derived `Event_State` on objects such as Event Enrollment (ASHRAE 135-2020 Clause 12.12). It is distinct from the network `write_property` route (`Event_State` is read-only over the network); the server evaluator calls it to persist a detected transition. Objects without an algorithmic `Event_State` keep the default (`OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED`). The `bacnet-objects` crate is a public dependency; downstream implementors of `BACnetObject` now have this method available to override.
- Add `ServerConfig::enable_event_enrollment` (default `true`) and `ServerConfig::event_enrollment_interval_secs` (default 10), with matching builder methods on all three server builders, so Event Enrollment evaluation is configured independently of `enable_fault_detection`. `ServerConfig` is not `#[non_exhaustive]`, so code that constructs it with an exhaustive struct literal must add the two new fields or switch to `..ServerConfig::default()`; the builder methods and `..default()` construction are unaffected.
- Add `Event_Detection_Enable` to the Event Enrollment object (ASHRAE 135-2020 Clause 12.12, Table 12-14 conformance code **R** — required, with no footnote qualifier), and honor it in the evaluator and in all three event-summarization services. Note the property is required on Alert Enrollment too (Table 12-61), which makes #205 — where `AlertEnrollmentObject` exposes it but applies none of the required reset — a required-property conformance gap rather than an optional-property nicety. The property was previously implemented only on `NotificationForwarderObject` and `AlertEnrollmentObject`, so Event Enrollment — the one object type where the standard *requires* it — was missing a required property and there was no conformant per-object way to disable detection. The server-level `enable_event_enrollment` switch added earlier is an implementation control over a background task and performs none of the required reset; it remains, and remains documented as distinct from this property.

  **The disabled state is an invariant, not a one-shot action.** Clause 13.2.2 puts it in the continuous tense — "When the event-state-detection process is disabled via the Event_Detection_Enable, both the event algorithm and the Reliability value are ignored, and Event_State *remains* NORMAL" — and Clause 12.12 as a predicate: "When this property is FALSE, Event_State shall be NORMAL, and the properties Acked_Transitions, Event_Time_Stamps, and Event_Message_Texts shall be equal to their respective initial conditions." An edge-only reset would be unsound, because it never fires for an object constructed or restored from persistence with the property already FALSE. So the reset runs on the write, and every other path that can change `Event_State` refuses a non-NORMAL value while detection is off: `set_event_state_internal` (the evaluator's lifecycle path) returns an error, and the public `set_event_state` seeder ignores the value. That last one matters — it is `pub` and bypasses both `write_property` and the trait method, so without its own guard the API would offer a documented way around a rule the rest of the object enforces, and the invariant would hold only by convention. With all three covered it holds by construction: no reachable path leaves an Event Enrollment with `Event_Detection_Enable` FALSE and a non-NORMAL `Event_State`. The evaluator also skips disabled enrollments, implementing Clause 13.2.2.1's separate requirement that "this state machine is not evaluated"; that guard is not independently observable (the object-level refusal already blocks the transition) and is documented in place as such so it is not removed on the assumption that a test covers it.

  **All three summarization services now filter on the property.** Clause 12.12 says it "controls whether (TRUE) or not (FALSE) the object will be considered by event summarization services", and each service repeats the exclusion independently in its own Service Procedure — Clauses 13.10, 13.11 and 13.12 — reinforced by Table 13-2 in Clause 13.2.4. The wording differs in a way that matters: 13.10 and 13.11 say an object with the property FALSE "shall be ignored", while 13.12 folds it into the search predicate as a double negative, which is what makes absence mean *included*. It is checked directly rather than inferred from `Event_State == NORMAL`: GetEnrollmentSummary applies no default `Event_State` filter at all, so for that service the exclusion cannot follow from the forced-NORMAL invariant, and `AlertEnrollmentObject` does not yet apply the reset (#205), so inferring it would leak a disabled object. Absence of the property means **enabled**, matching Clause 13.12's Service Procedure and its deliberate double negative ("objects that do not have an Event_Detection_Enable property with a value of FALSE"); the inverse reading would silently empty these responses for every object type that does not model the property, and that case is pinned by its own test.

  **`Event_Detection_Enable` is writable**, and `is_writable_property` is overridden on the object so PICS reports what dispatch accepts rather than inheriting the historical default (which omitted every property this object actually writes). Conformance code R means "required to be present and readable" and does not imply writability, but Clause 12.1.2 allows an R property to be writable "at the implementor's option unless specifically prohibited in the text describing that particular standard object's property" and Clause 12.12 prohibits nothing — "is expected to be set during system configuration" is guidance, not a "shall". Annex K's AE-AVM-A BIBB (K.2.17 with Table K-17) positively requires a conforming workstation to be able to *write* this property, without the footnote-1 present-only exemption that covers `Event_State`, `Acked_Transitions`, `Event_Time_Stamps` and `Event_Message_Texts`.

  **The default value TRUE is a project choice, not a spec requirement** — 135-2020 specifies no default for this property anywhere, and Clause 15.3's CreateObject Service Procedure makes the initial values of properties not named in a CreateObject request "a local matter". TRUE preserves the always-detecting behavior Event Enrollment had before the property existed, and matches the two objects that already implemented it. Observable consequences: the object's `Property_List` gains an entry (so array indices after the insertion point shift, and a ReadPropertyMultiple of ALL returns one more element — REQUIRED is unaffected, since it expands from the `required_properties()` trait default that no object type overrides), and an enrollment with the property written FALSE disappears from GetAlarmSummary, GetEnrollmentSummary and GetEventInformation. The `Acked_Transitions` half of the reset is written but cannot yet change anything — nothing on this object ever clears that field, since the Clause 13.2.3 alarm-acknowledgment process is unimplemented (#123) — and is marked in place as unprovable rather than covered by a test that would pass either way.
- Add `bacnet_objects::event::TransitionOutcome`, a `{ change: EventStateChange, event_type: EventType, distribute: bool }` record that separates *a transition occurred* from *its notification may be sent*, and carries the Event Type the detector's algorithm selected (the `event_type` field arrived later in this same unreleased block — see the Event Type entry under Fixed). `BACnetObject::evaluate_intrinsic_reporting` and `BACnetObject::tick_intrinsic_reporting` now return `Option<TransitionOutcome>` instead of `Option<EventStateChange>`. **This is a breaking signature change** on a public trait: implementors overriding either method must update their return type, and callers must read `.change` for the transition and branch on `.distribute` before sending a notification. `None` now means only "no transition fired"; a transition whose `Event_Enable` bit is clear is returned with `distribute: false` rather than as `None`. `EventEnrollmentTransition` likewise gains a `pub distribute: bool` field; the struct is not `#[non_exhaustive]`, so exhaustive struct literals and patterns over it must account for it. **That field addition is a compile error a consumer cannot miss, but it ships alongside a change that is silent:** `evaluate_event_enrollments` now returns transitions it previously filtered out, so a caller that iterates the returned `Vec` and emits a notification per element will begin emitting previously-suppressed notifications with nothing failing to compile. Per-element consumers must branch on `.distribute` before sending.
- Add the `Feedback_Value` property to Binary Output (`BACnetBinaryPV`, Table 12-8 code **O4**) and Multi-state Output (`Unsigned`, Table 12-22 code **O1**), both carrying the footnote "These properties are required if the object supports intrinsic reporting." It binds the `pFeedbackValue` parameter of the COMMAND_FAILURE algorithm those object types are required to apply — see the entry under Fixed. `PropertyIdentifier::FEEDBACK_VALUE` already existed as an enum variant and was referenced by no object type. Each object's `Property_List` gains an entry, so array indices after the insertion point shift and a ReadPropertyMultiple of ALL returns one more element. A ReadPropertyMultiple of REQUIRED is **not** affected: it expands from `required_properties()`, which is a trait default no object type overrides, returning only `Object_Identifier`, `Object_Name`, `Object_Type` and `Property_List`.

  It is writable, and `is_writable_property` is overridden to match what dispatch accepts. The standard makes the manner of its determination "a local matter", which on a device with no physical I/O leaves a network write as the only way to supply it. It initializes to each object's initial `Present_Value` (`0` for Binary Output, `1` for Multi-state Output) so a freshly constructed object reads as agreeing and starts in NORMAL rather than immediately in alarm.

  **The Multi-state Output value is deliberately not range-checked against `Number_Of_States`, unlike `Present_Value`.** Clause 12.19 treats an out-of-range `Feedback_Value` as a condition to be reported, not a write to refuse: "If any of those properties other than Present_Value are out of range, the value of the Reliability property shall remain CONFIGURATION_ERROR, unless the object is out of service." Rejecting the write would make that reliability unreachable. The asymmetry with `Present_Value` is intentional — `Present_Value` is commanded, so a value outside the state set is meaningless, whereas `Feedback_Value` reflects a sensed quantity that can legitimately fall outside it. Actually setting `CONFIGURATION_ERROR` (and `MULTI_STATE_OUT_OF_RANGE` for `Present_Value`) is unimplemented and tracked as #226; the accepting behavior is pinned by its own test so the check is not reinstated as an apparent oversight. The Binary Output check is a different thing and is kept: `BACnetBinaryPV` is a two-valued enumeration, so rejecting `2` enforces the datatype rather than a configurable range.
- Add `Event_Detection_Enable` to Binary Output and Multi-state Output (ASHRAE 135-2020 Clause 12.7 Table 12-8 code **O4,6**, Clause 12.19 Table 12-22 code **O1,3**), **defaulting to FALSE**. Both clauses print the property definition in full and identically, and both carry two footnotes: "These properties are required if the object supports intrinsic reporting" and "These properties shall be present only if the object supports intrinsic reporting." That pair is bidirectional — presence and support imply each other — so exposing this group is itself the declaration that these types support intrinsic reporting.

  **FALSE is a project choice the standard permits, not a value it specifies.** 135-2020 gives no default for this property for any object type, and Clause 15.3 makes the initial values of properties not named in a CreateObject request "a local matter". The choice differs from the TRUE chosen for Event Enrollment earlier in this same unreleased block, and for the same stated reason: preserve each object type's prior behavior. Event Enrollment always detected, so TRUE preserved it; Binary Output and Multi-state Output could never detect at all, so FALSE preserves theirs.

  **The disabled state is an invariant, not a one-shot action** — the same reading applied to Event Enrollment, and it matters more here because the default is FALSE, so a freshly constructed object never sees a write at all and an edge-triggered reset would never run for the common case. Clause 12.7 and 12.19: "When this property is FALSE, Event_State shall be NORMAL, and the properties Acked_Transitions, Event_Time_Stamps, and Event_Message_Texts shall be equal to their respective initial conditions." Clause 13.2.2.1 states it generally: "If the Event_Detection_Enable property is FALSE, then this state machine is not evaluated. In this case, no transitions shall occur, Event_State shall be set to NORMAL, and Event_Time_Stamps, Event_Message_Texts and Acked_Transitions shall be set to their respective initial conditions." Writing the property FALSE performs the reset (including discarding any in-flight `Time_Delay` countdown), and while it is FALSE both `evaluate_intrinsic_reporting` and `tick_intrinsic_reporting` return `None` without touching detector state. On these two object types the invariant holds by construction rather than by convention: unlike Event Enrollment there is no public `set_event_state` seeder and no `set_event_state_internal` override, so the gated detector is the only route that can change `Event_State`.

  `Status_Flags` needed no change and deliberately received none. Clause 13.2.2's "both the event algorithm and the Reliability value are ignored" is scoped to the event-state-detection process; both object clauses define FAULT as tracking `Reliability` directly, so it keeps doing so while detection is disabled, and IN_ALARM follows the forced-NORMAL `Event_State`. The three event-summarization services also needed no change — `event_detection_enabled()` reads the property generically through `read_property`, so these objects are filtered automatically now that the property exists.

  Multi-state Output had no `Event_Time_Stamps` property to reset at that point, although its footnote requires one once support is declared; adding storage without transition maintenance then would have left it stale after the first event, so it was tracked as #230 alongside #123 and #171 — and #230 is now closed by the `Event_Time_Stamps` entry above, whose stored initial conditions are correct until #123 populates them.
- Split `read_event_properties!` / `write_event_properties!` into generic and analog halves — `read_generic_event_properties!`, `read_analog_event_properties!`, `write_generic_event_properties!`, `write_analog_event_properties!` — and wire the generic halves into Binary Output and Multi-state Output. The generic half covers `Event_State` (read-only), `Event_Enable`, `Notify_Type`, `Notification_Class`, `Time_Delay` and `Acked_Transitions`, all of which touch detector fields every detector carries; the analog half keeps `High_Limit`, `Low_Limit`, `Deadband` and `Limit_Enable`; the `Event_Time_Stamps` / `Event_Message_Texts` reads, analog-only at the time of this split, have since moved to the shared storage described under *Changed*. The analog types call both halves.

  Without this the algorithm ran but could not be commissioned. Every detector defaults to `event_enable: 0`, and `Event_Enable` had only a read arm on these types, so all three transition bits were stuck false and no notification could ever reach the wire; `Time_Delay` and `Notify_Type` had no arm at all, so `pTimeDelay` was permanently 0. Clauses 12.7 and 12.19 are explicit that this is not a permitted restriction: "A device is allowed to restrict the set of supported values for this property but shall support (T, T, T) at a minimum." Both objects' `Property_List` and `is_writable_property` now include the group. The same gap on Binary Input, Binary Value, Multi-state Input and Multi-state Value was tracked as #229, which the split unblocked; it is closed under *Fixed* below.

  **`Acked_Transitions` is the one member of that group that stays read-only**, on every object type that uses either macro half. It is maintained by the alarm-acknowledgment process from event-state transitions and acknowledgment indications — the latter arriving from AcknowledgeAlarm or a local means — and an indication ORs the acknowledged bit in where a property write would assign — so a writable arm could both fabricate an acknowledgment and erase one, and GetAlarmSummary and GetEventInformation read the field straight off the object. It also carries the Clause 12.7 / 12.19 requirement that the field equal its initial condition while `Event_Detection_Enable` is FALSE, which an ungated write arm would be the only route to break. An intermediate revision of this change made it writable on the analog types; that was a regression against the denial they already had, and the mirror test `ai_is_writable_property_mirrors_write_property` — whose whole purpose is to keep `is_writable_property` and `write_property` in lock step — now asserts on this property, which it previously did not.
- Add gated arms to `impl_intrinsic_reporting!`: a five-ident form for feedback-driven detectors and a four-ident form for detectors without feedback, both taking an `Event_Detection_Enable` field. Binary Output and Multi-state Output use the five-ident arm; the other seven intrinsic-reporting types use the four-ident arm. There is deliberately no ungated form: the former three-ident arm was removed later in this same unreleased block, and no feedback-without-gate arm is provided, because either would offer downstream implementors a supported way to wire detection permanently on — the exact defect the gate was added to fix.

### Fixed

- Harden the Averaging `Object_Property_Reference` write arm, reachable
  over the network for the first time via the #182 multi-element decode:
  its `items.len() >= 2` acceptance silently dropped fourth-and-later
  members, retyped a non-Unsigned third member to no-index, and truncated
  64-bit property/index numerics with `as u32`. The arm now routes through
  the shared reference-arm decode used by Loop/Pulse Converter: exact
  two-or-three member shape, `u32::try_from` bounds (an oversized member
  refused instead of truncated), framed-form full consumption, and a
  device-qualifying member `[3]` refused PROPERTY / INVALID_DATA_ENCODING —
  Clause 12.5 leaves referencing another device's object OPTIONAL
  ("Optionally, the object property to be sampled may exist in a different
  BACnet device") and this object samples local objects only. The flat
  local form accepts its property-id member as Unsigned OR Enumerated (the
  Averaging convention carries Unsigned, kept on reads; the framed form
  stays the Clause 21 encoding). (adversary panel blocker on #182)

- Gate in-service `Reliability` writes on the remaining Out_Of_Service
  carriers: Loop accepted any Enumerated unconditionally and Schedule stored
  without validation; both now refuse PROPERTY / WRITE_ACCESS_DENIED in
  service, validate out-of-service writes against the BACnetReliability set,
  and save/restore the evaluated value across the Out_Of_Service edge like
  the nine intrinsic-reporting types. Trend Log's Reliability arm — an
  ungated, unvalidated store with no Clause 12.25 writability grant — now
  refuses PROPERTY / WRITE_ACCESS_DENIED entirely, and Trend Log Multiple's
  default denial is pinned as deliberate: neither table carries the
  writability footnote and neither object's Reliability_Evaluation_Inhibit
  text carries the out-of-service write provision. Previously-accepted
  writes now fail on all three. (#240)

- **Breaking (write validation):** `Notify_Type` writes validate against the
  BACnetNotifyType production — alarm(0), event(1), ack-notification(2) —
  and refuse out-of-production values with PROPERTY / VALUE_OUT_OF_RANGE
  instead of storing them (an accepted `Enumerated(99)` previously read back
  as 99 and could reach the wire as the notification's notifyType).
  `Event_Enable` and `Limit_Enable` writes now require the canonical
  encoding of their fixed-width productions (BACnetEventTransitionBits: one
  content octet with 5 unused bits; BACnetLimitEnable: one with 6); any
  other declared shape refuses PROPERTY / INVALID_DATA_ENCODING instead of
  being silently masked and normalized, and an empty content reports
  INVALID_DATA_ENCODING instead of INVALID_DATA_TYPE. Applies to the shared
  event-property macros (all nine intrinsic-reporting types), Event
  Enrollment, and Alert Enrollment. (#255)

- Make CHANGE_OF_STATE alarm parameters reachable over BACnet: Binary Input
  now exposes singular `Alarm_Value` as `BACnetBinaryPV` per Table 12-6
  (dropping the invented plural `Alarm_Values` arm, which read as an empty
  list and now returns UNKNOWN_PROPERTY), and Binary Value gains the same
  property; both default to ACTIVE(1) with the detector armed — an owner
  ruling, since the standard defines no default — so `Event_State` reacts to
  `Present_Value` out of the box while distribution still waits on
  `Event_Enable`. Multi-state Input and Value's `Alarm_Values` is now
  configurable over the network via AddListElement/RemoveListElement, with
  readback served from the detector-owned list — the disconnected mirror
  fields are gone, so what a client reads is what the event algorithm
  compares. A whole-list WriteProperty still fails with INVALID_DATA_TYPE:
  the write decoder handles a single application-tagged primitive (#182),
  so the dispatch-layer LIST arm added here becomes network-reachable for
  direct writes only when that lands. List writes reject an array index
  (the property is a BACnetLIST) and cap at 1,024 elements, the same
  resource-cap convention as COV subscriptions. `Property_List` changes:
  Binary Input/Value gain `Alarm_Value`, Multi-state Value gains
  `Alarm_Values`, Multi-state Input loses `Fault_Values`, and Multi-state
  Output loses both removed properties. `resolve_value` now names property
  6 (alarm-value) as `BinaryPV` — a new `ResolvedEnum` variant, which is a
  compile-time break for downstream exhaustive matches on that enum. (#228)
- Wire the generic event-configuration properties into Binary Input, Binary
  Value, Multi-state Input and Multi-state Value: `Event_Enable`,
  `Notification_Class`, `Notify_Type` and `Time_Delay` are now writable over
  the network (the latter two previously had no arm at all), advertised in
  `Property_List`, and reported through the same
  `is_generic_event_property_writable` truth source PICS reads — the #227
  macro split, already carried by Binary Output and Multi-state Output.
  Clauses 12.6, 12.8, 12.18 and 12.20 require the supported `Event_Enable`
  value set to include (T, T, T); these detectors default to (F, F, F) and
  had no commissioning path, so notification distribution was unreachable in
  practice. `Event_State` stays read-only, and `Acked_Transitions` writes
  stay denied — the alarm-acknowledgment process maintains it. Wire-level
  tests pin single-bit `Event_Enable` gating on Multi-state Input in both
  transition directions; MSI is the vehicle because the other three types'
  detectors cannot enter OFFNORMAL until #228 lands. (#229)
- Preserve WritePropertyMultiple atomicity when rolling back an
  `Out_Of_Service` write. The rollback now restores the client-simulated
  `Reliability` and reconstructs the saved evaluated value as well as restoring
  `Out_Of_Service`.
- Re-enter FAULT when `Reliability` changes to a *different* fault value (ASHRAE 135-2020 Clause 13.2.2.1). The Fault state defines a ToFault transition — "If reliability-evaluation indicates a different Reliability value and the new Reliability value is not NO_FAULT_DETECTED ... then perform the corresponding transition actions and re-enter the Fault state" — and the same clause makes the transition actions apply "even if the transition does not change the event state". `fault_precedence` reduced reliability to a boolean on its first line and no detector retained the previous value, so a change from `OVER_RANGE` to `NO_SENSOR` while already in FAULT held silently and no `CHANGE_OF_RELIABILITY` notification was produced.

  `FaultPrecedence` gains a `ReenterFault` variant, and each of the three detectors gains a `fault_reliability: Option<u32>` holding the value in force at the last entry to FAULT.

  **Whether FAULT holds is still a standing condition; only the transition is an edge.** That distinction is load-bearing rather than stylistic. `fault_step` runs at the head of both `probe` and `tick`, and the server drives `tick` once per second, so deriving re-entry from the standing condition instead of the changed value would emit a FAULT notification every second for as long as any object stayed faulted. A test asserts that a detector re-evaluated repeatedly with an unchanged non-normal `Reliability` fires exactly once, and it is one of the mutation targets.

  `fire()` needed no change: it never compared `from` to `to`, `EventStateChange::event_type` already returns `CHANGE_OF_RELIABILITY` when either end is FAULT, and `EventTransition::for_target_state(FAULT)` already yields `ToFault`. A same-state FAULT transition classified correctly as soon as it could be produced at all. The `Event_Detection_Enable` FALSE reset on Binary Output and Multi-state Output also clears `fault_reliability`. That line changes no observable behavior — the reset already sets `Event_State` to NORMAL, so the next evaluation takes the `EnterFault` path, which ignores and overwrites the field — and removing it leaves the suite green. It is there to preserve the field's own invariant, `Some` exactly while FAULT holds, which is what keeps the "in FAULT with no recorded value" case from being reachable in this crate.

  **Breaking:** `fault_reliability` is a new public field on `OutOfRangeDetector`, `ChangeOfStateDetector` and `CommandFailureDetector`. None is `#[non_exhaustive]`, so exhaustive struct literals must account for it.

  Only the first disjunct of the ToFault condition is implemented. The second — "reliability-evaluation indicates a transition to the Fault state with the same Reliability value" — has no source in this codebase, since nothing signals that the same fault occurred again, and inventing one was out of scope.

  #166 is the same normative requirement in the Event Enrollment evaluator and is **not** fixed here. It shares no code path — that evaluator has no fault branch, reads no `Reliability`, and its algorithm functions return an absolute `EventState` rather than an indication, so "nothing happened" and "a transition to the state I am already in" are the same value. Lifting its guard needs the per-enrollment change baseline from #137 first, which is recorded in a comment at the guard itself.
- Stop the server's `FaultDetector` overwriting a `Reliability` the client owns. `FaultDetector::evaluate` swept every Analog Input, Analog Output and Analog Value, recomputed `Reliability` from `Min_Pres_Value`/`Max_Pres_Value`, and wrote the result without ever consulting `Out_Of_Service` — so a value written to simulate a fault was reset within one evaluation interval. It now skips out-of-service objects entirely: not compared, not written, and not represented in the returned `Vec<ReliabilityChange>`.

  ASHRAE 135-2020 already says when the client owns the property. Clause 12.2(b) and 12.3(b) require that `Reliability` "shall be decoupled from the physical input" (respectively output) while `Out_Of_Service` is TRUE, and 12.2(c) / 12.3(c) that it "shall be writable to allow simulating specific conditions or for testing purposes". Clause 12.4(b) states the writability half for Analog Value, which has no decoupling item because it has no physical point. Re-deriving the value in that state defeats a behavior the standard mandates.

  **This became observable through #167.** Before that change `Reliability` did not drive `Event_State`, so the overwrite was silent; afterwards the 1-second re-derivation saw the reset value and emitted a spurious TO_NORMAL notification. The reach was limited by `enable_fault_detection` defaulting to `false`, so only deployments that opted into fault detection were affected.

  The check is deliberately fail-open — an object that does not report `Out_Of_Service`, or reports it at an unexpected type, is still evaluated — so anything unusual keeps its prior behavior rather than silently losing fault detection. Event-state-detection is deliberately *not* skipped for out-of-service objects: Clause 12.2(d) requires that "other functions that depend on the state of the Present_Value or Reliability properties shall respond to changes made to these properties as if those changes had occurred in the physical input", so a simulated `Reliability` still drives the object to FAULT. Only the overwrite stops.

  Two larger defects in the same evaluator were found while confirming this against the standard and are tracked rather than folded in. The standard nowhere authorizes deriving `OVER_RANGE`/`UNDER_RANGE` from `Min_Pres_Value`/`Max_Pres_Value` — those describe the engineering range, and the mechanism it defines is the FAULT_OUT_OF_RANGE fault algorithm with its own fault limits — and Clause 12.3 gives Analog Output no authorization to apply a fault algorithm at all, though the sweep includes it (#231). And `Reliability_Evaluation_Inhibit`, the standard's own runtime override for stopping reliability-evaluation, is unimplemented on all nine object types (#232); note its semantics are not what the name suggests, since TRUE *forces* `NO_FAULT_DETECTED` rather than freezing the current value.
- Apply the COMMAND_FAILURE event algorithm to Binary Output and Multi-state Output (ASHRAE 135-2020 Clauses 12.7 and 12.19), which both state that objects of that type "that support intrinsic reporting shall apply the COMMAND_FAILURE event algorithm". Both were wired to a `ChangeOfStateDetector`. On Binary Output the doc comment above the field already read "COMMAND_FAILURE event detector" — the intent was recorded and never implemented. `CommandFailureDetector` was fully written and wired to no object type at all; what blocked it was the missing `Feedback_Value` property to bind `pFeedbackValue` (added above) and a macro that structurally passed only two values to a three-input detector.

  **These two object types had no working intrinsic reporting at all, not merely the wrong algorithm.** `ChangeOfStateDetector` fires by comparing `Present_Value` against its own `alarm_values` list, and only two routes in the crate ever populate a detector's alarm parameters: the shared `HIGH_LIMIT`/`LOW_LIMIT` write arms used by the analog types, and `MultiStateInputObject`, which assigns `event_detector.alarm_values` directly. Neither Binary Output nor Multi-state Output had any path to it, so the list stayed `Vec::new()` and no `Present_Value` could ever raise an event. Multi-state Output does carry an object-level `alarm_values` field, but it was only ever read back over the network — it never reached the detector. That is why the swap is a complete fix rather than a partial one: COMMAND_FAILURE compares two live properties and needs no configured value list, so these types can report once enabled. A test pins that the still-readable `Alarm_Values` does not influence detection.

  **Intrinsic reporting on these two types is opt-in.** `Event_Detection_Enable` defaults to FALSE (see the entry below), so no existing object changes behavior on upgrade. That default is deliberate: an adversarial review measured that with detection unconditional, the first command to any Binary Output or Multi-state Output latched OFFNORMAL and IN_ALARM for the rest of uptime, because nothing updates `Feedback_Value` on a device with no physical I/O. Clauses 12.7 and 12.19 make the whole property group conditional — "required if the object supports intrinsic reporting" — so support is optional and always-on was the wrong reading.

  Clause 13.3.4 condition (a) is implemented as printed — "If pCurrentState is NORMAL, and pFeedbackValue is not equal to pMonitoredValue for pTimeDelay, then indicate a transition to the OFFNORMAL event state." Condition (b) as printed reads "If pCurrentState is OFFNORMAL, and pMonitoredValue is equal to pMonitoredValue for pTimeDelayNormal, then indicate a transition to the NORMAL event state", which compares a parameter to itself and is an apparent erratum in the standard; the detector implements the evident intent (`pFeedbackValue` equal to `pMonitoredValue`), and this is recorded here rather than asserted in a code comment as something the standard says. `Present_Value` binds `pMonitoredValue` per both object clauses — it is not the commanded value from the priority array, so the algorithm is not comparing a value against itself.

  Deferred rather than folded in, each tracked separately: the algorithm's separate `pTimeDelayNormal` return-to-normal delay, which no detector in the crate implements (#225 — today's single-delay behavior is conformant for the case where no value is available, since the standard then makes it take the value of `pTimeDelay`); the notification payload parameters (#135); and `Event_Detection_Enable`, required by the same conformance footnote across all nine intrinsic-reporting object types (#216). COMMAND_FAILURE produces only NORMAL and OFFNORMAL; FAULT continues to arrive through the shared reliability path.

- Take the notification's Event Type from the object's event algorithm instead of guessing it from the states involved (ASHRAE 135-2020 Clauses 13.8.1.1 and 13.9.1.1). `EventStateChange::event_type()` returned `OUT_OF_RANGE` when either end of the transition was `HIGH_LIMIT` or `LOW_LIMIT` and `CHANGE_OF_STATE` otherwise — two of the roughly twenty `BACnetEventType` values, derived from `EventState`, which is a different axis entirely. The standard says: "Otherwise, this parameter shall have the value associated with the event-initiating object's configured event algorithm." Clause 13.2.2 adds that "for intrinsic reporting in standard object types, the event algorithm is implied by the object type", and Clause 13.3 that "the event algorithms are indicated by the BACnetEventType value of the same name".

  `event_type()` now takes the algorithm as a **required parameter** and applies only the FAULT rule, returning the algorithm otherwise. Each detector carries its algorithm as a `pub const ALGORITHM` — `OutOfRangeDetector` → `OUT_OF_RANGE`, `ChangeOfStateDetector` → `CHANGE_OF_STATE`, `CommandFailureDetector` → `COMMAND_FAILURE` — and `fire()` computes the final value where both the algorithm and the transition are known. Making it a parameter rather than an inferred default is deliberate: a detector added later cannot silently fall back to a guess.

  **Breaking:** `TransitionOutcome` gains a `pub event_type: EventType` field, and `EventStateChange::event_type()` gains a parameter. The struct is not `#[non_exhaustive]`, so exhaustive struct literals and patterns must account for the new field.

  The FAULT override is retained and is correct unconditionally. Worth recording why, since the sentence quoted in issue #210 reads as though it were conditional on Notify Type: the Event Type definition has a **third** paragraph the issue omitted — "If 'Notify Type' is ACK_NOTIFICATION when 'To State' is FAULT, the Event Type shall be CHANGE_OF_RELIABILITY. When 'To State' is NORMAL, and the device can determine reporting acknowledgement of a transition from FAULT, the Event Type shall be CHANGE_OF_RELIABILITY." So the ACK case is covered explicitly rather than falling through to the algorithm, and Clause 13.2.5.3 states the underlying rule symmetrically for transitions "to, or from" FAULT.

  **The wrong value was previously unreachable only by coincidence**, because each wired detector's state vocabulary happened to map onto its algorithm. `CommandFailureDetector` is the counterexample and now has an explicit regression test: its OFFNORMAL transition reports `COMMAND_FAILURE`, where the old heuristic returned `CHANGE_OF_STATE` since OFFNORMAL is neither `HIGH_LIMIT` nor `LOW_LIMIT`.

  Two related defects were found while confirming this against the standard and are tracked rather than folded in. Binary Output and Multi-state Output apply the CHANGE_OF_STATE algorithm although Clauses 12.7 and 12.19 require COMMAND_FAILURE (#222) — after this change they report `CHANGE_OF_STATE`, which is truthful about the algorithm actually used and still non-conformant until they are rewired. And the Event Enrollment path carries its `Event_Type` onto FAULT transitions with no override, although Clause 12.12 forbids `CHANGE_OF_RELIABILITY` as an `Event_Type` value (#223).

- Route every `Event_State` → transition-bit decision through one classifier, `EventTransition::for_target_state`, and correct a residual that was inverted relative to ASHRAE 135-2020 Clause 13.2. Four sites mapped an `Event_State` to one of the three `Event_Enable` bits using **three different partitions**, and each ended in a catch-all sending the residual to TO_FAULT. Clause 13.2 puts it the other way round: "all states that are not normal and not fault are offnormal states, and transitions that result in an offnormal state are considered to be TO_OFFNORMAL transitions. Transitions to any fault state are considered to be TO_FAULT transitions. All other transitions are, by definition, TO_NORMAL transitions." Clause 13.2.2.1.2 says it outright in a parenthetical: "(Note that the OffNormal state includes all event states other than NORMAL and FAULT)."

  So `LIFE_SAFETY_ALARM` was misrouted at all four sites, `OFFNORMAL` at the OUT_OF_RANGE detector, and `HIGH_LIMIT` / `LOW_LIMIT` at the CHANGE_OF_STATE and COMMAND_FAILURE detectors — each to TO_FAULT rather than TO_OFFNORMAL.

  **The defect was latent, not live.** No detector currently produces the states that were misrouted, so the catch-all only ever saw `FAULT` and was accidentally correct; nothing observable changes today. It becomes live the moment a detector's state vocabulary widens, which is precisely what makes a shared classifier the right fix rather than four corrected copies. Clause 13.2's own wording anticipates that — "any number of possibly unique event states" — so the residual arm is not a theoretical concern.

  The correct partition already existed in `EventStateChange::transition()`; it simply was not what the four sites used. That method now delegates to the new classifier, so there is exactly one definition. Three identical `TO_OFFNORMAL` / `TO_FAULT` / `TO_NORMAL` constant sets (nine constants) and the bare `0x01` / `0x02` / `0x04` literals in the Event Enrollment evaluator are gone in favour of the existing `EventTransition::bit_mask()`.

  Note the transition bit is determined by the **destination state alone**, which is deliberately different from Clause 13.2.5.3's Event Type rule — `CHANGE_OF_RELIABILITY` applies to transitions *to or from* FAULT. `EventStateChange::event_type()` still reads both ends and is untouched; conflating the two rules would silently undo #168.

- Drive `Event_State` from `Reliability`, making the FAULT state and the TO_FAULT transition reachable for the first time (ASHRAE 135-2020 Clause 13.2.2). Nothing in the stack converted a `Reliability` other than `NO_FAULT_DETECTED` into `Event_State = FAULT`: the server's `FaultDetector` wrote the property and the periodic task consumed its `ReliabilityChange` records in a `debug!` and nothing else, while a repo-wide search for a producer of `EventState::FAULT` outside tests found none. Two consequences went with it — no TO_FAULT notification could ever be generated, so `Event_Time_Stamps[2]` and the TO_FAULT bit of `Acked_Transitions` could not move; and a faulted object reported `Status_Flags` FAULT TRUE with IN_ALARM FALSE, since Clause 12.2 derives IN_ALARM from `Event_State` and FAULT from `Reliability` and the standard reconciles them by requiring `Reliability` to drive `Event_State` upstream.

  **FAULT is modeled as a standing condition, not a latched state.** Clause 13.2.2 puts it in exactly that form — "when Reliability has a value other than NO_FAULT_DETECTED, the event-state-detection process will determine the object's event state to be FAULT" — and Clause 13.2.2.1 states the invariant directly: "In the Fault state reliability-evaluation indicates a value other than NO_FAULT_DETECTED." So the FAULT determination is re-derived from the object's current `Reliability` on every evaluation and is never latched. (The #217 entry above later added a stored `fault_reliability` — that records the value at the last entry to FAULT so a *transition* can be edge-detected, and does not latch the determination itself.) That also means a `Reliability` set by *any* route reaches detection: the server's fault detector, a local write, or a network write, with no route needing to notify anything. `enable_fault_detection` (still `false` by default) therefore governs only whether `Reliability` is *derived* from limits, never whether an existing `Reliability` is honored.

  **Reachable today on the three analog types only.** The detection wiring is applied uniformly to all nine intrinsically-reporting object types, but Binary and Multi-state objects expose no route that can set `Reliability` — no `RELIABILITY` write arm, no setter, and the server's `FaultDetector` iterates only the analog types — so their fault path is correct and inert. That is a pre-existing gap this change exposes rather than introduces, and it is tracked as #218. (The open question posed here — whether the standard makes `Reliability` writable only while `Out_Of_Service` is TRUE — was since settled: Clause 12.1.2 permits an O property to be writable at the implementor's option, and `Out_Of_Service` TRUE makes it *required* to be writable rather than being the only condition under which it is permitted.) Detection is also suspended while DeviceCommunicationControl is active, even though confirmed writes still execute (#220).

  **Recovery from FAULT enters NORMAL, not a state re-derived from the event algorithm.** Clause 13.2.2.1's Fault ToNormal transition reads "If reliability-evaluation indicates a value of NO_FAULT_DETECTED, then perform the corresponding transition actions and enter the Normal state." This is worth stating plainly because the obvious implementation — treat FAULT as an overlay and fall back to the algorithm when it clears — is wrong: with a present value still out of range it produces a `FAULT -> HIGH_LIMIT` transition the state machine does not define. The algorithm gets to move the object out of NORMAL afterwards, under its own conditions and its own `Time_Delay`, so an out-of-range value is re-detected rather than swallowed.

  **Entry to FAULT ignores `Time_Delay`.** Clause 13.2.2.1's ToFault transitions are unconditional and carry no delay term; `Time_Delay` is an event-algorithm parameter (Clause 13.3.1 defines pTimeDelay as the time "that the offnormal conditions must exist before an offnormal event state is indicated"), and the algorithm is precisely what fault detection takes precedence over. An in-flight countdown is discarded on entry to FAULT. **That last point is a project decision, not a citation** — Clauses 13.2.2, 13.2.2.1 and 13.3 are all silent on the pending timer's fate. Cancelling was chosen because recovery re-enters NORMAL, so a countdown seeded before the fault targets a transition out of a state the machine no longer occupies; the nearest analogous rule agrees in shape (Clause 13.2.2.1.5 restarts the *full* delay when `Event_Algorithm_Inhibit` clears rather than resuming a partial one) but is scoped to inhibition, not faults.

  **Breaking API change.** `OutOfRangeDetector`, `ChangeOfStateDetector` and `CommandFailureDetector` each take a trailing `reliability: u32` on `evaluate`, `probe` and `tick`, and the exported `impl_intrinsic_reporting!` macro takes a third argument naming the object's reliability field. Callers pass `Reliability::NO_FAULT_DETECTED.to_raw()` to preserve prior behavior. The rule itself lives once, in a shared `fault_precedence` function that every detector consults, so a detector added later that omits it is a visible omission rather than a silently missing clause.

  Also fixes #200 in the same change: `CommandFailureDetector::fire` hardcoded `distribute: false` for FAULT, ignoring the TO_FAULT `Event_Enable` bit (its `TO_FAULT` constant carried `#[allow(dead_code)]` for exactly that reason). Clause 13.2.5 scopes `Event_Enable` to distribution uniformly across all three transition directions, so there is no basis for treating TO_FAULT differently. The defect was unobservable while FAULT was unreachable and becomes live the moment it is not, which is why it is corrected here rather than deferred.

  Not addressed *by this change*: Clause 13.2.2.1 also defines a FAULT **re-entry** — a transition generated when reliability-evaluation reports a *different* non-NO_FAULT_DETECTED value while already in the Fault state. It was implemented later in this same unreleased block; see the #217 entry above. The reason given here for deferring it — that it depended on the same-state handling tracked in #166 — turned out to be wrong: #166 is the Event Enrollment evaluator, which shares no code path with the intrinsic detectors and is itself blocked on #137.

- Report FAULT transitions with Event Type `CHANGE_OF_RELIABILITY` (ASHRAE 135-2020 Clause 13.2.5.3). `EventStateChange::event_type()` never considered `EventState::FAULT`: it returned `OUT_OF_RANGE` when either end of the transition was `HIGH_LIMIT` or `LOW_LIMIT` and `CHANGE_OF_STATE` otherwise, and that value goes straight onto the wire as the notification's Event Type. Clause 13.2.5.3 (Fault Event Notifications) opens with an unconditional rule that no other clause carves an exception out of — "For all transitions to, or from, the FAULT state, the corresponding event notification shall use the Event Type CHANGE_OF_RELIABILITY" — and Table 13-3 states the same rule in exactly the shape the code now takes: "When 'To State' or 'From State' is FAULT, set to CHANGE_OF_RELIABILITY". The Event Type parameter of ConfirmedEventNotification (Clause 13.8) and UnconfirmedEventNotification (Clause 13.9) states the two directions separately, adding "The Event Type CHANGE_OF_RELIABILITY shall be used for reporting a transition from FAULT" after the to-FAULT rule. The FAULT test is evaluated **before** the limit-state test, which matters: a `HIGH_LIMIT -> FAULT` transition has `HIGH_LIMIT` at one end, so appending the new rule after the existing branch instead of before it would report a fault as `OUT_OF_RANGE`. That ordering has its own regression test, and it is the only test that fails under the mis-ordered variant.

  Latent until FAULT becomes reachable — no production path currently produces `EventState::FAULT` (#167) — but fixed now rather than later because #135 will populate `event_values`, and the Event Type selects which `BACnetNotificationParameters` alternative a receiver decodes context tag [12] against. The remainder of Clause 13.2.5.3 is exactly the specification #135 has to satisfy — Table 13-4 names the `reliability`, `Status-flags` and `Property-values` parameters of such a notification, Table 13-5 lists the properties each object type must convey "in the order shown in the table", and Event Enrollment gets what the clause calls "the only case where the properties conveyed in the CHANGE_OF_RELIABILITY are not from the event-initiating object" — the first entry is the enrollment's own `Object_Property_Reference`, the second is whatever that reference points at, and every remaining property comes from the monitored object. A wrong Event Type would then corrupt the parameters as well as misroute the notification. Directly testable today regardless, since `event_type()` is a pure function of a public type, so this ships with real coverage rather than an untestable branch: five to/from-FAULT combinations, four ordering cases against the limit states, and four non-FAULT cases proving the rule is additive rather than a rewrite. The existing `NORMAL -> FAULT` server test asserted only `priority` and `ack_required`; it now asserts the Event Type on the decoded wire bytes, and a `FAULT -> NORMAL` test covers the other direction — that one also pins that the transition coordinate stays TO_NORMAL for Priority/Ack_Required purposes while the Event Type is `CHANGE_OF_RELIABILITY`, which a fix keyed off the transition category rather than the states would get wrong.

  Two adjacent parts of the Clause 13.8/13.9 Event Type parameter are deliberately **not** addressed. The "otherwise" half — "this parameter shall have the value associated with the event-initiating object's configured event algorithm" — is still guessed from the states involved rather than taken from the algorithm; that guess is right for every object type wired today only by coincidence, and is tracked as #210. The ACK_NOTIFICATION cases in the same paragraph are unreachable because `AcknowledgeAlarm` does not yet issue the notification at all (#175).

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
- Give Event Enrollment evaluation its own lifecycle configuration instead of sharing the fault-detection switch (ASHRAE 135-2020 Clause 12.12). The periodic Event Enrollment evaluation task was gated on `ServerConfig::enable_fault_detection`, a setting documented solely as analog reliability evaluation ("evaluates analog objects every 10 s for OVER_RANGE / UNDER_RANGE faults"), so two unrelated subsystems shared one switch and neither could be configured on its own: a device that wanted enrollment evaluation was forced to also run reliability evaluation, and a device that wanted only reliability evaluation silently got enrollment evaluation as well. Evaluation is now controlled by `enable_event_enrollment` (default `true`, matching the unconditional `trend_log`, `schedule_tick`, and `intrinsic_reporting` tasks — an evaluation pass is a no-op on a database holding no Event Enrollment objects), with its cadence set by `event_enrollment_interval_secs` (default 10). A configured interval of `0` is clamped to one second with a warning rather than reaching `tokio::time::interval`, which panics on a zero period — and would panic inside a spawned task, leaving `start` returning `Ok` while enrollment evaluation was silently dead; use `enable_event_enrollment(false)` to actually disable evaluation. Existing configurations calling `enable_fault_detection(true)` keep enrollment evaluation. Startup and shutdown are now documented on the property itself: the task is spawned by `start` with its first pass running immediately and once per interval thereafter (`tokio::time::interval` completes its first tick at once), and `stop` aborts it and awaits the abort. The interval is documented as a sampling cadence with no basis in the standard — which prescribes no evaluation frequency and makes acquisition of a monitored value a local matter (Clause 12.12) — and as distinct from an event algorithm's `Time_Delay` (Clause 13.3). `enable_event_enrollment` is likewise documented as a switch over the evaluation task rather than the per-object `Event_Detection_Enable` property of Clause 13.2.2.1: setting it false stops evaluation without performing the reset that clause requires of a disabled detector, so a device carrying active enrollments holds its last detected state. Both regression tests are proven to fail under the prior shared gate. A paused-clock test drives the real spawned task and is proven to fail under both regressions it guards — deferring the first pass by one interval, and hardcoding the period so the config field is ignored. The evaluation task also adopts `MissedTickBehavior::Delay`, matching the intrinsic-reporting task, so a stalled runtime resumes with one pass rather than a burst of catch-up passes. **Note what enabling this by default does and does not do:** a detected transition updates `Event_State` and is logged, but routing transitions into the notification pipeline (#127) and the accompanying `Acked_Transitions` / `Event_Time_Stamps` updates (#123) are not implemented, so a device holding active enrollments changes `Event_State` without emitting an EventNotification and a client learns of the alarm only by polling. That was previously reachable only by opting in via `enable_fault_detection(true)`; it is now the default.

- Make the release gate wait on every Tier 1 CI job. `validate` — which every publishing job descends from, directly (`publish-crates`, `build-wheels`, `build-sdist`, `build-cli-binaries`) or transitively (`publish-pypi`, `github-release`) — listed 7 of the 9 Tier 1 jobs in `needs`, omitting `file-size-cap` and `msrv`. Both run on a `v*` tag (`file-size-cap` via its `startsWith(github.ref, 'refs/tags/v')` guard, `msrv` because a tag push is `event_name == 'push'`), so both could fail while the release published anyway. The MSRV case is the damaging one: a tag whose code needs a compiler newer than 1.93 fails `msrv` but still publishes crates declaring `rust-version = "1.93"`, and a published crates.io version can only be yanked, never replaced. `needs` now lists all nine in declaration order so it can be audited against the job list at a glance.

- Scope `Event_Enable` to notification distribution instead of letting it suppress the transition itself (ASHRAE 135-2020 Clauses 12.12, 13.2.2.1.4 and 13.2.5). Clause 13.2.2.1.4 ("Transition Actions") lists four actions a detected transition performs — store the new `Event_State`, store the time in `Event_Time_Stamps`, store the message text in `Event_Message_Texts` if present, and indicate the transition to *both* the alarm-acknowledgment process (Clause 13.2.3) and the event-notification-distribution process (Clause 13.2.5) — and states that they "shall be executed even if the transition does not change the event state." **None of the four is scoped to `Event_Enable`.** The property acts strictly downstream: Clause 12.12 defines it as three flags that "separately enable and disable the distribution of TO_OFFNORMAL, TO_FAULT, and TO_NORMAL notifications," and Clause 13.2.5 places the gate inside the distribution process, where it disables *external* distribution only (whether local objects such as Event Logs are affected is explicitly a local matter). `Acked_Transitions`, in particular, is not a transition action at all: Clause 13.2.3 has the alarm-acknowledgment process maintain it from the unconditionally-indicated transition, gated by `Ack_Required` and never by `Event_Enable`. The two *reachable* reporting families conflated detection with distribution, in different ways and to different effect. A third path, the published-but-uncalled `intrinsic_reporting::IntrinsicReportingEngine`, carried the same defect and was deliberately **not** changed here — its disposition (delete the duplicate, or fix it) was a semver decision tracked as #204, settled later in this same block by deleting the module (see *Removed*).

  The Event Enrollment evaluator was the damaging case: it wrapped the entire state-persisting update in `if transition_enabled`, so a suppressed transition never reached `set_event_state_internal`. With `Event_Enable` cleared for TO_OFFNORMAL, an object that went out of range left `Event_State` at NORMAL — and when the value later came back into range the evaluator compared NORMAL against NORMAL, saw no change, and dropped the *enabled* TO_NORMAL notification as well. Clearing one bit silently disabled a different one, and `Event_State` — a readable property, not a notification — reported a condition the device was no longer in.

  The intrinsic detectors (`OutOfRangeDetector`, `ChangeOfStateDetector`, `CommandFailureDetector`) were structurally rather than observably wrong: `fire` already assigned `self.event_state` before evaluating `Event_Enable`, so the state was correct, but it then returned `enabled.then_some(change)` — collapsing "no transition occurred" and "a transition occurred and may not be distributed" into the same `None`. A caller therefore could not learn that a suppressed transition had happened — which matters because Clause 13.2.2.1.4's fourth action indicates the transition to *two* processes, not one. Withholding it denies the alarm-acknowledgment process (Clause 13.2.3) as well as distribution — and that process is what maintains `Acked_Transitions`. The `Event_Time_Stamps` and `Event_Message_Texts` writes are separate, independent transition actions rather than consequences of it; all three remain unimplemented and are tracked as #123.

  Detection and distribution are now separate results. `fire` and the enrollment evaluator always report the transition and always persist `Event_State`, carrying the `Event_Enable` verdict alongside it as `TransitionOutcome::distribute` / `EventEnrollmentTransition::distribute`; the two notification senders (the per-write `fire_event_notifications` path and the 1-second `intrinsic_reporting_task`) apply that flag at the point of send, which is where #123's bookkeeping will hook in ahead of the check. The notification bytes are unchanged and no notification that was previously sent is now suppressed; the observable changes are confined to Event Enrollment, where `Event_State` now tracks the condition regardless of `Event_Enable`, and an enabled transition following a suppressed one is detected and surfaced as eligible for distribution rather than being lost outright. Note "eligible": the enrollment path still emits no notification for it (#127), so what the fix recovers there is the transition and the correct state, not yet a delivered notification.

  Two known non-conformances in this area are deliberately left in place, because fixing them here would be speculative: the enrollment evaluator's same-state skip still discards transitions Clause 13.2.2.1.4 requires to be acted on, since removing it needs a change baseline to distinguish a genuine same-state indication from "nothing changed" (#166, which depends on #137); and the enrollment path does not yet emit notifications at all (#127), so its `distribute` flag is logged rather than acted on. Separately, `CommandFailureDetector::fire` hardcodes `distribute = false` for FAULT where its two siblings consult the `TO_FAULT` bit — an arm unreachable today because that detector never computes FAULT, filed as #200 to be fixed alongside #167, the change that makes it reachable and therefore testable. All seven regression tests (four on the detectors, three on the enrollment evaluator) are proven to fail under a mutation that restores the previous gate.

### Changed

- Move the `bacnet-cli` clap surface — `Cli` and `Command` — out of `main.rs` into a sibling `args` module. `main.rs` measured exactly 700 of the 700 non-empty, non-comment lines the file-size cap allows, so any change to the CLI failed CI unless it split the file in the same commit. The first outside contribution to touch it (#213) hit precisely that, which is what prompted the split. Dispatch stays in `main.rs`, now 511 lines; the flags and subcommands — the part that actually grows when the CLI gains a feature — land in `args.rs` at 193. The `Cli` fields move from private to `pub(crate)` because `main.rs` now reads them across a module boundary; `bacnet-cli` is a binary, so no public API changes. The generated CLI surface is untouched, verified by diffing `--help` output for the root command and all 25 subcommands against `dev` (38 KB, byte-for-byte identical).

- Scope the MSRV gate to the publishable crates and their optional features, and take `sysinfo` 0.38 → 0.39. **The MSRV is unchanged at 1.93.** The `msrv` job ran `cargo check --workspace --exclude rusty-bacnet`, which also covered `bacnet-benchmarks` and `bacnet-integration-tests`. Neither is published, so neither is something a crates.io consumer can depend on — yet a dependency of the benchmarking crate could dictate the MSRV promised to consumers, which is what held the workspace at `sysinfo` 0.38 (0.39 declares `rust-version = "1.95"`). The gate now runs `.github/scripts/check-msrv.sh`, which derives the publishable set from `cargo metadata` instead of hardcoding it: a written-down list drifts in the dangerous direction silently, because a newly published member nobody remembers to add is simply never checked. The script also enables `sc-tls` and `ipv6` explicitly. That matters more than it looks — under resolver v2 the old `--workspace` run picked those features up incidentally, because `bacnet-benchmarks` depends on `bacnet-transport`, `bacnet-client` and `bacnet-server` with them turned on, so narrowing the package set without naming the features would have quietly stopped checking the whole BACnet/SC and IPv6 module surface, along with the MSRVs of `rustls`, `tokio-rustls` and `tokio-tungstenite`. `serial`, `serial-gpio`, `ethernet` and `pcap` are still unchecked at MSRV; they need Linux system packages this job does not install, and extending the gate to them is tracked as #196. Note also that the resolver is `"2"` and therefore not MSRV-aware: `sysinfo` 0.39.6 resolves cleanly under `rust-version = "1.93"` and cargo rejects it at build time instead (`rustc 1.93.1 is not supported by the following package`), which is why this presented as a CI failure rather than a resolution one. Verified on 1.93.1 — the gate exits 0, while the previous command exits 101 on `sysinfo` — and `bacnet-benchmarks` builds on the pinned 1.97.1 toolchain.

- Split CI into a lean and a heavy tier so feature→dev PRs run only the fast gate (`rustfmt`, `clippy`, file-size cap, no-secret scan, and the Linux test matrix) while the cross-OS test matrix (macOS, Windows), MSRV, `cargo audit`, and `cargo deny` run on dev→main PRs, pushes to `main`, and `v*` tags. The feature→dev loop is no longer held up by macOS/Windows runners or the advisory/license/MSRV jobs; the full matrix still gates every merge into `main` and every release (the `validate` job waits on `test-cross` as well as `test`).
- The Python `BACnetServer.write_property_local` now raises `BacnetProtocolError` (with `error_class`/`error_code`) for `UNKNOWN_OBJECT` and `DUPLICATE_NAME` instead of a generic `RuntimeError`, giving it structured parity with the network `WriteProperty` path. Python callers that caught `RuntimeError` specifically should catch `BacnetProtocolError` (or `Exception`) instead. The server lock is also held across the whole call (including post-write COV/event sends), so concurrent Python calls on the same `BACnetServer` serialize behind a local write; a confirmed-COV send to an unresponsive subscriber can stall other Python calls for up to the COV retry timeout. This prevents a `stop()` from racing the notification sends mid-flight.

- Bring every dependency current. `cargo update` moves 94 in-range packages, including tokio 1.52.3 → 1.53.1, bytes 1.11.1 → 1.12.1, rustls 0.23.40 → 0.23.42, serde 1.0.228 → 1.0.229, serde_json 1.0.149 → 1.0.151, clap 4.6.1 → 4.6.4, libc 0.2.186 → 0.2.189, aws-lc-rs 1.16.3 → 1.17.3, socket2 0.6.3 → 0.6.5, getrandom 0.4.2 → 0.4.3, and thiserror 2.0.18 → 2.0.19. One semver-major bump follows, and it needed no source change: `rustyline` 17 → 18 in `bacnet-cli`. `sysinfo` moves separately to 0.39 — see *Changed*. The lock shrinks from 362 entries to 330, largely because the `wit-bindgen`/`wit-component`/`wasm-metadata` build chain leaves, which also clears the single allowed `cargo audit` warning this workspace had been carrying; `cargo audit` now reports no findings at all. Duplicate-version warnings from `cargo deny` drop from 10 to 3. All ten on `dev` were Windows-family crates — `windows-sys` plus `windows-targets` and eight `windows_*` target shims — and every one is resolved: locked `windows-sys` goes from five versions to three, `windows_x86_64_msvc` from three to two, `windows-targets` from two to one, and `fd-lock` leaves entirely with rustyline 17. Of the three that remain, `syn` is new and not ours to resolve — `serde_derive`, `clap_derive`, and `thiserror-impl` have moved to `syn` 3 while `tokio-macros`, `tracing-attributes`, `futures-macro`, and `zerocopy-derive` still require `syn` 2, so both majors stay locked until those crates migrate. `r-efi` is newly reported with an unchanged locked version set. `cargo deny` reports advisories, bans, licenses, and sources all ok, and MSRV 1.93 still builds.
- `sysinfo` is deliberately held at 0.38.4. Version 0.39 requires rustc 1.95, which would break the 1.93 MSRV that the published crates promise: `RUSTUP_TOOLCHAIN=1.93 cargo check --workspace --exclude rusty-bacnet --locked` exits 101 against it. Holding it costs nothing measurable — the duplicate-warning count and every other gate are identical either way. **Superseded within this release**: the premise was wrong. `sysinfo` is used only by `bacnet-benchmarks`, which is not published, so it never constrained the MSRV the published crates promise — the gate was simply checking a crate it had no reason to. See the `sysinfo` 0.39 entry under *Changed*.
- Bump the pinned development and CI toolchain from 1.95.0 to 1.97.1 — the `rust-toolchain.toml` channel and the six `toolchain:` values in `.github/workflows/ci.yml`. **MSRV is unchanged at 1.93.** The workspace `rust-version`, the `MSRV (1.93)` CI job, and the `rust:1.93-alpine` Docker example are untouched, so the published crates still build on 1.93; `cargo check --workspace` under 1.93 was verified against this change. 1.97.1 surfaced no new deny-level lint, rustfmt requested no reformatting, and the suite is unchanged at 2,227 tests across 34 suites.

### Removed

- Remove the public `bacnet_server::intrinsic_reporting` module —
  `IntrinsicReportingEngine` and `IntrinsicEvent` (**breaking**: public API
  removal). The engine was a second, uncalled intrinsic-reporting evaluator
  with different conformance behavior than the production path: it had no
  production caller (only its own tests exercised it), it evaluated event
  algorithms without consulting `Event_Detection_Enable` — which Clause
  13.2.2.1 requires to suppress the state machine entirely when FALSE — and
  it keyed CHANGE_OF_STATE off the plural `Alarm_Values` identifier, a list
  that was always empty and that Binary Input/Value no longer serve at all
  since the singular-`Alarm_Value` correction earlier in this block. Its
  deleted tests lose no live coverage: every assertion either maps to the
  detector suites in `bacnet-objects` or exercised algorithms
  (FLOATING_LIMIT, CHANGE_OF_BITSTRING, CHANGE_OF_VALUE) for which no
  intrinsic detector exists — those run on the separate Event Enrollment and
  COV paths, which have their own suites. Downstream consumers should drive
  objects through the long-standing `BACnetObject` trait path,
  `evaluate_intrinsic_reporting` and `tick_intrinsic_reporting`, gated on
  `Event_Detection_Enable` since earlier in this block — the route the
  production server uses. (#237, #204)
- Remove the non-standard `Alarm_Values`/`Fault_Values` surface from
  Multi-state Output and the unimplemented `Fault_Values` surface from
  Multi-state Input/Value, including the public
  `MultiStateInputObject::set_fault_values` method (**breaking**: public API
  removal). Multi-state Output's property table (Table 12-22) defines
  neither property — its COMMAND_FAILURE algorithm compares Present_Value
  against Feedback_Value — so both arms were invented surface. On
  Multi-state Input/Value, `Fault_Values` is optional (its only conformance
  footnote requires `Reliability` be present alongside it), and it
  parameterizes the Clause 13.4 FAULT_STATE fault algorithm this codebase
  does not implement; omitting an optional property whose parameter nothing
  consumes keeps the advertised surface honest. This deliberately reverses
  the #222-era decision to keep Multi-state Output's inert readback. (#228)
- Remove `bacnet_types::enums::LiftCarDoorStatus`. **This is a breaking removal
  of a public type** — it was glob-exported from `enums`, though nothing in the
  workspace referenced it. ASHRAE 135-2020 defines no lift-car-door-status
  enumeration; the Lift object's `Car_Door_Status` property (Clause 12.59) is
  `BACnetARRAY[N] of BACnetDoorStatus`, whose Clause 21 production `DoorStatus`
  already models value-for-value, and the property resolver already routes
  car-door-status (450) through `DoorStatus`. The removed type assigned
  incompatible numbers to eight of the production's ten named values (its
  `CLOSED` was 3; the standard's closed is 0) and did not name safety-locked
  (8) or limited-opened (9) at all — which is why it is removed rather than
  aliased. (#245)
- Remove the three-ident ungated arm of the exported `impl_intrinsic_reporting!` macro. **This is a breaking change to a `#[macro_export]` macro.** All seven in-tree callers moved to a new four-ident gated arm `($detector, $present_value, $reliability, $event_detection_enable)` as part of adding `Event_Detection_Enable` to those types, leaving the ungated form with no callers. It is removed rather than retained for the reason its sibling comment already gave for the four-ident feedback arm: exporting an ungated form offers downstream implementors a supported way to wire a detector with event detection permanently on, which is the exact defect the gate exists to prevent — Clause 12's conformance footnotes make intrinsic reporting optional per object, and an ungated detector makes it mandatory. Downstream implementors using the three-ident form must add an `Event_Detection_Enable` field and pass it as the fourth argument.
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
