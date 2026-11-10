---
section: Added
---
- A Multi-state Input, Output or Value built `with_reporting_persistence`
  keeps a written State_Text, and with it its state count, until
  `clear_written_reporting`, so an Alarm_Values list naming a state a peer
  added comes back with the state (#2414). **Python API:** `storage_path`
  covers it.
