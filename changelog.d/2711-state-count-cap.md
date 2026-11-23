---
section: Fixed
---
- **Breaking (Rust API):** a multi-state object's constructors and `set_number_of_states` refuse more than 1024 states, the cap a peer's write already met, so a huge count no longer aborts the process; the Python builders raise `BacnetError` for it (#2711).
