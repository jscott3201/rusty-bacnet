---
section: Removed
---
- **Breaking (Rust API):** The endpoint's narrow server settings are gone:
  `with_device_writes`, `with_reinitialize`, `with_reinit_password`,
  `SessionConfig::read_work_limit`, and the builders' `device_writes`,
  `reinitialize`, `reinit_password` and `read_work_limit` (#2617).
