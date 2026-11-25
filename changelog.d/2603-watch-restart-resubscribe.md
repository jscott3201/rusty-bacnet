---
section: Changed
---
- A watched device that announces itself is subscribed again early when
  its Time_Of_Device_Restart moved (one read a lifetime), or, lacking that
  property, at most once a lifetime; a restart notification is honoured
  too, and devices are spread over 10 s (#2603).
