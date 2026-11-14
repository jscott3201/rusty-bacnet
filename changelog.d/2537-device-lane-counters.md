---
section: Changed
---
- **Rust and Python API:** A Device recipient waiting for its I-Am holds one
  of the 256 lanes and up to 65 of their 1,024 entries, not a queue with a
  1,024 of its own; one dropped with 1,024 waiting counts in
  `lanes_entries_full`, not `device_recipient_unbound` (#2537).
