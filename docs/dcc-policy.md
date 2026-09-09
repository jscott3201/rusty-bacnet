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
VMAC for authenticated source identity. Source authorization, auditing and rate
policy remain future work under the partial, separate #522 scope.

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

Existing admission, duplicate handling and DCC ingress discards are unchanged.
After admission: decode, existing constant-time password check, deprecated DISABLE
rejection, then local policy for ENABLE/DISABLE_INITIATION, then live state/timer
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
of policy or password. It can occupy that capacity until the handler denies it;
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
