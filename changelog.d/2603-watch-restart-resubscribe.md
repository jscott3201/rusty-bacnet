---
section: Changed
---
- A watched device's Time_Of_Device_Restart is read when it is first
  subscribed and, at most once a lifetime, when it announces itself; its
  points are subscribed again early when that time moved. Restart
  notifications count too, and devices are spread over 10 s (#2603).
