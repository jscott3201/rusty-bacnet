# ReadPropertyMultiple service budgets

Configured Rust and Python servers default to **256 expanded result elements**
and **16,384 encoded service-ACK bytes** per ReadPropertyMultiple (RPM) request.
These positive limits are local owner policy, not Standard-mandated values or
benchmark-derived guarantees.

## Configuration and compatibility

Rust exposes `bacnet_server::server::ReadPropertyMultipleBudget` with public
`max_result_elements` and `max_service_ack_bytes` fields. Set it through
`ServerConfig::read_property_multiple_budget` or the
`read_property_multiple_budget(...)` method on generic, BACnet/IP, and SC
builders. `validate()` rejects zero; values through `usize::MAX` are accepted
without reserving that much storage. Validation precedes transport startup and
SC dialing, while retaining preceding route, APDU, admission, and SC-specific
validation priorities.

The added public `ServerConfig` field is a Rust source expansion: exhaustive
struct literals need the new field; literals using `..Default::default()` do
not. Existing large RPM consumers may need explicitly larger budgets. Success
within the limits retains ordinary ComplexACK encoding and segmentation.

Python adds keyword-only constructor arguments:

```python
server = BACnetServer(
    123,
    rpm_max_result_elements=256,
    rpm_max_service_ack_bytes=16384,
)
```

Zero raises `ValueError`; negative or native-integer-overflow values raise
`OverflowError`. Validation occurs in the constructor, before any transport,
including synchronous MS/TP serial open. All transport startup paths propagate
the same policy through the common Rust builder.

The existing public low-level
`handlers::handle_read_property_multiple(db, data, buf)` remains unconfigured
and retains its legacy behavior. It is not the configured server dispatch path.
Manual callers of that helper do not receive these server limits.

## Work admission

The handler decodes the request, then plans the whole request before invoking
property reads. Each explicit reference counts once. For known objects,
ALL/REQUIRED/OPTIONAL count their actual expanded results, preserving order and
duplicate occurrences. Unknown objects count each original reference once,
including wildcard references that produce genuine UNKNOWN_OBJECT results.
Invalid array-index results count even though no property read is called.
Empty expansions count zero and retain their object wrappers.

Checked aggregate accounting refuses before appending a result beyond the
limit. A late overflowing specification therefore causes **no handler property
reads**, not successful results for an earlier prefix. Empty object-plan
wrappers remain bounded by the existing decoded request's specification count.
The whole service returns a server Abort with **OUT_OF_RESOURCES** and the
original invoke ID.

## Encoded response accumulation

Results are read and encoded one at a time, not collected into a complete
value-bearing ACK vector. Checked pre-append accounting keeps the scratch
service buffer's logical length within `max_service_ack_bytes`, reserving the
current object's closing tag. All object/property tags, array indexes, encoded
values and genuine property errors count; APDU and NPDU bytes do not.

If an append cannot fit, the scratch response is discarded, no further
properties are read, and the whole service returns a server Abort with
**BUFFER_OVERFLOW**. No partial successful RPM ACK or fabricated per-property
resource error is sent. Budget refusal precedes generic response segmentation
and is independent of whether the client accepts segmented responses. The
internal bounded helper leaves the caller's buffer prefix unchanged on failure.

## Explicit exclusions

This slice does **not** bound allocations/work inside the existing request
decoder, opaque object metadata, one `read_property` call (including a returned
whole array/list), or one value/result encoding. Scratch allocator capacity
may exceed logical length. It is not a peak-RSS, whole-request allocation,
CPU-time, or arbitrary-object preemption guarantee. A byte overflow can occur
after earlier property reads; their side effects are **not rolled back**.

Genuine per-property access and encoding error projection, Device-wildcard
lookup and response identifiers, array-index semantics, request decoding,
admission quotas, DCC checks, recovery partitions, and duplicate handling are
unchanged. This is partial resource hardening for #521, not full issue closure;
#522, fairness/authentication, and other service work budgets are unchanged.

## Protocol rationale (ASHRAE 135-2020, paraphrased)

RPM requires complete results, not a success prefix (§15.7, printed 742–743 /
PDF 744–745). Per-property errors describe actual access failures (§15.7.3.2,
printed 744 / PDF 746). The ACK production provides no continuation/truncation
flag (§21, printed 864 / PDF 866; ReadAccessResult, printed 950–951 /
PDF 952–953).

Local work-admission refusal uses OUT_OF_RESOURCES; actual configured encoded
ACK exhaustion uses BUFFER_OVERFLOW (§18.10, printed 800–801 / PDF 802–803).
Planning itself may already constitute processing; this is a defensible local
policy, not a claim that the Standard mandates this planning model. The local
whole-transaction Abort and server flag/invoke/reason follow §5.4.5.3 (printed
49 / PDF 51) and §20.1.9 (printed 838–839 / PDF 840–841). RESOURCES/OUT_OF_MEMORY
is not substituted for a configured cap: §18.11 (printed 801 / PDF 803)
concerns actual allocation failure.
