# ReadRange response pages

The server defaults to `ReadRangeBudget { max_returned_items: 256,
max_service_ack_bytes: 16384 }`. Both settings must be positive; these are local
policy defaults, not protocol-mandated thresholds or measured performance claims.
The byte limit includes the complete logical ReadRange service ACK, including
count, flags, optional array index and first sequence number, but excludes APDU
and NPDU envelopes.

Configure `ServerConfig::read_range_budget` or `.read_range_budget(budget)` on the
generic, B/IP or SC server builder. Validation occurs before transport startup or
SC dialing. Existing exhaustive Rust `ServerConfig` literals must add the field
(typically `ReadRangeBudget::default()`), or use `..Default::default()` where
appropriate. Positive `usize` values are accepted; returned counts remain checked
against the wire representation.

Python `BACnetServer` adds keyword-only `read_range_max_returned_items=256` and
`read_range_max_service_ack_bytes=16384`. Zero raises `ValueError`; negative or
out-of-`usize` integers raise `OverflowError`. Installed native signatures and
stubs use the same defaults. Existing positional arguments are unchanged.

## Paging and continuation

Existing selection semantics are unchanged. An absent Range or positive Count
returns a contiguous prefix of the selected matches. Negative Count returns a
contiguous suffix nearest the requested reference, in normal forward wire order.
FIRST_ITEM and LAST_ITEM describe actual list endpoints included in the page;
MORE_ITEMS means page capacity omitted selected matches, not that unrelated list
items exist. Item_Count is the actual returned count. Nonempty sequence/time pages
take First_Sequence_Number from the actual first returned resident identity,
including sparse or wrapped identities. Missing references and genuinely empty
matches still return zero count without a first sequence number if the envelope
fits.

Continue in the requested direction without treating a negative page as a prefix.
For stable positional lists, advance a forward reference by the returned count;
move a backward reference earlier by the returned count and prepend each received
page. Sequence identities are not resident ordinals: do not calculate a page's
first identity by adding an ordinal to a sequence number. For time-based reads,
use returned record identities and the sequence continuation contract rather than
inventing timestamp increments (timestamps may repeat). List mutation between
requests still requires the application's existing missing-reference handling;
paging is not a cross-request snapshot guarantee.

If segmentation is unavailable because the server cannot transmit segments or
the peer declines them, the service byte cap is also limited by the smaller
peer/local APDU size minus the actually encoded unsegmented ComplexACK envelope.
When segmentation is available, the configured service cap remains in force and
the existing generic segmentation and accepted segment-count constraints apply.

If the header or first directional item cannot fit, the server returns whole
Abort BUFFER_OVERFLOW rather than an empty MORE_ITEMS page. A later oversized item
ends the page; it is not skipped. Current-page item encoding errors remain service
errors, discarding scratch without changing caller output. Deferred items are not
encoded, including after the item cap is reached. The public unconfigured Rust
`handle_read_range` retains its prior unlimited behavior for direct callers.

## Scope of the bounds

Only response item count and accumulated accepted logical service bytes are
bounded. Complete-list property production/allocations, identity vector retrieval,
existing selection scans and time validation, one trial item's recursive encoding
and allocations, callbacks, allocator capacity/OOM/RSS/CPU/deadlines and rollback
are excluded. One trial encoding can exceed the cap and is discarded. No total
read-work, whole-memory, authorization, fairness or full-conformance claim follows.
Object models, log storage, identity assignment, status projection and other
services are unchanged.

Protocol rationale is paraphrased from the local licensed ASHRAE 135-2020
§15.8 (selection, directional partial results and continuation), §21 ReadRange
request/ACK and ResultFlags productions, §18.10 (buffer Abort), and §5.4.5.3
(ordinary segmentation). The local limits and first-item refusal policy are owner
policy; this document does not redistribute licensed source text.
