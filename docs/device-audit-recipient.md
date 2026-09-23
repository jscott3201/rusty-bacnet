# Target Device Audit recipient

The standalone server's opt-in target Audit profile stores its recipient in the
actual built-in Device. Provision `DeviceObject::provision_audit_recipient` before
startup and select the Reporter with `AuditReporterConfig { reporter }`. Python
uses `configure_audit_recipient(AuditRecipientInput)` before `start()` and selects
settings separately with `configure_audit_reporter`. The former Rust configuration
field and Python `recipient_device_instance` keyword are removed before 1.0.

Startup requires exactly one concrete built-in Device, a typed provision, and the
selected Audit Reporter. Provisioning alone does not expose a network property.
While the target runtime is installed, `Audit_Notification_Recipient` is readable,
required and writable, appears in Property_List and RPM REQUIRED, and has one
Device-owned value. Its metadata retains the optional base table code and active
Audit Reporting condition. Device Audit_Level and Auditable_Operations are not
made mandatory. The endpoint's existing static source profile does not expose
this property; its migration remains part of #728.

A Device recipient uses an immutable explicitly configured local or routed Device
binding. Observed I-Am entries are not eligible. An unresolved Device provision
may start with CONFIGURATION_ERROR; it emits no ordinary records. A live change
must resolve both old and new destinations, so an unavailable old binding requires
restart with corrected configuration. Address recipients require an explicit
IPv4 B/IP transport and network zero, a six-octet unicast IPv4/port address, and a
nonzero port. Broadcast, multicast, unspecified, routed Address, IPv6, SC and
MS/TP Address choices are outside this runtime subset. The generic BACnetRecipient
codec continues to represent the wider protocol grammar.

Direct database object writes, trusted `write_local`, and authorized network
WP/WPM share the same mutation owner. An actual change prepares and reserves two
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

The selected policy is the existing server mutation policy. Its default remains
Permissive without an authorizer; enabling Audit does not authenticate callers.
Applications needing admission control must configure that policy. WPM preserves
its committed prefix and stops at the failing element. Successful delivery is
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
membership outside an installed target runtime.

Evidence is in `server::audit_reporter_tests::recipient_changes`, the startup and
identity suites, the real UDP `device_recipient_bip_address_change_delivers_to_both_real_loggers`
test, and installed Python `test_audit_api.py` provisioning/loopback cases. This is
a bounded target subset. Source migration, other address/link choices, per-object
policy, batching/send delay, durable delivery and broader Audit/BIBB conformance
remain incomplete; #728 and #345 remain open.
