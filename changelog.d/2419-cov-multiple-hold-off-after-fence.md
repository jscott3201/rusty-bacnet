---
section: Fixed
---
- **Wire:** A failed ConfirmedCOVNotificationMultiple of a subscription
  that was replaced or cancelled meanwhile no longer holds off the
  subscription made since. That one's timestamped changes could go out up
  to one retry cycle late, 12 s by default (#2419).
