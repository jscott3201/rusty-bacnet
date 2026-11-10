---
section: Changed
---
- **Breaking (Rust API):** on a multi-state object built
  `with_reporting_persistence`, `set_number_of_states` gives way to a saved
  State_Text, and a write leaving over 1,024 labels or a label past 512
  octets is refused (#2414).
