# BACnet Standard 135-2020 Support Summary

> DRAFT internal support evidence. Generated from `docs/conformance/bacnet-135-2020.json`; this is not a BTL certification claim or formal PICS/BIBB declaration.

- Standard: ANSI/ASHRAE Standard 135-2020
- Reviewed at: 2026-08-13
- Implementation evidence SHA reviewed: `f485021f5cd7058ac406d57d3d317936cbe7b361`
- Scope: Event-state disabled reset and lossless WPM rollback tranche (branch codex/c3-event-reset-rollback, tranche C3; issues #205, #209, #289): Alert Enrollment now exposes Event_State and Acked_Transitions and applies their Clause 13.2.2.1 initial conditions while Event_Detection_Enable is FALSE, including guarded internal updates and projection after direct assignment to the existing public flag. This is partial Alert Enrollment evidence, not a complete Table 12-61 claim; #264 and #291 track the remaining property-model gaps. The repository's WPM rollback policy now uses object-owned tokens when property readback is not state-equivalent. Covered state includes event detection and history, raw Time_Delay_Normal storage, Network Port Changes_Pending, Access Door command slots, and log buffers cleared through Record_Count. Restoration failures are returned instead of being hidden in tracing. Clause 15.10 permits preceding successful writes to remain applied and does not require this policy. Fences: no Alert Enrollment evaluator, notification sending (#127), Event Enrollment history properties (#264), complete Alert Enrollment property model (#291), or DCC behavior (#220).
- Addenda/errata: No external addenda/errata check was performed for this tranche. The local Standard 135-2020 source was reviewed for Clause 13.2.2.1's disabled-state initial conditions, the Clause 13.3 pTimeDelayNormal fallback, Alert Enrollment Table 12-61's required event-state properties, and Clause 15.10's ordered partial-success procedure. The rollback behavior is identified as repository policy, not a conformance requirement.

## Counts

| Dimension | Value | Count |
|---|---|---|
| Priority | P0 | 16 |
| Priority | P1 | 32 |
| Priority | P2 | 5 |
| Priority | P3 | 4 |
| Status | deferred-pending-owner-decision | 2 |
| Status | implementation-present-needs-conformance-tests | 13 |
| Status | implementation-present-needs-negative-tests | 6 |
| Status | implementation-present-needs-platform-tests | 1 |
| Status | implementation-present-needs-security-tests | 1 |
| Status | implementation-present-needs-source-review | 3 |
| Status | implementation-present-needs-state-machine-audit | 3 |
| Status | implementation-present-needs-timeout-tests | 1 |
| Status | implementation-present-needs-window-tests | 1 |
| Status | in-progress | 4 |
| Status | supported-with-clause-evidence | 15 |
| Status | unknown-pending-source-review | 4 |
| Status | unsupported-by-design | 3 |

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
| `BACNET-12-PROPERTY-METADATA-CORE` | Clause 12.6, Table 12-6 (pp. 189-190); Clause 12.42, Table 12-49 (pp. 444-445); Clause 15.7.3.1 (p. 743); Annex A (pp. 964-965) | P1 | in-progress | 1 |
| `BACNET-12-ESCALATOR-STATUS-WRITABILITY` | Clause 12 general property conformance rules; Clause 12.60 Table 12-78 and Out_Of_Service; Clause 15.9.1.3; Clause 21 BACnetEscalatorMode, BACnetEscalatorOperationDirection, and BACnetEscalatorFault; Clause 23.1 | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-DEVICE-MAX-SEGMENTS` | Clause 12.11, Table 12-13 | P1 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-12-NOTIFICATION-FORWARDER-WITHDRAWAL` | Clause 12.51 (pp. 497-503), Table 12-58 (p. 500); Clause 13.2.5.1 (p. 643); Clause 21 BACnetEventNotificationSubscription and BACnetProcessIdSelection productions (pp. 904, 924) | P1 | unsupported-by-design | 0 |
| `BACNET-12-CHANNEL-WITHDRAWAL` | Clause 12.53 (pp. 508-517), Table 12-62 (pp. 509-510) | P1 | unsupported-by-design | 0 |
| `BACNET-15-WRITEGROUP-SERVER-WITHDRAWAL` | Clause 15.11 (pp. 757-758); Clause 19.2.1.6 (p. 809) | P1 | unsupported-by-design | 0 |
| `BACNET-12-RECIPIENT-LIST-FRAMING` | Clause 12.21, Clause 21 | P1 | supported-with-clause-evidence | 1 |
| `BACNET-12-EVENT-PARAMETERS-FRAMING` | Clause 12.12, Clause 21 | P1 | supported-with-clause-evidence | 1 |
| `BACNET-12-OOS-RELIABILITY-WRITABILITY` | Clause 12.17 Table 12-20 footnote 7 (Loop); Clause 12 Out_Of_Service property texts (12.2/12.3/12.4/12.6/12.7/12.8/12.19/12.21/12.22 families); Clause 12.24 Schedule Reliability_Evaluation_Inhibit text; Clause 12.25 Table 12-29 and Clause 12.30 Table 12-35 (Trend Log / Trend Log Multiple); Clause 21 BACnetReliability | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-RELINQUISH-DEFAULT-WRITABILITY` | Clause 12.3 Table 12-3 (Analog Output), Clause 12.7 Table 12-8 (Binary Output), Clause 12.8 Table 12-10 (Binary Value), Clause 12.19 Table 12-22 (Multi-state Output), Clause 12.20 Table 12-23 (Multi-state Value), Clause 12.26 Table 12-30 (Access Door), Clause 12.54 Table 12-64 (Lighting Output), Clause 12.55 Table 12-69 (Binary Lighting Output), Clause 12 value object tables; Clause 19 command prioritization | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-REFERENCE-PROPERTY-WRITABILITY` | Clause 12.17 with Table 12-20 (Loop), Clause 12.23 with Table 12-27 (Pulse Converter Input_Reference), Clause 12.5 Table 12-5 (Averaging Object_Property_Reference - BACnetDeviceObjectPropertyReference), Clause 21 BACnetObjectPropertyReference / BACnetSetpointReference productions | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-TIME-DELAY-NORMAL` | Clause 13.3.2 CHANGE_OF_STATE, Clause 13.3.4 COMMAND_FAILURE, Clause 13.3.6 OUT_OF_RANGE (pTimeDelayNormal definitions and condition letters); Clause 12.2 Table 12-2 (Analog Input, O5), 12.3 Table 12-3 (Analog Output, O4), 12.4 Table 12-4 (Analog Value, O6), 12.6 Table 12-6 (Binary Input, O7), 12.7 Table 12-8 (Binary Output, O6), 12.8 Table 12-10 (Binary Value, O8), 12.18 Table 12-21 (Multi-state Input, O5), 12.19 Table 12-22 (Multi-state Output, O3), 12.20 Table 12-23 (Multi-state Value, O6) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-13-COV-SUBSCRIPTIONS` | Clauses 13.14-13.18 | P1 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-13-AUDIT-WIRE-MODELS` | Clauses 13.19-13.21; Clause 21.2.1, Clause 21.2.3, Clause 21.3.1, BACnetAuditNotification, BACnetAuditLogQueryParameters, and BACnetAuditOperationFlags productions | P1 | implementation-present-needs-source-review | 0 |
| `BACNET-15-ARRAY-INDEX-GATING` | Clause 15.5.1.3, Clause 15.9.1.3 (with Clause 12.1.5) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-15-WP-EVENT-FIELD-VALIDATION` | Clause 15.9.1.3 (WriteProperty error table) with Clause 21 BACnetNotifyType / BACnetEventTransitionBits / BACnetLimitEnable productions | P1 | supported-with-clause-evidence | 0 |
| `BACNET-15-STRUCTURED-WRITE-DECODE` | Clause 15.9 WriteProperty (15.9.1.2 Result(+), 15.9.1.3 Result(-)), Clause 15.10 WritePropertyMultiple, Clause 20.2.1 (concatenated elements) | P1 | supported-with-clause-evidence | 0 |
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
| `BACNET-13-EVENT-DISABLE-WPM-ROLLBACK` | Clause 12.52 Table 12-61 (Alert Enrollment Event_State, Acked_Transitions, Event_Detection_Enable); Clause 13.2.2.1 disabled-state initial conditions; Clause 13.3 pTimeDelayNormal fallback; Clause 15.10 ordered WritePropertyMultiple procedure | P1 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-13-EVENT-ENROLLMENT-EVALUATOR` | Clause 12.12 with Table 12-14 (Event Enrollment Object Type; Time_Delay_Normal Unsigned, conformance O, extract 16444-16446, property text 16887-16889); Clause 12.12 Table 12-15 (Time_Delay -> pTimeDelay mapping for every evaluated algorithm); Clause 13.2.2.1.4 (transition actions incl. the same-state rule); Clause 13.2.3 (Acked_Transitions on a received transition); Clause 13.3 common introduction and 13.3.1/13.3.2/13.3.3/13.3.5/13.3.6 direction rules with the pTimeDelayNormal fallback; Figure 13-10 | P1 | supported-with-clause-evidence | 0 |
| `BACNET-13-LIFE-SAFETY-OPERATION` | Clause 13.13; Clauses 12.15 and 12.16 Silenced and Operation_Expected properties; Clause 18 errors | P0 | implementation-present-needs-conformance-tests | 1 |
| `BACNET-14-FILE-ACCESS-METHOD` | Clause 12.13 (Table 12-16 File_Access_Method), Clauses 14.1 and 14.2 (Incorrect File access method), Clauses 14.1.4.1 and 14.2.4.1 (non-File Object Identifier), Clause 14.2.4.1 (Write to a read-only File), Clause 18 (INVALID_FILE_ACCESS_METHOD, FILE_ACCESS_DENIED, and INCONSISTENT_OBJECT_TYPE), Clause 21 (BACnetFileAccessMethod production; AtomicReadFile/AtomicWriteFile access-method CHOICE) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-14-FILE-STORAGE` | Clause 12.13 / Table 12-16 (File object property model; no File Data property; File_Size and Record_Count), Clause 14.1 (AtomicReadFile parameters, End Of File, Service Procedure), Clause 14.2 (AtomicWriteFile parameters, -1 append, Service Procedure, Result(+) position), Clauses 14.1.4.1 and 14.2.4.1 (INVALID_FILE_START_POSITION), Clause 14.2.4.1 (FILE_FULL), Clause 18 (INVALID_FILE_START_POSITION, FILE_FULL, FILE_ACCESS_DENIED), Annex F (AtomicWriteFile append example) | P1 | supported-with-clause-evidence | 0 |

## Follow-Up Source

Rows not marked `supported-with-clause-evidence` are the initial follow-up backlog. Later PRs should split broad family rows into smaller clause-backed rows before strengthening public support claims.
