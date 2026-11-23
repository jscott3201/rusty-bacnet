---
section: Added
---
- `accept_complex_ack_for_simple_services` (`ClientConfig`, every client
  builder, `SessionConfig`, Python `BACnetClient`; off by default) takes a
  Complex-ACK for a write-like service as success, for a device in the
  field that answers so (#2721).
