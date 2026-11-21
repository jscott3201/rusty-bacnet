---
section: Migration notes
---
- **RouterPort (Rust API, #2684):** replace each
  `RouterPort { transport, network_number }` literal with
  `RouterPort::new(transport, network_number)`; the fields stay public.
