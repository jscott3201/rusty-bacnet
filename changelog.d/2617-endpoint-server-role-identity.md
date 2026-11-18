---
section: Changed
---
- **Breaking (Rust API):** A session with a server role needs a Device
  identity, and its start refuses the narrow server role's opt-ins:
  `with_device_writes`, `with_reinitialize`, `with_reinit_password`,
  `with_cov` and a non-default `read_work_limit`; `responder()` is always
  `None` (#2617).
