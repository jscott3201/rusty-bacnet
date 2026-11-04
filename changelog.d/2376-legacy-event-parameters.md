---
section: Fixed
---
- **Breaking (Rust API):** Every property value encoder refuses a legacy-form
  Event_Parameters value that wouldn't decode back as itself, so the stack no
  longer sends octets its own decoder misreads or refuses (#2376).
