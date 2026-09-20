# Immediate Audit Log forwarding (RB-22a / RB-22b evidence)

The standalone Rust server can forward accepted notification batches from its
one explicit `ServerConfig::audit_notification_sink`. Configure that
`AuditLogObject` locally with `set_member_of(Some(BACnetDeviceObjectReference))`
before adding it to the database, and provide a configured `DeviceBinding` for
the referenced remote Device. The parent object must be an Audit Log. The
reference identifies the parent configuration; the AuditNotification service
has no target-log parameter, so the receiving device's own sink policy chooses
the actual log. Configure both ends consistently.

## Storage and delivery are separate

- The existing authorizers remain mandatory and fail closed. Transport
  provenance, not a notification's claimed Source_Device, governs admission.
- A confirmed batch commits records and its completed receipt atomically before
  forwarding or sending the local SimpleACK. An unconfirmed batch commits only
  records and never responds. Local storage failure admits no forward.
- A batch that changes retained record content admits **at most one confirmed
  forward attempt**, preserving the original service payload and every Audit
  notification parameter. Only the outer transaction header/invoke ID is new.
  Matching/merging still follows the existing storage rules. A complete-record
  match, a receipt-only commit, and a zero-capacity log do not forward.
- Exact confirmed receipt duplicates are still silently discarded, without a
  second ACK or forward. The identity includes the requester and complete
  confirmed request; identical notification content from a different peer is
  not a receipt duplicate. The existing 60-second/256-entry retention remains.
- Parent delivery failure never rolls back the local commit or changes the
  inbound response. Records remain queryable; successful forwarding never
  deletes them.
- A full nonzero-capacity ring is not a storage error: normal oldest-record
  eviction and stable sequence/Total_Record_Count advancement commit with the
  whole accepted batch. A backend commit failure instead leaves records,
  counter, and receipts unchanged and admits no forward.

This is **best effort after commit**, not durable delivery or at-least-once
forwarding. There is no outbox, retry queue, pending-send snapshot, backlog
replay, or catch-up when a parent recovers. Shutdown, cancellation, or restart
can lose forwarding progress even when local receipt succeeded. Reapply the
local parent configuration after reopening storage. Schema v2, v1 migration,
the two-slot checksummed backend, and receipt recovery are unchanged.
Schema v1 has **no receipt ledger**: after reopening it, a later exact historical
request is fresh for receipt purposes. It receives an ACK and a v2 receipt on
successful commit. If its complete record already matches, it still does not
forward; an exact retransmission of that newly receipted request is silent,
including after another reopen. No historical receipt is inferred from records.

## Configuration, health, and resource bounds

With a local parent configured, Member_Of is a BACnetDeviceObjectReference,
Delete_On_Forward is fixed FALSE, Issue_Confirmed_Notifications is fixed TRUE,
and Reliability is present. These four properties are read-only over BACnet;
Property_List, RPM, and generated runtime PICS use the same effective metadata.
Without the local configuration they remain absent.

Startup resolves the selected log's configuration without forwarding I/O.
Missing or observed-only bindings, omitted/local Device references, self-parent
Device references (including another log on the same Device), non-Device or
non-Audit-Log identifiers, and a direct binding to the local MAC are unusable:
Reliability reports CONFIGURATION_ERROR and no forward is sent. No discovery
traffic is initiated. Direct and explicitly routed unicast bindings are
supported. A configured binding does not authenticate the peer.
Each changed accepted batch resolves the route anew. If a valid configured route
is present at that attempt, delivery can resume and a successful ACK can clear
the earlier failure. Merely making a route available does not replay work or
poll for health recovery. The recovery test uses the existing internal validated
binding-table construction; it does not add a public dynamic-binding API or
upgrade observed I-Am information into configured authority.

Admission shares the existing server-wide 64-active Audit delivery permits.
There is no waiting queue or per-peer forwarding history. Saturation, binding
lock contention, DCC initiation suppression, an oversized outbound APDU,
transport failure, missing ACK, rejected worker admission, and cancellation
are failed best-effort attempts. After admission, one absolute three-second
deadline covers scheduling, transport send, and ACK wait. There are no retries,
outbound segmentation, fanout, detached workers, or recursive audit reports.
Workers are cancelled and joined through NotificationTransactions at shutdown.
No ObjectDatabase guard is retained during network I/O.

Delivery failures set COMMUNICATION_FAILURE and Status_Flags.FAULT; configuration
errors take precedence. A later successful attempt may clear communication
failure, but an older success cannot hide a failure recorded since it started.
Status belongs to the object/configuration instance, so stale completion cannot
update a replacement object. Health is memory-only, not another durable record.

## Cycles and limits of the claim

Configure an **acyclic parent hierarchy**. Self-parent rejection, complete-record
matching, finite receipt retention, the shared active-work cap, and deadlines
bound local work; they do not prove that every distributed cycle terminates.
In particular, repeated single-actor reports need not be complementary matches,
and different peers/fresh outer invoke IDs are not the same receipt. An isolated
A-to-B-to-A test covers a complete-record cycle; no wire hop field is invented.

This slice forwards only accepted inbound AuditNotification batches. It does
not forward locally appended LogStatus/time-change/purge records, add
Maximum_Send_Delay/Send_Now, delete records after forwarding, send unconfirmed
outbound requests, support multiple parents, or expand Reporter/Python APIs.
It is not full Audit, AR-L-A, BIBB, BTL, or issue #345 completion.

## RB-22b boundary evidence

These tests harden the existing policy, not a new delivery or durability profile:

| Evidence | Boundary proved |
| --- | --- |
| `audit_forwarder_boundary_tests.rs::audit_forwarding_full_ring_evicts_atomically_without_delivery_rollback` | Capacities 1 and 2; one commit/forward at full capacity; byte preservation, survivors and counters after ACK, send failure, or ACK timeout; backend failure retains no receipt. |
| `audit_forwarder_boundary_tests.rs::audit_forwarding_partial_batch_is_one_commit_and_one_complete_wire_request` | One item merges and another creates; one atomic commit and original whole-batch forward; all-or-nothing failure, silent exact retry after success, and different-peer receipt-only acceptance. |
| `audit_forwarder_boundary_tests.rs::audit_forwarding_window_boundary_changes_once_and_complete_match_is_silent` and `audit/notification_tests.rs::forwarding_window_is_inclusive_at_wire_tick_and_timeout_edges` | Inclusive `2 * APDU_Timeout` matching: Time/DateTime at the boundary and ±1 hundredth (the wire resolution), plus APDU_Timeout ±1 ms at the object layer. Each changed batch attempts once; complete matches do not. |
| `audit_forwarder_boundary_tests.rs::audit_forwarding_mixed_timestamp_variants_create_distinct_records_and_attempts` and `audit/notification_tests.rs::forwarding_mixed_timestamp_variants_never_complement_in_either_direction` | Every mixed Time/DateTime/SequenceNumber pair stays distinct in both directions. |
| `audit_forwarder_boundary_tests.rs::audit_forwarding_zero_capacity_commits_receipt_without_forward` | Confirmed ACK and durable receipt with no retained record/forward; exact duplicate stays silent. |
| `audit_forwarder_recovery_tests.rs::audit_forwarding_configuration_recovers_only_on_next_changed_batch` | Unresolved/observed-only configuration, no replay on route availability, next-attempt resolution and ACK-driven recovery. |
| `audit_forwarder_recovery_tests.rs::audit_forwarding_file_v1_reopen_has_no_replay_or_historical_receipt` | Real file-backed v1 reopen, local Member_Of reapplication, no replay, fresh receipt-only acceptance, and v2 receipt recovery. The server fixture removes the empty v2 receipt ledger from an encoded file; `audit/persistence_receipt_tests.rs::forwarding_v1_reopen_reapplies_member_of_without_rewriting_snapshot` also exercises the existing v1 encoder seam directly. |
| `audit_forwarder_recovery_tests.rs::audit_forwarding_old_instance_completion_and_cancellation_cannot_update_replacement` | Old-instance success, rejection, deadline and joined shutdown cannot update replacement health; no durable mutation or detached work. |

Server test paths are under `crates/bacnet-server/src/server/`; object test paths
are under `crates/bacnet-objects/src/`. Existing `audit_forwarder_edge_tests.rs`
retains the isolated complete-record A→B→A proof, and `audit_forwarder_tests.rs`
retains shared-capacity, absolute-deadline, joined-shutdown and no-DB-lock-during-I/O
evidence. These are controlled transport and file-reopen tests, not an independent
peer interoperability, distributed-cycle, or power-loss certification.

Source: licensed ANSI/ASHRAE 135-2020, Clause 12.64 (PDF pp. 626-630, printed
624-628) and Clauses 19.6.7.2-.3 (PDF pp. 827-828, printed 825-826), inspected
locally through the release-plan source navigation. This document paraphrases
the contract and distinguishes the selected narrower delivery policy.
