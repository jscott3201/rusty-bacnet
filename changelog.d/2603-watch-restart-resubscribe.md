---
section: Changed
---
- A watched device's Time_Of_Device_Restart is read when it is first
  subscribed and, at most once a lifetime, when it announces itself or
  sends a restart notification; its points are subscribed again early
  when that time moved. Devices are spread over 10 s (#2603).
