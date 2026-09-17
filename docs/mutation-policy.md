# Local mutation authorization

Rust operators can set `ServerConfig::mutation_policy` or call
`.mutation_policy(bacnet_server::mutation::MutationPolicy::DenyAll)` on the generic,
B/IP or SC server builder. The default, `Permissive`, preserves existing behavior:
an absent authorizer allows; an installed authorizer must approve. `DenyAll` denies
covered decisions even with an allow-all authorizer, without invoking it, using
SERVICES / SERVICE_REQUEST_DENIED. Both modes accept any authorizer configuration.

**SC mTLS channel/peer authentication is not service authorization.** Identities at
this layer are claimed link/routed addresses, never certificate principals.
Distinguishing SC certificate principals is out of scope: none reaches this layer.

## Authorization context

Each decision receives a `mutation::MutationAuthorizationContext`:

- `source_mac` / `source_network` — claimed immediate peer and routed origin.
  Never authenticated identities, never certificate principals.
- `provenance` — the reassembled ingress snapshot (`TransportProvenance`)
  threaded from dispatch through `handle_admitted_confirmed_request[_with_lso]`
  into `mutations::Request`. Cross-segment provenance mismatches already fail
  closed at reassembly, so every element of one request (including each WPM
  element) observes the same snapshot.
- `trust` — `mutation::MutationTrust` derived from the snapshot, mirroring
  RB-09 `ControlTrust`: `Unverified`, `VerifiedChannel` (direct SC-TLS peer),
  or `VerifiedRelay` (SC-hub relayed origin). Channel/relay scope only, never
  leaf identity: a verified ingress asserts the channel/relay validation, not
  that a claimed SNET/SADR leaf is the authenticated peer.
- `invoke_id`, `service_choice`, `target` — the confirmed identity and the
  decoded mutation (current element for WPM).

`Debug` for the context is redacted by construction: address lengths, the
provenance/trust labels, and the target kind only — never MAC bytes, property
values, file payloads, or other decoded inputs.

## Baseline-only profile

There is no verified-leaf authentication beyond the baseline (that is the
approved-relay follow-up; Refs #524 stays open):

- An unknown origin — including a hub-mediated unknown leaf, which arrives
  `Unverified` — never satisfies a baseline-only allow rule. The gate delivers
  the snapshot; the operator's callback owns the rule.
- Receive-permission is not write-permission: allowing one covered service
  (for example a COV subscription) never implies allowing another (for
  example a property write). Each covered decision needs its own allow.
- No cert-bound leaf identity exists at this layer: the SC VMAC is
  payload-claimed inside the TLS channel, not bound to the operational
  certificate.

## Timing and side effects

Order per request is validation, then authorization, then mutation, with no
audit-log write on deny: a denial performs no database mutation, no COV/event
fan-out, and no audit-log write (that write would itself be a mutation). It is
recorded in the saturating per-service decision counters and bounded tracing
diagnostics only. The callback runs after DCC prechecks, request admission,
service decoding, and per-element validation, and before the database write;
WPM invokes it per validated element in wire order while holding the database
write lock, so callbacks must stay fast, nonblocking, reentry-free, and
side-effect-free, and may run concurrently. A panicking callback denies
fail-closed. Neither the allow nor the deny path generates an audit record for
the decision itself; audit records arrive only as explicitly authorized
AuditNotification service receptions.

Coverage is the ten `mutation::MutationTarget` services. Reads, discovery, DCC,
TimeSync, LifeSafety/Audit, direct handler calls and trusted local writes retain
their existing behavior. Admission, duplicate detection, decoding and WPM validation
retain precedence. WPM makes one decision per reached element; empty requests make
none, and a denial stops the suffix without rolling back an allowed prefix.
WPM denials keep the `first_failed` error shape; other denials use
SERVICES / SERVICE_REQUEST_DENIED.

## Exclusions

- Direct `pub handle_*` calls and trusted local writes (`write_local`,
  `set_present_value_local`, life-safety arming) stay ungated by design — the
  same documented bypass contract as the RB-09 precedent. No raw
  server-receive path skips `mutations::Request`.
- The shared endpoint adapter stays read-only: mutation choices are rejected
  through the existing responder (`UNRECOGNIZED_SERVICE`, or
  `SEGMENTATION_NOT_SUPPORTED` for segmented traffic) with no second policy
  gate and no mutation extension.
- LifeSafetyOperation keeps its own authorizer without provenance; the same
  unknown-origin gap exists there for consistency and is out of scope.
- The Python surface exposes no mutation policy knobs: `write_property_local`
  is trusted-local by design, the same contract as the Rust `write_local`.
  Python-hosted servers therefore run the default permissive behavior for
  network mutations.

`BACnetServer::mutation_decision_counters()` exposes fixed per-service saturating
`u64` totals, including after `stop()`: `allow_total`, `deny_total` (all denials),
and `policy_deny_total` (the deny-all subset). Samples are independent, not atomic
aggregates. They count decisions, not successful mutations or delivered responses;
they never affect authorization and retain no per-source state or durable history.
