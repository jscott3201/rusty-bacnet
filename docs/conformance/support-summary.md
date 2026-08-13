# BACnet Standard 135-2020 Support Summary

> DRAFT internal support evidence. Generated from `docs/conformance/bacnet-135-2020.json`; this is not a BTL certification claim or formal PICS/BIBB declaration.

- Standard: ANSI/ASHRAE Standard 135-2020
- Reviewed at: 2026-08-12
- Implementation evidence SHA reviewed: `8680203e8cb986afb58d0ad53b84eb75aec0959a`
- Scope: Event Enrollment evaluator tranche (branch codex/ee-evaluator-delays, tranche C2; issues #163, #166, #137): the EE evaluator honors Time_Delay / Time_Delay_Normal per Clause 13.3's direction split and Table 12-15's pTimeDelay mapping (with the EE object gaining the optional Table 12-14 Time_Delay_Normal property), executes Clause 13.2.2.1.4's transition actions for same-state transitions too (specific Event_State stored; Acked_Transitions maintained per Clause 13.2.3 from the referenced Notification Class's Ack_Required; Event_Enable stays distribution-scoped), and tracks the Clause 13.3.3 CHANGE_OF_VALUE detection baseline (first-sample initialization is the clause's local matter and never indicates; the only indication is NORMAL->NORMAL per Figure 13-10). CHANGE_OF_STATE condition (c) is implemented (last-offnormal-causing value retained per enrollment); CHANGE_OF_BITSTRING condition (c) deliberately not. Object-owned in-memory evaluation state (pending countdown, baseline, causal value) sits behind guarded internal trait methods; the detection-disabled reset clears it. Fences: no notification sending (#127), no Event_Time_Stamps/Event_Message_Texts (#264), no Status_Flags algorithm input (Table 12-15.1, follow-up), no intrinsic-detector changes. Conventions unchanged: repo_sha names the last code commit of the PR branch (this docs-refresh commit lands after it), matching the previous tranches.
- Addenda/errata: No external addenda/errata check was performed for this tranche. Local source _spec/2020_ASHRAE_Standard-135-BACnet-Data-Communication-Protocol.pdf (text extract) was reviewed for: the Time_Delay_Normal row of Table 12-14 (conformance O, read in name/type/conformance triples adjacent to the O1 Event_Algorithm_Inhibit and R Status_Flags rows because of the extract's two-column page-break interleave) and its Clause 12.12 property text; Table 12-15's Time_Delay -> pTimeDelay mapping for all five evaluated algorithms; Clauses 13.2.2.1.4 (transition actions incl. same-state and the specific-state requirement), 13.2.3 (Acked_Transitions on a received transition), and 13.2.4 (Time_Delay as debounce); the Clause 13.3 common introduction (condition ordering; 'If no condition evaluates to true, then no transition'); the per-algorithm pTimeDelay/pTimeDelayNormal texts, condition letters, and the fallback sentence (13.3.1/13.3.2/13.3.3/13.3.5/13.3.6); CHANGE_OF_VALUE's baseline sentence and Figure 13-10; Clause 13.9's AcknowledgeAlarm parameter semantics with Table 13-10's Result(-) situation-to-error mapping (NO_ALARM_CONFIGURED / UNKNOWN_OBJECT / INVALID_EVENT_STATE); and the seconds phrasing of the delay parameters ('the time, in seconds, that the offnormal conditions must exist'). Per-row scope is recorded in each row's notes.

## Counts

| Dimension | Value | Count |
|---|---|---|
| Priority | P0 | 15 |
| Priority | P1 | 22 |
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
| Status | supported-with-clause-evidence | 12 |
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
| `BACNET-12-REFERENCE-PROPERTY-WRITABILITY` | Clause 12.17 with Table 12-20 (Loop), Clause 12.23 with Table 12-27 (Pulse Converter Input_Reference), Clause 12.5 Table 12-5 (Averaging Object_Property_Reference - BACnetDeviceObjectPropertyReference), Clause 21 BACnetObjectPropertyReference / BACnetSetpointReference productions | P1 | supported-with-clause-evidence | 0 |
| `BACNET-12-TIME-DELAY-NORMAL` | Clause 13.3.2 CHANGE_OF_STATE, Clause 13.3.4 COMMAND_FAILURE, Clause 13.3.6 OUT_OF_RANGE (pTimeDelayNormal definitions and condition letters); Clause 12.2 Table 12-2 (Analog Input, O5), 12.3 Table 12-3 (Analog Output, O4), 12.4 Table 12-4 (Analog Value, O6), 12.6 Table 12-6 (Binary Input, O7), 12.7 Table 12-8 (Binary Output, O6), 12.8 Table 12-10 (Binary Value, O8), 12.18 Table 12-21 (Multi-state Input, O5), 12.19 Table 12-22 (Multi-state Output, O3), 12.20 Table 12-23 (Multi-state Value, O6) | P1 | supported-with-clause-evidence | 0 |
| `BACNET-13-COV-SUBSCRIPTIONS` | Clauses 13.14-13.18 | P1 | implementation-present-needs-conformance-tests | 0 |
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
| `BACNET-13-EVENT-ENROLLMENT-EVALUATOR` | Clause 12.12 with Table 12-14 (Event Enrollment Object Type; Time_Delay_Normal Unsigned, conformance O, extract 16444-16446, property text 16887-16889); Clause 12.12 Table 12-15 (Time_Delay -> pTimeDelay mapping for CHANGE_OF_BITSTRING / CHANGE_OF_STATE / CHANGE_OF_VALUE / FLOATING_LIMIT / OUT_OF_RANGE, 16530-16679); Clause 13.2.2.1.4 transition actions incl. the same-state rule and the specific-state requirement (45051-45062); Clause 13.2.3 Acked_Transitions on a received transition (45140-45141); Clause 13.3 common introduction (condition ordering, no-condition -> no transition, 45610-45613); Clause 13.3.1 / 13.3.2 / 13.3.3 / 13.3.5 / 13.3.6 (per-algorithm pTimeDelay / pTimeDelayNormal direction rules and the fallback text, e.g. 45663-45667; CHANGE_OF_STATE conditions (a)-(c) 45749-45758; CHANGE_OF_VALUE baseline and Figure 13-10, 45797-45885; OUT_OF_RANGE conditions (a)-(h), 46133-46215) | P1 | supported-with-clause-evidence | 0 |

## Follow-Up Source

Rows not marked `supported-with-clause-evidence` are the initial follow-up backlog. Later PRs should split broad family rows into smaller clause-backed rows before strengthening public support claims.
