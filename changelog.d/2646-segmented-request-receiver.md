---
section: Added
---
- **Rust API:** `bacnet_endpoint_core::segmented_request` reassembles segmented
  Confirmed-Requests on the answering side, without I/O, with the full server's limits and
  timers; the full client uses it (#2646).
