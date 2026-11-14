---
section: Migration notes
---
- Rust `acknowledge_alarm(mac, process, oid, state, source)` becomes `acknowledge_alarm(mac, &AcknowledgeAlarmRequest)` (the old `acknowledge_alarm_request` name); Python adds `timestamp` and `time_of_acknowledgment`. `from_legacy_bool(true/false)` becomes `SUCCESSES_ONLY` / `ALL`. bacnet-ops `hex(bytes)` becomes `octets::hex`, and the colon form `octets::hex_sep(bytes, ":")` (#2559).
