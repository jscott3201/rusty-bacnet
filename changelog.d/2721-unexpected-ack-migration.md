---
section: Migration notes
---
- **Acknowledgement kinds (Rust and Python, #2721):** a request answered
  with the wrong kind of acknowledgement for its service is now an error,
  `Error::UnexpectedAck { service, complex }` (Python: `BacnetError`), and
  whether the device carried it out is unknown: treat it as a write of
  unknown outcome, not as a success. For a device that acknowledges writes
  with a Complex-ACK, set `accept_complex_ack_for_simple_services`. A
  service the standard doesn't define still takes either kind.
