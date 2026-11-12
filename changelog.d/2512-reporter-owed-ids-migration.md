---
section: Migration notes
---
- **Reporter invoke IDs (Rust API, #2512):** a host whose `InvokeIds::take`
  refuses because IDs it shares with other work are all out calls
  `Reporter::invoke_ids_released(free)` when some come back; `next_deadline`
  no longer comes due for those reports by itself. IDs the reporter gives back
  itself, and a host's own bound on them, need no call.
