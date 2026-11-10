---
section: Added
---
- An Access Door or Access Zone built `with_reporting_persistence` keeps
  the event algorithm parameters a peer writes (the two time delays and
  the alarm, fault and masked lists) until `clear_written_reporting`
  (#2331). **Python API:** `storage_path` covers them; a saved list goes
  over its keyword.
