# Delayed target Audit reporting

A configured standalone target Audit Reporter may author `maximum_send_delay` as
`Some(AuditSendDelay::new(seconds)?)` in Rust or `maximum_send_delay=seconds` in its
Python configuration dictionary. The local accepted range is 0–3600 seconds.
`None` omits both Maximum_Send_Delay and Send_Now; zero exposes both properties
and sends ordinary records immediately. Presence cannot change while target
ownership is active. The source endpoint profile rejects any present value,
including zero, before activation and at its live configuration boundary.

The existing Rust aggregate setter has a sixth `Option<AuditSendDelay>` argument.
Python configures the pair through `configure_audit_reporters` before startup.
Once present, both scalar properties are readable and writable through WP/WPM.
Maximum_Send_Delay uses Unsigned and Send_Now uses Boolean. Any array index is
rejected; an unindexed NULL relinquishment succeeds unchanged. Unindexed absent
properties return UNKNOWN_PROPERTY. For these known scalar identifiers the shared
network index gate returns PROPERTY_IS_NOT_AN_ARRAY even when the optional pair
is absent. This is local precedence for a doubly-invalid request, not a mandated
ordering or datatype inference for unknown proprietary properties. Other Reporter
configuration remains locally authored, except the existing Description writer.

## Deadlines, batching and resource limits

Ordinary eligible records capture their original timestamp, route, confirmation
mode, APDU limit, and configuration/recipient generation. Positive delay retains
records until the first record's monotonic deadline. Later appends and delay
increases cannot postpone that deadline; a decrease may accelerate it. Adjacent
records with identical captured delivery settings share an AuditNotification
while the complete APDU fits the server limit. Outbound segmentation is not
implemented. An oversized record is a known local resource loss and does not
block later fitting records.

Local unsent storage is bounded to 256 records and 256 KiB per server, and 64
records and 64 KiB per Reporter. Each retained record is charged its encoded
single-record APDU size, including headers and the list wrapper. Charges remain
while waiting for local transport acceptance. The existing 64 active-send permits
remain a separate shared limit. All limits and timing choices here are local
resource policy, not Standard-prescribed capacities.

Actual Reporter configuration changes, protected AV/BV policy changes, the
Device recipient old/new pair, and the selected dedicated network Reporter WRITE
attempt records remain immediate. Protected actual changes prepare resources
before committing and may return SERVICES/SERVICE_REQUEST_DENIED. Ordinary queue
pressure does not reject the underlying service mutation. Ordinary filters still
apply; this capability does not add producer families.

## Send_Now and completion

Every TRUE write captures a new fence over the currently unsent delayed prefix,
including when readback is already TRUE. An empty fence settles immediately.
FALSE clears readback without canceling retained work. An older completion cannot
clear a newer fence. Internal reset creates no additional network WRITE record.
The external command retains its original source and invoke ID under the existing
Reporter attempt policy.

Local transport completion and confirmed ACK health are separate. Readback can
be FALSE while a confirmed ACK is still pending. Each send has one bounded attempt
and no retry. Terminal failure also retires its fence: FALSE means quiescent,
not proof of recipient delivery or storage. Unconfirmed success means only local
transport acceptance.

## Historical losses and shutdown

Each target Reporter retains at most eight captured loss contexts, including its
current baseline; all Reporters share a 256-context limit. Live configuration and
whole-Device recipient changes reserve every affected context and mandatory send
before any assignment. A late refusal rolls back preparation without changing
fields, routes, generations, deadlines, sequence numbers, or worker ownership.
Pinned older contexts survive A-to-B-to-A changes and send summaries to their
captured destinations. They cannot change current delivery health. Earliest loss
is selected by original admission order, including a later drop of an older
record, rather than callback order or timestamp comparison.

Known local queue/admission losses feed the existing bounded AUDITING_FAILURE
coalescer. Confirmed errors, timeouts and ambiguous transport failures do not
invent remote-loss counts. A summary never recursively counts itself. Captured
non-NONE Audit_Level and the AUDITING_FAILURE bit still filter summaries in this
partial profile. The tension between that filtering and the complete §19.6.6
requirements remains tracked by issues #345/#732/#783; this is not a full Audit
conformance claim. Source reporting retains its separate current-context policy.

The first target stop seals new application work and starts one absolute
three-second drain deadline. ACK/control ingress continues while application
requests are gated before reassembly and inline dispatch. Cancellation of stop
does not restart or remove the deadline. The owned scheduler enforces it, and a
later stop joins remaining tasks and uninstalls under the database guard.
Known never-attempted records at terminal closure are cancellation/resource losses;
there is no durable outbox. Drop promises task cancellation and eventual ownership
release, not an asynchronous flush. Non-target shutdown retains its existing
sequential cancellation-safe behavior.

Behavioral evidence is in `audit_batching_tests`, `audit_batch_resources_tests`,
`audit_batch_history_tests`, `audit_batch_shutdown_tests`,
`audit_historical_loss_tests`, `reporter_delay_tests`, and installed
`test_delayed_target_audit.py`. See the [conformance summary](conformance/support-summary.md)
for the bounded claim and remaining qualification limits.
