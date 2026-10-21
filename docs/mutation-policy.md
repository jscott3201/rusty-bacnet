# Server authorization policy

The server asks one object, its `bacnet_server::ServerPolicy`, every "may this
peer do this?" question it puts to an application (#524). Install it with
`.policy(..)` on the generic, B/IP or SC server builder, or in
`ServerConfig::policy`. Each method has a default, so a policy overrides only what
it decides:

| Method | Decides | Default |
|---|---|---|
| `authorize_mutation` | the ten covered confirmed services, and each Channel write of an inbound WriteGroup | allow |
| `authorize_life_safety_operation` | LifeSafetyOperation | deny |
| `authorize_audit_notification` | storing a ConfirmedAuditNotification in the configured sink | deny |
| `authorize_unconfirmed_audit_notification` | storing an UnconfirmedAuditNotification there | deny |

With no policy installed (`ServerConfig::policy` is `None`) the server decides as
those defaults do, except that it allows a covered mutation without decoding it
for authorization first. A policy whose `decides_mutations()` returns `false`
takes that same path, so a policy that only decides LifeSafetyOperation or Audit
receipt leaves mutations exactly as they were. Returning `false`, or panicking,
denies, and the request changes nothing. `bacnet_server::FnPolicy` builds a
policy from closures, one per decision; a decision with no closure keeps its
default, and with no mutation closure it doesn't decide mutations at all.
`Arc<P>` and `Box<P>` are policies when `P` is.

```rust
use bacnet_server::{server::BACnetServer, FnPolicy};

let builder = BACnetServer::bip_builder().policy(
    FnPolicy::new()
        .mutation(|context| context.principal().is_some())
        .life_safety_operation(|_| true),
);
```

Outside the policy, and why:

- `MutationPolicy::DenyAll` is a static hardening switch. It denies every covered
  mutation without asking the policy, and its denials are counted apart.
- DeviceCommunicationControl's password, source restriction and rate limit are
  static settings ([DCC policy](dcc-policy.md)).
- `on_reinitialize` carries out a ReinitializeDevice after the server's own checks;
  it acts rather than decides, and stays its own handler.
- `on_time_sync` observes a clock change it can't refuse.
- Routing-control messages belong to the router's `ControlAuthorizer`
  (`bacnet-network`), not the server.

**Pre-1.0 API change (#524):** `ServerConfig::policy` replaces
`mutation_authorizer`, `life_safety_operation_authorizer`,
`audit_notification_authorizer` and `unconfirmed_audit_notification_authorizer`,
and the builders' `.policy(..)` replaces their four setters. Each `.policy(..)`
call replaces the policy before it, so put every decision in one policy, for
example one `FnPolicy` chain. The `MutationAuthorizer`,
`LifeSafetyOperationAuthorizer`, `AuditNotificationAuthorizer` and
`UnconfirmedAuditNotificationAuthorizer` aliases are gone; the shared endpoint's
Device-write callback type is now `mutation::DeviceWriteAuthorizer`. Decision
counters behave as before: only a policy that decides mutations makes the server
decode a mutation for authorization first, as an installed mutation authorizer
did.

## Mutation policy modes

`ServerConfig::mutation_policy`, or `.mutation_policy(MutationPolicy::DenyAll)` on
a builder, selects the mode. The native default is `Permissive`: with no policy
every covered mutation is allowed, and an installed policy must approve each one.
`DenyAll` denies covered decisions even with an allow-all policy, without asking
it, using SERVICES / SERVICE_REQUEST_DENIED.

**Inbound WriteGroup is decided per Channel write (#1319).** A WriteGroup can
write several Channels, so policy judges each Channel write on its own, just before
it is made: `DenyAll` denies it, and an installed policy can allow some Channels
and not others. A denied write is skipped silently, since nothing answers an
unconfirmed request, makes no Audit record, and is counted in the `write_group` row of
`mutation_decision_counters()`.

**Check an existing policy for WriteGroup.** Before #1319 an installed authorizer
never saw a WriteGroup, which was dropped whatever it returned. A policy that allows
what it doesn't recognize, through `matches!`, `if let` or a `_ => true` arm, now
allows WriteGroup Channel writes. `MutationTarget` isn't `#[non_exhaustive]`, but
such code still compiles, so handle `MutationTarget::WriteGroup` explicitly.

## Principals

**SC mutual TLS is authentication, not authorization.** Any peer a trusted CA
admits may use every service the server's defaults allow, until a policy says
otherwise.

Every decision context, and `ReinitializeContext`, has `principal()`: an
`Option<ScPrincipal>` (`bacnet_server::policy::ScPrincipal`). It names the peer
that **originated** the request, and is `Some` only when the request arrived over
a verified **direct** BACnet/SC connection, accepted or built-in outbound, with no
routed SNET/SADR source. It holds the SHA-256 of the exact leaf certificate DER that TLS
verified, the digest hub certificate bindings pin. It has no connection
incarnation, so a reconnect with the same certificate keeps its principal, and a
renewed certificate is a new one; pin both while it rolls out. `Display` writes
the 64 hex digits; `Debug` shows only the first eight hex digits, and the
contexts' `Debug` says only whether a principal is present.

It is `None` for:

- B/IP, B/IPv6, MS/TP and Ethernet requests;
- requests a BACnet/SC hub relays. The hub authenticated its own TLS client, and
  the origin's certificate never reaches this node, so a policy can't tell one
  hub member from another. A hub's certificate bindings (#800) constrain which
  leaf may register which VMAC and UUID at that hub; they don't forward the leaf;
- the hub admission callback's own channel;
- a routed request, one carrying an SNET/SADR source, even over a direct
  connection: that connection authenticates the router that forwarded it, not the
  device behind it.

So `principal()` means the direct SC peer that sent the NPDU without an SNET. A
router that doesn't follow the standard, or an application gateway that
re-originates requests, shows its own principal for what it relays, so trust a
principal as far as everything its holder may relay.

`forwarding_peer_principal()` returns the principal of the direct peer that
handed the request over, a router included. A policy that trusts a router with
everything it forwards can allow on it; that trusts every claimed origin behind
that router.

A source VMAC, UUID, routed SNET/SADR, Requesting Source or process ID is a
claim, never a principal. A policy that should admit only known operators can
allow `Some(principal)` from a pinned list and deny everything else; that denies
all hub-relayed and non-SC traffic too, which is the conservative choice.

Replies to a direct request go back only over the connection it arrived on
([accepted direct server responses](rust-api.md#accepted-direct-server-responses)).
If another certificate later claims the same VMAC, or the first reconnects, a
reply still owed to the first connection is dropped rather than sent to the new
one. The server's `direct_principal_*` and `direct_response_*` tests show this
over real TLS for success, segmented, Error and Abort replies; Reject takes the
same route check.
Admitted work may still finish under its original principal after its connection
closes; a lost reply doesn't roll it back.

## Authorization context

Each mutation decision receives a `mutation::MutationAuthorizationContext`:

- `source_mac` / `source_network` — claimed immediate peer and routed origin.
  Never authenticated identities, never certificate principals.
- `provenance` — the reassembled ingress snapshot (`TransportProvenance`)
  threaded from dispatch through `handle_admitted_confirmed_request[_with_lso]`
  into `mutations::Request`. Cross-segment provenance mismatches already fail
  closed at reassembly, so every element of one request (including each WPM
  element) observes the same snapshot.
- `principal()` / `forwarding_peer_principal()` — as above, from `provenance`
  and `source_network`.
- `trust` — `mutation::MutationTrust` derived from the snapshot, mirroring
  RB-09 `ControlTrust`: `Unverified`, `VerifiedChannel` (direct SC-TLS peer),
  or `VerifiedRelay` (SC-hub relayed origin). Channel/relay scope only, never
  leaf identity.
- `direct_sc_identity()` — the immediate direct-SC hop's leaf digest (the
  `forwarding_peer_principal()` one, so for a routed request the router that
  forwarded it) plus the process-lifetime connection incarnation, when present. Same-leaf reconnects get new
  incarnations; queued complete work retains its original snapshot after
  replacement.
- `invoke_id`, `service_choice`, `target` — the request's identity and the
  decoded mutation (current element for WPM). `service_choice` is a
  `mutation::MutationService`: `Confirmed(choice)` with `Some` invoke ID for the ten
  confirmed services, `Unconfirmed(WRITE_GROUP)` with no invoke ID for a WriteGroup,
  whose `target` is `MutationTarget::WriteGroup` naming the Channel, the group, the
  channel number, the priority used, the encoded value and the inhibit flag.

`Debug` for the context is redacted by construction: address lengths, the
provenance/trust labels, whether a principal is present and the target kind only —
never MAC bytes, the fingerprint, property values, file payloads, or other decoded
inputs. The LifeSafetyOperation and both AuditNotification contexts carry the same
addresses, `provenance`, `principal()` and `forwarding_peer_principal()`.

## Timing and side effects

Order per request is validation, then authorization, then mutation, with no
audit-log write on deny: a denial performs no database mutation, no COV/event
fan-out, and no audit-log write (that write would itself be a mutation). It is
recorded in the saturating per-service decision counters and bounded tracing
diagnostics only. `authorize_mutation` runs after DCC prechecks, request admission,
service decoding, and per-element validation, and before the database write;
WPM asks it per validated element in wire order while holding the database
write lock, so policies must stay fast, nonblocking, reentry-free, and
side-effect-free, and may be called concurrently. A panicking policy denies
fail-closed. Neither the allow nor the deny path generates an audit record for
the decision itself; audit records arrive only as explicitly authorized
AuditNotification service receptions.

**WPM elements an object saves first are decided ahead (#1321).** A Notification
Forwarder's or Notification Class's list, an Access Rights object's rule arrays,
Enable or Accompaniment, and an Audit Log's Log_Enable or Buffer_Size are saved
before the object serves them, and the server stages that save so it runs with the
database write lock released. For such an element the policy is asked before the
save is staged, under a database read guard, after the element's own validation and
in wire order among such elements, so ahead of earlier elements no object saves
first; only allowed elements are staged, up to one whose value doesn't decode, and
the handler applies the recorded decision when it reaches the element instead of
asking again. The policy is still asked once per element, but it can be asked
about an element the request never reaches, when an earlier element fails for
another reason. A request with no such element asks nothing ahead. The decision
counters and audit records cover only the elements the handler reaches, as before.

Coverage is the ten confirmed `mutation::MutationTarget` services and the Channel
writes of an inbound WriteGroup, each decided after the change list is decoded and
matched to the Channels, with no database guard held. Reads, discovery, DCC,
TimeSync, direct handler calls and trusted local writes retain their existing
behavior. Admission, duplicate detection, decoding and WPM validation retain
precedence. WPM makes one decision per reached element; empty requests make
none, and a denial stops the suffix without rolling back an allowed prefix.
WPM denials keep the `first_failed` error shape; other denials use
SERVICES / SERVICE_REQUEST_DENIED.

## Exclusions

- Direct `pub handle_*` calls and trusted local writes (`write_local`,
  `set_present_value_local`, life-safety arming) stay ungated by design — the
  same documented bypass contract as the RB-09 precedent. No raw
  server-receive path skips `mutations::Request`.
- The shared endpoint has a separate narrow [Device-write authorizer](rust-api.md#authorized-endpoint-device-writes);
  the server policy does not configure it.
- DeviceCommunicationControl and ReinitializeDevice keep their separate
  configuration paths.

## Python configuration

`BACnetServer(..., mutation_policy="permissive")` selects the same native default.
Set the keyword-only option to `"deny_all"` to deny valid inbound WriteProperty,
WritePropertyMultiple, CreateObject, DeleteObject, AddListElement,
RemoveListElement, AtomicWriteFile, SubscribeCOV, SubscribeCOVProperty and
SubscribeCOVPropertyMultiple decisions through the existing Rust gate. Denials
use SERVICES / SERVICE_REQUEST_DENIED (with WPM's existing failed-reference shape).
No Python policy callback is installed: Python has no direct BACnet/SC listener,
so a principal would always be `None` there.

Other strings raise `ValueError`, and non-strings raise `TypeError` synchronously
in the constructor, before startup drains registrations or performs transport
I/O. Reads and trusted local `write_property_local` calls remain available under
`"deny_all"`. It also denies each Channel write of an inbound WriteGroup to a Channel
added with `add_channel`. The option does not configure DCC, ReinitializeDevice, LifeSafety,
Audit or endpoint authorization.

## Decision counters (Rust)

`BACnetServer::mutation_decision_counters()` exposes fixed per-service saturating
`u64` totals, including after `stop()`: `allow_total`, `deny_total` (all denials),
and `policy_deny_total` (the deny-all subset). Samples are independent, not atomic
aggregates. They count decisions, not successful mutations or delivered responses;
they never affect authorization and retain no per-source state or durable history.
Each Channel write of an inbound WriteGroup counts once in the `write_group` row.
