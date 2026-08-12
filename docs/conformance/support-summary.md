# BACnet Standard 135-2020 Support Summary

> DRAFT internal support evidence. Generated from `docs/conformance/bacnet-135-2020.json`; this is not a BTL certification claim or formal PICS/BIBB declaration.

- Standard: ANSI/ASHRAE Standard 135-2020
- Reviewed at: 2026-08-12
- Implementation evidence SHA reviewed: `926559c539c58043f1c77afb1148be0288bc34cc`
- Scope: Objects-layer write-path validation tranche (branch codex/write-path-validation; issues #252, #240, #255, #270): the Reliability write-path validity predicate derives from Reliability::ALL_NAMED plus the explicit 64..=65535 vendor range (#252); Loop, Schedule, and the nine intrinsic-reporting carriers gate in-service Reliability writes while Trend Log / Trend Log Multiple refuse them after their Clause 12 tables showed no writability grant (#240); Notify_Type writes validate against the three-value production and Event_Enable/Limit_Enable writes require the canonical fixed-width bit-string encoding (#255); and Relinquish_Default is network-writable and locally settable, with per-type validation, on the commandable object types (#270). Conventions unchanged: `repo_sha` names the last code commit of the PR branch (this docs-refresh commit lands after it), matching how the previous tranches recorded their reviewed SHA.
- Addenda/errata: Local source `_spec/2020_ASHRAE_Standard-135-BACnet-Data-Communication-Protocol.pdf` was reviewed for the Clause 12.17 Loop table and footnotes (Table 12-20), the Clause 12.24/12.25 Schedule/Trend Log tables and Reliability_Evaluation_Inhibit texts, the Clause 12.30 Trend Log Multiple table, the Clause 21 BACnetNotifyType / BACnetEventTransitionBits / BACnetLimitEnable / BACnetReliability productions, the Clause 15.9.1.3 error table, and the Clause 12 commandable-type tables cited by the Relinquish_Default row (Tables 12-3, 12-8, 12-10, 12-22, 12-23, 12-30, 12-64, 12-69 and the Clause 12.41+ value object tables). No external addenda/errata check was performed for this tranche; per-row scope is recorded in each row's notes.

## Counts

| Dimension | Value | Count |
|---|---|---|
| Priority | P0 | 15 |
| Priority | P1 | 18 |
| Priority | P2 | 5 |
| Priority | P3 | 4 |
| Status | deferred-pending-owner-decision | 2 |
| Status | implementation-present-needs-conformance-tests | 10 |
| Status | implementation-present-needs-negative-tests | 6 |
| Status | implementation-present-needs-platform-tests | 1 |
| Status | implementation-present-needs-security-tests | 1 |
| Status | implementation-present-needs-source-review | 2 |
| Status | implementation-present-needs-state-machine-audit | 3 |
| Status | implementation-present-needs-timeout-tests | 1 |
| Status | implementation-present-needs-window-tests | 1 |
| Status | in-progress | 3 |
| Status | supported-with-clause-evidence | 8 |
| Status | unknown-pending-source-review | 4 |

## Ledger Rows

| ID | Anchor | Priority | Status | Public Claims |
|---|---|---|---|---|
| `BACNET-4-ARCHITECTURE` | Clause 4 | P2 | implementation-present-needs-source-review | 2 |
| `BACNET-5-TSM-CLIENT` | Clause 5.4.4 | P1 | implementation-present-needs-state-machine-audit | 2 |
| `BACNET-5-TSM-SERVER` | Clause 5.4.5 | P1 | implementation-present-needs-state-machine-audit | 1 |
| `BACNET-5-SEGMENTATION-WINDOW` | Clauses 5.2-5.4 | P1 | implementation-present-needs-window-tests | 1 |
| `BACNET-6-NPDU-CONTROL` | Clause 6.2 | P1 | implementation-present-needs-negative-tests | 1 |
| `BACNET-6-ROUTER-MESSAGES` | Clauses 6.4-6.6 | P1 | implementation-present-needs-conformance-tests | 1 |
| `BACNET-7-ETHERNET-LLC` | Clause 7 | P2 | implementation-present-needs-platform-tests | 2 |
| `BACNET-8-ARCNET` | Clause 8 | P3 | unknown-pending-source-review | 0 |
| `BACNET-9-MSTP-FRAMES` | Clause 9.3 | P2 | implementation-present-needs-source-review | 2 |
| `BACNET-10-PTP` | Clause 10 | P3 | unknown-pending-source-review | 0 |
| `BACNET-11-LONTALK` | Clause 11 | P3 | unknown-pending-source-review | 0 |
| `BACNET-12-OBJECT-MODEL` | Clauses 12-19 | P1 | implementation-present-needs-conformance-tests | 3 |
| `BACNET-12-RECIPIENT-LIST-FRAMING` | Clause 12.21, Clause 21 | P1 | supported-with-clause-evidence | 1 |
| `BACNET-12-EVENT-PARAMETERS-FRAMING` | Clause 12.12, Clause 21 | P1 | supported-with-clause-evidence | 1 |
| `BACNET-12-OOS-RELIABILITY-WRITABILITY` | Clause 12.17 Table 12-20 footnote 7 (Loop); Clause 12 Out_Of_Service property texts (12.2/12.3/12.4/12.6/12.7/12.8/12.19/12.21/12.22 families); Clause 12.24 Schedule Reliability_Evaluation_Inhibit text; Clause 12.25 Table 12-29 and Clause 12.30 Table 12-35 (Trend Log / Trend Log Multiple); Clause 21 BACnetReliability | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-RELINQUISH-DEFAULT-WRITABILITY` | Clause 12.3 Table 12-3 (Analog Output), Clause 12.7 Table 12-8 (Binary Output), Clause 12.8 Table 12-10 (Binary Value), Clause 12.19 Table 12-22 (Multi-state Output), Clause 12.20 Table 12-23 (Multi-state Value), Clause 12.26 Table 12-30 (Access Door), Clause 12.54 Table 12-64 (Lighting Output), Clause 12.55 Table 12-69 (Binary Lighting Output), Clause 12 value object tables; Clause 19 command prioritization | P1 | supported-with-clause-evidence | 0 |
| `BACNET-13-COV-SUBSCRIPTIONS` | Clauses 13.14-13.18 | P1 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-15-ARRAY-INDEX-GATING` | Clause 15.5.1.3, Clause 15.9.1.3 (with Clause 12.1.5) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-15-WP-EVENT-FIELD-VALIDATION` | Clause 15.9.1.3 (WriteProperty error table) with Clause 21 BACnetNotifyType / BACnetEventTransitionBits / BACnetLimitEnable productions | P1 | supported-with-clause-evidence | 0 |
| `BACNET-20-ENCODING` | Clause 20 | P1 | implementation-present-needs-negative-tests | 2 |
| `BACNET-21-FORMAL-APDUS` | Clause 21 | P1 | implementation-present-needs-conformance-tests | 2 |
| `BACNET-21-TIMESTAMP-CHOICE` | Clause 21 (BACnetTimeStamp), Clause 20.2.1.5 | P1 | supported-with-clause-evidence | 1 |
| `BACNET-A-PICS` | Annex A | P1 | in-progress | 2 |
| `BACNET-J-BVLC-FUNCTION-CODES` | Annex J.2 | P0 | implementation-present-needs-conformance-tests | 2 |
| `BACNET-J-ORIGINAL-UNICAST-NPDU` | Annex J | P0 | implementation-present-needs-negative-tests | 1 |
| `BACNET-J-ORIGINAL-BROADCAST-NPDU` | Annex J | P0 | implementation-present-needs-negative-tests | 1 |
| `BACNET-J-FORWARDED-NPDU` | Annex J | P0 | implementation-present-needs-negative-tests | 2 |
| `BACNET-J-BBMD-BDT` | Annex J.4/J.5 | P0 | implementation-present-needs-conformance-tests | 3 |
| `BACNET-J-FOREIGN-DEVICE-FDT` | Annex J.5 | P0 | implementation-present-needs-conformance-tests | 1 |
| `BACNET-J-NAT-TRAVERSAL` | Annex J.7.5 | P0 | deferred-pending-owner-decision | 0 |
| `BACNET-J-IP-MULTICAST` | Annex J.8 | P0 | deferred-pending-owner-decision | 0 |
| `BACNET-K-BIBBS` | Annex K | P1 | in-progress | 0 |
| `BACNET-L-PROFILES` | Annex L | P2 | in-progress | 0 |
| `BACNET-O-ZIGBEE` | Annex O | P3 | unknown-pending-source-review | 0 |
| `BACNET-U-IPV6-BVLL` | Annex U | P2 | implementation-present-needs-conformance-tests | 2 |
| `BACNET-AB-SC-FRAME` | Annex AB.2 | P0 | implementation-present-needs-negative-tests | 1 |
| `BACNET-AB-SC-BVLC-RESULT` | Annex AB.2.4 | P0 | implementation-present-needs-conformance-tests | 1 |
| `BACNET-AB-SC-DATA-ATTRIBUTES` | Annex AB.3.4 | P0 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-AB-SC-CONNECTION-STATE` | Annex AB.6.2 | P0 | implementation-present-needs-state-machine-audit | 1 |
| `BACNET-AB-SC-HUB-CONNECTOR` | Annex AB.5 | P0 | supported-with-clause-evidence | 2 |
| `BACNET-AB-SC-WEBSOCKET-TLS` | Annex AB.7 | P0 | implementation-present-needs-security-tests | 2 |
| `BACNET-AB-SC-HEARTBEAT` | Annex AB.6.3 | P0 | implementation-present-needs-timeout-tests | 2 |

## Follow-Up Source

Rows not marked `supported-with-clause-evidence` are the initial follow-up backlog. Later PRs should split broad family rows into smaller clause-backed rows before strengthening public support claims.
