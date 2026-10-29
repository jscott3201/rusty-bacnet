---
section: Changed
---
- The router's opt-in duplicate suppression also drops repeats of relayed
  I-Am-Router, Router-Busy and Router-Available messages and of a
  Who-Is-Router passed on for an unknown network, which otherwise circulate
  for good around a loop of routers (#1810, #2020).
