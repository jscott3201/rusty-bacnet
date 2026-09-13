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

Coverage is the ten `mutation::MutationTarget` services. Reads, discovery, DCC,
TimeSync, LifeSafety/Audit, direct handler calls and trusted local writes retain
their existing behavior. Admission, duplicate detection, decoding and WPM validation
retain precedence. WPM makes one decision per reached element; empty requests make
none, and a denial stops the suffix without rolling back an allowed prefix.

`BACnetServer::mutation_decision_counters()` exposes fixed per-service saturating
`u64` totals, including after `stop()`: `allow_total`, `deny_total` (all denials),
and `policy_deny_total` (the deny-all subset). Samples are independent, not atomic
aggregates. They count decisions, not successful mutations or delivered responses;
they never affect authorization and retain no per-source state or durable history.
