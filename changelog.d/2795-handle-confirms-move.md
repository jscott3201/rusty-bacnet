---
section: Security
---
- A following device handle no longer moves on the discovery table alone,
  which I-Ams anyone can send fill: it asks the device at the new address
  (and before a request, its own) which it is, so forged announcements can't
  take its writes to another address (#2795).
