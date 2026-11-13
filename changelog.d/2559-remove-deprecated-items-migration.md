---
section: Migration notes
---
- Replace `acknowledge_alarm(...)` with `acknowledge_alarm_request` and the original notification's timestamp plus your own acknowledgment time; replace `from_legacy_bool(true/false)` with `SUCCESSES_ONLY` / `ALL` (#2559).
