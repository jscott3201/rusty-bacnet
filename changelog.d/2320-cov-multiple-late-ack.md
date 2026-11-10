---
section: Changed
---
- **Wire:** a ConfirmedCOVNotificationMultiple acknowledged only after its
  report's deadline no longer retires the timestamped changes it carried:
  they go out again with the next report. A report sent at the
  Max_Notification_Delay deadline lists its items in one fixed order
  (#2320).
