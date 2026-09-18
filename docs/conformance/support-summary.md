# BACnet Standard 135-2020 Support Summary

> DRAFT internal support evidence. Generated from `docs/conformance/bacnet-135-2020.json`; this is not a BTL certification claim or formal PICS/BIBB declaration.

- Standard: ANSI/ASHRAE Standard 135-2020
- Reviewed at: 2026-09-17
- Implementation evidence SHA reviewed: `b4c845caf920db279b0aefbd2824ac1348bba1cb`
- Scope: RB-01 baseline reconciliation (R0 working order) at dev b4c845c: the corrected-2020 target is ANSI/ASHRAE 135-2020 plus the 2024-04-29 Errata Summary for the supported subset. Owner-approved decisions recorded here: (1) corrected 2020 baseline (135-2020 + 2024-04-29 errata for the supported subset; optional later addenda and external qualification remain separate); (2) no CP authenticated-origin expansion — baseline keeps unknown-origin plus hardened denial; (3) MS/TP is non-routing standard-frame only (MAX_STANDARD_MPDU_DATA 501, crates/bacnet-transport/src/mstp_frame.rs:22, enforced crates/bacnet-transport/src/mstp/mod.rs:420-427 and 690-697); extended-frame/COBS routing is not claimed. The Audit query corrected contract (filter BOOLEAN to BACnetSuccessFilter, cursor Unsigned32 to Unsigned64) is recorded in BACNET-13-AUDIT-WIRE-MODELS; the RB-02 codec migration and the RB-20 runtime/Python migration are done, and #345 stays open for reporting/forwarding (RB-21/22). Per-feature status stays separate from any whole-product Protocol_Revision claim; no Protocol_Revision or workspace version change (workspace stays 0.11.0). Preserved prior owner decisions: #431 transport-neutral composition above sibling roles (not the older B/IP-only facade); #195 publish bacnet-cli in the next release (bacnet-cli is publishable but still omitted from CI publish-crates — delivery gap remains open). No protocol code changes in this tranche.
- Addenda/errata: ASHRAE 135-2020 Errata Summary 2024-04-29 (v1) reviewed for the supported subset. Item 7 (Clause 21.6, p. 886): successful-actions-only corrected from BOOLEAN (struck through, removed) to BACnetSuccessFilter (italic, added), tags [7] (by-target) and [4] (by-source). Item 8 (Clause 21.2.3, p. 865): start-at-sequence-number corrected from Unsigned32 (struck through, removed) to Unsigned64 (italic, added), tag [2] OPTIONAL. Both items visually verified from the rendered errata p. 3 under the p. 1 convention (strikeout = removed, italics = added); not inferred from concatenated text extraction. This visual verification closes the RB-02 wire-type hold noted in the release plan. The implementation encodes the corrected filter/u64 contract, so BACNET-13-AUDIT-WIRE-MODELS keeps implementation-present-needs-source-review after the RB-02 codec migration and the RB-20 runtime/Python migration pending broader Audit review. Optional later addenda and external qualification remain separate; no whole-product Protocol_Revision claim.

## Counts

| Dimension | Value | Count |
|---|---|---|
| Priority | P0 | 16 |
| Priority | P1 | 43 |
| Priority | P2 | 5 |
| Priority | P3 | 4 |
| Status | deferred-pending-owner-decision | 2 |
| Status | implementation-present-needs-conformance-tests | 14 |
| Status | implementation-present-needs-negative-tests | 6 |
| Status | implementation-present-needs-platform-tests | 1 |
| Status | implementation-present-needs-security-tests | 1 |
| Status | implementation-present-needs-source-review | 3 |
| Status | implementation-present-needs-state-machine-audit | 4 |
| Status | implementation-present-needs-timeout-tests | 1 |
| Status | implementation-present-needs-window-tests | 1 |
| Status | in-progress | 9 |
| Status | supported-with-clause-evidence | 19 |
| Status | unknown-pending-source-review | 4 |
| Status | unsupported-by-design | 3 |

## Ledger Rows

| ID | Anchor | Priority | Status | Public Claims |
|---|---|---|---|---|
| `BACNET-4-ARCHITECTURE` | Clause 4 | P2 | implementation-present-needs-source-review | 2 |
| `BACNET-5-TSM-CLIENT` | Clause 5.4.4 | P1 | implementation-present-needs-state-machine-audit | 2 |
| `BACNET-5-TSM-SERVER` | Clause 5.4.5 | P1 | implementation-present-needs-state-machine-audit | 1 |
| `BACNET-5-SEGMENTATION-WINDOW` | Clauses 5.2-5.4 | P1 | implementation-present-needs-window-tests | 1 |
| `BACNET-5-ROUTED-PATH-LIMIT` | Clauses 5.2.1.2, 6.4.4, and 19.4 | P1 | implementation-present-needs-conformance-tests | 1 |
| `BACNET-6-NPDU-CONTROL` | Clause 6.2 | P1 | implementation-present-needs-negative-tests | 1 |
| `BACNET-6-ROUTER-MESSAGES` | Clauses 6.4-6.6 | P1 | implementation-present-needs-conformance-tests | 1 |
| `BACNET-7-ETHERNET-LLC` | Clause 7 | P2 | implementation-present-needs-platform-tests | 2 |
| `BACNET-8-ARCNET` | Clause 8 | P3 | unknown-pending-source-review | 0 |
| `BACNET-9-MSTP-FRAMES` | Clause 9.3 | P2 | implementation-present-needs-source-review | 2 |
| `BACNET-10-PTP` | Clause 10 | P3 | unknown-pending-source-review | 0 |
| `BACNET-11-LONTALK` | Clause 11 | P3 | unknown-pending-source-review | 0 |
| `BACNET-12-OBJECT-MODEL` | Clauses 12-19 | P1 | implementation-present-needs-conformance-tests | 3 |
| `BACNET-12-LOG-RECORD-IDENTITY` | Clause 12.25 (p. 319), Clause 12.27 (p. 337), Clause 12.30 (p. 361); Clause 15.8 (pp. 745-750); Clause 21.6 (pp. 902-914) | P1 | in-progress | 0 |
| `BACNET-12-LOG-STATUS-LIFECYCLE` | Clause 12.25 Trend Log (pp. 319-336); Clause 12.27 Event Log (pp. 337-346); Clause 12.30 Trend Log Multiple (pp. 361-378); Clause 21.6 BACnetLogStatus (p. 914) | P1 | in-progress | 0 |
| `BACNET-15-READ-RANGE-LOG-BUFFER` | Clause 12.1.5.2 (PDF p. 164 / printed p. 162); Clause 12.27 (PDF pp. 337-340 / printed pp. 335-338); Clause 15.8 and Clause 15.8.1.1.4-15.8.1.3 (PDF pp. 747-753 / printed pp. 745-751); Clause 21.6 BACnetEventLogRecord, BACnetLogRecord, and BACnetLogMultipleRecord (PDF pp. 906, 916) | P1 | in-progress | 0 |
| `BACNET-12-PROPERTY-METADATA-CORE` | Clause 12.6, Table 12-6 (pp. 189-190); Clause 12.42, Table 12-49 (pp. 444-445); Clause 15.7.3.1 (p. 743); Annex A (pp. 964-965) | P1 | in-progress | 1 |
| `BACNET-12-ESCALATOR-STATUS-WRITABILITY` | Clause 12 general property conformance rules; Clause 12.60 Table 12-78 and Out_Of_Service; Clause 15.9.1.3; Clause 21 BACnetEscalatorMode, BACnetEscalatorOperationDirection, and BACnetEscalatorFault; Clause 23.1 | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-DEVICE-MAX-SEGMENTS` | Clause 12.11, Table 12-13 | P1 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-12-DEVICE-ACTIVE-COV-SUBSCRIPTIONS` | Clause 12.11, Table 12-13 and 12.11.31; Clause 12.1.5.2; Clauses 20 and 21 BACnetCOVSubscription, BACnetRecipientProcess, BACnetRecipient, BACnetObjectPropertyReference, ReadProperty-ACK, and ReadAccessResult productions | P1 | in-progress | 0 |
| `BACNET-12-NOTIFICATION-FORWARDER-WITHDRAWAL` | Clause 12.51 (pp. 497-503), Table 12-58 (p. 500); Clause 13.2.5.1 (p. 643); Clause 21 BACnetEventNotificationSubscription and BACnetProcessIdSelection productions (pp. 904, 924) | P1 | unsupported-by-design | 0 |
| `BACNET-12-CHANNEL-WITHDRAWAL` | Clause 12.53 (pp. 508-517), Table 12-62 (pp. 509-510) | P1 | unsupported-by-design | 0 |
| `BACNET-15-WRITEGROUP-SERVER-WITHDRAWAL` | Clause 15.11 (pp. 757-758); Clause 19.2.1.6 (p. 809) | P1 | unsupported-by-design | 0 |
| `BACNET-12-RECIPIENT-LIST-FRAMING` | Clause 12.21, Clause 21 | P1 | supported-with-clause-evidence | 1 |
| `BACNET-12-EVENT-PARAMETERS-FRAMING` | Clause 12.12, Clause 21 | P1 | supported-with-clause-evidence | 1 |
| `BACNET-12-OOS-RELIABILITY-WRITABILITY` | Clause 12.17 Table 12-20 footnote 7 (Loop); Clause 12 Out_Of_Service property texts (12.2/12.3/12.4/12.6/12.7/12.8/12.19/12.21/12.22 families); Clause 12.24 Schedule Reliability_Evaluation_Inhibit text; Clause 12.25 Table 12-29 and Clause 12.30 Table 12-35 (Trend Log / Trend Log Multiple); Clause 21 BACnetReliability | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-RELINQUISH-DEFAULT-WRITABILITY` | Clause 12.3 Table 12-3 (Analog Output), Clause 12.7 Table 12-8 (Binary Output), Clause 12.8 Table 12-10 (Binary Value), Clause 12.19 Table 12-22 (Multi-state Output), Clause 12.20 Table 12-23 (Multi-state Value), Clause 12.26 Table 12-30 (Access Door), Clause 12.54 Table 12-64 (Lighting Output), Clause 12.55 Table 12-69 (Binary Lighting Output), Clause 12 value object tables; Clause 19 command prioritization | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-BINARY-LIGHTING-OPERATIONS` | Clause 12.55 and Table 12-70, including 12.55.4.1 and 12.55.10.1; Clause 19.2 command prioritization; Clause 21 BACnetBinaryLightingPV | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-REFERENCE-PROPERTY-WRITABILITY` | Clause 12.17 with Table 12-20 (Loop), Clause 12.23 with Table 12-27 (Pulse Converter Input_Reference), Clause 12.5 Table 12-5 (Averaging Object_Property_Reference - BACnetDeviceObjectPropertyReference), Clause 21 BACnetObjectPropertyReference / BACnetSetpointReference productions | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-ALERT-ENROLLMENT-TABLE-12-61` | Clause 12.52 and Table 12-61; Clause 21 BACnetNotifyType; Clause 15.7 ReadPropertyMultiple | P1 | supported-with-clause-evidence | 4 |
| `BACNET-12-ENROLLMENT-EVENT-TIME-STAMPS` | Clause 12.12 Table 12-14; Clause 12.52 Table 12-61; Clause 12.1.5.1; Clause 13.2.2.1; Clause 21 BACnetTimeStamp | P1 | implementation-present-needs-state-machine-audit | 0 |
| `BACNET-12-TIME-DELAY-NORMAL` | Clause 13.3.2 CHANGE_OF_STATE, Clause 13.3.4 COMMAND_FAILURE, Clause 13.3.6 OUT_OF_RANGE (pTimeDelayNormal definitions and condition letters); Clause 12.2 Table 12-2 (Analog Input, O5), 12.3 Table 12-3 (Analog Output, O4), 12.4 Table 12-4 (Analog Value, O6), 12.6 Table 12-6 (Binary Input, O7), 12.7 Table 12-8 (Binary Output, O6), 12.8 Table 12-10 (Binary Value, O8), 12.18 Table 12-21 (Multi-state Input, O5), 12.19 Table 12-22 (Multi-state Output, O3), 12.20 Table 12-23 (Multi-state Value, O6) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-13-COV-SUBSCRIPTIONS` | Clauses 13.14-13.18 | P1 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-13-AUDIT-WIRE-MODELS` | Clauses 13.19-13.21; Clause 21.2.1, Clause 21.2.3, Clause 21.3.1, BACnetAuditNotification, BACnetAuditLogQueryParameters, and BACnetAuditOperationFlags productions | P1 | implementation-present-needs-source-review | 0 |
| `BACNET-15-ARRAY-INDEX-GATING` | Clause 15.5.1.3, Clause 15.9.1.3 (with Clause 12.1.5) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-15-WP-EVENT-FIELD-VALIDATION` | Clause 15.9.1.3 (WriteProperty error table) with Clause 21 BACnetNotifyType / BACnetEventTransitionBits / BACnetLimitEnable productions | P1 | supported-with-clause-evidence | 0 |
| `BACNET-15-STRUCTURED-WRITE-DECODE` | Clause 15.9 WriteProperty (15.9.1.2 Result(+), 15.9.1.3 Result(-)), Clause 15.10 WritePropertyMultiple, Clause 20.2.1 (concatenated elements) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-15-WPM-ORDERED-PREFIX-ERROR` | Clause 15.10 and 15.10.1.3 (WritePropertyMultiple service procedure and Result(-)); Clause 18.9 (Reject reasons); Clause 21 (Error and BACnetObjectPropertyReference productions) | P1 | supported-with-clause-evidence | 0 |
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
| `BACNET-13-EVENT-DISABLE-WPM-PREFIX` | Clause 12.52 Table 12-61 (Alert Enrollment Event_State, Acked_Transitions, Event_Detection_Enable); Clause 13.2.2.1 disabled-state initial conditions; Clause 13.3 pTimeDelayNormal fallback; Clause 15.10 ordered WritePropertyMultiple procedure | P1 | in-progress | 0 |
| `BACNET-13-EVENT-ENROLLMENT-EVALUATOR` | Clause 12.12 with Table 12-14 (Event Enrollment Object Type; configured Event_Type excludes CHANGE_OF_RELIABILITY; Time_Delay_Normal Unsigned, conformance O, extract 16444-16446, property text 16887-16889); Clause 12.12 Table 12-15 (Time_Delay -> pTimeDelay mapping for every evaluated algorithm); Clause 13.2.2.1 (changed nonzero Reliability while FAULT is a To-Fault re-entry); Clause 13.2.2.1.4 (transition actions incl. the same-state rule); Clause 13.2.3 (Acked_Transitions on a received transition); Clause 13.2.5.2 Table 13-3 and Clause 13.2.5.3 (CHANGE_OF_RELIABILITY whenever From or To is FAULT); Clause 13.3 common introduction and 13.3.1/13.3.2/13.3.3/13.3.5/13.3.6 direction rules with the pTimeDelayNormal fallback; Figure 13-10 | P1 | supported-with-clause-evidence | 0 |
| `BACNET-13-ACKED-TRANSITIONS-NETWORK-OWNERSHIP` | Clause 12.1.2; Acked_Transitions property paragraphs and Tables 12-2 (Analog Input), 12-3 (Analog Output), 12-4 (Analog Value), 12-6 (Binary Input), 12-8 (Binary Output), 12-10 (Binary Value), 12-14 (Event Enrollment), 12-21 (Multi-state Input), 12-22 (Multi-state Output), 12-23 (Multi-state Value), and 12-61 (Alert Enrollment); Clauses 13.2.3, 13.2.5, and 13.5; Clauses 15.9 and 15.10; Annex K Table K-17 footnote 1 | P1 | supported-with-clause-evidence | 0 |
| `BACNET-13-EVENT-ENROLLMENT-NOTIFICATION-LIFECYCLE` | Event Enrollment object and event-reporting lifecycle requirements for configured event type, transition enablement, Notification Class policy, committed acknowledgment/timestamp/message history, same-state indication, distribution gating, and committed notification fields | P1 | implementation-present-needs-conformance-tests | 0 |
| `BACNET-13-LIFE-SAFETY-OPERATION` | LifeSafetyOperation service procedures and errors; Life Safety COV reporting; Life Safety Point/Zone Present_Value, Status_Flags, Tracking_Value, Silenced, and Operation_Expected property requirements | P0 | implementation-present-needs-conformance-tests | 1 |
| `BACNET-14-FILE-ACCESS-METHOD` | Clause 12.13 (Table 12-16 File_Access_Method), Clauses 14.1 and 14.2 (Incorrect File access method), Clauses 14.1.4.1 and 14.2.4.1 (non-File Object Identifier), Clause 14.2.4.1 (Write to a read-only File), Clause 18 (INVALID_FILE_ACCESS_METHOD, FILE_ACCESS_DENIED, and INCONSISTENT_OBJECT_TYPE), Clause 21 (BACnetFileAccessMethod production; AtomicReadFile/AtomicWriteFile access-method CHOICE) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-14-FILE-STORAGE` | Clause 12.13 / Table 12-16 and footnotes 1-2 (File object property model; File_Size stream-only conditional writability; Record_Count record-only presence and conditional writability; truncate, clear, local-matter expansion fill; Modification_Date events and Archive reset), Clause 14.1 (AtomicReadFile parameters, End Of File, Service Procedure), Clause 14.2 (AtomicWriteFile parameters, -1 append, Service Procedure, Result(+) position), Clauses 14.1.4.1 and 14.2.4.1 (INVALID_FILE_START_POSITION), Clause 14.2.4.1 (FILE_FULL), Clause 18 (INVALID_FILE_START_POSITION, FILE_FULL, FILE_ACCESS_DENIED), Clause 19.1.3.3 (restore context writes File_Size=0 before rewriting a differently-sized stream configuration file), Clause 21 (Date, Time, and BACnetDateTime), Annex F (AtomicWriteFile append example) | P1 | supported-with-clause-evidence | 0 |

## Follow-Up Source

Rows not marked `supported-with-clause-evidence` are the initial follow-up backlog. Later PRs should split broad family rows into smaller clause-backed rows before strengthening public support claims.
