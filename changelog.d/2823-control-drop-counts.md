---
section: Added
---
- **Rust API:** `NetworkLayer::enable_network_control_receiver_counting_drops`
  gives the raw control receiver with a count of the controls its queue
  turned away, so a consumer waiting on an answer can tell one may be lost
  (#2823).
