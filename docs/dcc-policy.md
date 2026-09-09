# DeviceCommunicationControl local authorization

Configured servers now **deny DCC by default**, including valid ENABLE and
DISABLE_INITIATION requests with the correct configured password. A password
alone no longer enables the service. This is stricter **local operator policy**,
not a BACnet-mandated default and not authentication of a source principal.

| Rust `server::DccPolicy` | Python keyword-only `dcc_policy` | Authorization |
|---|---|---|
| `DenyAll` (default) | `"deny_all"` (default) | Deny valid supported modes |
| `RequirePassword` | `"require_password"` | Explicit opt-in; require a configured nonempty `dcc_password` and matching request password |
| `LegacyPermissive` | `"legacy_permissive"` | **INSECURE compatibility opt-in**: preserve optional-password behavior |

LegacyPermissive does **not** bypass a configured password. Without one, any
requester whose request reaches the handler can change communications. Do not
mistake this compatibility option, a shared password, a routed address, or an SC
VMAC for authenticated source identity. Exact address restriction below is not
principal authentication. Full auditing and rate policy remain future work under
the partial, separate #522 scope.

## Migration

Rust: set `ServerConfig::dcc_policy` or call `.dcc_policy(DccPolicy::RequirePassword)`
alongside `.dcc_password(...)` on the generic, B/IP or SC builder. Exhaustive
`ServerConfig` literals must add the new field (or an appropriate default update).
The default is DenyAll, even for applications already setting `dcc_password`.

Python: pass `dcc_policy="require_password"` alongside `dcc_password=...` to
`BACnetServer`. The new argument is keyword-only; existing positional password
and ReinitializeDevice arguments retain their positions. Exact lower-case values
above are accepted; unknown strings raise `ValueError`, nonstrings `TypeError`.
RequirePassword with absent or empty configuration fails before startup/dial
(Python constructor: `ValueError`; Rust startup/build: configuration error).
DenyAll and LegacyPermissive introduce no password-length restrictions.

The public unconfigured Rust
`handlers::handle_device_communication_control` keeps its legacy optional-password
API and behavior. It does not inherit the configured server default.
ReinitializeDevice's separate password and handler are unchanged.

## Ordering and side effects

### Optional exact-source restriction

`ServerConfig::dcc_source_restriction` defaults to `None`, preserving existing
policy behavior. Generic, B/IP and SC builders expose `.dcc_source_restriction(...)`.
Use `Some(DccSourceRestriction::new(entries)?)` with `DccSource::Direct(Vec<u8>)`
or `DccSource::Routed { network, address: Vec<u8> }`. Exhaustive Rust config
literals need `dcc_source_restriction: None` (or a suitable default update).
The validated list permits at most 256 entries and 1–255 octets per address;
routed networks must be 1–65534. These are static local limits, not transport
support promises. No CIDR, prefixes, ranges or dynamic callbacks are supported.

Python's keyword-only `dcc_source_restriction` accepts `None` or a list of
`(network_or_none, address_bytes)` tuples: for example `[(7, b"\x2a")]` permits
the claimed routed address 42 on network 7; `[(None, b"\x7f\x00\x00\x01\xba\xc0")]`
permits that exact direct IPv4-plus-UDP-port address. `[]` explicitly denies all
sources; it is **not** equivalent to `None`. Configuration is copied at construction.
Invalid limits raise `ValueError`, invalid types `TypeError`, and out-of-u16
network values may raise `OverflowError`.

A configured list, including empty, requires explicit `RequirePassword` and a
nonempty password. Other policies reject configuration before transport startup
or SC dialing (Python constructor, Rust build/start), never silently ignoring it.
Direct entries only match requests without a routed source. Routed entries match
the exact network and full source address, not the immediate router's MAC.
Malformed routed identities fail closed rather than falling back to direct matching.
All address bytes participate, independently of the 32-byte DEBUG truncation.
Claimed addresses are spoofable and unauthenticated, **including SC VMACs**.
An allowed address still needs the correct password; a shared password plus an
address match does not establish principal identity or authenticated-SC provenance.

### Validation and timer order

Existing admission, duplicate handling and DCC ingress discards are unchanged.
After admission: decode, existing constant-time password check, deprecated DISABLE
rejection, then local policy and optional source restriction for ENABLE/DISABLE_INITIATION, then live state/timer
commit. Missing/wrong configured passwords return SECURITY/PASSWORD_FAILURE even
under DenyAll or for DISABLE. Otherwise denied valid requests return
SERVICES/SERVICE_REQUEST_DENIED. Unknown-mode decoding/encoding errors retain
their existing precedence. Deprecated DISABLE remains denied in **every** policy.

Denial happens before the live timer lock, cancellation or state mutation.
Repeated denied requests cannot create, cancel or extend a timer, including when
its expiry is waiting for the same lock; denied ENABLE cannot clear state 2.
Admitted requests preserve existing behavior: absent duration is indefinite,
zero schedules immediate expiry, positive values use minutes, and subsequent
admitted requests replace the timer. Existing ENABLE timer handling is retained.
These are compatibility semantics, **not new timer conformance evidence**:
ASHRAE 135-2020 §16.1 specifies ignoring duration for ENABLE; this slice does not
change that existing implementation nuance. Explicit stop still cancels and
joins the owned timer.

Decode-accepted ENABLE remains eligible for protected recovery capacity regardless
of policy, password or source restriction. It can occupy that capacity until the handler denies it;
there is no pre-admission authorization. Capacity exhaustion may still Abort
before handler validation. [Request admission](request-admission.md) and the
completed bounded #521 acceptance remain unchanged, not reopened.

## Source and limits

Local licensed ASHRAE 135-2020 §16.1 (printed 759–760) supplies the existing
optional-password and deprecated-DISABLE rules; §18.6 (printed 795) describes
SERVICE_REQUEST_DENIED for lack of authorization. The three configuration modes,
nonempty startup requirement and deny-all default are operator policy, not new
normative claims. This does not expand authenticated-SC, physical-transport,
full-conformance, Audit/#125, EventLog integration, additional #181 fault-family
or GATE0007 qualification.

## Completed-handler observability

Rust `BACnetServer::dcc_outcome_counters()` returns `DccOutcomeCounters`.
Python `await server.dcc_outcome_counters()` returns the corresponding typed
dictionary. Its five stable fields are `accepted_total`, `policy_denied_total`,
`password_failure_total`, `deprecated_denied_total`, and `malformed_total`.
Each is an independently sampled cumulative `u64` (Python `int`), saturating
at `2**64 - 1`. New server lifetimes start at zero; snapshots are not an atomic
whole view. Rust snapshots remain readable after stop; Python raises
`RuntimeError("server not started")` before start and after stop.

Exactly one counter increments per completed admitted DCC handler, independently
of tracing filters. The dedicated structured DEBUG target
`bacnet_server::dcc_outcome` emits one event at that same completion boundary,
before response construction/send or any further await. `accepted` means live
state/timer replacement committed, not successful response delivery or the
current communication state. Other outcomes follow existing validation order:
decode failure → `malformed`; configured password failure → `password_failure`;
deprecated DISABLE → `deprecated_denied`; unknown mode → `malformed`; valid mode
refused by local policy → `policy_denied`.

Event fields are fixed: `outcome`, `invoke_id`, `service` (17), optional
`decoded_mode` (raw u32) and `duration_minutes` (u16), `source_kind`
(`claimed_direct` / `claimed_routed`), `claimed_source_mac`,
`source_mac_truncated`, optional `claimed_snet`, `claimed_sadr`, and
`sadr_truncated`. Missing routed SADR is an empty string, distinguished by source
kind and absent SNET. Each address is at most 32 bytes rendered as 64 lowercase
hex characters, with its own truncation flag. Both immediate and routed claims
are included when present. These are untrusted address claims, **not** authenticated
identity or canonical principals. Failed decoding exposes no decoded metadata.
Passwords, password-presence flags, request/error text and payloads are excluded.
No address formatting occurs when the DEBUG target is disabled.

This is bounded local operational telemetry, **not** a durable audit log or
BACnet Audit service. There is no internal history, queue, task, destination or
new callback API; the Python accessor installs no logger/subscriber or trace
bridge. Standard tracing subscribers are caller-owned and may filter, discard,
backpressure, or perform arbitrary work. Event delivery, global concurrent
ordering, rate limiting and flood resistance are not guaranteed. Counters and
events exclude duplicates, pre-handler admission/shutdown/Abort-fallback
rejections, handlers cancelled before completion, timer expiry, and response
delivery failures after commit. Existing admission counters cover their separate
admission boundary. Source mismatch reuses `policy_denied_total` and the existing
`policy_denied` DEBUG event/schema, without new counters. This remains partial
#522 work, not full auditing, rate policy or principal authentication; #521 remains closed.
