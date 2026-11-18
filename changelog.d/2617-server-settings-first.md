---
section: Changed
---
- **Rust API:** A server's start checks its settings (Audit Reporters and
  their recipient, device bindings, segment timeout, budgets and policies)
  before the Device identity and the database, so a configuration with
  several faults may report a different one first (#2617).
