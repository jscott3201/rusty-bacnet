---
section: Added
---
- A Multi-state Input, Output or Value built `with_reporting_persistence`
  keeps a written State_Text (`AlgorithmWriteConfig::state_text`), and so
  its state count, until `clear_written_reporting`: a list naming a state a
  peer added comes back with it (#2414). **Python API:** `storage_path`
  covers it.
