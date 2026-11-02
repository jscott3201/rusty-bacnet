---
section: Fixed
---
- **Wire:** Decoding an event notification now refuses an ACK_NOTIFICATION
  that carries ack-required, from-state or event values, which an
  acknowledgment leaves out; the client, Event Log records and the
  Notification Forwarder all read it the same way (#2247).
