# AtomicWriteFile payload admission

Configured Rust and Python servers apply three independent, positive local
limits before invoking `FileStorage::write_stream` or `write_records`:

| Rust `AtomicWriteFileBudget` field | Python keyword-only constructor argument | Default |
|---|---|---:|
| `max_stream_payload_octets` | `atomic_write_file_max_stream_payload_octets` | 16384 |
| `max_records` | `atomic_write_file_max_records` | 256 |
| `max_record_payload_bytes` | `atomic_write_file_max_record_payload_bytes` | 16384 |

Stream octets count only the actual payload. Record count includes empty records;
record bytes are the checked sum of actual record lengths, stopping at the first
excess. Neither byte limit includes service tags or APDU/NPDU headers. Exact
limits admit; any excess refuses the **entire request with server Abort
OUT_OF_RESOURCES**, regardless of peer APDU size or segmented-response acceptance.
There is no budget-driven prefix write, truncation, partial ACK, ACK-size cap, or
BUFFER_OVERFLOW policy. Empty payloads and zero records retain legacy behavior.

## Configuration and migration

```rust
use bacnet_server::server::{AtomicWriteFileBudget, ServerConfig};

let config = ServerConfig {
    atomic_write_file_budget: AtomicWriteFileBudget {
        max_stream_payload_octets: 8192,
        max_records: 128,
        max_record_payload_bytes: 8192,
    },
    ..Default::default()
};
```

Generic, B/IP and SC builders expose `.atomic_write_file_budget(budget)`.
All three values must be positive; validation precedes transport startup and SC
dialing. The new public `ServerConfig` field is a **Rust source-compatibility
change**: exhaustive struct literals must add
`atomic_write_file_budget: Default::default()` (or a chosen policy), or use a
suitable `..Default::default()` update. The public
`handlers::handle_atomic_write_file` remains unconfigured and retains its behavior;
both paths share the same validation/write implementation.

```python
from rusty_bacnet import BACnetServer

server = BACnetServer(
    123,
    atomic_write_file_max_stream_payload_octets=8192,
    atomic_write_file_max_records=128,
    atomic_write_file_max_record_payload_bytes=8192,
)
```

Python zero settings raise `ValueError`; negative/out-of-`usize` values raise
`OverflowError`, and non-integer values fail native integer conversion. Arguments
are keyword-only and are reflected in installed stubs.

## Ordering and limits of the guarantee

The existing sequence is unchanged: decode (including the independent 10000-record
decoder ceiling), record cardinality cross-check, File type, object lookup,
immutable storage availability, fail-closed Read_Only, File_Access_Method,
mutable storage availability, and write-start validation. Only then does payload
admission run, followed by exactly one storage write and checked actual-position
ACK conversion/encoding. Earlier errors retain precedence even for oversized
payloads. `-1` still means append; other negative starts remain invalid. Admitted
backend errors, including FILE_FULL and opaque Abort/Protocol errors, retain the
existing service mapping. ACKs report the actual start, not the requested sentinel.

Budget refusal invokes no storage write and causes no handler prefix mutation;
caller output is untouched. This excludes effects of pre-admission metadata and
storage hooks. It is not a rollback promise: an opaque backend may mutate before
returning an error, including the existing post-write unrepresentable ACK case.

These are input payload admission limits, **not file-size, total-memory, CPU or
deadline limits**. Excluded are decode-time allocation and record Vec copies,
ingress and lock wait (the database write guard is already held), metadata/hook
effects, opaque backend execution/allocations, full-file size, gap filling, RSS,
actual OOM, CPU, deadlines and rollback. A small payload at a large offset or an
append can cause disproportionate file growth or backend work. Existing
FileObject growth limits remain independent. No decoder or storage API behavior
is changed, and AtomicReadFile and other services are unaffected.

## Source and evidence scope

Local licensed ASHRAE 135-2020 §14.2 (printed 727–729) supplies stream/record,
append and actual-position ACK semantics and existing service failures. §18.10
(printed 800–801) and §5.4.5.3 (printed 48–49) supply Abort classification and
server response context; §18.11 distinguishes actual memory allocation failure.
The numeric thresholds and pre-write OUT_OF_RESOURCES policy are owner-selected
local resource policy, not mandated BACnet thresholds or a conformance expansion.
Tests exercise boundary admission, precedence and no-call/state evidence, legacy
parity, direct/routed wire replies, real segmented-request reassembly, and an
installed native Python constructor/runtime. No broader support claim follows.
