# AtomicReadFile local budgets

The configured server applies positive, operator-configurable local defaults:

| Rust `AtomicReadFileBudget` field | Default |
| --- | ---: |
| `max_requested_stream_octets` | 16384 |
| `max_requested_records` | 256 |
| `max_service_ack_bytes` | 16384 |

These are owner policy, not BACnet-mandated thresholds or benchmark results.
The byte limit measures the **complete logical service ACK**, including Boolean,
CHOICE tags, start integer, returned record count, every octet-string prefix and
closing tag. APDU/NPDU overhead is excluded. A full 16384-octet payload therefore
exceeds the default 16384-byte ACK limit; raise that limit if it must be returned.

Validation retains this order: decode, File identifier/type and lookup, storage
hook availability, File_Access_Method, negative start conversion, then the **raw
request count** limit. Excess counts return a whole-service server Abort
`OUT_OF_RESOURCES` before `read_stream`/`read_records`, even near/at EOF or for an
empty file, and before storage checks for beyond-EOF starts or read failures.
No extra storage-size property or metadata reads are performed. Admitted storage
errors retain their prior behavior. Zero request counts remain admitted; zero
configuration limits are invalid.

For records the raw limit precedes the existing `min(requested, 10000)` storage
read window. Raising the limit above 10000 does not remove that legacy local
window; it is not a normative BACnet limit. This change makes no new claim about
zero-count, empty-file, at-end, or short-read compliance.

After one successful storage read, bounded checked size arithmetic inspects only
lengths, without payload copies or per-record clones. An oversized ACK returns
whole-service server Abort `BUFFER_OVERFLOW` before encoding payload into the
response. Budget and service failures leave caller output unchanged; there is
no new budget-driven short successful read or fabricated EOF. Within-budget
encoding, storage EOF, routing, and generic response segmentation are unchanged.
The policy is independent of peer APDU size and segmentation capability.

## Configuration and migration

Rust exports `bacnet_server::server::AtomicReadFileBudget`. Set
`ServerConfig::atomic_read_file_budget` or use `atomic_read_file_budget(...)` on
generic, B/IP, or SC builders. Positive validation precedes transport startup and
SC dialing. Existing exhaustive `ServerConfig` literals need
`atomic_read_file_budget: Default::default()` (or a default struct update).
The public unconfigured `handle_atomic_read_file` helper remains unconfigured
and compatible; configured server dispatch uses the budgeted path.

Python `BACnetServer` adds keyword-only arguments:
`atomic_read_file_max_requested_stream_octets=16384`,
`atomic_read_file_max_requested_records=256`, and
`atomic_read_file_max_service_ack_bytes=16384`. Stubs match native defaults.
Zero raises `ValueError`; negative/out-of-platform-range integers raise
`OverflowError` before transport setup.

## Deliberate limits of this policy

Excluded: existing request decoding and metadata/property/storage hooks, opaque
storage callback work and allocations, the single owned read result (including
one giant record), allocator capacity, actual OOM, RSS, CPU, deadlines and rollback.
**Requested record count is not a total storage-byte bound.** This is not a
whole-memory guarantee or preemption mechanism. No FileStorage API changes,
AtomicWriteFile changes, file writes, authorization/fairness policy, other-service
budgets, or broader support/issue-closure claims are included. Audit/#125,
EventLog/#181, GATE0007 and SC identity hard stops remain unchanged.

## Source rationale

Local licensed ASHRAE 135-2020: §14.1 (printed 725–727 / PDF 727–729) supplies the
read procedure and existing validations; §21 (printed 861–863 / PDF 863–865)
supplies the stream/record service productions. §18.10 (printed 800–801 / PDF
802–803) distinguishes inability to start due to resources from buffer overflow;
§18.11 distinguishes actual dynamic allocation failure. §5.4.5.3 (printed 48–49 /
PDF 50–51) provides the surrounding Abort/segmentation behavior. The thresholds,
raw-count precedence over opaque storage failures, and resource exclusions here
are explicitly approved local policy, not new interpretations of normal reads.
