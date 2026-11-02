---
section: Fixed
---
- **Wire:** An event notification of type ACK_NOTIFICATION that carries
  ack-required, from-state or event values is now refused: a confirmed sender
  gets a Reject, and an unconfirmed one is dropped, not logged or forwarded
  (#2247).
