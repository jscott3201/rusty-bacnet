# Device Audit recipient

The standalone server's opt-in target Audit profile stores its recipient in the
actual built-in Device. Provision `DeviceObject::provision_audit_recipient` before
startup and select the Reporter with `AuditReporterConfig { reporter }`. Python
uses `configure_audit_recipient(AuditRecipientInput)` before `start()` and selects
settings separately with `configure_audit_reporter`. The former Rust configuration
field and Python `recipient_device_instance` keyword are removed before 1.0.

Startup requires exactly one concrete built-in Device, a typed provision, and the
selected Audit Reporter. Provisioning alone does not expose a network property.
While a complete target or source runtime is active, `Audit_Notification_Recipient` is readable,
required and writable, appears in Property_List and RPM REQUIRED, and has one
Device-owned value. Its metadata retains the optional base table code and active
Audit Reporting condition. Device Audit_Level and Auditable_Operations are not
made mandatory. Endpoint service scope remains narrower than the object metadata:
its responder accepts WP only with the explicit Device authorizer, and does not
execute RPM or WPM.

The standalone target uses immutable explicitly configured local or routed Device
bindings. The endpoint source uses `BipEndpointBuilder::source_audit_device_binding`
for direct IPv4 Device route facts. Observed I-Am entries are not eligible. An unresolved Device provision
may start with CONFIGURATION_ERROR; it emits no ordinary records. A live change
must resolve both old and new destinations, so an unavailable old binding requires
restart with corrected configuration. Address recipients require an explicit
IPv4 B/IP transport and network zero, a six-octet unicast IPv4/port address, and a
nonzero port. Broadcast, multicast, unspecified, routed Address, IPv6, SC and
MS/TP Address choices are outside this runtime subset. The generic BACnetRecipient
codec continues to represent the wider protocol grammar.

In the standalone target, direct database object writes, trusted `write_local`,
and authorized network WP/WPM share the same mutation owner. The endpoint source
exposes `EndpointSession::write_audit_recipient(Some(value))` for trusted local
changes in running `ClientOnly` and `Both` sessions; `None` performs unchanged
NULL relinquishment. The call uses the actual Device mutation owner and rechecks
sealed state after taking the database lock. It does not call the inbound network
authorizer. `Both` additionally accepts authorized network WP. An actual change prepares and reserves two
bounded notification attempts before committing the property, its Reporter
generation, and an owned delivery worker. Both attempts carry the same WRITE
record: local target Device/Object, recipient property, new Target_Value and old
Current_Value. Remote writes retain requester identity and invoke ID; local
writes identify the local Device and omit invoke ID. Two different recipient
values that resolve to one route still produce two attempts. No ordinary observer
adds a third record.

This dedicated change path applies even when the ordinary WRITE bit is clear or
Audit_Level is NONE. That precedence is the selected interpretation of the
property-specific old/new notification requirement; ordinary filters remain
unchanged. Exact NULL is a relinquishment operation that succeeds unchanged after
normal access checks. Equal values also succeed unchanged. Neither consumes a
sequence, advances a generation, reserves resources, nor sends notifications.
Malformed values, invalid scalar indices/priorities, policy denial, unavailable
routes and precommit admission/encoding failure leave the old state intact.
Valid priorities 1–16 are ignored for this noncommandable property.

The standalone target uses the existing server mutation policy. Its default remains
Permissive without an authorizer; enabling Audit does not authenticate callers.
Applications needing admission control must configure that policy. WPM preserves
its committed prefix and stops at the failing element. The endpoint source in
`Both` instead requires an explicit `with_device_writes` authorizer; no callback
means startup rejection. Endpoint WPM remains unsupported. Successful delivery is
separate from write acceptance: after commit, each destination has an independent
attempt within one three-second deadline, with no retry or durable outbox. A send
or ACK failure does not roll back the recipient. Either attempt's failure survives
a sibling success. Old-generation completions cannot change current health, and
recipient changes retire and wake pending old resource-loss summaries.

A valid Device-local clock supplies the target commit timestamp; unavailable or
invalid clocks use the database's shared sequence, consumed once only after
successful commit. Timestamp retention and bounded failure behavior are local
policies, not additional claims of normative requirements.

The installed owner protects removal, replacement or adaptation of its Device and
Reporter, and insertion of an additional Device. Shutdown seals writes before
closing admissions, joins owned producers and workers, then uninstalls the
capability under the database guard. Canceled stop retains sealed protection until
a later stop finishes. Drop requests cancellation and keeps membership protection
until retained task frames actually quiesce; an externally retained database then
becomes structurally editable. These restrictions do not change general database
membership outside an installed runtime. Endpoint source adapters hold only weak
owner references: once sealed they forward the original Reporter behavior, and
external dormant READ futures cannot retain membership after owned frames end.

Evidence is in `server::audit_reporter_tests::recipient_changes`, the startup and
identity suites, the real UDP `device_recipient_bip_address_change_delivers_to_both_real_loggers`
test, and installed Python `test_audit_api.py` provisioning/loopback cases.
Endpoint source evidence is in `source_recipient_tests`, its admission/lifecycle
modules, the external `source_recipient_public` test, startup cleanup cancellation
cases and the ingress canceled-stop test.

The complete endpoint source profile replaces the old static selector and
ownership-only mode. Select `with_source_audit_reporter` with a typed Device
provision even at NONE. `ClientOnly` and explicitly authorized `Both` are supported;
`ServerOnly`, non-B/IP links and Monitored_Objects are rejected before startup.
With no source selection, provision and bindings remain inert. Startup sends no
notifications. Preflight validation errors leave configuration retryable. A
profile initialization error after ingress starts joins cleanup and leaves a
terminal session. Canceling that cleanup leaves `Stopping`; another `stop` or
Drop completes teardown rather than permitting restart. Ordinary source READ captures the selected route at admission;
an in-flight request and its loss context retain that route after a later change.
A missing Device route suppresses ordinary records without changing READ results
or consuming audit resources. Direct Address choices require no Device binding.

The pair's logical permits, confirmed leases, encoded size and owned worker are
secured before commit. Bounded endpoint egress admission occurs afterward, so a
full/closed queue is an independent delivery failure and cannot roll back the
value or cancel the sibling. There is no durable queue or notification retry.

Other address/link choices, ordinary source operations beyond READ, per-object
policy, batching/send delay, durable delivery and broader Audit/BIBB conformance
remain incomplete; #345 remains open.
