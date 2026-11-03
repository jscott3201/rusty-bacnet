---
section: Added
---
- Timers and Channels built `with_persistence` keep the configuration a peer
  writes across a restart, refusing a write they can't save; a saved part
  wins over its setter until `clear_written_config` drops it (#2243).
