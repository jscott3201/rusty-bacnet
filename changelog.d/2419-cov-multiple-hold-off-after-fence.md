---
section: Fixed
---
- **Wire:** A failed ConfirmedCOVNotificationMultiple no longer holds off
  a subscription made after it: one that replaced or re-made the context
  while the report was out, or listed its references again inside the
  hold-off. Its timestamped changes could go out up to 12 s late (#2419).
