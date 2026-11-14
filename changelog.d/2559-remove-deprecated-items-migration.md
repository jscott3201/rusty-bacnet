---
section: Migration notes
---
- `acknowledge_alarm(mac, process, oid, state, source)` becomes `acknowledge_alarm(mac, &AcknowledgeAlarmRequest)` (Python: add `timestamp` and `time_of_acknowledgment`); the former `acknowledge_alarm_request*` names are renamed. `from_legacy_bool(true/false)` becomes `SUCCESSES_ONLY` / `ALL` (#2559).
