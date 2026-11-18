---
section: Added
---
- On an audited endpoint the client's WriteGroup, subscriptions, object and device management,
  alarm acknowledgment and life safety requests are not audited yet, and a raw request of an
  audited service that doesn't decode is refused before it is sent (#2617).
