# Completed local access authorization

`BACnetServer::authorize_access_local(request, tag)` evaluates one already
authenticated local presentation and records its terminal authorization result
at the native Access Point. It accepts the complete `AuthorizationRequest` from
`bacnet_objects::access_control`; the application supplies the criterion order,
rights-completeness policy, operational facts, companion evidence and any external
phase observation. The server installs no policy defaults.

The returned `LocalAuthorizationResult` contains the full
`AuthorizationEvaluation` and `committed_events`, the records actually installed
with their assigned timestamps. These Rust APIs are new in 0.15.0. This
route adds no Python method or network service.

| Evaluation outcome | Point mutation and publication |
|---|---|
| `Granted` | Install every event intent, in order, ending with the grant. |
| `Denied` | Install every event intent, in order, ending with the specific denial. |
| `Pending` or `Unresolved` | Return all diagnostic evidence and an empty committed-record list. Even intermediate intents remain unpublished. |
| `NoAuthorization` | Return the evaluation without changing or publishing an event. |

An `ACCOMPANIMENT_BY` record identifies its companion. Other records identify the
original credential. Every record uses the supplied tag. The request carries no
physical authentication factor; the point therefore serves its existing
`UNDEFINED` factor representation. The relevant Access Point fields are specified
in Clauses 12.31.27–12.31.31; their ordinary COV reporting follows Table 13-1.

## Caller responsibilities

A call represents a completed evaluation attempt, without a stored transaction
controller. Repeating a terminal request publishes another attempt, even with the
same tag. The application owns retry decisions, tag assignment and any replay
tracking. A pending result does not schedule a continuation: a later call must
supply the later observation and the entire request again.

Inspect the full evaluation, including excluded-rights evidence, companion
diagnostics and optional passback-reporting problems. Successful rights alone do
not establish a grant when a later ordered criterion fails. The two explicit
rights-completeness policies retain their evaluator meanings.

This route does not authenticate a factor, fetch remote rights, operate a door,
update credential-use or zone accounting, or provide durable replay tracking.
It adds no intrinsic Access Point alarm algorithm. The terminal event record is
evidence of the evaluation, not proof of physical passage or remote receipt.

## Atomic installation and time

One database write guard covers admission, evaluation, native staging, installation
and timestamped COV capture. An unhealthy point is refused before evaluation.
An unknown or wrong object, malformed admission properties or a custom object
without atomic batch support is also refused. Native staging validates the entire
batch before replacing any event fields; it never installs a successful prefix.
The object remains registered throughout.

The point assigns each omitted event time using its Device clock, or its sequence
mechanism when no usable clock exists. Equal or backward clock readings advance
the event trigger by a hundredth where the representation permits it. Sequence
numbers wrap from 65535 to 1. If the complete batch cannot keep distinct triggers,
the call fails before installation. Timestamped COV uses each event's coherent
Device-clock sample for `Time_Of_Change`; this can differ from the advanced
`Access_Event_Time` trigger.

Custom objects must explicitly opt in through the internal batch trait seam and
honor its atomic refusal and retained-snapshot contract. The server does not
emulate atomicity by calling a fallible single-record setter repeatedly.

## Contention, cancellation and delivery

Acquisition follows database then COV-table order. When the COV read cannot be
obtained immediately, the route releases the database guard before waiting for
COV readiness. It releases that readiness guard, yields, then retries from fresh
state. Failed attempts do not evaluate authorization or cache subscriptions.
Quiescent contention permits cooperative progress; continuous contention has no
starvation or latency guarantee.

Cancellation before installation changes nothing. After installation and capture,
the existing server task set owns the notification work before the caller can
await again. The database guard is dropped before notification I/O. Retained
intermediate states preserve this call's event order through ordinary COV,
selected-property COV and both forms of COV multiple, even if another transaction
changes the live point during a blocked send. Independent transactions have no
global notification-order guarantee.

Existing notification generations, replacement rules, DCC suppression, resource
budgets, send failures and shutdown behavior still apply. Closing the task set or
stopping the server can discard undelivered work without undoing a committed
event. A post-commit panic propagates to a waiting caller; without a waiting caller
the existing task mechanism logs it. A successful call does not promise a remote
acknowledgment or durable delivery.

## Evidence scope

Deterministic object tests and paused-clock server tests cover atomic refusal,
timestamp boundaries, every evaluator denial family, ordered multi-event wire
payloads, diagnostic-only results, lock contention, competing changes and the
notification lifecycle. They use the existing in-memory `TestTransport`; this
evidence does not qualify a physical reader, access controller or door.
