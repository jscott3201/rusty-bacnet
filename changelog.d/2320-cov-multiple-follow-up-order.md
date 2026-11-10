---
section: Changed
---
- **Wire:** COV follow-ups (after an Ack, a fence or a
  Max_Notification_Delay deadline) go out in one fixed order, by monitored
  object: ordinary and Single notifications, COV-multiple reports, and the
  untimestamped items inside one. Each varied from run to run (#2320).
