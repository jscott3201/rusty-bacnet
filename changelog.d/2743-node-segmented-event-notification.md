---
section: Fixed
---
- A node with client and server roles publishes a ConfirmedEventNotification that arrives in
  segments to its client's `event_notifications()`, once, as it does one that arrives whole: the
  server, which reassembles and answers it, hands the client the whole notification (#2743).
