# Target Audit Reporters

The standalone server can select one through 64 concrete, distinct built-in
Audit Reporters using `AuditReportersConfig { reporters }` and the
`audit_reporters` builder method. Startup validates the complete set and orders
it by instance. The cap is a local resource policy. Python configures the same
owner before startup through `configure_audit_reporters(list[AuditReporterConfiguration])`.
Each entry supplies `instance`, `audit_level`, `auditable_operations`, and
`issue_confirmed_notifications`, with optional `monitored_objects` and
`audit_priority_filter` and `maximum_send_delay`. A valid call replaces the complete selected set and
settings; invalid input leaves all pending objects unchanged. Settings are copied,
not retained from caller dictionaries. The singular configuration APIs are removed
before 1.0. Source reporting remains a separate exactly-one-Reporter profile.

Enabled nominal selector membership determines association before operation,
priority, or AV/BV value filters are applied. Absent Monitored_Objects selects all
subjects; an empty or all-NULL list selects none. The lowest-instance enabled
match owns an operation. Every enabled Reporter whose nominal selection overlaps
another on an actual database object exposes CONFIGURATION_ERROR. Adding/removing
subjects and changing settings refresh that health. Mandatory self-reporting does
not add nominal membership and cannot manufacture overlap. Per-object AV/BV policy
inherits the elected Reporter's settings.

Each Reporter owns its configuration, delivery generation, Reliability/Status_Flags
and bounded resource-loss coalescer. Overlap, unavailable route, and communication
failure remain distinct state. All Reporters share one global 64-operation budget;
optional delayed storage has separate bounded capacity and does not multiply this
send budget. Alternating Reporters cannot erase each other's pending losses.
Configuration/recipient changes fence stale health completions while retaining
bounded captured target loss contexts, including A-to-B-to-A transitions.
Source mode retains its existing single current-context coalescer. See
[delayed target Audit reporting](delayed-target-audit.md) for queue, control,
historical-loss and shutdown contracts.

Trusted live Rust setters, the aggregate
`configure_audit_reporter_internal(level, operations, confirmed, selectors, priorities, maximum_send_delay)`,
and concrete Description writes use one object-owned change boundary. These APIs
are fallible. Actual changes prepare all records, permits, confirmed leases,
encoding and worker ownership before any configuration field commits. The aggregate
emits one record per changed property in numeric property-identifier order; equal
configuration emits none. Silent active changes also work from synchronous threads
without a Tokio runtime, while still serializing with seal/close; they create no
worker. Notification-producing changes require the runtime before any commit.
Absent and present-empty Monitored_Objects are distinct.
These cardinality and ordering rules are local API policy. Pre-start changes emit
nothing. Local records identify the local Device and omit invoke ID.

An own NONE-to-enabled transition uses post-state eligibility and election;
disabling and enabled-to-enabled changes use pre-state. The lowest enabled nominal
match emits, falling back to the changed configured Reporter itself when eligible.
A Reporter remaining NONE has no own fallback for other properties, although an
eligible other Reporter can observe it. The fallback algorithm is a local policy.
The selected network WRITE attempt-accounting policy also retains successful no-op
and known execution-failure observations for an enabled Reporter, independent of
its WRITE bit and selectors; failures include Result. This applies at the existing
WP/WPM, list and file observation boundaries, without adding service execution.
The canonical change sink and network observer coordinate so an actual Description
change yields exactly one record with the original requester and invoke ID.
No-op/failure accounting is a local choice, not a claim that every such record is
required by the Standard. Description NULL relinquishment succeeds unchanged;
it never stores NULL or creates a local change record. An eligible network NULL
attempt still has one record. Any Description array index is rejected before
value handling with PROPERTY/PROPERTY_IS_NOT_AN_ARRAY; other non-string values
remain INVALID_DATA_TYPE. The optional Maximum_Send_Delay/Send_Now pair is also network writable when present;
other network configuration remains unsupported.

All target Reporters share the typed [Device recipient](device-audit-recipient.md).
An actual recipient change still prepares exactly one old/new pair. Its owner is
the lowest enabled nominal Device match, else the lowest enabled configured
Reporter, else the lowest configured Reporter. This local ownership choice preserves
the dedicated pair even when all levels are NONE. The change fences every target
Reporter's old recipient context. Both routes must be usable before commit.

The active owner protects every configured Reporter and its Device through sealed,
canceled and dropped lifetimes until DB-capable task frames quiesce. Normal target
stop seals producers, allows a bounded three-second drain with ACK-only progress,
then joins workers and uninstalls under the database guard. This extends the existing
ownership boundary without changing general database authoring or source roles.
Python configuration freezes at startup transfer; its existing read_property API
reads each Reporter's Reliability and Status_Flags. Live configuration described
above is a Rust API, not a new Python callback or runtime authoring API.

Evidence is in `server::audit_reporter_tests::live`, the existing target producer
and recipient suites, per-Reporter `notification_worker_owner_tests`, and installed
Python Audit integration tests. These are bounded target-profile claims; wider
network configuration, other source families and full Audit/Reporter/BIBB
conformance remain outside this outcome.
