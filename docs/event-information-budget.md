# GetEventInformation local budgets

Server dispatch uses `ServerConfig::get_event_information_budget`, a
`GetEventInformationBudget` with positive defaults:

| Rust field | Python keyword-only constructor argument | Default |
| --- | --- | --- |
| `max_objects` | `event_information_max_objects` | 4096 |
| `max_returned_summaries` | `event_information_max_returned_summaries` | 256 |
| `max_service_ack_bytes` | `event_information_max_service_ack_bytes` | 16384 |

Generic, B/IP and SC Rust builders accept `.get_event_information_budget(...)`.
Zero is rejected before startup or SC dialing. Python rejects zero with
`ValueError`; negative or nonrepresentable integers raise `OverflowError`.

Adding `ServerConfig::get_event_information_budget` is a Rust source-compatibility
change. Existing exhaustive `ServerConfig` literals must add
`get_event_information_budget: Default::default()`, or use `..Default::default()`
where defaulting the remaining fields is appropriate.

After request decoding, dispatch checks total database cardinality using the
same held read view as the scan, before object callbacks, identifier collection
or sorting. All objects count, including objects before the cursor, non-event
objects and Notification Classes. Exceeding the limit produces a whole server
Abort `OUT_OF_RESOURCES`, even when no objects remain after the cursor.

Admitted requests preserve strict projection validation of every remaining
object in encoded ObjectIdentifier order. Filling a page stops retained item
encodings, **not validation**. A later projection error, including one on a
NORMAL object, takes precedence over a byte refusal. Notification Class lookup
still matches the class-number property, not its object instance; missing and
duplicate matches retain their existing errors.

Only the fitting prefix is encoded and retained. Once a qualifying item is
omitted, later smaller items cannot fill the gap. `More_Events` is true exactly
when a qualifying summary was omitted; trailing normal, disabled or non-event
objects alone do not set it. Resume with the last returned identifier. An absent
or deleted cursor retains numeric encoded-identifier successor behavior.

The byte limit includes the list wrappers and `More_Events`, but excludes APDU
and NPDU envelopes. Without response segmentation, bytes are further limited by
the effective peer/local APDU limit minus the actual encoded ComplexACK envelope.
With segmentation available the configured logical byte limit still applies;
generic segmentation constraints remain unchanged. An empty ACK requires four
service bytes. If its wrappers or the first summary cannot fit, dispatch returns
server Abort `BUFFER_OVERFLOW` only after the strict scan finds no service error.
There is no oversized-first-item fallback in configured dispatch.

The public `handle_get_event_information` remains an unconfigured, full-result
handler. The old crate-internal optional-byte helper retains its historical
first-item fallback for compatibility tests; configured dispatch does not use it.

These are local retained-encoding and database-cardinality policies, not BACnet
mandated numeric limits or an RSS/CPU guarantee. At default limits at most 256
retained summaries plus one byte-overflow candidate are encoded. Request decoding,
individual property/metadata callbacks and their allocations, opaque property
results, allocator failure, RSS, deadlines and rollback are outside these bounds.
Repeated Notification Class scans remain O(N*C); there is no lookup cache or
constant-work-per-projection claim.

Protocol basis: local licensed ASHRAE 135-2020 §13.12 (printed 699–700, PDF
701–702) supplies selection, cursor continuation and exact qualifying-omission
semantics. The finite limits and first-item refusal are owner-selected local
resource policies. This change does not expand object, service or conformance
support claims.
