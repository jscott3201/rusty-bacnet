---
section: Fixed
---
- A node with client and server roles publishes a ConfirmedEventNotification that arrives in
  segments to its client's `event_notifications()` once per transmission, as one that arrives
  whole: the server hands the client the whole request, its queued copies held to 4 MiB (#2743).
